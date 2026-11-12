//! Reusable finishing products, stored with the device's material catalog.
use benchup_core::tr;
use benchup_core::surface_treatment::{Appearance, Sheen};
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Formula {
    #[default]
    Standard,
    Dye,
    Pigment,
    Gel,
    WaterBased,
    OilBased,
}

impl Formula {
    fn options(kind: FinishType) -> &'static [Self] {
        match kind {
            FinishType::Stain => &[Self::Standard, Self::Dye, Self::Pigment, Self::Gel],
            FinishType::Varnish => &[Self::Standard, Self::WaterBased, Self::OilBased],
            _ => &[Self::Standard],
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Standard => tr!("finish_settings.standard_formula"),
            Self::Dye => tr!("finish_settings.dye"),
            Self::Pigment => tr!("finish_settings.pigment"),
            Self::Gel => tr!("finish_settings.gel"),
            Self::WaterBased => tr!("finish_settings.water_based"),
            Self::OilBased => tr!("finish_settings.oil_based"),
        }
    }
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

    pub(super) fn preset(self) -> (Option<[u8; 3]>, u8, Sheen) {
        use Sheen::{Gloss, Matte, Satin};
        match self {
            Self::PreStain => (None, 55, Matte),
            Self::Stain => (Some([117, 67, 34]), 36, Matte),
            Self::Paint => (Some([233, 231, 222]), 86, Satin),
            Self::Varnish => (None, 62, Satin),
            Self::Primer => (Some([213, 207, 193]), 76, Matte),
            Self::Sealer => (None, 50, Satin),
            Self::Lacquer => (None, 73, Gloss),
            Self::Oil => (Some([170, 112, 57]), 23, Satin),
            Self::Wax => (None, 42, Satin),
            Self::Other => (None, 45, Matte),
        }
    }
}

pub(super) fn tint_input(
    ui: &mut egui::Ui,
    tint: &mut Option<[u8; 3]>,
    inherited: Option<[u8; 3]>,
) {
    ui.horizontal(|ui| {
        let mut color = tint.or(inherited).unwrap_or([255; 3]);
        if ui.color_edit_button_srgb(&mut color)
            .on_hover_text(tr!("finish_settings.appearance_color"))
            .changed()
        {
            *tint = Some(color);
        }
        if tint.is_some() && ui.small_button("↶")
            .on_hover_text(tr!("finish_settings.reset_appearance_value")).clicked()
        {
            *tint = None;
        } else if tint.is_none() && inherited.is_none() {
            ui.text_weak(tr!("finish_settings.clear_color"));
        }
    });
}

pub(super) fn strength_input(ui: &mut egui::Ui, strength: &mut Option<u8>, inherited: u8) {
    ui.horizontal(|ui| {
        let mut value = strength.unwrap_or(inherited);
        if ui.add(egui::DragValue::new(&mut value).range(0..=100).suffix("%"))
            .on_hover_text(tr!("finish_settings.coat_strength_help")).changed()
        {
            *strength = Some(value);
        }
        if strength.is_some() && ui.small_button("↶")
            .on_hover_text(tr!("finish_settings.reset_appearance_value")).clicked()
        {
            *strength = None;
        }
    });
}

fn sheen_label(sheen: Sheen) -> &'static str {
    match sheen {
        Sheen::Matte => tr!("finish_settings.matte"),
        Sheen::Satin => tr!("finish_settings.satin"),
        Sheen::Gloss => tr!("finish_settings.gloss"),
    }
}

pub(super) fn sheen_input(ui: &mut egui::Ui, sheen: &mut Option<Sheen>, inherited: Sheen) {
    egui::ComboBox::from_id_salt("sheen")
        .width(ui.available_width())
        .truncate()
        .selected_text(sheen_label(sheen.unwrap_or(inherited)))
        .show_ui(ui, |ui| {
            ui.selectable_value(sheen, None, tr!("finish_settings.default_sheen", sheen_label(inherited)));
            for value in [Sheen::Matte, Sheen::Satin, Sheen::Gloss] {
                ui.selectable_value(sheen, Some(value), sheen_label(value));
            }
        });
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
    #[serde(skip_serializing_if = "Appearance::is_default")]
    pub appearance: Appearance,
    pub formula: Formula,
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
            appearance: Appearance::default(),
            formula: Formula::default(),
        }
    }
}

impl Finish {
    pub(super) fn visual_preset(&self) -> (Option<[u8; 3]>, u8, Sheen) {
        let (mut tint, mut strength, sheen) = self.finish_type.preset();
        match self.formula {
            Formula::Dye => strength = 30,
            Formula::Pigment => strength = 43,
            Formula::Gel => strength = 52,
            Formula::WaterBased => strength = 55,
            Formula::OilBased => {
                tint = Some([222, 164, 83]);
                strength = 65;
            }
            Formula::Standard => {}
        }
        (tint, strength, sheen)
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
        finish.appearance.strength = finish.appearance.strength.map(|value| value.min(100));
        if !Formula::options(finish.finish_type).contains(&finish.formula) {
            finish.formula = Formula::Standard;
        }
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
                        Column::at_least(92.),
                        Column::at_least(98.),
                        Column::at_least(100.),
                        Column::at_least(115.),
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
                            tr!("finish_settings.appearance_color"),
                            tr!("finish_settings.coat_strength"),
                            tr!("finish_settings.sheen"),
                            tr!("finish_settings.formula"),
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
                            6 => {
                                let tint = finish.visual_preset().0;
                                tint_input(ui, &mut finish.appearance.tint, tint);
                            }
                            7 => {
                                let strength = finish.visual_preset().1;
                                strength_input(ui, &mut finish.appearance.strength, strength);
                            }
                            8 => sheen_input(ui, &mut finish.appearance.sheen, finish.finish_type.preset().2),
                            9 => {
                                let options = Formula::options(finish.finish_type);
                                if options.len() > 1 {
                                    egui::ComboBox::from_id_salt("formula")
                                        .width(ui.available_width())
                                        .selected_text(finish.formula.label())
                                        .show_ui(ui, |ui| {
                                            for &formula in options {
                                                ui.selectable_value(&mut finish.formula, formula, formula.label());
                                            }
                                        });
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
        assert_eq!(finish.appearance, Appearance::default());
        assert_eq!(finish.formula, Formula::Standard);
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
