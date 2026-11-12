//! Collapse the ordered treatment recipe into a compact material for the GPU.
//! Appearance inputs intentionally exclude purchasing, labor and drying data.
use super::{finish_settings, stock_settings};
use crate::renderer::EntityCache;
use benchup_core::{
    DVec3, MaterialId, Model, StockMaterial,
    surface_treatment::{Appearance, Settings, Sheen, Step},
};
use eframe::egui;
use finish_settings::{FinishType, Formula};
use glam::{DMat3, Vec3};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

#[derive(Clone, Copy, PartialEq)]
struct Layer {
    kind: FinishType,
    formula: Formula,
    tint: Option<[u8; 3]>,
    strength: u8,
    sheen: Sheen,
}

impl Layer {
    fn new(kind: FinishType, formula: Formula, preset: (Option<[u8; 3]>, u8, Sheen)) -> Self {
        Self {
            kind,
            formula,
            tint: preset.0,
            strength: preset.1,
            sheen: preset.2,
        }
    }

    fn apply(mut self, edit: Appearance) -> Self {
        self.tint = edit.tint.or(self.tint);
        self.strength = edit.strength.unwrap_or(self.strength).min(100);
        self.sheen = edit.sheen.unwrap_or(self.sheen);
        self
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Wood {
    color: [u8; 3],
    grain: f32,
}

impl Default for Wood {
    fn default() -> Self {
        let (color, grain) = stock_settings::WoodType::default().appearance();
        Self { color, grain }
    }
}

#[derive(PartialEq)]
pub(crate) struct Catalog {
    wood: BTreeMap<MaterialId, Wood>,
    legacy_wood: BTreeMap<StockMaterial, Wood>,
    products: BTreeMap<MaterialId, Layer>,
    defaults: Recipe,
}

impl Catalog {
    #[cfg(test)]
    pub(crate) fn from_defaults_for_test(defaults: &Settings) -> Self {
        Self::new(&stock_settings::Settings::default(), defaults)
    }

    pub(super) fn new(stock: &stock_settings::Settings, defaults: &Settings) -> Self {
        let wood = |stock: &stock_settings::Stock| {
            let (color, grain) = stock.wood_type.appearance();
            Wood {
                color: stock.appearance_color.unwrap_or(color),
                grain,
            }
        };
        Self {
            wood: stock.wood.iter().map(|s| (s.id, wood(s))).collect(),
            legacy_wood: stock
                .wood
                .iter()
                .map(|s| (s.definition(), wood(s)))
                .collect(),
            products: stock
                .finishes
                .iter()
                .map(|product| {
                    (
                        product.id,
                        Layer::new(
                            product.finish_type,
                            product.formula,
                            product.visual_preset(),
                        )
                        .apply(product.appearance),
                    )
                })
                .collect(),
            defaults: Recipe::from_settings(defaults),
        }
    }

    pub(super) fn for_viewport(
        ctx: &egui::Context,
        stock: &stock_settings::Settings,
        defaults: &Settings,
    ) -> Arc<Self> {
        let candidate = Self::new(stock, defaults);
        ctx.data_mut(|data| {
            let key = egui::Id::unique("viewport surface appearance catalog");
            if let Some(current) = data.get_temp::<Arc<Self>>(key)
                && *current == candidate
            {
                return current;
            }
            let catalog = Arc::new(candidate);
            data.insert_temp(key, Arc::clone(&catalog));
            catalog
        })
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Coats {
    product_id: Option<MaterialId>,
    count: u32,
    appearance: Appearance,
}
impl Coats {
    fn enabled(step: &Step) -> Option<Self> {
        (step.enabled && step.coats > 0).then_some(Self {
            product_id: step.product_id,
            count: step.coats.min(100),
            appearance: step.appearance,
        })
    }
}

#[derive(PartialEq)]
struct Recipe {
    bare_sanding: Vec<u32>,
    coat_sanding: Vec<u32>,
    layers: [Option<Coats>; 3],
}
impl Recipe {
    fn from_settings(settings: &Settings) -> Self {
        let layers = [&settings.pre_stain, &settings.stain, &settings.varnish].map(Coats::enabled);
        Self {
            bare_sanding: settings
                .sanding_passes(false)
                .map(|(grit, _)| grit)
                .collect(),
            coat_sanding: if layers[2].is_some_and(|coats| coats.count > 1) {
                settings
                    .sanding_passes(true)
                    .map(|(grit, _)| grit)
                    .collect()
            } else {
                Vec::new()
            },
            layers,
        }
    }
}

pub(crate) fn appearance_changed(
    before: &benchup_core::Object,
    after: &benchup_core::Object,
) -> bool {
    before.stock_material_id != after.stock_material_id
        || before.stock_material != after.stock_material
        || before.grain_axis() != after.grain_axis()
        || (before.surface_treatment != after.surface_treatment
            || before.surface_treatment_customized != after.surface_treatment_customized)
            && before
                .surface_treatment_override()
                .map(Recipe::from_settings)
                != after
                    .surface_treatment_override()
                    .map(Recipe::from_settings)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Surface {
    color: [u8; 3],
    // Grain contrast, sanding traces, grit frequency, and specular sheen.
    params: [u8; 4],
}

impl Surface {
    fn from_recipe(wood: Wood, recipe: &Recipe, products: &BTreeMap<MaterialId, Layer>) -> Self {
        fn srgb_to_linear(v: u8) -> f32 {
            let v = f32::from(v) / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        }
        let mut color = Vec3::from_array(wood.color.map(srgb_to_linear));
        let mut grain = wood.grain;
        let mut scratch = 0.12;
        let mut frequency = 0.22;
        let mut sheen = 0.;
        let mut conditioner = 0.;
        let mut film = 0.;
        let sand = |grit: u32, scratch: &mut f32, frequency: &mut f32| {
            let tooth = (100. / grit.max(25) as f32).sqrt().min(1.);
            // Each pass leaves its own traces and can remove much of the previous pass.
            *scratch = *scratch * 0.31 + tooth * tooth * 0.56;
            *frequency = (grit.max(25) as f32 / 80.)
                .log2()
                .mul_add(0.16, 0.26)
                .clamp(0., 1.);
            tooth
        };
        for &grit in &recipe.bare_sanding {
            sand(grit, &mut scratch, &mut frequency);
        }
        for (index, (stage, coats)) in
            [FinishType::PreStain, FinishType::Stain, FinishType::Varnish]
                .into_iter()
                .zip(recipe.layers)
                .enumerate()
        {
            let Some(coats) = coats else { continue };
            let layer = coats
                .product_id
                .and_then(|id| products.get(&id).copied())
                .unwrap_or_else(|| Layer::new(stage, Formula::Standard, stage.preset()))
                .apply(coats.appearance);
            let amount = f32::from(layer.strength) / 100.;
            let tint = layer
                .tint
                .map(|tint| Vec3::from_array(tint.map(srgb_to_linear)));
            for coat in 1..=coats.count {
                match layer.kind {
                    FinishType::PreStain => {
                        conditioner += (1. - conditioner) * amount;
                        if let Some(tint) = tint {
                            color = color.lerp(tint, amount * 0.18);
                        }
                    }
                    FinishType::Stain | FinishType::Oil => {
                        let absorption = amount
                            * (0.58 + 0.34 * scratch)
                            * (1. - 0.42 * conditioner)
                            * (1. - 0.85 * film);
                        let (tint_gain, grain_gain) = match layer.formula {
                            Formula::Dye => (0.9, 0.16),
                            Formula::Pigment => (1.1, 0.38),
                            Formula::Gel => (1.2, -0.5),
                            _ => (1., 0.2),
                        };
                        if let Some(tint) = tint {
                            color = color.lerp(tint, absorption * tint_gain);
                        }
                        grain = (grain * (1. + absorption * grain_gain)).min(1.);
                    }
                    _ => {
                        let opacity = match layer.kind {
                            FinishType::Paint | FinishType::Primer => amount,
                            FinishType::Varnish if layer.formula == Formula::OilBased => {
                                amount * 0.12
                            }
                            _ => amount * 0.22,
                        };
                        if let Some(tint) = tint {
                            color = color.lerp(tint, opacity);
                        }
                        grain *= 1. - opacity * 0.96;
                        scratch *= 1. - amount * 0.82;
                        film += (1. - film) * amount * 0.72;
                    }
                }
                let target = match layer.sheen {
                    Sheen::Matte => 0.07,
                    Sheen::Satin => 0.42,
                    Sheen::Gloss => 1.,
                };
                let build = match layer.kind {
                    FinishType::PreStain => 0.07,
                    FinishType::Stain => 0.15,
                    _ => 0.88,
                };
                sheen += (target - sheen) * amount * build;

                // The estimate schedules intercoat sanding after every coat except
                // the last. Abrade that film before the next coat fills the marks.
                if index == 2 && coat < coats.count {
                    for &grit in &recipe.coat_sanding {
                        let tooth = sand(grit, &mut scratch, &mut frequency);
                        film *= 1. - 0.45 * tooth;
                        sheen *= 1. - 0.62 * tooth;
                    }
                }
            }
        }
        let byte = |value: f32| (value.clamp(0., 1.) * 255.).round() as u8;
        let linear_to_srgb = |v: f32| {
            byte(if v <= 0.0031308 {
                12.92 * v
            } else {
                1.055 * v.powf(1. / 2.4) - 0.055
            })
        };
        Self {
            color: color.to_array().map(linear_to_srgb),
            params: [byte(grain), byte(scratch), byte(frequency), byte(sheen)],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FaceMaterial {
    pub color: [u8; 3],
    pub params: [u8; 4],
    local: DMat3,
    origin: DVec3,
}

impl FaceMaterial {
    pub(crate) fn wood_glue(
        plane: benchup_core::faces::Plane,
        frame: benchup_core::InstanceTransform,
    ) -> Self {
        let linear = frame.linear();
        let inverse = linear.inverse();
        // Use the part's axes, rather than the face's arbitrary first edge.
        // Canonicalize the normal too: reversing a face must not mirror the
        // curves on an otherwise identical coplanar coating.
        let mut normal = (linear.transpose() * plane.normal).normalize();
        let dominant = normal.abs().max_position();
        if normal[dominant] < 0. {
            normal = -normal;
        }
        let axis = if normal.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
        let along = (axis - normal * axis.dot(normal)).normalize();
        let across = normal.cross(along);
        Self {
            color: [0; 3], // The receiving face supplies the base color.
            params: [255, 255, 255, 0],
            local: DMat3::from_cols(along, across, normal).transpose() * inverse,
            origin: plane.origin,
        }
    }

    /// The model keeps vertices in world space. Sample in the inverse instance
    /// frame so moving, rotating, mirroring or scaling does not swim the grain.
    pub fn coordinates(self, point: DVec3) -> [f32; 3] {
        (self.local * (point - self.origin)).as_vec3().to_array()
    }
}

pub(crate) struct Materials<'a> {
    model: &'a Model,
    entities: &'a EntityCache,
    catalog: Option<&'a Catalog>,
    objects: HashMap<u64, Option<FaceMaterial>>,
}

impl<'a> Materials<'a> {
    pub fn new(model: &'a Model, entities: &'a EntityCache, catalog: Option<&'a Catalog>) -> Self {
        Self {
            model,
            entities,
            catalog,
            objects: HashMap::new(),
        }
    }

    pub fn for_face(&mut self, face: u64) -> Option<FaceMaterial> {
        let catalog = self.catalog?;
        let owner = self.entities.parent(face)?;
        *self.objects.entry(owner).or_insert_with(|| {
            let object = self.model.objects.get(&owner)?;
            let mut stock_owner = object;
            let wood = loop {
                if let Some(id) = stock_owner.stock_material_id {
                    break catalog.wood.get(&id).copied().unwrap_or_default();
                }
                if let Some(legacy) = &stock_owner.stock_material {
                    break catalog.legacy_wood.get(legacy).copied().unwrap_or_default();
                }
                stock_owner = self
                    .model
                    .objects
                    .get(&self.entities.parent(stock_owner.id)?)?;
            };
            let recipe = object
                .surface_treatment_override()
                .map(Recipe::from_settings);
            let surface = Surface::from_recipe(
                wood,
                recipe.as_ref().unwrap_or(&catalog.defaults),
                &catalog.products,
            );
            let frame = object.transform.unwrap_or_default();
            let axis = object
                .grain_axis()
                .and_then(DVec3::try_normalize)
                .unwrap_or(DVec3::X);
            let helper = if axis.z.abs() < 0.9 {
                DVec3::Z
            } else {
                DVec3::Y
            };
            let across = helper.cross(axis).normalize();
            let local = DMat3::from_cols(axis, across, axis.cross(across)).transpose()
                * frame.linear().inverse();
            local.is_finite().then_some(FaceMaterial {
                color: surface.color,
                params: surface.params,
                local,
                origin: frame.translation,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glue_direction_is_independent_of_face_edge_order_and_winding() {
        use benchup_core::{InstanceTransform, faces::Plane};
        for normal in [DVec3::Z, DVec3::X, DVec3::new(0.2, 0.7, 0.5).normalize()] {
            let u = normal.any_orthonormal_vector();
            let v = normal.cross(u);
            let origin = DVec3::new(0.3, -0.2, 1.);
            let first = FaceMaterial::wood_glue(Plane { origin, u, v, normal }, InstanceTransform::default());
            for (u, v, normal) in [(v, -u, normal), (-u, v, -normal)] {
                let second = FaceMaterial::wood_glue(Plane {
                    origin: origin + u * 0.5, u, v, normal,
                }, InstanceTransform::default());
                for offset in [u * 0.12, v * 0.08] {
                    let delta = |material: FaceMaterial| {
                        Vec3::from_array(material.coordinates(origin + offset))
                            - Vec3::from_array(material.coordinates(origin))
                    };
                    assert!((delta(first) - delta(second)).length() < 1e-6);
                }
            }
        }
    }

    #[test]
    fn glue_coordinates_follow_rotated_mirrored_and_scaled_parts() {
        use benchup_core::{InstanceTransform, faces::Plane};
        let plane = Plane { origin: DVec3::new(0.2, 0.4, 0.), u: DVec3::X, v: DVec3::Y, normal: DVec3::Z };
        let original = FaceMaterial::wood_glue(plane, InstanceTransform::default());
        for axes in [DVec3::ONE, DVec3::new(-2., 0.5, 3.)] {
            let frame = InstanceTransform {
                translation: DVec3::new(7., -2., 4.),
                basis: DMat3::from_rotation_y(0.7) * DMat3::from_rotation_z(1.1),
                axes,
                scale: 1.7,
            };
            let linear = frame.linear();
            let world = |p| linear * p + frame.translation;
            let moved = FaceMaterial::wood_glue(Plane {
                origin: world(plane.origin),
                u: (linear * plane.u).normalize(),
                v: (linear * plane.v).normalize(),
                normal: (linear.inverse().transpose() * plane.normal).normalize(),
            }, frame);
            let point = DVec3::new(0.47, 0.61, 0.);
            let error = Vec3::from_array(moved.coordinates(world(point)))
                - Vec3::from_array(original.coordinates(point));
            assert!(error.length() < 1e-6);
        }
    }

    fn render(settings: &Settings, catalog: &Catalog) -> Surface {
        Surface::from_recipe(
            Wood::default(),
            &Recipe::from_settings(settings),
            &catalog.products,
        )
    }

    #[test]
    fn sanding_order_conditioner_stain_and_topcoat_change_the_preview() {
        let dye = finish_settings::Finish {
            formula: Formula::Dye,
            appearance: Appearance {
                tint: Some([90, 43, 24]),
                ..Default::default()
            },
            ..Default::default()
        };
        let varnish = finish_settings::Finish {
            finish_type: FinishType::Varnish,
            formula: Formula::OilBased,
            appearance: Appearance {
                sheen: Some(Sheen::Gloss),
                ..Default::default()
            },
            ..Default::default()
        };
        let stock = stock_settings::Settings {
            finishes: vec![dye.clone(), varnish.clone()],
            ..Default::default()
        };
        let mut recipe = Settings::default();
        let catalog = Catalog::new(&stock, &recipe);
        let raw = render(&recipe, &catalog);
        recipe.sanding[0].enabled = true;
        recipe.sanding[4].enabled = true;
        let sanded = render(&recipe, &catalog);
        assert_ne!(raw.params, sanded.params);
        recipe.sanding_grits.swap(0, 4);
        assert_ne!(sanded.params, render(&recipe, &catalog).params);

        recipe.stain.enabled = true;
        recipe.stain.product_id = Some(dye.id);
        let stained = render(&recipe, &catalog);
        assert_ne!(sanded.color, stained.color);
        recipe.stain.coats = 2;
        assert_ne!(stained.color, render(&recipe, &catalog).color);
        let two_coats = render(&recipe, &catalog);
        recipe.pre_stain.enabled = true;
        let conditioned = render(&recipe, &catalog);
        assert_ne!(two_coats.color, conditioned.color);

        recipe.varnish.enabled = true;
        recipe.varnish.product_id = Some(varnish.id);
        recipe.sanding[5].enabled = true;
        recipe.sanding_after_varnish[5] = true;
        let gloss = render(&recipe, &catalog);
        assert_ne!(conditioned.color, gloss.color);
        recipe.sanding[5].enabled = false;
        assert_ne!(gloss.params, render(&recipe, &catalog).params);
        recipe.varnish.appearance.sheen = Some(Sheen::Satin);
        let satin = render(&recipe, &catalog);
        recipe.varnish.appearance.sheen = Some(Sheen::Matte);
        let matte = render(&recipe, &catalog);
        assert!(gloss.params[3] > satin.params[3] && satin.params[3] > matte.params[3]);

        let mut changed = stock.clone();
        changed.finishes[0].formula = Formula::Gel;
        let gel_catalog = Catalog::new(&changed, &recipe);
        assert_ne!(matte.color, render(&recipe, &gel_catalog).color);
        changed.finishes[1].formula = Formula::WaterBased;
        assert_ne!(
            render(&recipe, &gel_catalog).color,
            render(&recipe, &Catalog::new(&changed, &recipe)).color
        );
    }

    #[test]
    fn quote_edits_do_not_invalidate_the_visual_catalog() {
        let mut stock = stock_settings::Settings {
            finishes: vec![finish_settings::Finish::default()],
            ..Default::default()
        };
        let mut defaults = Settings::default();
        defaults.stain.enabled = true;
        defaults.stain.product_id = Some(stock.finishes[0].id);
        let before = Catalog::new(&stock, &defaults);
        defaults.stain.minutes_per_m2 = 42.;
        defaults.stain.drying_hours = 16.;
        defaults.stain.final_drying_hours = Some(24.);
        stock.finishes[0].price = 55.;
        stock.finishes[0].coverage_m2_per_container = Some(7.);
        let after = Catalog::new(&stock, &defaults);
        assert!(before == after);
        defaults.stain.appearance.tint = Some([70, 45, 20]);
        assert!(after != Catalog::new(&stock, &defaults));
    }

    #[test]
    fn grain_is_in_the_part_frame_even_when_stock_is_inherited() {
        let mut model = Model::default();
        model
            .draw_polyline(
                &[
                    DVec3::ZERO,
                    DVec3::X,
                    DVec3::X + DVec3::Y,
                    DVec3::Y,
                    DVec3::ZERO,
                ],
                false,
            )
            .unwrap();
        let part = model
            .make_object(&model.root_entities(), false, "Part")
            .unwrap();
        let assembly = model
            .make_object(&std::collections::BTreeSet::from([part]), false, "Assembly")
            .unwrap();
        let face = *model.faces.keys().next().unwrap();
        let part = model.objects[&assembly]
            .entities
            .iter()
            .copied()
            .find(|id| model.objects.contains_key(id))
            .unwrap();
        model.objects.get_mut(&assembly).unwrap().stock_material_id = Some(MaterialId::new_v4());
        model.edit_context = Some(assembly);
        model.set_grain_direction(part, Some(DVec3::Y)).unwrap();
        model.edit_context = None;

        let catalog = Catalog::new(&stock_settings::Settings::default(), &Settings::default());
        let lookup = |model: &Model| {
            let mut entities = EntityCache::default();
            entities.update(model);
            Materials::new(model, &entities, Some(&catalog))
                .for_face(face)
                .unwrap()
        };
        let material = lookup(&model);
        let p = DVec3::new(0.17, 0.61, 0.);
        let initial = material.coordinates(p);
        assert!((initial[0] - 0.61).abs() < 1e-6);
        let roots = std::collections::BTreeSet::from([assembly]);
        let delta = DVec3::new(7., -2., 4.);
        model.move_selection(&roots, delta).unwrap();
        model
            .rotate_selection(&roots, DVec3::ZERO, DVec3::Z, 90.)
            .unwrap();
        model.scale_selection(&roots, DVec3::ZERO, 2.).unwrap();
        let world = 2. * (DMat3::from_rotation_z(std::f64::consts::FRAC_PI_2) * (p + delta));
        let moved = lookup(&model);
        assert_eq!(moved.color, material.color);
        let error = Vec3::from_array(moved.coordinates(world)) - Vec3::from_array(initial);
        assert!(error.length() < 1e-6);
    }
}
