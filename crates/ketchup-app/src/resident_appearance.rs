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
}
pub(super) struct Hierarchy {
    slots: BTreeMap<u64, u32>,
    parents: BTreeMap<u64, u64>,
    objects: Vec<(u64, Vec<u64>)>,
    base: Vec<u32>,
    pub bounds: BTreeMap<u64, (DVec3, DVec3)>,
}
impl Hierarchy {
    pub fn new(model: &Model, entities: &EntityCache, faces: &FaceCache) -> Self {
        let parents: BTreeMap<_, _> = model
            .objects
            .values()
            .flat_map(|o| o.entities.iter().map(|id| (*id, o.id)))
            .collect();
        let slots: BTreeMap<_, _> = model
            .objects
            .keys()
            .chain(model.faces.keys())
            .chain(model.edges.keys())
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
            if entities.get(id).locked {
                base[slot as usize] |= LOCKED;
            }
        }
        let mut bounds = BTreeMap::new();
        for (&id, &slot) in &slots {
            let _ = slot;
            let state = entities.get(id);
            if state.hidden {
                continue;
            }
            let points = if let Some(edge) = model.edges.get(&id) {
                let (a, b) = model.points(edge);
                Some((a.min(b), a.max(b)))
            } else if model.faces.contains_key(&id) {
                Some(faces.get(id).bounds)
            } else {
                None
            };
            if let Some((lo, hi)) = points {
                let mut current = id;
                while let Some(&parent) = parents.get(&current) {
                    bounds
                        .entry(parent)
                        .and_modify(|(a, b): &mut (DVec3, DVec3)| {
                            *a = a.min(lo);
                            *b = b.max(hi);
                        })
                        .or_insert((lo, hi));
                    current = parent;
                }
            }
        }
        Self {
            slots,
            parents,
            objects,
            base,
            bounds,
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
    pub fn update(&self, cache: &mut Cache, key: &SceneKey) -> Arc<Vec<u32>> {
        let next = Key {
            selected: key.selected.clone(),
            hover: key.hover,
            outliner: key.outliner_hover.clone(),
            context: key.edit_context,
        };
        if cache.key.as_ref() == Some(&next) {
            return Arc::clone(&cache.values);
        }
        if cache.values.len() != self.base.len() {
            cache.values = Arc::new(self.base.clone());
        }
        let flags = Arc::make_mut(&mut cache.values);
        if let Some(previous) = &cache.key {
            for id in previous.selected.iter().chain(&previous.outliner) {
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
            flags[self.slot(*id) as usize] = state;
        }
        for (&bit, ids) in [&SELECTED, &OUTLINER]
            .into_iter()
            .zip([&next.selected, &next.outliner])
        {
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
