//! Incremental visible bounds. Editing one part updates its ancestors only.
use super::*;
use std::collections::{BTreeSet, HashMap};
type Aabb = (DVec3, DVec3);
fn coordinate(value: f64) -> u64 {
    let bits = value.to_bits();
    if bits >> 63 != 0 {
        !bits
    } else {
        bits ^ (1 << 63)
    }
}
fn value(key: u64) -> f64 {
    f64::from_bits(if key >> 63 != 0 {
        key ^ (1 << 63)
    } else {
        !key
    })
}
#[derive(Default)]
struct Node {
    axes: [BTreeMap<u64, usize>; 6],
}
impl Node {
    fn change(&mut self, bounds: Aabb, add: bool) {
        for axis in 0..3 {
            for (index, point) in [(axis, bounds.0[axis]), (axis + 3, bounds.1[axis])] {
                let key = coordinate(point);
                if add {
                    *self.axes[index].entry(key).or_default() += 1;
                } else if let Some(count) = self.axes[index].get_mut(&key) {
                    *count -= 1;
                    if *count == 0 {
                        self.axes[index].remove(&key);
                    }
                }
            }
        }
    }
    fn bounds(&self) -> Option<Aabb> {
        if self.axes[0].is_empty() {
            return None;
        }
        Some((
            DVec3::from_array(std::array::from_fn(|axis| {
                value(*self.axes[axis].first_key_value().unwrap().0)
            })),
            DVec3::from_array(std::array::from_fn(|axis| {
                value(*self.axes[axis + 3].last_key_value().unwrap().0)
            })),
        ))
    }
}
#[derive(Default)]
pub(super) struct Index {
    nodes: HashMap<Option<u64>, Node>,
    parents: HashMap<u64, Option<u64>>,
    objects: HashMap<u64, Aabb>,
    items: HashMap<u64, (Option<u64>, Aabb)>,
    guides: HashMap<u64, DVec3>,
    guide_bounds: Node,
}
impl Index {
    pub fn new(
        model: &Model,
        entities: &EntityCache,
        display: &benchup_core::DisplaySettings,
    ) -> Self {
        let mut index = Self::default();
        let dirty = model
            .edges
            .keys()
            .chain(model.faces.keys())
            .copied()
            .collect();
        index.update(model, entities, display, &dirty);
        index
    }
    pub fn bounds(&self, context: Option<u64>) -> Option<Aabb> {
        let geometry = self.nodes.get(&context).and_then(Node::bounds);
        if context.is_some() {
            return geometry;
        }
        match (geometry, self.guide_bounds.bounds()) {
            (Some((a, b)), Some((c, d))) => Some((a.min(c), b.max(d))),
            (a, b) => a.or(b),
        }
    }
    pub fn update(
        &mut self,
        model: &Model,
        entities: &EntityCache,
        display: &benchup_core::DisplaySettings,
        dirty: &BTreeSet<u64>,
    ) {
        let mut pending = BTreeSet::new();
        for object in model.objects.values() {
            self.nodes.entry(Some(object.id)).or_default();
            let parent = entities.parent(object.id);
            if let Some(previous) = self.parents.insert(object.id, parent) {
                if previous != parent {
                    if let Some(bounds) = self.objects.get(&object.id).copied() {
                        self.nodes
                            .entry(previous)
                            .or_default()
                            .change(bounds, false);
                        self.nodes.entry(parent).or_default().change(bounds, true);
                        pending.insert(previous);
                        pending.insert(parent);
                    }
                }
            }
        }
        for id in dirty {
            let next = if !entities.get(*id).displayed(display) {
                None
            } else if let Some(edge) = model.edges.get(id) {
                let (a, b) = model.points(edge);
                Some((entities.parent(*id), (a.min(b), a.max(b))))
            } else {
                model.faces.get(id).and_then(|face| {
                    let mut points = face
                        .vertices
                        .iter()
                        .chain(face.holes.iter().flatten())
                        .map(|id| model.vertices[id].position);
                    let first = points.next()?;
                    let bounds = points.fold((first, first), |(a, b), p| (a.min(p), b.max(p)));
                    Some((entities.parent(*id), bounds))
                })
            };
            if self.items.get(id).copied() == next {
                continue;
            }
            if let Some((parent, bounds)) = self.items.remove(id) {
                self.nodes.entry(parent).or_default().change(bounds, false);
                pending.insert(parent);
            }
            if let Some((parent, bounds)) = next {
                self.nodes.entry(parent).or_default().change(bounds, true);
                self.items.insert(*id, (parent, bounds));
                pending.insert(parent);
            }
        }
        while let Some(scope) = pending.pop_first() {
            let Some(id) = scope else {
                continue;
            };
            let next = self.nodes.get(&scope).and_then(Node::bounds);
            let previous = self.objects.get(&id).copied();
            if previous == next {
                continue;
            }
            let parent = self.parents.get(&id).copied().flatten();
            if let Some(bounds) = previous {
                self.nodes.entry(parent).or_default().change(bounds, false);
                self.objects.remove(&id);
            }
            if let Some(bounds) = next {
                self.nodes.entry(parent).or_default().change(bounds, true);
                self.objects.insert(id, bounds);
            }
            pending.insert(parent);
        }
        let guide_ids: BTreeSet<_> = self
            .guides
            .keys()
            .chain(model.guides.keys())
            .copied()
            .collect();
        for id in guide_ids {
            let next = model
                .guides
                .get(&id)
                .filter(|_| entities.get(id).displayed(display))
                .map(|guide| guide.origin);
            if self.guides.get(&id).copied() == next {
                continue;
            }
            if let Some(point) = self.guides.remove(&id) {
                self.guide_bounds.change((point, point), false);
            }
            if let Some(point) = next {
                self.guides.insert(id, point);
                self.guide_bounds.change((point, point), true);
            }
        }
        let removed: Vec<_> = self
            .parents
            .keys()
            .filter(|id| !model.objects.contains_key(id))
            .copied()
            .collect();
        for id in removed {
            self.parents.remove(&id);
            self.nodes.remove(&Some(id));
            self.objects.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extrema_follow_shared_coordinates_nested_additions_deletions_and_undo() {
        let mut model = Model::default();
        let a = model.add_edge(DVec3::splat(-4.), DVec3::splat(2.)).unwrap();
        let b = model
            .add_edge(DVec3::new(-4., 3., 0.), DVec3::new(2., 5., 1.))
            .unwrap();
        let inner = model
            .make_object(&BTreeSet::from([a, b]), false, "Inner")
            .unwrap();
        let outer = model
            .make_object(&BTreeSet::from([inner]), false, "Outer")
            .unwrap();
        let original = model.clone();
        let mut entities = EntityCache::default();
        entities.update(&model);
        let display = Default::default();
        let mut index = Index::new(&model, &entities, &display);
        for step in 0..4 {
            let before = model.clone();
            model.edit_context = Some(inner);
            match step {
                0 => {
                    model.add_edge(DVec3::splat(-8.), DVec3::splat(9.)).unwrap();
                }
                1 => model.erase(&BTreeSet::from([a])),
                2 => model.erase(&model.objects[&inner].entities.clone()),
                _ => model = original.clone(),
            }
            entities.update(&model);
            let mut dirty: BTreeSet<_> = before.edges.keys().copied().collect();
            dirty.extend(model.edges.keys());
            index.update(&model, &entities, &display, &dirty);
            assert_eq!(index.bounds(None), model.bounds());
            for id in [inner, outer] {
                assert_eq!(
                    index.bounds(Some(id)),
                    model.object_contents(id).unwrap().bounds()
                );
            }
        }
    }
}

#[cfg(test)]
mod visibility_tests {
    use super::*;
    #[test]
    fn visible_faces_keep_bounds_when_their_edges_are_hidden() {
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
                true,
            )
            .unwrap();
        model.hidden.extend(model.edges.keys());
        let mut entities = EntityCache::default();
        entities.update(&model);
        let index = Index::new(&model, &entities, &Default::default());
        assert!(model.bounds().is_some());
        assert_eq!(index.bounds(None), model.bounds());
    }
}
