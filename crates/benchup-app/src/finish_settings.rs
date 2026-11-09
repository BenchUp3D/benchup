//! Reusable finishing-product prices, stored with the device's material catalog.
use benchup_core::tr;
use super::*;
use crate::inspector_table::{Column, InspectorTable};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum FinishType {
    PreStain,
    #[default]
    Stain,
    Paint,
    Varnish,
    Primer,
    Sealer,
    Lacquer,
    Oil,
    Wax,
    Other,
}

impl FinishType {
    pub(super) const ALL: [Self; 10] = [
        Self::PreStain,
        Self::Stain,
        Self::Paint,
        Self::Varnish,
        Self::Primer,
        Self::Sealer,
        Self::Lacquer,
        Self::Oil,
        Self::Wax,
        Self::Other,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::PreStain => tr!("finish_settings.pre_stain"),
            Self::Stain => tr!("finish_settings.stain"),
            Self::Paint => tr!("finish_settings.paint"),
            Self::Varnish => tr!("finish_settings.varnish"),
            Self::Primer => tr!("finish_settings.primer"),
            Self::Sealer => tr!("finish_settings.sealer"),
            Self::Lacquer => tr!("finish_settings.lacquer"),
            Self::Oil => tr!("finish_settings.oil"),
            Self::Wax => tr!("finish_settings.wax"),
            Self::Other => tr!("finish_settings.other"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum ContainerUnit {
    Milliliters,
    #[default]
    Liters,
    UsFluidOunces,
    UsQuarts,
    UsGallons,
    Grams,
    Kilograms,
}

impl ContainerUnit {
    const ALL: [Self; 7] = [
        Self::Milliliters,
        Self::Liters,
        Self::UsFluidOunces,
        Self::UsQuarts,
        Self::UsGallons,
        Self::Grams,
        Self::Kilograms,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Milliliters => tr!("finish_settings.ml"),
            Self::Liters => tr!("finish_settings.l"),
            Self::UsFluidOunces => tr!("finish_settings.us_fl_oz"),
            Self::UsQuarts => tr!("finish_settings.us_qt"),
            Self::UsGallons => tr!("finish_settings.us_gal"),
            Self::Grams => "g",
            Self::Kilograms => "kg",
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Finish {
    #[serde(default)]
    pub id: benchup_core::MaterialId,
    pub name: String,
    pub finish_type: FinishType,
    pub container_size: f64,
    pub container_unit: ContainerUnit,
    pub price: f64,
    /// Single-coat coverage of the configured container, in square metres.
    pub coverage_m2_per_container: Option<f64>,
}

impl Default for Finish {
    fn default() -> Self {
        Self {
            id: benchup_core::MaterialId::new_v4(),
            name: tr!("finish_settings.finishing_product").into(),
            finish_type: FinishType::default(),
            container_size: 1.,
            container_unit: ContainerUnit::default(),
            price: 0.,
            coverage_m2_per_container: None,
        }
    }
}

pub(super) fn normalize(finishes: &mut [Finish]) {
    let mut used = BTreeSet::new();
    for finish in finishes {
        if finish.id.is_nil() || !used.insert(finish.id) {
            finish.id = benchup_core::MaterialId::new_v4();
            used.insert(finish.id);
        }
        finish.container_size = if finish.container_size.is_finite() {
            finish.container_size.clamp(0.001, 1_000_000.)
        } else {
            1.
        };
        finish.price = if finish.price.is_finite() {
            finish.price.clamp(0., 1e9)
        } else {
            0.
        };
        finish.coverage_m2_per_container = finish
            .coverage_m2_per_container
            .filter(|value| value.is_finite() && *value > 0.);
    }
}

pub(super) fn selector(
    ui: &mut egui::Ui,
    key: &str,
    selected_id: &mut Option<benchup_core::MaterialId>,
    products: &[Finish],
    filter: &mut String,
) -> egui::Response {
    use super::searchable_selector::{Choice, SearchableSelector};
    let selected = selected_id.map_or(tr!("finish_settings.no_product"), |id| {
        products
            .iter()
            .find(|product| product.id == id)
            .map_or(tr!("finish_settings.unavailable_product"), |product| product.name.as_str())
    });
    let mut none = Choice::new(None, tr!("finish_settings.no_product"));
    none.always_visible = true;
    let mut choices = vec![none];
    choices.extend(products.iter().map(|product| {
        let mut choice = Choice::new(Some(product.id), &product.name);
        choice.search_text = format!("{} {}", product.name, product.finish_type.label());
        choice.tooltip = Some(product.finish_type.label().into());
        choice
    }));
    SearchableSelector {
        id_salt: egui::IdSalt::new((key, "product")),
        scroll_key: &format!("{key}/product/results"),
        selected_label: selected,
        width: ui.available_width(),
        popup_width: 260.,
        hint: tr!("finish_settings.search_name_or_type"),
        empty_text: tr!("finish_settings.no_products_match_your_search"),
    }
    .show(ui, selected_id, filter, choices)
    .response
}

pub(super) fn show(ui: &mut egui::Ui, finishes: &mut Vec<Finish>, currency: &str) {
    sidebar::inspector_section(
        ui,
        tr!("finish_settings.finishing_products"),
        "settings/stock/finishes",
        true,
        |ui| {
            ui.text_weak(tr!("finish_settings.pre_stain_conditioner_stain_paint_varnish_and_other"));
            if finishes.is_empty() {
                ui.text_weak(tr!("finish_settings.no_finishing_products_yet_add_a_product_to"));
            } else {
                let key = "settings/stock/finishes/table";
                let scroll = sidebar::restored_scroll_area(
                    ui,
                    key,
                    egui::ScrollArea::horizontal()
                        .id_salt(key)
                        .auto_shrink([false, true]),
                );
                let mut table = InspectorTable::new(
                    ui.available_width(),
                    [
                        Column::at_least(150.),
                        Column::at_least(100.),
                        Column::at_least(66.),
                        Column::at_least(80.),
                        Column::at_least(112.),
                        Column::at_least(110.),
                        Column::fixed(28.),
                    ],
                )
                .striped();
                let mut remove = None;
                let output = scroll.show(ui, |ui| {
                    table.header(
                        ui,
                        [
                            tr!("finish_settings.product"),
                            tr!("finish_settings.type"),
                            tr!("finish_settings.quantity"),
                            tr!("finish_settings.unit"),
                            tr!("finish_settings.price_container"),
                            tr!("finish_settings.m2_container_coat"),
                            "",
                        ],
                    );
                    for finish in finishes.iter_mut() {
                        table.row(ui, finish.id, |column, ui| match column {
                            0 => {
                                ui.add(
                                    egui::TextEdit::singleline(&mut finish.name)
                                        .hint_text(tr!("finish_settings.e_g_golden_oak_stain"))
                                        .char_limit(120),
                                );
                            }
                            1 => {
                                egui::ComboBox::from_id_salt("type")
                                    .width(ui.available_width())
                                    .truncate()
                                    .selected_text(finish.finish_type.label())
                                    .show_ui(ui, |ui| {
                                        for kind in FinishType::ALL {
                                            ui.selectable_value(
                                                &mut finish.finish_type,
                                                kind,
                                                kind.label(),
                                            );
                                        }
                                    });
                            }
                            2 => {
                                ui.add(
                                    egui::DragValue::new(&mut finish.container_size)
                                        .range(0.001..=1_000_000.)
                                        .speed(0.1)
                                        .max_decimals(3),
                                );
                            }
                            3 => {
                                egui::ComboBox::from_id_salt("unit")
                                    .width(ui.available_width())
                                    .truncate()
                                    .selected_text(finish.container_unit.label())
                                    .show_ui(ui, |ui| {
                                        for unit in ContainerUnit::ALL {
                                            ui.selectable_value(
                                                &mut finish.container_unit,
                                                unit,
                                                unit.label(),
                                            );
                                        }
                                    });
                            }
                            4 => {
                                ui.add(currency::price_input(currency, &mut finish.price))
                                    .on_hover_text(tr!("finish_settings.per_container", currency = currency));
                            }
                            5 => {
                                let mut coverage = finish.coverage_m2_per_container.unwrap_or(0.);
                                if ui.add(egui::DragValue::new(&mut coverage)
                                    .range(0.0..=1_000_000.)
                                    .speed(0.1)
                                    .max_decimals(3))
                                    .on_hover_text(tr!("finish_settings.area_covered_by_one_container_for_one_coat"))
                                    .changed()
                                {
                                    finish.coverage_m2_per_container = (coverage > 0.).then_some(coverage);
                                }
                            }
                            _ => {
                                if ui
                                    .button(tr!("finish_settings.format"))
                                    .on_hover_text(tr!("finish_settings.remove", finish.name))
                                    .clicked()
                                {
                                    remove = Some(finish.id);
                                }
                            }
                        });
                    }
                });
                sidebar::remember_scroll(ui.ctx(), key, output.state.clamped_offset());
                if let Some(id) = remove {
                    finishes.retain(|finish| finish.id != id);
                }
                ui.text_weak(tr!("finish_settings.prices_in_per_container_coverage_in_m2_per", currency = currency));
            }
            if ui.button(tr!("finish_settings.add_finishing_product")).clicked() {
                finishes.push(Finish::default());
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_products_get_defaults_and_invalid_values_are_repaired() {
        let mut finish: Finish = serde_json::from_str(r#"{"name":"Oak stain"}"#).unwrap();
        assert_eq!(finish.finish_type, FinishType::Stain);
        assert_eq!(finish.container_size, 1.);
        assert_eq!(finish.container_unit, ContainerUnit::Liters);
        assert_eq!(finish.price, 0.);
        assert_eq!(finish.coverage_m2_per_container, None);
        assert!(finish.id.is_nil());
        finish.container_size = f64::NAN;
        finish.price = f64::INFINITY;
        let mut finishes = vec![finish];
        normalize(&mut finishes);
        assert!(!finishes[0].id.is_nil());
        assert_eq!(finishes[0].container_size, 1.);
        assert_eq!(finishes[0].price, 0.);
        finishes[0].coverage_m2_per_container = Some(f64::NAN);
        normalize(&mut finishes);
        assert_eq!(finishes[0].coverage_m2_per_container, None);
        finishes.push(finishes[0].clone());
        finishes[0].container_size = -1.;
        finishes[0].price = -10.;
        let id = finishes[0].id;
        normalize(&mut finishes);
        assert_eq!(finishes[0].id, id);
        assert_ne!(finishes[1].id, id);
        assert_eq!(finishes[0].container_size, 0.001);
        assert_eq!(finishes[0].price, 0.);
    }

    #[test]
    fn existing_wood_catalogs_gain_an_empty_finish_catalog() {
        let mut settings: stock_settings::Settings =
            serde_json::from_str(r#"{"currency":"CAD","wood":[{"name":"Oak","price":12.50}]}"#)
                .unwrap();
        settings.normalize();
        assert!(settings.finishes.is_empty());
        assert_eq!(settings.currency, "CAD");
        assert_eq!(settings.wood.len(), 1);
        assert_eq!(settings.wood[0].name, "Oak");
        assert_eq!(settings.wood[0].price, 12.50);
    }
}
