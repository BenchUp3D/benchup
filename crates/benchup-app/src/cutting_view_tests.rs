use super::*;
use crate::cutting_layout::Piece;
use crate::cutting_plan::MaterialSection;
use std::time::Duration;

/// A 100 x 1000 mm board, so one piece of 1000 x 50 mm uses half of each sheet.
fn board_catalog(price: f64) -> stock_settings::Settings {
    let stock = stock_settings::Stock {
        thickness_mm: 10.,
        width_mm: 100.,
        length_mm: 1000.,
        price,
        ..Default::default()
    };
    stock_settings::Settings {
        wood: vec![stock],
        ..Default::default()
    }
}

fn section(catalog: &stock_settings::Settings, pieces: &[Piece]) -> MaterialSection {
    let stock = &catalog.wood[0];
    let layout = cutting_layout::sheets(
        pieces,
        LayoutSettings {
            sheet_width_mm: stock.width_mm,
            sheet_length_mm: stock.length_mm,
            kerf_mm: 0.,
            allow_rotation: false,
            ..LayoutSettings::default()
        },
    );
    MaterialSection {
        material_id: Some(stock.id),
        material: stock.definition(),
        rows: Vec::new(),
        layout,
        area: 0.,
    }
}

fn piece(length: u64, width: u64, quantity: usize) -> Piece {
    Piece {
        mark: 0,
        length,
        width,
        quantity,
        grain_along_length: false,
    }
}

#[test]
fn cost_totals_price_whole_sheets_and_the_area_cut_from_them() {
    let catalog = board_catalog(10.);
    let view = CutView {
        sections: vec![section(&catalog, &[piece(10000, 500, 1)])],
        skipped: 0,
        joinery_omitted: false,
    };
    let summary = cost_summary(&view, &catalog);
    assert_eq!(summary.rows.len(), 1);
    assert_eq!(summary.rows[0].sheets, 1);
    assert_eq!(summary.rows[0].whole, Some(10.));
    assert!((summary.rows[0].used.unwrap() - 5.).abs() < 1e-9);
    assert!((summary.whole_total - 10.).abs() < 1e-9);
    assert!((summary.used_total - 5.).abs() < 1e-9);
    assert_eq!(summary.unpriced, 0);
}

#[test]
fn zero_price_is_a_price_of_zero_as_in_the_time_estimates() {
    let catalog = board_catalog(0.);
    let view = CutView {
        sections: vec![section(&catalog, &[piece(10000, 500, 2)])],
        skipped: 0,
        joinery_omitted: false,
    };
    let summary = cost_summary(&view, &catalog);
    assert_eq!(summary.rows[0].price, Some(0.));
    assert_eq!(summary.rows[0].whole, Some(0.));
    assert_eq!(summary.rows[0].used, Some(0.));
    assert_eq!(summary.unpriced, 0);
    assert_eq!(summary.whole_total, 0.);
    assert_eq!(summary.used_total, 0.);
}

#[test]
fn stock_without_area_costs_nothing_instead_of_nan() {
    let mut catalog = board_catalog(10.);
    catalog.wood[0].width_mm = 0.;
    let view = CutView {
        sections: vec![section(&catalog, &[piece(10000, 500, 1)])],
        skipped: 0,
        joinery_omitted: false,
    };
    let summary = cost_summary(&view, &catalog);
    assert_eq!(summary.rows[0].sheets, 0);
    assert_eq!(summary.rows[0].whole, Some(0.));
    assert_eq!(summary.rows[0].used, Some(0.));
    assert_eq!(summary.whole_total, 0.);
    assert_eq!(summary.used_total, 0.);
}

#[test]
fn sheets_of_a_material_missing_from_the_catalog_are_unpriced() {
    let catalog = board_catalog(10.);
    let mut missing = section(&catalog, &[piece(10000, 500, 1)]);
    missing.material_id = None;
    let view = CutView {
        sections: vec![missing],
        skipped: 0,
        joinery_omitted: false,
    };
    let summary = cost_summary(&view, &catalog);
    assert_eq!(summary.rows[0].whole, None);
    assert_eq!(summary.unpriced, 1);
}

fn settle_for(
    cache: &mut Cache,
    ctx: &egui::Context,
    revision: u64,
    model: &Arc<Model>,
    selected: &BTreeSet<u64>,
    settings: LayoutSettings,
) {
    for _ in 0..1000 {
        cache.view(ctx, revision, model, selected, settings);
        let finished = cache.current.as_ref().is_some_and(|(key, _)| {
            key.0 == revision && key.1 == *selected && key.2 == settings
        });
        if cache.pending.is_none() && finished {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the cutting layout did not finish");
}

fn settle(cache: &mut Cache, ctx: &egui::Context, revision: u64, model: &Arc<Model>) {
    settle_for(cache, ctx, revision, model, &BTreeSet::new(), LayoutSettings::default());
}

#[test]
fn cache_repacks_only_when_the_document_revision_or_scope_changes() {
    let ctx = egui::Context::default();
    let model = Arc::new(Model::default());
    let mut cache = Cache::default();
    settle(&mut cache, &ctx, 1, &model);
    // The same revision and scope reuse the finished layout without a new worker.
    cache.view(&ctx, 1, &model, &BTreeSet::new(), LayoutSettings::default());
    assert!(cache.pending.is_none());
    // A document edit starts a new layout while the previous one stays visible.
    assert!(cache.view(&ctx, 2, &model, &BTreeSet::new(), LayoutSettings::default()).is_some());
    assert!(cache.pending.is_some());
    settle(&mut cache, &ctx, 2, &model);
}

#[test]
fn layout_settings_and_selection_are_part_of_the_cache_key() {
    let ctx = egui::Context::default();
    let model = Arc::new(Model::default());
    let mut cache = Cache::default();
    settle(&mut cache, &ctx, 1, &model);
    let kerf = LayoutSettings {
        kerf_mm: 4.,
        ..LayoutSettings::default()
    };
    cache.view(&ctx, 1, &model, &BTreeSet::new(), kerf);
    assert!(cache.pending.as_ref().is_some_and(|(key, _)| key.2 == kerf));
    settle_for(&mut cache, &ctx, 1, &model, &BTreeSet::new(), kerf);
    let selected = BTreeSet::from([7_u64]);
    cache.view(&ctx, 1, &model, &selected, LayoutSettings::default());
    assert!(cache.pending.as_ref().is_some_and(|(key, _)| key.1 == selected));
}

#[test]
fn a_late_layout_for_an_older_key_does_not_replace_the_matching_one() {
    let ctx = egui::Context::default();
    let model = Arc::new(Model::default());
    let mut cache = Cache::default();
    let default = LayoutSettings::default();
    settle(&mut cache, &ctx, 1, &model);
    let kerf = LayoutSettings {
        kerf_mm: 4.,
        ..default
    };
    // Change to kerf 4, then back to the settled layout before the kerf job finishes.
    cache.view(&ctx, 1, &model, &BTreeSet::new(), kerf);
    for _ in 0..1000 {
        cache.view(&ctx, 1, &model, &BTreeSet::new(), default);
        if cache.pending.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(cache.pending.is_none());
    assert!(cache.current.as_ref().is_some_and(|(key, _)| key.2 == default));
}
