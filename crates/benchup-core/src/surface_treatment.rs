//! Per-object sanding and finishing configuration, saved with project geometry.
const GRITS: [u16; 6] = [80, 120, 150, 180, 220, 320];

// Older preferences use RON tuples for fixed arrays; editable lists use RON
// sequences. Accept both so one migrated field cannot reset all preferences.
fn deserialize_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    struct List<T>(std::marker::PhantomData<T>);
    impl<'de, T: serde::Deserialize<'de>> serde::de::Visitor<'de> for List<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str(tr!(
                "core.surface_treatment.a_list_or_legacy_tuple_of_sanding_settings"
            ))
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut sequence: A,
        ) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = sequence.next_element()? {
                values.push(value);
            }
            Ok(values)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }
    }
    deserializer.deserialize_any(List(std::marker::PhantomData))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Sheen {
    Matte,
    Satin,
    Gloss,
}

/// Optional appearance overrides. Unset values come from the product or the
/// treatment stage's preset. These fields do not affect time or cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub tint: Option<[u8; 3]>,
    pub strength: Option<u8>,
    pub sheen: Option<Sheen>,
}

impl Appearance {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Step {
    pub enabled: bool,
    /// Stable ID in the finishing-product catalog; unassigned for older data.
    pub product_id: Option<crate::MaterialId>,
    pub minutes_per_m2: f64,
    pub coats: u32,
    pub drying_hours: f64,
    // None preserves the original per-coat wait for existing saved settings.
    pub final_drying_hours: Option<f64>,
    #[serde(skip_serializing_if = "Appearance::is_default")]
    pub appearance: Appearance,
}

impl Default for Step {
    fn default() -> Self {
        Self {
            enabled: false,
            product_id: None,
            minutes_per_m2: 16.,
            coats: 1,
            drying_hours: 0.,
            final_drying_hours: None,
            appearance: Appearance::default(),
        }
    }
}

impl Step {
    // Coat numbers are one-based; only the last coat uses the final wait.
    pub fn coat_estimate(&self, area: f64, coat: u32) -> Duration {
        if !self.enabled || area <= 0. {
            return Duration::default();
        }
        Duration {
            work: area * self.minutes_per_m2,
            wait: if coat == self.coats {
                self.final_drying_hours.unwrap_or(self.drying_hours)
            } else {
                self.drying_hours
            } * 60.,
        }
    }

    pub fn sanitize(&mut self, defaults: &Self, sanding: bool) {
        if !self.minutes_per_m2.is_finite() || self.minutes_per_m2 < 0. {
            self.minutes_per_m2 = defaults.minutes_per_m2;
        }
        if !self.drying_hours.is_finite() || self.drying_hours < 0. {
            self.drying_hours = defaults.drying_hours;
        }
        if self
            .final_drying_hours
            .is_some_and(|hours| !hours.is_finite() || hours < 0.)
        {
            self.final_drying_hours = defaults.final_drying_hours;
        }
        self.coats = self.coats.clamp(1, 100);
        self.appearance.strength = self.appearance.strength.map(|value| value.min(100));
        if sanding {
            self.product_id = None;
            self.coats = 1;
            self.drying_hours = 0.;
            self.final_drying_hours = None;
            self.appearance = Appearance::default();
        }
    }

    pub fn estimate(&self, area: f64) -> Duration {
        if !self.enabled || area <= 0. {
            return Duration::default();
        }
        Duration {
            work: area * self.minutes_per_m2 * f64::from(self.coats),
            wait: (self.drying_hours * f64::from(self.coats - 1)
                + self.final_drying_hours.unwrap_or(self.drying_hours))
                * 60.,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    pub sanding_grits: Vec<u32>,
    #[serde(deserialize_with = "deserialize_list")]
    pub sanding: Vec<Step>,
    #[serde(deserialize_with = "deserialize_list")]
    pub sanding_after_varnish: Vec<bool>,
    pub pre_stain: Step,
    pub stain: Step,
    pub varnish: Step,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sanding_grits: GRITS.iter().map(|&grit| u32::from(grit)).collect(),
            sanding: vec![Step::default(); GRITS.len()],
            sanding_after_varnish: vec![false; GRITS.len()],
            pre_stain: Step {
                minutes_per_m2: 5.,
                drying_hours: 0.25,
                ..Step::default()
            },
            stain: Step {
                minutes_per_m2: 5.,
                drying_hours: 8.,
                ..Step::default()
            },
            varnish: Step {
                minutes_per_m2: 6.,
                coats: 3,
                drying_hours: 6.,
                final_drying_hours: Some(24.),
                ..Step::default()
            },
        }
    }
}

/// An estimate step identified independently of its translated label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Task {
    Sanding(u32),
    PreStain(u32),
    Stain(u32),
    Varnish {
        coat: u32,
        sanding_grit: Option<u32>,
    },
}

impl Task {
    pub fn label(self) -> String {
        match self {
            Self::Sanding(grit) => tr!("core.surface_treatment.sanding_grit", grit = grit),
            Self::PreStain(coat) => tr!("core.surface_treatment.pre_stain_coat", coat = coat),
            Self::Stain(coat) => tr!("core.surface_treatment.stain_coat", coat = coat),
            Self::Varnish {
                coat,
                sanding_grit: None,
            } => {
                tr!("core.surface_treatment.varnish_coat", coat = coat)
            }
            Self::Varnish {
                coat,
                sanding_grit: Some(grit),
            } => {
                tr!(
                    "core.surface_treatment.sanding_grit_after_coat",
                    grit = grit,
                    coat = coat
                )
            }
        }
    }

    pub fn short_label(self) -> String {
        match self {
            Self::Sanding(grit) => tr!("core.surface_treatment.grit", grit = grit),
            Self::Varnish {
                coat,
                sanding_grit: Some(grit),
            } => {
                tr!(
                    "core.surface_treatment.grit_after_coat",
                    grit = grit,
                    coat = coat
                )
            }
            _ => self.label(),
        }
    }
}

impl Settings {
    /// Sanding order follows the recipe's row order in both the appearance
    /// preview and the time estimate.
    pub fn sanding_passes(&self, between_coats: bool) -> impl Iterator<Item = (u32, &Step)> {
        self.sanding
            .iter()
            .zip(&self.sanding_grits)
            .zip(&self.sanding_after_varnish)
            .filter_map(move |((step, &grit), &between)| {
                (step.enabled && between == between_coats).then_some((grit, step))
            })
    }

    pub fn sanitize(&mut self) {
        let defaults = Self::default();
        self.sanding
            .resize(self.sanding_grits.len(), Step::default());
        self.sanding_after_varnish
            .resize(self.sanding_grits.len(), false);
        for grit in &mut self.sanding_grits {
            *grit = (*grit).max(1);
        }
        for step in &mut self.sanding {
            step.sanitize(&Step::default(), true);
        }
        self.pre_stain.sanitize(&defaults.pre_stain, false);
        self.stain.sanitize(&defaults.stain, false);
        self.varnish.sanitize(&defaults.varnish, false);
    }

    pub fn any_enabled(&self) -> bool {
        self.tasks(0.).next().is_some()
    }

    pub fn task_estimates(&self, area: f64) -> impl Iterator<Item = (String, Duration)> {
        self.tasks(area)
            .map(|(task, duration)| (task.label(), duration))
    }

    pub fn tasks(&self, area: f64) -> impl Iterator<Item = (Task, Duration)> {
        let mut tasks = Vec::new();
        for (grit, step) in self.sanding_passes(false) {
            tasks.push((Task::Sanding(grit), step.estimate(area)));
        }
        for (step, task) in [
            (&self.pre_stain, Task::PreStain as fn(u32) -> Task),
            (&self.stain, Task::Stain),
            (&self.varnish, |coat| Task::Varnish {
                coat,
                sanding_grit: None,
            }),
        ] {
            if !step.enabled {
                continue;
            }
            for coat in 1..=step.coats {
                let task = task(coat);
                tasks.push((task, step.coat_estimate(area, coat)));
                if matches!(task, Task::Varnish { .. }) && coat < step.coats {
                    for (grit, sanding) in self.sanding_passes(true) {
                        tasks.push((
                            Task::Varnish {
                                coat,
                                sanding_grit: Some(grit),
                            },
                            sanding.estimate(area),
                        ));
                    }
                }
            }
        }
        tasks.into_iter()
    }

    pub fn estimate(&self, area: f64) -> Duration {
        self.tasks(area)
            .fold(Duration::default(), |total, (_, step)| Duration {
                work: total.work + step.work,
                wait: total.wait + step.wait,
            })
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Duration {
    pub work: f64,
    pub wait: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_visual_overrides_do_not_change_the_estimate() {
        let mut settings = Settings::default();
        settings.varnish.enabled = true;
        let estimate = settings.estimate(2.);
        let mut legacy = serde_json::to_value(&settings).unwrap();
        legacy["varnish"].as_object_mut().unwrap().remove("appearance");
        let old: Settings = serde_json::from_value(legacy).unwrap();
        assert_eq!(old.varnish.appearance, Appearance::default());
        settings.varnish.appearance = Appearance {
            tint: Some([133, 60, 38]),
            strength: Some(47),
            sheen: Some(Sheen::Matte),
        };
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        assert_eq!(restored.estimate(2.), estimate);
    }

    #[test]
    fn legacy_grits_migrate_and_custom_grits_round_trip_with_estimates() {
        let mut legacy = serde_json::to_value(Settings::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("sanding_grits");
        let mut settings: Settings = serde_json::from_value(legacy).unwrap();
        settings.sanitize();
        assert_eq!(settings.sanding_grits, vec![80, 120, 150, 180, 220, 320]);
        settings.sanding_grits = vec![400, 600, 1000];
        settings.sanding = vec![
            Step {
                enabled: true,
                minutes_per_m2: 2.,
                ..Step::default()
            };
            3
        ];
        settings.sanding_after_varnish = vec![false; 3];
        settings.sanitize();
        let restored: Settings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        assert_eq!(restored.estimate(2.).work, 12.);
        assert_eq!(
            restored.task_estimates(2.).last().unwrap().0,
            "Sanding · 1000 grit"
        );
        settings.sanding_grits.clear();
        settings.sanitize();
        assert!(settings.sanding.is_empty());
        assert!(!settings.any_enabled());
    }
}
