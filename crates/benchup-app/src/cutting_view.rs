//! In-app cutting plan preview. Marks, sheets and quantities come from CuttingPlan::view,
//! so the preview and the PDF draw the same layout.
use super::{currency, stock_settings};
use crate::cutting_layout::{self, Settings as LayoutSettings};
use crate::cutting_plan::{self, CutRow, CutView, Grain, metric_length, mark_color, piece_mark};
use crate::inspector_table::{Column, InspectorTable};
use crate::natural_text::TextUi as _;
use benchup_core::{Model, tr};
use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Shape, Stroke, Vec2};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        mpsc::{Receiver, TryRecvError},
    },
};

type Key = (u64, BTreeSet<u64>, LayoutSettings);
type CutResult = Result<CutView, String>;

#[derive(Default)]
pub(crate) struct Cache {
    current: Option<(Key, CutResult)>,
    pending: Option<(Key, Receiver<CutResult>)>,
}

impl Cache {
    /// Packing runs off the UI thread. The last finished layout stays on screen
    /// until a newer one arrives. Prices stay outside the cache, so catalog edits
    /// update without repacking.
    pub(crate) fn view(
        &mut self,
        ctx: &egui::Context,
        revision: u64,
        model: &Arc<Model>,
        selected: &BTreeSet<u64>,
        settings: LayoutSettings,
    ) -> Option<&CutResult> {
        let wanted = |key: &Key| key.0 == revision && key.1 == *selected && key.2 == settings;
        // A layout that finishes late for an older key must not replace one that already matches.
        let current_matches = self.current.as_ref().is_some_and(|(key, _)| wanted(key));
        if let Some((key, receiver)) = self.pending.take() {
            match receiver.try_recv() {
                Ok(result) => {
                    if !current_matches {
                        self.current = Some((key, result));
                    }
                }
                Err(TryRecvError::Empty) => self.pending = Some((key, receiver)),
                // The worker ended without a result. Show that for this key instead of retrying every frame.
                Err(TryRecvError::Disconnected) => {
                    if !current_matches {
                        self.current = Some((key, Err(tr!("cutting_view.layout_failed").into())));
                    }
                }
            }
        }
        // One layout runs at a time. A newer key waits until the running layout finishes.
        if self.pending.is_none() && !self.current.as_ref().is_some_and(|(key, _)| wanted(key)) {
            self.start(ctx, model, (revision, selected.clone(), settings));
        }
        self.current.as_ref().map(|(_, result)| result)
    }

    fn start(&mut self, ctx: &egui::Context, model: &Arc<Model>, key: Key) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (sender, receiver) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            let model = model.clone();
            let (selected, settings) = (key.1.clone(), key.2);
            benchup_core::i18n::spawn_localized(move || {
                let _ = sender.send(compute(&model, &selected, settings));
                ctx.request_repaint();
            });
            self.pending = Some((key, receiver));
        }
        // Browsers have no worker threads here, so the layout is computed inline and shows this frame.
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            let result = compute(model, &key.1, key.2);
            self.current = Some((key, result));
        }
    }
}

fn compute(model: &Model, selected: &BTreeSet<u64>, settings: LayoutSettings) -> CutResult {
    settings
        .validate()
        .and_then(|()| cutting_plan::CuttingPlan::collect_with_layout(model, selected, settings))
        .map(|plan| plan.view(settings))
}

pub(crate) fn show(
    ui: &mut egui::Ui,
    view: Option<&CutResult>,
    catalog: &stock_settings::Settings,
    kerf_mm: f64,
    used_cost: bool,
) {
    let view = match view {
        Some(Ok(view)) => view,
        Some(Err(message)) => {
            ui.text_label(message.as_str());
            return;
        }
        None => {
            ui.text_weak(tr!("cutting_view.calculating"));
            return;
        }
    };
    if view.skipped > 0 {
        ui.text_weak(tr!("cutting_plan.omitted_items", count = view.skipped));
    }
    if view.joinery_omitted {
        ui.text_weak(tr!("cutting_plan.joinery_omitted"));
    }
    let kerf = metric_length(cutting_layout::tenths(kerf_mm));
    for (index, section) in view.sections.iter().enumerate() {
        ui.add_space(8.);
        let [t, w, l] = section.material.dimensions.map(metric_length);
        ui.text_heading(tr!(
            "cutting_plan.material_x_x",
            section.material.name.as_str(),
            t.as_str(),
            w.as_str(),
            l.as_str()
        ));
        let sheets = section.sheets();
        let pieces: usize = section.rows.iter().map(|row| row.quantity).sum();
        ui.text_label(tr!(
            "cutting_plan.stock_required",
            count = sheets,
            kerf = kerf.as_str()
        ));
        ui.text_label(tr!(
            "cutting_plan.net_cut_area_m2",
            pieces = tr!("common.pieces", count = pieces),
            area = section.area
        ));
        let mut table = InspectorTable::new(
            ui.available_width(),
            [
                Column::fixed(64.),
                Column::flex(1.),
                Column::fixed(176.),
                Column::fixed(80.),
            ],
        )
        .striped();
        table.header(
            ui,
            [
                tr!("cutting_view.mark"),
                tr!("cutting_view.parts"),
                tr!("cutting_view.size"),
                tr!("time_estimates.quantity"),
            ],
        );
        for row in &section.rows {
            table.row(ui, (index, row.mark), |column, cell| match column {
                0 => {
                    cell.horizontal(|ui| {
                        swatch(ui, row.mark);
                        ui.text_label(piece_mark(row.mark));
                    });
                }
                1 => {
                    cell.text_label(row.names.join(", "));
                }
                2 => {
                    cell.text_label(size_text(row));
                    if row.grain == Grain::ThroughThickness {
                        cell.text_weak(tr!("cutting_plan.grain_through_thickness"));
                    }
                    if !row.rectangular {
                        cell.text_weak(tr!("cutting_plan.check_shape_bounding_blank_only"));
                    }
                }
                _ => {
                    cell.text_label(row.quantity.to_string());
                }
            });
        }
        for (number, stock) in section.layout.stock.iter().enumerate() {
            ui.text_label(tr!(
                "cutting_plan.stock_page_heading",
                tr!("cutting_plan.stock"),
                number + 1,
                sheets
            ));
            let (grained, joined) = cutting_plan::sheet_legend(stock, |mark| {
                row_for(&section.rows, mark).map(|row| (row.grain, &row.joinery))
            });
            if grained {
                ui.text_weak(tr!("cutting_plan.grain_along_stock_length"));
            }
            if joined {
                ui.text_weak(tr!("cutting_plan.joinery_dashed_outlines"));
            }
            draw_sheet(ui, stock, &section.rows);
        }
        if !section.layout.oversized.is_empty() {
            ui.text_heading(tr!("cutting_plan.does_not_fit_selected_stock"));
            for piece in &section.layout.oversized {
                ui.text_label(tr!(
                    "cutting_plan.oversized_cut",
                    count = piece.quantity,
                    mark = piece_mark(piece.mark),
                    width = metric_length(piece.width),
                    length = metric_length(piece.length)
                ));
            }
        }
    }
    show_costs(ui, view, catalog, used_cost);
}

fn row_for(rows: &[CutRow], mark: usize) -> Option<&CutRow> {
    rows.iter().find(|row| row.mark == mark)
}

fn size_text(row: &CutRow) -> String {
    let [t, w, l] = row.dimensions.map(metric_length);
    tr!("cutting_view.dimensions", t, w, l)
}

fn cost_table<const N: usize>(
    ui: &mut egui::Ui,
    columns: [Column; N],
    headers: &[String],
    rows: &[Vec<String>],
) {
    let mut table = InspectorTable::new(ui.available_width(), columns).striped();
    table.header(ui, std::array::from_fn(|column| headers[column].as_str()));
    for (index, row) in rows.iter().enumerate() {
        table.row(ui, index, |column, cell| {
            cell.text_label(row[column].as_str());
        });
    }
}

struct CostRow {
    material: String,
    sheets: usize,
    price: Option<f64>,
    whole: Option<f64>,
    used: Option<f64>,
}

#[derive(Default)]
struct CostSummary {
    rows: Vec<CostRow>,
    whole_total: f64,
    used_total: f64,
    unpriced: usize,
}

/// Unpriced sheets add nothing to the totals and are counted for the note.
fn cost_summary(view: &CutView, catalog: &stock_settings::Settings) -> CostSummary {
    let mut summary = CostSummary::default();
    for section in &view.sections {
        let sheets = section.sheets();
        let price = catalog.wood_price(section.material_id, &section.material);
        let whole = price.map(|price| sheets as f64 * price);
        // Each sheet is priced by the fraction of its area that the pieces cut from it use.
        // Stock with no area holds no sheets, so nothing is used.
        let sheet_area = section.sheet_area();
        let used = price.map(|price| {
            if sheet_area > 0. {
                price * section.used_area() / sheet_area
            } else {
                0.
            }
        });
        summary.whole_total += whole.unwrap_or(0.);
        summary.used_total += used.unwrap_or(0.);
        if price.is_none() {
            summary.unpriced += sheets;
        }
        summary.rows.push(CostRow {
            material: section.material.name.clone(),
            sheets,
            price,
            whole,
            used,
        });
    }
    summary
}

fn show_costs(ui: &mut egui::Ui, view: &CutView, catalog: &stock_settings::Settings, used_cost: bool) {
    let name = catalog.currency.as_str();
    let code = currency::code(name);
    ui.add_space(12.);
    ui.text_heading(tr!("cutting_view.costs"));
    let amount = |value: Option<f64>| {
        value.map_or_else(
            || tr!("time_estimates.unavailable").to_owned(),
            |value| currency::format_value(name, value),
        )
    };
    let summary = cost_summary(view, catalog);
    let mut headers = vec![
        tr!("cutting_view.material").to_owned(),
        tr!("cutting_view.sheets").to_owned(),
        tr!("time_estimates.unit_price", currency = code).to_owned(),
        tr!("cutting_view.whole_sheets_amount", currency = code).to_owned(),
    ];
    let mut rows: Vec<Vec<String>> = summary
        .rows
        .iter()
        .map(|row| {
            let mut cells = vec![
                row.material.clone(),
                row.sheets.to_string(),
                amount(row.price),
                amount(row.whole),
            ];
            if used_cost {
                cells.push(amount(row.used));
            }
            cells
        })
        .collect();
    let mut total = vec![
        tr!("cutting_view.total").to_owned(),
        String::new(),
        String::new(),
        currency::format_value(name, summary.whole_total),
    ];
    if used_cost {
        headers.push(tr!("cutting_view.used_area_amount", currency = code).to_owned());
        total.push(currency::format_value(name, summary.used_total));
    }
    rows.push(total);
    // InspectorTable spaces every column, so the hidden used-area column is left out rather than zero-width.
    if used_cost {
        cost_table(
            ui,
            [
                Column::flex(1.),
                Column::fixed(80.),
                Column::fixed(128.),
                Column::fixed(176.),
                Column::fixed(176.),
            ],
            &headers,
            &rows,
        );
        ui.text_weak(tr!("cutting_view.used_area_note"));
    } else {
        cost_table(
            ui,
            [
                Column::flex(1.),
                Column::fixed(80.),
                Column::fixed(128.),
                Column::fixed(176.),
            ],
            &headers,
            &rows,
        );
    }
    if summary.unpriced > 0 {
        ui.text_weak(tr!("time_estimates.stock_pieces_without_price", count = summary.unpriced));
    }
    if view.sections.iter().any(|section| section.excluded() > 0) {
        ui.text_weak(tr!("cutting_plan.stock_count_covers_fitting_cuts_only_oversized_cuts"));
    }
}

fn color(mark: usize) -> Color32 {
    let [r, g, b] = mark_color(mark);
    Color32::from_rgb((r * 255.) as u8, (g * 255.) as u8, (b * 255.) as u8)
}

fn swatch(ui: &mut egui::Ui, mark: usize) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(10.), Sense::hover());
    ui.painter().rect_filled(rect, 2., color(mark));
}

fn draw_sheet(ui: &mut egui::Ui, stock: &cutting_layout::Stock, rows: &[CutRow]) {
    // Fill the dialog width, capped in height so narrow stock stays on screen.
    let scale = (ui.available_width() / stock.length as f32).min(480. / stock.width as f32);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(stock.length as f32 * scale, stock.width as f32 * scale),
        Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0., Color32::from_gray(235));
    for placement in &stock.placements {
        let r = placement.rect;
        let piece = Rect::from_min_size(
            rect.min + Vec2::new(r.x as f32 * scale, r.y as f32 * scale),
            Vec2::new(r.length as f32 * scale, r.width as f32 * scale),
        );
        painter.rect_filled(piece, 0., color(placement.mark));
        let Some(row) = row_for(rows, placement.mark) else {
            continue;
        };
        if let Some(lines) = row.joinery.get(&placement.part_index) {
            let position = |joint: [u64; 2]| {
                let [u, v] = cutting_plan::joinery_offset(row.dimensions, placement.rotated, joint);
                piece.min + Vec2::new(u as f32 * scale, v as f32 * scale)
            };
            for line in lines {
                let points = [position(line[0]), position(line[1])];
                painter.extend(Shape::dashed_line(
                    &points,
                    Stroke::new(1., Color32::from_gray(60)),
                    4.,
                    3.,
                ));
            }
        }
        if piece.width() >= 22. && piece.height() >= 12. {
            painter.text(
                piece.center(),
                Align2::CENTER_CENTER,
                piece_mark(placement.mark),
                FontId::proportional(11.),
                Color32::from_gray(20),
            );
        }
    }
}

#[cfg(test)]
#[path = "cutting_view_tests.rs"]
mod tests;
