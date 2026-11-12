//! Selection and editing scope are small appearance updates, independent of
//! triangulation and vertex buffers. Object states propagate through cached
//! ancestry on the CPU; each GPU vertex reads only its owner and its own state.
use super::*;
use std::collections::{BTreeMap, BTreeSet};
const WIDTH: usize = 512;
pub(super) const SELECTED: u32 = 1;
pub(super) const HOVERED: u32 = 2;
pub(super) const ACTIVE: u32 = 4;
pub(super) const OUTLINER: u32 = 8;
pub(super) const LOCKED: u32 = 16;
pub(super) const ERASING: u32 = 32;
pub(super) const REMOVED: u32 = 64;
#[derive(Default)]
pub(super) struct Cache {
    key: Option<Key>,
    pub values: Arc<Vec<u32>>,
}
#[derive(Clone, PartialEq)]
struct Key {
    selected: BTreeSet<u64>,
    hover: Option<u64>,
    outliner: BTreeSet<u64>,
    context: Option<u64>,
    erasing: BTreeSet<u64>,
    removed: BTreeSet<u64>,
}
#[derive(Clone)]
pub(super) struct Hierarchy {
    slots: BTreeMap<u64, u32>,
    parents: BTreeMap<u64, u64>,
    objects: Vec<(u64, Vec<u64>)>,
    base: Vec<u32>,
}
impl Hierarchy {
    pub fn new(model: &Model, entities: &EntityCache, _faces: &FaceCache) -> Self {
        let mut parents: BTreeMap<_, _> = model
            .objects
            .values()
            .flat_map(|o| o.entities.iter().map(|id| (*id, o.id)))
            .collect();
        for (&id, control) in model.routing_appearance.endpoint_controls() {
            if let Some(parent) = control.parent.filter(|id| model.objects.contains_key(id)) {
                parents.insert(id, parent);
            }
        }
        let slots: BTreeMap<_, _> = model
            .objects
            .keys()
            .chain(model.faces.keys())
            .chain(model.edges.keys())
            .chain(model.dimensions.keys())
            .chain(model.routing_appearance.edge_controls.keys().filter(|id| !model.contains_entity(*id)))
            .chain(model.routing_appearance.stock_edge_controls.keys().filter(|id| !model.contains_entity(*id)))
            .enumerate()
            .map(|(i, id)| (*id, i as u32 + 1))
            .collect();
        let objects: Vec<_> = model
            .objects
            .keys()
            .map(|&id| {
                let mut path = vec![id];
                while let Some(&parent) = parents.get(path.last().unwrap()) {
                    path.push(parent);
                }
                path.reverse();
                (id, path)
            })
            .collect();
        let mut base = vec![0; (slots.len() + 1).div_ceil(WIDTH) * WIDTH];
        for (&id, &slot) in &slots {
            if entities.get(id).locked || model.routing_appearance.endpoint_control(id).is_some_and(|c| c.locked) {
                base[slot as usize] |= LOCKED;
            }
        }
        Self {
            slots,
            parents,
            objects,
            base,
        }
    }
    pub fn synchronize(
        &mut self,
        source: &Model,
        model: &Model,
        entities: &EntityCache,
        dirty: &BTreeSet<u64>,
    ) {
        for id in model
            .objects
            .keys()
            .chain(dirty.iter())
            .filter(|id| model.contains_entity(id))
        {
            if !self.slots.contains_key(id) {
                let slot = self.slots.len() as u32 + 1;
                self.slots.insert(*id, slot);
            }
        }
        self.base
            .resize((self.slots.len() + 1).div_ceil(WIDTH) * WIDTH, 0);
        // Include the preceding edit when undo restores the original geometry.
        // Its ownership then matches the base, but differs from the last frame.
        let mut ownership = dirty.clone();
        ownership.extend(model.objects.keys());
        for (&id, before) in &source.objects {
            if let Some(after) = model.objects.get(&id) {
                ownership.extend(before.entities.symmetric_difference(&after.entities));
            } else {
                ownership.extend(&before.entities);
            }
        }
        for object in model
            .objects
            .values()
            .filter(|object| !source.objects.contains_key(&object.id))
        {
            ownership.extend(&object.entities);
            ownership.insert(object.id);
        }
        for id in ownership {
            if let Some(parent) = entities.parent(id) {
                self.parents.insert(id, parent);
            } else {
                self.parents.remove(&id);
            }
        }
        self.objects = model
            .objects
            .keys()
            .map(|&id| {
                let mut path = vec![id];
                while let Some(&parent) = self.parents.get(path.last().unwrap()) {
                    path.push(parent);
                }
                path.reverse();
                (id, path)
            })
            .collect();
        for id in model.objects.keys().chain(dirty.iter()) {
            let slot = self.slot(*id) as usize;
            if slot != 0 {
                self.base[slot] = if entities.get(*id).locked { LOCKED } else { 0 };
            }
        }
    }
    pub fn slot(&self, id: u64) -> u32 {
        self.slots.get(&id).copied().unwrap_or(0)
    }
    pub fn pair(&self, id: u64) -> [u32; 2] {
        [
            self.parents.get(&id).map_or(0, |id| self.slot(*id)),
            self.slot(id),
        ]
    }
    pub fn update(
        &self,
        cache: &mut Cache,
        key: &SceneKey,
        removed: &BTreeSet<u64>,
    ) -> Arc<Vec<u32>> {
        // Primitive hover is already a uniform lookup in the vertex shader.
        // Only assembly hover needs inherited state in the interaction texture.
        let hover = key.hover.filter(|id| self.objects.iter().any(|(object, _)| object == id));
        if cache.key.as_ref().is_some_and(|previous|
            previous.hover == hover && previous.context == key.edit_context
                && previous.selected == key.selected && previous.outliner == key.outliner_hover
                && previous.erasing == key.erasing && previous.removed == *removed)
        {
            return Arc::clone(&cache.values);
        }
        let next = Key {
            selected: key.selected.clone(), hover,
            outliner: key.outliner_hover.clone(), context: key.edit_context,
            erasing: key.erasing.clone(), removed: removed.clone(),
        };
        if cache.values.len() != self.base.len() {
            cache.values = Arc::new(self.base.clone());
        }
        let flags = Arc::make_mut(&mut cache.values);
        if let Some(previous) = &cache.key {
            for id in previous
                .selected
                .iter()
                .chain(&previous.outliner)
                .chain(&previous.erasing)
                .chain(&previous.removed)
            {
                let slot = self.slot(*id) as usize;
                if slot != 0 {
                    flags[slot] = self.base[slot];
                }
            }
        }
        flags[0] = if next.context.is_none() { ACTIVE } else { 0 };
        for (id, path) in &self.objects {
            let active = next.context.is_none() || path.contains(&next.context.unwrap());
            let root = if let Some(scope) = next.context {
                path.iter()
                    .position(|id| *id == scope)
                    .and_then(|i| path.get(i + 1))
                    .copied()
            } else {
                path.first().copied()
            };
            let mut state = self.base[self.slot(*id) as usize];
            if active {
                state |= ACTIVE;
            }
            if path.iter().any(|id| next.selected.contains(id)) {
                state |= SELECTED;
            }
            if root.is_some() && root == next.hover {
                state |= HOVERED;
            }
            if path.iter().any(|id| next.outliner.contains(id)) {
                state |= OUTLINER;
            }
            if path.iter().any(|id| next.erasing.contains(id)) {
                state |= ERASING;
            }
            flags[self.slot(*id) as usize] = state;
        }
        for (&bit, ids) in [&SELECTED, &OUTLINER, &ERASING, &REMOVED].into_iter().zip([
            &next.selected,
            &next.outliner,
            &next.erasing,
            &next.removed,
        ]) {
            for id in ids {
                let slot = self.slot(*id) as usize;
                if slot != 0 {
                    flags[slot] |= bit;
                }
            }
        }
        cache.key = Some(next);
        Arc::clone(&cache.values)
    }
}
pub(super) struct StateTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    uploaded: Mutex<Option<Arc<Vec<u32>>>>,
}
impl StateTexture {
    pub fn new(device: &wgpu::Device, count: usize) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Resident interaction states"),
            size: wgpu::Extent3d {
                width: WIDTH as u32,
                height: (count / WIDTH) as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self {
            texture,
            view,
            uploaded: Mutex::new(None),
        }
    }
    pub fn update(&self, queue: &wgpu::Queue, values: &Arc<Vec<u32>>) {
        let mut previous = self.uploaded.lock().unwrap();
        if previous
            .as_ref()
            .is_some_and(|old| Arc::ptr_eq(old, values))
        {
            return;
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(values),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH as u32 * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: WIDTH as u32,
                height: (values.len() / WIDTH) as u32,
                depth_or_array_layers: 1,
            },
        );
        *previous = Some(Arc::clone(values));
    }
}
