//! Stock, finishing, and whole-project joinery estimates.
use benchup_core::tr;
use super::*;
use crate::inspector_table::{Column, InspectorTable, product_cell_text};
use benchup_core::{Model, surface_treatment::Task};
use benchup_core::{connections::ConnectionOperation, joinery::JointKind};
use std::collections::{BTreeMap, BTreeSet};

// Research checked 2026-10-29. These are planning assumptions, not universal benchmarks.
// Sanding: Teri Masaschi, 8–10 seconds/foot of travel:
// https://www.finewoodworking.com/project-guides/finishing/are-you-sanding-right
// Assume a 5-inch pad, 50% overlap and two traversals per grit:
// 2 / ((0.3048 / 9) * (0.127 / 2)) / 60 = 15.5 min/m², rounded to 16.
// Resene's stain-only allowance is 0.08 hours/m² = 4.8 min/m² (rounded to 5).
// Its three-coat polyurethane floor system is 0.29 hours/m² = 5.8 min/m²/coat
// (rounded to 6). This is a flat-surface proxy, not a furniture time study.
// Conditioner uses the stain apply/wipe rate as an explicit proxy.
// https://www.resene.com.au/paintdec/productivity-tables/index.htm
// Oil-based Minwax conditioner: 5–15 min penetration, stain within 2 hours.
// https://www.minwax.com/en/products/preparation/pre-stain-wood-conditioner
// PPG/Olympic penetrating oil-based stain: 8 hours minimum before topcoat.
// Faster formulations differ, so the user's label takes precedence.
// https://buyat.ppg.com/rep_pafpainttools_files/Olympic/TDB/626501.pdf
// Oil-based polyurethane: 4–6 hours between coats, 24 hours after the last
// coat before light use. Light use does not mean fully cured.
// https://www.minwax.com/en/products/protective-finishes/fast-drying-polyurethane
type WoodMaterials =
    std::collections::BTreeMap<benchup_core::MaterialId, benchup_core::StockMaterial>;

pub(super) use benchup_core::surface_treatment::{Duration, Settings, Step};

pub(super) const JOINERY_SECTION_KEY: &str = "time_estimates_joinery";
const JOINERY_SCOPE: &str =
    tr!("time_estimates.counts_all_configured_wood_joints_regardless_of_selection");
const JOINERY_ALLOWANCE_NOTE: &str = tr!("time_estimates.allowances_cover_layout_cutting_and_fitting_both_parts");

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct JoineryRates(BTreeMap<JointKind, f64>);

impl JoineryRates {
    // Planning allowances for a small shop using power tools and jigs. Simple
    // profiles take less time than joints with multiple cuts and more fitting.
    fn default_minutes(kind: JointKind) -> f64 {
        match kind {
            JointKind::MortiseTenon => 15.,
            JointKind::HalfLap => 8.,
            JointKind::TongueGroove => 8.,
            JointKind::Rabbet => 5.,
            JointKind::Dovetail => 25.,
            JointKind::Finger => 15.,
        }
    }

    pub(super) fn minutes(&self, kind: JointKind) -> f64 {
        self.0
            .get(&kind)
            .copied()
            .unwrap_or_else(|| Self::default_minutes(kind))
    }

    pub(super) fn set_minutes(&mut self, kind: JointKind, minutes: f64) {
        if minutes.is_finite() && minutes >= 0. {
            self.0.insert(kind, minutes);
        }
    }

    pub(super) fn sanitize(&mut self) {
        self.0
            .retain(|_, minutes| minutes.is_finite() && *minutes >= 0.);
    }
}

struct JoineryRow {
    kind: JointKind,
    count: usize,
    minutes_per_joint: f64,
}

impl JoineryRow {
    fn work_minutes(&self) -> f64 {
        self.count as f64 * self.minutes_per_joint
    }
}

struct TimeBreakdown {
    finishing: Vec<(Task, Duration, usize)>,
    joinery: [JoineryRow; JointKind::ALL.len()],
    joinery_work: f64,
    duration: Duration,
}

impl TimeBreakdown {
    fn has_joints(&self) -> bool {
        self.joinery.iter().any(|row| row.count > 0)
    }
}

// Model collections allow direct edits, so compare a source snapshot rather than
// relying on edit notifications. Scrolling only compares inputs; it never repeats
// face triangulation, solid measurement, or stock packing. Prices and units stay
// outside this cache so changing them updates the UI immediately.
#[derive(Default)]
pub(super) struct EstimateCache {
    source: Option<(Model, BTreeSet<u64>)>,
    area: Option<(WoodMaterials, std::collections::BTreeMap<u64, f64>)>,
    materials: Option<(
        crate::cutting_layout::Settings,
        Result<MaterialEstimate, String>,
    )>,
    #[cfg(test)]
    material_recomputations: usize,
}

struct MaterialEstimate {
    requirements: Vec<crate::cutting_plan::MaterialRequirement>,
    skipped: usize,
}

impl EstimateCache {
    fn validate(&mut self, model: &Model, selected: &BTreeSet<u64>) {
        if self
            .source
            .as_ref()
            .is_none_or(|(source, selection)| source != model || selection != selected)
        {
            self.source = Some((model.clone(), selected.clone()));
            self.area = None;
            self.materials = None;
        }
    }

    fn area(
        &mut self,
        model: &Model,
        selected: &BTreeSet<u64>,
        catalog: &stock_settings::Settings,
    ) -> f64 {
        self.validate(model, selected);
        let wood: WoodMaterials = catalog
            .wood
            .iter()
            .map(|stock| (stock.id, stock.definition()))
            .collect();
        if self
            .area
            .as_ref()
            .is_none_or(|(previous, _)| *previous != wood)
        {
            let area = treatment_areas(model, selected, &wood);
            self.area = Some((wood, area));
        }
        self.area.as_ref().unwrap().1.values().sum()
    }

    fn materials(
        &mut self,
        model: &Model,
        selected: &BTreeSet<u64>,
        layout: crate::cutting_layout::Settings,
    ) -> &Result<MaterialEstimate, String> {
        self.validate(model, selected);
        if self
            .materials
            .as_ref()
            .is_none_or(|(previous, _)| *previous != layout)
        {
            #[cfg(test)]
            {
                self.material_recomputations += 1;
            }
            self.materials = Some((
                layout,
                crate::cutting_plan::CuttingPlan::collect_assigned(model, selected).map(|plan| {
                    MaterialEstimate {
                        requirements: plan.material_requirements(layout),
                        skipped: plan.skipped,
                    }
                }),
            ));
        }
        &self.materials.as_ref().unwrap().1
    }
}

fn object_has_wood_material(
    model: &Model,
    mut id: u64,
    parents: &std::collections::BTreeMap<u64, u64>,
    wood: &WoodMaterials,
) -> bool {
    loop {
        let object = &model.objects[&id];
        if let Some(material) = model.object_stock_material(object) {
            return if let Some(id) = object.stock_material_id {
                wood.contains_key(&id)
            } else {
                wood.values().any(|stock| stock == material)
            };
        }
        let Some(parent) = parents.get(&id) else {
            return false;
        };
        id = *parent;
    }
}

// Attribute each face once to its nearest object. Material inheritance still follows
// the nearest assigned ancestor, so a child's material overrides its parent's.
fn treatment_areas(
    model: &Model,
    selected: &BTreeSet<u64>,
    wood: &WoodMaterials,
) -> std::collections::BTreeMap<u64, f64> {
    let expanded = model.expand_entities(selected);
    let parents: std::collections::BTreeMap<_, _> = model
        .objects
        .values()
        .flat_map(|object| object.entities.iter().map(move |id| (*id, object.id)))
        .collect();
    let mut areas = std::collections::BTreeMap::new();
    for face in model.faces.values().filter(|face| {
        !model.is_hidden(face.id) && (selected.is_empty() || expanded.contains(&face.id))
    }) {
        let Some(owner) = parents.get(&face.id).copied() else {
            continue;
        };
        let area = model.face_area(face);
        if object_has_wood_material(model, owner, &parents, wood) && area.is_finite() && area > 0. {
            *areas.entry(owner).or_default() += area;
        }
    }
    areas
}

// Apply only edited fields when a selection contains different configurations.
fn apply_treatment_changes(target: &mut Settings, before: &Settings, after: &Settings) {
    fn step(target: &mut Step, before: &Step, after: &Step) {
        macro_rules! apply { ($($field:ident),*) => { $(
            if before.$field != after.$field { target.$field = after.$field; }
        )* }; }
        apply!(
            enabled,
            product_id,
            minutes_per_m2,
            coats,
            drying_hours,
            final_drying_hours
        );
    }
    target.sanitize();
    // Match by grit rather than position: mixed selections may have different
    // custom lists. Removing a row must not shift edits onto another grit.
    for i in (0..target.sanding_grits.len()).rev() {
        let grit = target.sanding_grits[i];
        if before.sanding_grits.contains(&grit) && !after.sanding_grits.contains(&grit) {
            target.sanding_grits.remove(i);
            target.sanding.remove(i);
            target.sanding_after_varnish.remove(i);
        }
    }
    for (i, grit) in after.sanding_grits.iter().enumerate() {
        let previous = before.sanding_grits.iter().position(|value| value == grit);
        let edited = previous.is_none_or(|j| {
            before.sanding[j] != after.sanding[i]
                || before.sanding_after_varnish[j] != after.sanding_after_varnish[i]
        });
        if !edited {
            continue;
        }
        if let Some(j) = target.sanding_grits.iter().position(|value| value == grit) {
            if let Some(k) = previous {
                step(
                    &mut target.sanding[j],
                    &before.sanding[k],
                    &after.sanding[i],
                );
                if before.sanding_after_varnish[k] != after.sanding_after_varnish[i] {
                    target.sanding_after_varnish[j] = after.sanding_after_varnish[i];
                }
            } else {
                target.sanding[j] = after.sanding[i].clone();
                target.sanding_after_varnish[j] = after.sanding_after_varnish[i];
            }
        } else {
            target.sanding_grits.push(*grit);
            target.sanding.push(after.sanding[i].clone());
            target
                .sanding_after_varnish
                .push(after.sanding_after_varnish[i]);
        }
    }
    step(&mut target.pre_stain, &before.pre_stain, &after.pre_stain);
    step(&mut target.stain, &before.stain, &after.stain);
    step(&mut target.varnish, &before.varnish, &after.varnish);
    target.sanitize();
}

fn object_tasks(
    model: &Model,
    areas: &std::collections::BTreeMap<u64, f64>,
    defaults: &Settings,
) -> Vec<(Task, Duration, usize)> {
    let mut groups = BTreeMap::<Task, (Duration, usize)>::new();
    for (id, area) in areas {
        let mut settings = model.objects[id]
            .surface_treatment_override()
            .unwrap_or(defaults)
            .clone();
        settings.sanitize();
        for (task, duration) in settings.tasks(*area) {
            let (total, count) = groups.entry(task).or_default();
            total.work += duration.work;
            // Objects dry together after each coat; wait for the slowest one.
            total.wait = total.wait.max(duration.wait);
            *count += 1;
        }
    }
    groups
        .into_iter()
        .map(|(task, (duration, count))| (task, duration, count))
        .collect()
}

fn time_breakdown(
    model: &Model,
    areas: &BTreeMap<u64, f64>,
    settings: &Settings,
    rates: &JoineryRates,
) -> TimeBreakdown {
    let mut counts = BTreeMap::new();
    // The persisted relation is the unit of work, even if it has multiple cuts
    // or shares a member with other joints.
    for connection in model.connections.values() {
        if let ConnectionOperation::Joinery(settings) = &connection.operation {
            *counts.entry(settings.kind).or_insert(0) += 1;
        }
    }
    let joinery = JointKind::ALL.map(|kind| JoineryRow {
        kind,
        count: counts.get(&kind).copied().unwrap_or_default(),
        minutes_per_joint: rates.minutes(kind),
    });
    let joinery_work = joinery.iter().map(JoineryRow::work_minutes).sum();
    let finishing = object_tasks(model, areas, settings);
    let duration = finishing.iter().fold(
        Duration {
            work: joinery_work,
            wait: 0.,
        },
        |total, (_, task, _)| Duration {
            work: total.work + task.work,
            wait: total.wait + task.wait,
        },
    );
    TimeBreakdown {
        finishing,
        joinery,
        joinery_work,
        duration,
    }
}

#[derive(Debug)]
struct FinishCost {
    product_id: Option<benchup_core::MaterialId>,
    name: String,
    kind: Option<finish_settings::FinishType>,
    containers: Option<f64>,
    price: Option<f64>,
    amount: Option<f64>,
    issue: Option<&'static str>,
}

fn finishing_costs(
    model: &Model,
    areas: &std::collections::BTreeMap<u64, f64>,
    defaults: &Settings,
    products: &[finish_settings::Finish],
) -> Vec<FinishCost> {
    let mut demand = std::collections::BTreeMap::<Option<benchup_core::MaterialId>, f64>::new();
    for (id, area) in areas {
        if !area.is_finite() || *area <= 0. {
            continue;
        }
        let mut settings = model.objects[id]
            .surface_treatment_override()
            .unwrap_or(defaults)
            .clone();
        settings.sanitize();
        for step in [&settings.pre_stain, &settings.stain, &settings.varnish] {
            if step.enabled {
                *demand.entry(step.product_id).or_default() += area * f64::from(step.coats);
            }
        }
    }
    demand
        .into_iter()
        .map(|(id, area)| {
            let product = id.and_then(|id| products.iter().find(|product| product.id == id));
            let Some(product) = product else {
                return FinishCost {
                    product_id: id,
                    name: if id.is_some() {
                        tr!("time_estimates.unavailable_finishing_product")
                    } else {
                        tr!("time_estimates.unassigned_finishing_product")
                    }
                    .into(),
                    kind: None,
                    containers: None,
                    price: None,
                    amount: None,
                    issue: Some(tr!("time_estimates.choose_a_product_for_each_enabled_finishing_step")),
                };
            };
            // Coverage is specified for the exact configured container, regardless of
            // its volume or mass unit. Never convert mass to volume or assume density.
            let containers = product
                .coverage_m2_per_container
                .filter(|coverage| coverage.is_finite() && *coverage > 0.)
                .map(|coverage| (area / coverage).ceil())
                .filter(|quantity| quantity.is_finite() && *quantity > 0.);
            let price = (product.price.is_finite() && product.price > 0.).then_some(product.price);
            let amount = containers
                .zip(price)
                .map(|(quantity, price)| quantity * price)
                .filter(|amount| amount.is_finite());
            FinishCost {
                product_id: id,
                name: product.name.clone(),
                kind: Some(product.finish_type),
                containers,
                price,
                amount,
                issue: if containers.is_none() {
                    Some(tr!("time_estimates.coverage_missing_set_m2_per_container_per_coat"))
                } else if price.is_none() {
                    Some(tr!("time_estimates.price_missing_set_the_price_per_container_in"))
                } else if amount.is_none() {
                    Some(tr!("time_estimates.cost_exceeds_estimate_range"))
                } else {
                    None
                },
            }
        })
        .collect()
}

// Monetary values always occupy one line; egui exposes the full value on hover
// when it needs to elide the text to fit a narrow inspector column.
fn price_text(ui: &mut egui::Ui, text: RichText) {
    ui.add(crate::natural_text::Label::new(text).truncate());
}

fn cost_group(ui: &mut egui::Ui, label: &str) {
    let colors = Palette::for_ui(ui);
    ui.separator();
    ui.add(crate::natural_text::Label::new(RichText::new(label).small().strong().color(colors.muted)).truncate());
}

fn finishing_cost_rows(
    ui: &mut egui::Ui,
    table: &mut InspectorTable<4>,
    costs: &[FinishCost],
    currency: &str,
) {
    let colors = Palette::for_ui(ui);
    for kind in finish_settings::FinishType::ALL
        .into_iter()
        .map(Some)
        .chain([None])
    {
        if !costs.iter().any(|cost| cost.kind == kind) {
            continue;
        }
        cost_group(
            ui,
            kind.map_or(tr!("time_estimates.unresolved_finishing_products"), |kind| kind.label()),
        );
        for (index, cost) in costs
            .iter()
            .enumerate()
            .filter(|(_, cost)| cost.kind == kind)
        {
            table.row(ui, ("finish_cost", index), |column, ui| {
                let text = match column {
                    0 => cost.name.clone(),
                    1 => cost
                        .containers
                        .map(|value| tr!("time_estimates.container_count", value = value))
                        .unwrap_or_else(|| "—".into()),
                    2 => cost
                        .price
                        .map(|value| currency::format_value(currency, value))
                        .unwrap_or_else(|| "—".into()),
                    _ => cost
                        .amount
                        .map(|value| currency::format_value(currency, value))
                        .unwrap_or_else(|| "—".into()),
                };
                if column >= 2 {
                    price_text(ui, RichText::new(text).strong());
                    return;
                }
                let mut lines = vec![RichText::new(text).strong()];
                if column == 0 {
                    if let Some(issue) = cost.issue {
                        lines.push(RichText::new(issue).small().color(colors.accent));
                    }
                }
                product_cell_text(ui, lines);
            });
        }
    }
}

fn format_minutes(minutes: f64) -> String {
    // Keep fractional-minute work visible without inflating the total of zero steps.
    if !minutes.is_finite() || minutes >= u64::MAX as f64 {
        return tr!("time_estimates.exceeds_estimate_range").into();
    }
    if minutes > 0. && minutes < 1. {
        return "<1 min".into();
    }
    let rounded = minutes.round() as u64;
    tr!("time_estimates.h_min", rounded / 60, rounded % 60)
}

fn joinery_controls(ui: &mut egui::Ui, rows: &[JoineryRow], rates: &mut JoineryRates) {
    sidebar::inspector_section(
        ui,
        tr!("time_estimates.joinery_whole_project"),
        JOINERY_SECTION_KEY,
        true,
        |ui| {
            ui.text_weak(JOINERY_SCOPE);
            let mut table = InspectorTable::new(
                ui.available_width(),
                [
                    Column::flex(0.35),
                    Column::flex(0.15),
                    Column::flex(0.25),
                    Column::flex(0.25),
                ],
            )
            .striped();
            table.row(ui, "joinery_headers", |column, ui| {
                ui.text_label(
                    RichText::new([tr!("time_estimates.joint"), tr!("time_estimates.count"), tr!("time_estimates.min_joint"), tr!("time_estimates.hands_on")][column]).strong(),
                );
            });
            for row in rows {
                table.row(ui, row.kind, |column, ui| match column {
                    0 => {
                        ui.text_label(row.kind.label());
                    }
                    1 => {
                        ui.text_label(row.count.to_string());
                    }
                    2 => {
                        let mut minutes = row.minutes_per_joint;
                        if ui
                            .add_sized(
                                [ui.available_width(), ui.spacing().interact_size.y],
                                egui::DragValue::new(&mut minutes)
                                    .speed(0.5)
                                    .range(0.0..=f64::MAX),
                            )
                            .on_hover_text(tr!(
                                "time_estimates.default_allowance_is_minutes_per_joint",
                                JoineryRates::default_minutes(row.kind)
                            ))
                            .changed()
                        {
                            rates.set_minutes(row.kind, minutes);
                            ui.ctx().request_repaint();
                        }
                    }
                    _ => {
                        ui.text_label(format_minutes(row.work_minutes()));
                    }
                });
            }
            ui.text_weak(JOINERY_ALLOWANCE_NOTE);
        },
    );
}

fn rate_input(ui: &mut egui::Ui, rate: &mut f64) {
    ui.add(egui::DragValue::new(rate).speed(0.5).range(0.0..=f64::MAX));
}

fn wait_input(ui: &mut egui::Ui, hours: &mut f64) {
    ui.add(
        egui::DragValue::new(hours)
            .speed(0.25)
            .range(0.0..=f64::MAX)
            .suffix(" h"),
    );
}

fn finish_controls(
    ui: &mut egui::Ui,
    title: &str,
    wait_label: &str,
    key: &'static str,
    step: &mut Step,
    products: &[finish_settings::Finish],
    filter: &mut String,
) {
    sidebar::inspector_section(ui, title, key, false, |ui| {
        ui.checkbox(&mut step.enabled, tr!("time_estimates.include_in_estimate"));
        if !step.enabled {
            return;
        }
        let mut table =
            InspectorTable::new(ui.available_width(), [Column::flex(0.4), Column::flex(0.6)])
                .centered();
        for (row, label) in [
            tr!("time_estimates.product"),
            tr!("time_estimates.minutes_m2"),
            tr!("time_estimates.coats"),
            wait_label,
        ]
        .into_iter()
        .enumerate()
        {
            table.row(ui, (key, row), |column, ui| {
                if column == 0 {
                    ui.text_label(label);
                } else {
                    match row {
                        0 => {
                            finish_settings::selector(
                                ui,
                                key,
                                &mut step.product_id,
                                products,
                                filter,
                            );
                        }
                        1 => rate_input(ui, &mut step.minutes_per_m2),
                        2 => {
                            ui.add(egui::DragValue::new(&mut step.coats).range(1..=100));
                        }
                        _ => wait_input(ui, &mut step.drying_hours),
                    }
                }
            });
        }
        if products.is_empty() {
            ui.text_weak(tr!("time_estimates.add_finishing_products_in_settings_material"));
        } else if step
            .product_id
            .is_some_and(|id| !products.iter().any(|product| product.id == id))
        {
            ui.text_weak(tr!("time_estimates.the_selected_product_is_no_longer_in_settings"));
        }
        let mut separate = step.final_drying_hours.is_some();
        if ui
            .checkbox(&mut separate, tr!("time_estimates.different_wait_after_final_coat"))
            .changed()
        {
            step.final_drying_hours = separate.then_some(step.drying_hours);
        }
        if let Some(hours) = &mut step.final_drying_hours {
            table.row(ui, (key, "final_wait"), |column, ui| {
                if column == 0 {
                    ui.text_label(tr!("time_estimates.final_wait"));
                } else {
                    wait_input(ui, hours);
                }
            });
        }
    });
}

/// Shared editor for device defaults and document-owned object configurations.
pub(super) fn treatment_controls(
    ui: &mut egui::Ui,
    settings: &mut Settings,
    defaults: bool,
    products: &[finish_settings::Finish],
    filters: &mut std::collections::BTreeMap<String, String>,
) {
    let colors = Palette::for_ui(ui);
    let keys = if defaults {
        [
            "settings/treatment/sanding",
            "settings/treatment/finishing",
            "settings/treatment/pre_stain",
            "settings/treatment/stain",
            "settings/treatment/varnish",
        ]
    } else {
        [
            "entity/treatment/sanding",
            "entity/treatment/finishing",
            "entity/treatment/pre_stain",
            "entity/treatment/stain",
            "entity/treatment/varnish",
        ]
    };
    sidebar::inspector_section(ui, tr!("time_estimates.sanding"), keys[0], false, |ui| {
        settings.sanitize();
        let mut remove = None;
        for i in 0..settings.sanding_grits.len() {
            let mut grit = settings.sanding_grits[i];
            let alias = format!("{}/grit/{grit}", keys[0]);
            let section_key = sidebar::item_section_key(ui.ctx(), &alias);
            if sidebar::inspector_section_with_remove(
                ui,
                &tr!("time_estimates.grit_2", grit = grit),
                &section_key,
                |ui| {
                    ui.checkbox(&mut settings.sanding[i].enabled, tr!("time_estimates.include_in_estimate"));
                    let mut table = InspectorTable::new(
                        ui.available_width(),
                        [Column::flex(0.4), Column::flex(0.6)],
                    )
                    .centered();
                    for (row, label) in [tr!("time_estimates.grit_3"), tr!("time_estimates.minutes_m2"), tr!("time_estimates.between_coats")]
                        .into_iter()
                        .enumerate()
                    {
                        table.row(ui, ("sanding", row), |column, ui| {
                            if column == 0 {
                                ui.text_label(label);
                            } else {
                                match row {
                                    0 => {
                                        ui.add(
                                            egui::DragValue::new(&mut grit)
                                                .range(1..=u32::MAX)
                                                .speed(1.),
                                        )
                                        .on_hover_text(tr!("time_estimates.enter_any_positive_grit_number_including_values_above"));
                                    }
                                    1 => {
                                        ui.add_enabled_ui(settings.sanding[i].enabled, |ui| {
                                            rate_input(ui, &mut settings.sanding[i].minutes_per_m2);
                                        });
                                }
                                _ => {
                                    ui.checkbox(&mut settings.sanding_after_varnish[i], "")
                                        .on_hover_text(tr!("time_estimates.between_varnish_coats_sand_after_each_coat_except"));
                                }
                            }
                        }
                    });
                    }
                },
            ) {
                remove = Some(i);
            }
            if !settings
                .sanding_grits
                .iter()
                .enumerate()
                .any(|(j, value)| j != i && *value == grit)
            {
                if settings.sanding_grits[i] != grit {
                    sidebar::rename_item_section(
                        ui.ctx(),
                        &alias,
                        &format!("{}/grit/{grit}", keys[0]),
                    );
                    settings.sanding_grits[i] = grit;
                }
            }
        }
        if let Some(i) = remove {
            settings.sanding_grits.remove(i);
            settings.sanding.remove(i);
            settings.sanding_after_varnish.remove(i);
        }
        if ui
            .add_sized(
                egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                egui::Button::new(tr!("time_estimates.add_grit")),
            )
            .clicked()
        {
            let mut grit = settings
                .sanding_grits
                .iter()
                .copied()
                .max()
                .unwrap_or(0)
                .saturating_add(80)
                .max(1);
            if settings.sanding_grits.contains(&grit) {
                grit = (1..=u32::MAX)
                    .find(|value| !settings.sanding_grits.contains(value))
                    .unwrap_or(1);
            }
            settings.sanding_grits.push(grit);
            settings.sanding.push(Step {
                enabled: true,
                ..Step::default()
            });
            settings.sanding_after_varnish.push(false);
        }
        ui.text_label(
            RichText::new(
                tr!("time_estimates.between_coats_applies_to_varnish_only_enable_varnish"),
            )
            .small()
            .color(colors.muted),
        );
    });
    ui.add_space(8.);
    sidebar::inspector_section(ui, tr!("time_estimates.finishing"), keys[1], false, |ui| {
        let wait = tr!("time_estimates.wait_per_coat");
        for (title, wait_label, key, step) in [
            (
                tr!("time_estimates.pre_stain"),
                tr!("time_estimates.soak_per_coat"),
                keys[2],
                &mut settings.pre_stain,
            ),
            (tr!("time_estimates.stain"), wait, keys[3], &mut settings.stain),
            (tr!("time_estimates.varnish"), wait, keys[4], &mut settings.varnish),
        ] {
            finish_controls(
                ui,
                title,
                wait_label,
                key,
                step,
                products,
                filters.entry(key.into()).or_default(),
            );
        }
    });
    ui.add_space(8.);
    settings.sanitize();
}

/// Only the inputs needed for a report cross the worker boundary; no history,
/// rendering resources or account state is copied.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PdfInput {
    name: String,
    model: std::sync::Arc<Model>,
    selected: BTreeSet<u64>,
    display: benchup_core::DisplaySettings,
    settings: Settings,
    joinery_rates: JoineryRates,
    stock: stock_settings::Settings,
    layout: crate::cutting_layout::Settings,
}
impl PdfInput {
    pub(super) fn render(self) -> Result<Vec<u8>, String> {
        let mut app = BenchUpApp::with_document(Document {
            name: self.name, model: std::sync::Arc::unwrap_or_clone(self.model), display: self.display,
            ..Default::default()
        }, None, false);
        app.selected = self.selected;
        app.time_estimates = self.settings;
        app.joinery_rates = self.joinery_rates;
        app.general_settings.stock = self.stock;
        app.project.cutting_plan_layout = self.layout;
        crate::estimate_pdf::render(&app.time_estimate_report())
    }
}

impl BenchUpApp {
    pub(super) fn export_time_estimates(&mut self, ctx: &egui::Context) {
        if self.project.blocked() {
            return;
        }
        self.queue_pdf_export(ctx, pdf_export::Kind::Estimates);
    }

    pub(super) fn estimate_pdf_input(&mut self, ctx: &egui::Context) -> PdfInput {
        self.document_revision(ctx);
        PdfInput {
            name: self.doc.name.clone(), model: self.document_model.shared(),
            selected: self.selected.clone(), display: self.doc.display.clone(),
            settings: self.time_estimates.clone(), joinery_rates: self.joinery_rates.clone(),
            stock: self.general_settings.stock.clone(), layout: self.project.cutting_plan_layout,
        }
    }

    fn time_estimate_report(&mut self) -> crate::estimate_pdf::Report {
        use crate::estimate_pdf::{Group, Report};
        let area = self.estimate_cache.area(
            &self.doc.model,
            &self.selected,
            &self.general_settings.stock,
        );
        let areas = &self.estimate_cache.area.as_ref().unwrap().1;
        let time = time_breakdown(
            &self.doc.model,
            areas,
            &self.time_estimates,
            &self.joinery_rates,
        );
        let finishes = finishing_costs(
            &self.doc.model,
            areas,
            &self.time_estimates,
            &self.general_settings.stock.finishes,
        );
        let duration = &time.duration;
        let catalog = &self.general_settings.stock;
        let mut total: f64 = finishes.iter().filter_map(|cost| cost.amount).sum();
        let mut partial = finishes.iter().any(|cost| cost.amount.is_none());
        let mut report = Report {
            name: self.doc.name.clone(),
            currency: currency::code(&catalog.currency).into(),
            tasks: time
                .has_joints()
                .then(|| {
                    [
                        tr!("time_estimates.joinery_whole_project_2").into(),
                        format_minutes(time.joinery_work),
                        format_minutes(0.),
                    ]
                })
                .into_iter()
                .chain(time.finishing.iter().map(|(task, duration, count)| {
                    [
                        tr!("time_estimates.objects", count = *count, title = task.label()),
                        format_minutes(duration.work),
                        format_minutes(duration.wait),
                    ]
                }))
                .collect(),
            joinery: time
                .joinery
                .iter()
                .filter(|row| row.count > 0)
                .map(|row| {
                    [
                        row.kind.label().into(),
                        row.count.to_string(),
                        row.minutes_per_joint.to_string(),
                        format_minutes(row.work_minutes()),
                    ]
                })
                .collect(),
            totals: [
                format_minutes(duration.work + duration.wait),
                format_minutes(duration.work),
                format_minutes(duration.wait),
            ],
            ..Default::default()
        };
        report.notes.push(tr!("time_estimates.wood_surfaces_finishing_applies_only_to_objects_assigned",
            self.doc.display.units.format_area(area), if self.selected.is_empty() { tr!("time_estimates.visible") } else { tr!("time_estimates.selected") }));
        report.notes.push(tr!(
            "time_estimates.joinery_whole_project_3",
            JOINERY_SCOPE = JOINERY_SCOPE,
            JOINERY_ALLOWANCE_NOTE = JOINERY_ALLOWANCE_NOTE,
        ));
        report.notes.push(tr!("time_estimates.kerf_rotation_stock_uses_each_part_s_full",
            self.doc.display.units.format_length(self.project.cutting_plan_layout.kerf_mm / 1000.),
            if self.project.cutting_plan_layout.allow_rotation { tr!("time_estimates.rotation_on") } else { tr!("time_estimates.rotation_off") }));
        report.notes.push(tr!("time_estimates.finishes_whole_containers_purchased_surface_area_and_coats").into());
        match self.estimate_cache.materials(
            &self.doc.model,
            &self.selected,
            self.project.cutting_plan_layout,
        ) {
            Ok(plan) => {
                partial |= plan.skipped > 0;
                if plan.skipped > 0 {
                    report.notes.push(tr!(
                        "time_estimates.estimate_incomplete_items_omitted",
                        count = plan.skipped
                    ));
                }
                let mut group = Group {
                    title: tr!("time_estimates.wood_stock").into(),
                    rows: Vec::new(),
                };
                for required in &plan.requirements {
                    let price = required
                        .material_id
                        .and_then(|id| catalog.wood.iter().find(|stock| stock.id == id))
                        .filter(|stock| {
                            stock.definition().dimensions == required.material.dimensions
                        })
                        .map(|stock| stock.price);
                    total += required.quantity as f64 * price.unwrap_or(0.);
                    partial |= price.is_none() || required.excluded > 0;
                    let mut item = required.material.name.clone();
                    if price.is_none() {
                        item.push_str(tr!("time_estimates.price_unavailable_add_a_price_in_settings_material"));
                    }
                    if required.excluded > 0 {
                        item.push_str(&tr!(
                            "time_estimates.cuts_excluded_note",
                            count = required.excluded
                        ));
                    }
                    group.rows.push([
                        item,
                        tr!("common.pieces", count = required.quantity),
                        price
                            .map(|value| currency::format_value(&catalog.currency, value))
                            .unwrap_or_else(|| tr!("time_estimates.unavailable").into()),
                        price
                            .map(|value| {
                                currency::format_value(
                                    &catalog.currency,
                                    required.quantity as f64 * value,
                                )
                            })
                            .unwrap_or_else(|| tr!("time_estimates.unavailable").into()),
                    ]);
                }
                if !group.rows.is_empty() {
                    report.costs.push(group);
                }
            }
            Err(message) => {
                partial = true;
                report.notes.push(tr!("time_estimates.no_stock_estimate_yet_assign_stock_materials_to", message = message));
            }
        }
        for kind in finish_settings::FinishType::ALL
            .into_iter()
            .map(Some)
            .chain([None])
        {
            let mut group = Group {
                title: kind
                    .map_or(tr!("time_estimates.unresolved_finishing_products"), |kind| kind.label())
                    .into(),
                rows: Vec::new(),
            };
            for cost in finishes.iter().filter(|cost| cost.kind == kind) {
                let mut item = cost.name.clone();
                if let Some(product) = cost
                    .product_id
                    .and_then(|id| catalog.finishes.iter().find(|product| product.id == id))
                {
                    item.push_str(&tr!(
                        "time_estimates.container",
                        product.container_size,
                        product.container_unit.label()
                    ));
                    if let Some(coverage) = product.coverage_m2_per_container {
                        item.push_str(&tr!("time_estimates.coverage_m2_per_container_per_coat", coverage = coverage));
                    }
                }
                if let Some(issue) = cost.issue {
                    item.push_str(&tr!("time_estimates.product_issue", issue = issue));
                }
                group.rows.push([
                    item,
                    cost.containers
                        .map(|value| tr!("time_estimates.containers", count = value))
                        .unwrap_or_else(|| tr!("time_estimates.unknown").into()),
                    cost.price
                        .map(|value| currency::format_value(&catalog.currency, value))
                        .unwrap_or_else(|| tr!("time_estimates.unavailable").into()),
                    cost.amount
                        .map(|value| currency::format_value(&catalog.currency, value))
                        .unwrap_or_else(|| tr!("time_estimates.unavailable").into()),
                ]);
            }
            if !group.rows.is_empty() {
                report.costs.push(group);
            }
        }
        if area <= 0. {
            report.notes.push(
                tr!("time_estimates.no_visible_surfaces_with_assigned_wood_materials_in").into(),
            );
        } else if time.finishing.is_empty() {
            report.notes.push(tr!("time_estimates.no_treatment_tasks_enabled_choose_sanding_grits_or").into());
        }
        report.summary = [
            if report.costs.is_empty() && partial {
                tr!("time_estimates.unavailable").into()
            } else {
                format!(
                    "{} {}{}",
                    report.currency,
                    currency::format_value(&catalog.currency, total),
                    if partial { tr!("time_estimates.partial") } else { "" }
                )
            },
            format_minutes(duration.work),
            format_minutes(duration.wait),
        ];
        if partial {
            report.notes.insert(0, tr!("time_estimates.estimate_incomplete_the_priced_subtotal_excludes_items_with").into());
        }
        report
    }

    pub(super) fn apply_materials_to_children(&mut self, ctx: &egui::Context) {
        if self.doc.model.selection_locked(&self.selected) {
            return;
        }
        let parents: std::collections::BTreeMap<_, _> = self
            .doc
            .model
            .objects
            .values()
            .flat_map(|object| object.entities.iter().map(move |id| (*id, object.id)))
            .collect();
        // Snapshot every source before editing. For nested selections the nearest
        // selected ancestor wins, and selected objects retain their own settings.
        let updates: Vec<_> = self
            .doc
            .model
            .objects
            .keys()
            .filter_map(|id| {
                if self.selected.contains(id) {
                    return None;
                }
                let mut ancestor = parents.get(id).copied();
                while let Some(parent) = ancestor {
                    if self.selected.contains(&parent) {
                        let source = &self.doc.model.objects[&parent];
                        return Some((
                            *id,
                            source.stock_material_id,
                            source.stock_material.clone(),
                            source
                                .surface_treatment_override()
                                .unwrap_or(&self.time_estimates)
                                .clone(),
                        ));
                    }
                    ancestor = parents.get(&parent).copied();
                }
                None
            })
            .collect();
        let before = self.doc.model.clone();
        for (id, material_id, material, treatment) in updates {
            let object = self.doc.model.objects.get_mut(&id).unwrap();
            object.stock_material_id = material_id;
            object.stock_material = material;
            object.surface_treatment = treatment;
            object.surface_treatment_customized = true;
        }
        if self.doc.model != before {
            self.finish_edit(tr!("time_estimates.apply_materials_to_children"), before, ctx);
        }
    }

    fn material_cost_estimate_contents(&mut self, ui: &mut egui::Ui) {
        let colors = Palette::for_ui(ui);
        self.estimate_cache.area(
            &self.doc.model,
            &self.selected,
            &self.general_settings.stock,
        );
        let finish_costs = finishing_costs(
            &self.doc.model,
            &self.estimate_cache.area.as_ref().unwrap().1,
            &self.time_estimates,
            &self.general_settings.stock.finishes,
        );
        let empty_plan = MaterialEstimate {
            requirements: Vec::new(),
            skipped: 0,
        };
        let (plan, stock_issue) = match self.estimate_cache.materials(
            &self.doc.model,
            &self.selected,
            self.project.cutting_plan_layout,
        ) {
            Ok(plan) => (plan, None),
            Err(message) => (&empty_plan, Some(message.as_str())),
        };
        let catalog = &self.general_settings.stock;
        let requirements: Vec<_> = plan
            .requirements
            .iter()
            .map(|required| {
                let price = required
                    .material_id
                    .and_then(|id| catalog.wood.iter().find(|stock| stock.id == id))
                    .filter(|stock| stock.definition().dimensions == required.material.dimensions)
                    .map(|stock| stock.price);
                (required, price)
            })
            .collect();
        let unpriced: usize = requirements
            .iter()
            .filter(|(_, price)| price.is_none())
            .map(|(required, _)| required.quantity)
            .sum();
        let excluded: usize = requirements
            .iter()
            .map(|(required, _)| required.excluded)
            .sum();
        let partial = unpriced > 0
            || excluded > 0
            || plan.skipped > 0
            || finish_costs.iter().any(|cost| cost.amount.is_none());

        egui::Frame::new()
            .fill(colors.canvas)
            .stroke(Stroke::new(1., colors.border))
            .corner_radius(5)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.text_label(RichText::new(tr!("time_estimates.cost_estimate")).small().color(colors.muted));
                ui.separator();
                let mut table = InspectorTable::new(
                    ui.available_width(),
                    [
                        Column::flex(0.4),
                        Column::flex(0.14),
                        Column::flex(0.23),
                        Column::flex(0.23),
                    ],
                )
                .striped();
                let currency = currency::code(&catalog.currency);
                let headers = [
                    tr!("time_estimates.item").to_owned(),
                    tr!("time_estimates.quantity").to_owned(),
                    tr!("time_estimates.unit_price", currency = currency),
                    tr!("time_estimates.amount", currency = currency),
                ];
                table.header(ui, headers.each_ref().map(String::as_str));
                if !requirements.is_empty() { cost_group(ui, tr!("time_estimates.wood_stock")); }
                for (required, price) in &requirements {
                    let values = [
                        required.quantity.to_string(),
                        price
                            .map(|price| currency::format_value(&catalog.currency, price))
                            .unwrap_or_else(|| "—".into()),
                        price
                            .map(|price| {
                                currency::format_value(
                                    &catalog.currency,
                                    required.quantity as f64 * price,
                                )
                            })
                            .unwrap_or_else(|| "—".into()),
                    ];
                    table.row(
                        ui,
                        ("material_cost", &required.material.name),
                        |column, ui| {
                            if column == 0 {
                                let mut lines =
                                    vec![RichText::new(&required.material.name).strong()];
                                if price.is_none() {
                                    lines.push(
                                        RichText::new(tr!("time_estimates.price_unavailable"))
                                            .small()
                                            .color(colors.accent),
                                    );
                                }
                                if required.excluded > 0 {
                                    lines.push(
                                        RichText::new(tr!(
                                            "time_estimates.cuts_excluded_label",
                                            count = required.excluded,
                                        ))
                                        .small()
                                        .color(colors.accent),
                                    );
                                }
                                product_cell_text(ui, lines);
                            } else {
                                price_text(ui, RichText::new(&values[column - 1]).strong());
                            }
                        },
                    );
                }
                finishing_cost_rows(ui, &mut table, &finish_costs, currency);
                if let Some(message) = stock_issue {
                    ui.text_label(RichText::new(tr!("time_estimates.no_stock_estimate_yet")).small().color(colors.muted))
                        .on_hover_text(message);
                }
                if !finish_costs.is_empty() {
                    ui.text_label(RichText::new(tr!("time_estimates.finishes_whole_containers_needs_combined_across_objects_and"))
                        .small().color(colors.muted));
                }
            });
        ui.add_space(6.);
        if partial {
            egui::Frame::new()
                .fill(colors.canvas)
                .corner_radius(5)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.text_label(RichText::new(tr!("time_estimates.estimate_incomplete")).strong().color(colors.accent));
                    if unpriced > 0 {
                        ui.text_label(tr!("time_estimates.stock_pieces_without_price", count = unpriced));
                    }
                    if excluded > 0 {
                        ui.text_label(tr!("time_estimates.some_cuts_need_larger_stock_their_cost_is"));
                    }
                    if plan.skipped > 0 {
                        ui.text_label(tr!("time_estimates.assigned_items_omitted", count = plan.skipped));
                    }
                });
            ui.add_space(6.);
        }
        ui.text_label(RichText::new(tr!("time_estimates.kerf_rotation",
            self.doc.display.units.format_length(self.project.cutting_plan_layout.kerf_mm / 1000.),
            if self.project.cutting_plan_layout.allow_rotation { tr!("time_estimates.rotation_on") } else { tr!("time_estimates.rotation_off") }
        )).small().color(colors.muted))
            .on_hover_text(tr!("time_estimates.uses_each_part_s_full_bounding_box_with"));
    }

    pub(super) fn surface_treatment_info(&mut self, ui: &mut egui::Ui) {
        // Unassigned imported assemblies have no wood treatment to display.
        // Avoid constructing a parent entry for every piece of mesh geometry.
        if self.doc.model.objects.values().all(|object| object.stock_material_id.is_none() && object.stock_material.is_none()) {return;}
        let wood: WoodMaterials = self
            .general_settings
            .stock
            .wood
            .iter()
            .map(|stock| (stock.id, stock.definition()))
            .collect();
        let parents = self
            .doc
            .model
            .objects
            .values()
            .flat_map(|object| object.entities.iter().map(move |id| (*id, object.id)))
            .collect();
        let objects: Vec<_> = self
            .doc
            .model
            .expand_entities(&self.selected)
            .into_iter()
            .filter(|id| self.doc.model.objects.contains_key(id))
            .filter(|id| object_has_wood_material(&self.doc.model, *id, &parents, &wood))
            .collect();
        let Some(first) = objects.first() else {
            return;
        };
        let effective = |id: &u64| {
            self.doc.model.objects[id]
                .surface_treatment_override()
                .unwrap_or(&self.time_estimates)
                .clone()
        };
        let mut settings = effective(first);
        let original = settings.clone();
        let mixed = objects.iter().any(|id| effective(id) != original);
        let all_defaults = objects.iter().all(|id| {
            self.doc.model.objects[id]
                .surface_treatment_override()
                .is_none()
        });
        if mixed {
            ui.text_weak(tr!("time_estimates.mixed_sanding_and_finishing_settings_changes_apply_to"));
        }
        let mut reset = false;
        let mut apply_children = false;
        let has_children = self.selected.iter().any(|id| {
            self.doc.model.objects.get(id).is_some_and(|object| {
                self.doc
                    .model
                    .expand_entities(&object.entities)
                    .iter()
                    .any(|child| self.doc.model.objects.contains_key(child))
            })
        });
        ui.add_enabled_ui(!self.doc.model.selection_locked(&self.selected), |ui| {
            treatment_controls(
                ui,
                &mut settings,
                false,
                &self.general_settings.stock.finishes,
                &mut self.finish_product_filters,
            );
            apply_children = ui
                .add_enabled(has_children, egui::Button::new(tr!("time_estimates.apply_to_all_children"))
                    .min_size(egui::vec2(ui.available_width(), ui.spacing().interact_size.y)))
                .on_hover_text(tr!("time_estimates.copy_each_selected_object_s_stock_material_sanding"))
                .clicked();
            reset = ui
                .add_enabled(!all_defaults, egui::Button::new(tr!("time_estimates.revert_to_defaults"))
                    .min_size(egui::vec2(ui.available_width(), ui.spacing().interact_size.y)))
                .on_hover_text(tr!("time_estimates.restore_sanding_and_finishing_defaults_from_settings_for"))
                .clicked();
        });
        if reset || settings != original {
            let before = self.doc.model.clone();
            for id in objects {
                let object = self.doc.model.objects.get_mut(&id).unwrap();
                if reset {
                    object.surface_treatment = Settings::default();
                    object.surface_treatment_customized = false;
                } else {
                    let mut target = object
                        .surface_treatment_override()
                        .unwrap_or(&self.time_estimates)
                        .clone();
                    apply_treatment_changes(&mut target, &original, &settings);
                    object.surface_treatment = target;
                    object.surface_treatment_customized = true;
                }
            }
            self.finish_edit(tr!("time_estimates.change_sanding_and_finishing"), before, ui.ctx());
        }
        if apply_children {
            self.apply_materials_to_children(ui.ctx());
        }
    }

    pub(super) fn time_estimates_contents(&mut self, ui: &mut egui::Ui) {
        let colors = Palette::for_ui(ui);
        let area = self.estimate_cache.area(
            &self.doc.model,
            &self.selected,
            &self.general_settings.stock,
        );
        let time = time_breakdown(
            &self.doc.model,
            &self.estimate_cache.area.as_ref().unwrap().1,
            &self.time_estimates,
            &self.joinery_rates,
        );
        if self.doc.model.connections.is_empty()
            && self.doc.model.edges.is_empty()
            && self.doc.model.faces.is_empty()
            && self.doc.model.objects.is_empty()
        {
            crate::empty_state::EmptyState::new(tr!("time_estimates.no_estimates_yet"),
                tr!("time_estimates.draw_or_import_parts_to_estimate_time_and")).show(ui);
            ui.add_space(8.);
            joinery_controls(ui, &time.joinery, &mut self.joinery_rates);
            return;
        }
        let finish_costs = finishing_costs(
            &self.doc.model,
            &self.estimate_cache.area.as_ref().unwrap().1,
            &self.time_estimates,
            &self.general_settings.stock.finishes,
        );
        let finish_total: f64 = finish_costs.iter().filter_map(|cost| cost.amount).sum();
        let finish_partial = finish_costs.iter().any(|cost| cost.amount.is_none());
        let duration = &time.duration;
        let catalog = &self.general_settings.stock;
        let material_summary = match self.estimate_cache.materials(
            &self.doc.model,
            &self.selected,
            self.project.cutting_plan_layout,
        ) {
            Ok(plan) => {
                let mut total = finish_total;
                let mut partial = plan.skipped > 0 || finish_partial;
                for required in &plan.requirements {
                    let price = required
                        .material_id
                        .and_then(|id| catalog.wood.iter().find(|stock| stock.id == id))
                        .filter(|stock| {
                            stock.definition().dimensions == required.material.dimensions
                        })
                        .map(|stock| stock.price);
                    total += required.quantity as f64 * price.unwrap_or(0.);
                    partial |= price.is_none() || required.excluded > 0;
                }
                format!(
                    "{} {}{}",
                    currency::code(&catalog.currency),
                    currency::format_value(&catalog.currency, total),
                    if partial { tr!("time_estimates.partial") } else { "" },
                )
            }
            Err(_) if !finish_costs.is_empty() => tr!(
                "time_estimates.partial_2",
                currency::code(&catalog.currency),
                currency::format_value(&catalog.currency, finish_total),
            ),
            Err(_) => tr!("time_estimates.unavailable").to_owned(),
        };
        let mut summary = InspectorTable::new(
            ui.available_width(),
            [Column::flex(1.), Column::flex(1.)],
        )
        .striped()
        .cell_padding(8.);
        summary.header(ui, [tr!("time_estimates.estimate"), tr!("time_estimates.value")]);
        for (key, label, value, accent) in [
            ("price", tr!("time_estimates.price"), material_summary, colors.green),
            (
                "work",
                tr!("time_estimates.hands_on_work"),
                format_minutes(duration.work),
                colors.accent,
            ),
            (
                "wait",
                tr!("time_estimates.drying_waiting"),
                format_minutes(duration.wait),
                colors.muted,
            ),
        ] {
            // Darken accents in the light theme to keep normal-sized values readable.
            let value_color = if ui.visuals().dark_mode {
                accent
            } else {
                let darken = |channel: u8, ink: u8| {
                    (channel as f32 * 0.65 + ink as f32 * 0.35).round() as u8
                };
                egui::Color32::from_rgb(
                    darken(accent.r(), colors.ink.r()),
                    darken(accent.g(), colors.ink.g()),
                    darken(accent.b(), colors.ink.b()),
                )
            };
            summary.row(ui, ("estimate_summary", key), |column, cell| {
                cell.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                    if column == 0 {
                        ui.text_label(label);
                    } else {
                        let text = RichText::new(&value).strong().color(value_color);
                        if key == "price" {
                            price_text(ui, text);
                        } else {
                            ui.text_label(text);
                        }
                    }
                });
            });
        }
        ui.text_label(
            RichText::new(if self.selected.is_empty() {
                tr!("time_estimates.materials_all_visible_parts")
            } else {
                tr!("time_estimates.materials_selected_parts")
            })
            .small()
            .color(colors.muted),
        );
        ui.add_space(8.);
        ui.text_label(
            RichText::new(
                tr!("time_estimates.time_includes_whole_project_joinery_sanding_finishing_and"),
            )
            .small()
            .color(colors.muted),
        );
        ui.add_space(8.);
        ui.separator();
        ui.add_space(4.);
        ui.horizontal_wrapped(|ui| {
            ui.text_label(RichText::new(self.doc.display.units.format_area(area)).strong());
            ui.text_label(
                RichText::new(if self.selected.is_empty() {
                    tr!("time_estimates.visible_wood_surfaces")
                } else {
                    tr!("time_estimates.selected_wood_surfaces")
                })
                .small()
                .color(colors.muted),
            );
        });
        ui.text_label(RichText::new(tr!("time_estimates.finishing_applies_only_to_objects_assigned_a_material")).small().color(colors.muted));
        ui.add_space(6.);
        egui::Frame::new()
            .fill(colors.canvas)
            .stroke(Stroke::new(1., colors.border))
            .corner_radius(5)
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if area <= 0. {
                    crate::empty_state::EmptyState::new(tr!("time_estimates.no_surfaces_to_estimate"),
                        if self.selected.is_empty() { tr!("time_estimates.no_visible_surfaces_with_assigned_wood_materials") }
                        else { tr!("time_estimates.select_faces_or_groups_with_assigned_wood_materials") }).show(ui);
                } else if time.finishing.is_empty() {
                    crate::empty_state::EmptyState::new(tr!("time_estimates.no_finishing_tasks"),
                        tr!("time_estimates.choose_sanding_grits_or_finishes_for_objects_in")).show(ui);
                }
                if time.has_joints() || !time.finishing.is_empty() {
                    ui.text_label(
                        RichText::new(tr!("time_estimates.sequential_total"))
                            .small()
                            .color(colors.muted),
                    );
                    ui.text_label(
                        RichText::new(format_minutes(duration.work + duration.wait))
                            .size(19.)
                            .strong()
                            .color(colors.green),
                    );
                    ui.separator();
                    let mut table = InspectorTable::new(
                        ui.available_width(),
                        [Column::flex(1.), Column::flex(1.), Column::flex(1.)],
                    )
                    .striped();
                    table.header(ui, [tr!("time_estimates.task"), tr!("time_estimates.hands_on"), tr!("time_estimates.drying_waiting")]);
                    if time.has_joints() {
                        let values = [
                            tr!("time_estimates.joinery_whole_project_2").into(),
                            format_minutes(time.joinery_work),
                            "-".into(),
                        ];
                        table.row(ui, "joinery_total", |column, ui| {
                            ui.text_label(RichText::new(&values[column]).small());
                        });
                    }
                    for (index, (task, duration, _)) in time.finishing.iter().enumerate() {
                        table.row(ui, ("task", index), |column, ui| {
                            if column == 0 {
                                ui.text_label(
                                    RichText::new(task.short_label())
                                    .small(),
                                )
                                .on_hover_text(task.label());
                            } else {
                                let minutes = if column == 1 { duration.work } else { duration.wait };
                                ui.text_label(
                                    RichText::new(if minutes == 0. {
                                        "-".to_owned()
                                    } else {
                                        format_minutes(minutes)
                                    })
                                    .small(),
                                );
                            }
                        });
                    }
                    ui.separator();
                    table.row(ui, "total", |column, ui| {
                        let text = match column {
                            0 => tr!("time_estimates.total").to_owned(),
                            _ => {
                                let minutes = if column == 1 {
                                    duration.work
                                } else {
                                    duration.wait
                                };
                                if minutes == 0. {
                                    "-".to_owned()
                                } else {
                                    format_minutes(minutes)
                                }
                            }
                        };
                        ui.text_label(RichText::new(text).small().strong());
                    });
                    ui.text_label(
                        RichText::new(tr!("time_estimates.includes_final_drying_objects_dry_together_between_steps"))
                            .small()
                            .color(colors.muted),
                    );
                }
            });
        ui.add_space(8.);
        self.material_cost_estimate_contents(ui);
        ui.add_space(8.);
        joinery_controls(ui, &time.joinery, &mut self.joinery_rates);
    }
}

#[cfg(test)]
#[path = "time_estimates_tests.rs"]
mod tests;
