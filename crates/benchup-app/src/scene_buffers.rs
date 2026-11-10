//! Retained viewport buffers, split at triangle boundaries for device limits.
use super::Vertex;
use eframe::wgpu;
use std::{collections::HashMap, ops::Range};

pub(super) struct GpuBuffers {
    pub(super) chunks: Vec<GpuChunk>,
}

pub(super) struct GpuChunk {
    pub(super) vertices: wgpu::Buffer,
    pub(super) indices: wgpu::Buffer,
    index_range: Range<u32>,
}

// Keep allocation growth within the device limit, including devices whose
// limit is not a power of two. Buffer uploads are always four-byte aligned.
fn allocation_size(required: u64, limit: u64) -> u64 {
    let limit = limit / wgpu::COPY_BUFFER_ALIGNMENT * wgpu::COPY_BUFFER_ALIGNMENT;
    assert!(required <= limit);
    required
        .max(4096)
        .checked_next_power_of_two()
        .unwrap_or(limit)
        .min(limit)
}

impl GpuBuffers {
    pub(super) fn upload(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        vertices: &[Vertex],
        indices: &[u32],
        cached: Option<Self>,
    ) -> Self {
        let limit = device.limits().max_buffer_size;
        let mut cached = cached.into_iter().flat_map(|buffers| buffers.chunks);
        let mut chunks = Vec::new();
        for_each_batch(
            vertices,
            indices,
            limit,
            |index_start, vertices, indices| {
                let old = cached.next();
                let (old_vertices, old_indices) = old
                    .map(|chunk| (Some(chunk.vertices), Some(chunk.indices)))
                    .unwrap_or_default();
                let upload = |data: &[u8], old: Option<wgpu::Buffer>, usage, label| {
                    let buffer = old
                        .filter(|buffer| {
                            buffer.size() >= data.len() as u64 && buffer.size() <= limit
                        })
                        .unwrap_or_else(|| {
                            device.create_buffer(&wgpu::BufferDescriptor {
                                label: Some(label),
                                size: allocation_size(data.len() as u64, limit),
                                usage: usage | wgpu::BufferUsages::COPY_DST,
                                mapped_at_creation: false,
                            })
                        });
                    if !data.is_empty() {
                        queue.write_buffer(&buffer, 0, data);
                    }
                    buffer
                };
                chunks.push(GpuChunk {
                    vertices: upload(
                        bytemuck::cast_slice(vertices),
                        old_vertices,
                        wgpu::BufferUsages::VERTEX,
                        "BenchUp vertices",
                    ),
                    indices: upload(
                        bytemuck::cast_slice(indices),
                        old_indices,
                        wgpu::BufferUsages::INDEX,
                        "BenchUp indices",
                    ),
                    index_range: index_start..index_start + indices.len() as u32,
                });
            },
        );
        Self { chunks }
    }

    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, range: Range<u32>) {
        for chunk in &self.chunks {
            if let Some(local) = local_range(&chunk.index_range, &range) {
                pass.set_vertex_buffer(0, chunk.vertices.slice(..));
                pass.set_index_buffer(chunk.indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(local, 0, 0..1);
            }
        }
    }
}

fn local_range(chunk: &Range<u32>, requested: &Range<u32>) -> Option<Range<u32>> {
    let start = chunk.start.max(requested.start);
    let end = chunk.end.min(requested.end);
    (start < end).then(|| start - chunk.start..end - chunk.start)
}

fn for_each_batch(
    vertices: &[Vertex],
    indices: &[u32],
    limit: u64,
    mut emit: impl FnMut(u32, &[Vertex], &[u32]),
) {
    if std::mem::size_of_val(vertices) as u64 <= limit
        && std::mem::size_of_val(indices) as u64 <= limit
    {
        // Ordinary scenes keep their original upload, with no remapping/copy.
        emit(0, vertices, indices);
        return;
    }
    // Only oversized scenes need remapping. Bound temporary CPU storage and
    // GPU allocation slack; this work happens once per retained scene upload.
    let batch_limit = limit.min(16 * 1024 * 1024) as usize;
    let max_vertices = batch_limit / std::mem::size_of::<Vertex>();
    let max_indices = batch_limit / std::mem::size_of::<u32>();
    assert!(max_vertices >= 3 && max_indices >= 3);
    let mut remap = HashMap::<u32, u32>::new();
    let mut batch_vertices = Vec::new();
    let mut batch_indices = Vec::new();
    let mut index_start = 0;
    for triangle in indices.chunks(3) {
        let new_vertices = triangle
            .iter()
            .enumerate()
            .filter(|&(i, index)| !remap.contains_key(index) && !triangle[..i].contains(index))
            .count();
        if batch_vertices.len() + new_vertices > max_vertices
            || batch_indices.len() + triangle.len() > max_indices
        {
            emit(index_start, &batch_vertices, &batch_indices);
            index_start += batch_indices.len() as u32;
            batch_vertices.clear();
            batch_indices.clear();
            remap.clear();
        }
        for &index in triangle {
            let local = *remap.entry(index).or_insert_with(|| {
                let local = batch_vertices.len() as u32;
                batch_vertices.push(vertices[index as usize]);
                local
            });
            batch_indices.push(local);
        }
    }
    if !batch_indices.is_empty() {
        emit(index_start, &batch_vertices, &batch_indices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffer_growth_respects_power_of_two_and_other_device_limits() {
        let limit = 256 * 1024 * 1024;
        assert_eq!(allocation_size(0, limit), 4096);
        assert_eq!(allocation_size(4097, limit), 8192);
        assert_eq!(allocation_size(limit, limit), limit);
        assert_eq!(allocation_size(5000, 6000), 6000);
        assert_eq!(allocation_size(84, 84), 84);
        assert_eq!(allocation_size(u64::MAX - 3, u64::MAX), u64::MAX - 3);
    }

    fn vertices(count: usize) -> Vec<Vertex> {
        (0..count)
            .map(|i| Vertex {
                position: [i as f32, 0., 0.5],
                color: [255; 4],
                lateral: 0.,
                shadow_color: [0; 4],
                dash_phase: -1.,
            })
            .collect()
    }

    #[test]
    fn fitting_meshes_upload_without_remapping() {
        let vertices = vertices(6);
        let indices = [5, 0, 4];
        let mut count = 0;
        for_each_batch(&vertices, &indices, 168, |start, v, i| {
            assert_eq!(start, 0);
            assert_eq!(v.as_ptr(), vertices.as_ptr());
            assert_eq!(i.as_ptr(), indices.as_ptr());
            count += 1;
        });
        assert_eq!(count, 1);
    }

    #[test]
    fn empty_meshes_keep_a_valid_empty_upload() {
        let mut count = 0;
        for_each_batch(&[], &[], 84, |start, v, i| {
            assert_eq!(start, 0);
            assert!(v.is_empty() && i.is_empty());
            count += 1;
        });
        assert_eq!(count, 1);
    }

    #[test]
    fn oversized_meshes_preserve_triangles_and_pass_ranges() {
        let vertices = vertices(12);
        // Shared, sparse, reversed, and degenerate indices across three passes.
        let indices = [11, 0, 5, 5, 0, 11, 9, 8, 7, 7, 7, 7, 0, 2, 4];
        let passes = [0..6, 6..12, 12..15];
        let mut restored = [Vec::new(), Vec::new(), Vec::new()];
        let mut next = 0;
        let mut count = 0;
        for_each_batch(&vertices, &indices, 84, |start, v, i| {
            assert_eq!(start, next);
            assert_eq!(i.len() % 3, 0);
            assert!(std::mem::size_of_val(v) <= 84);
            assert!(std::mem::size_of_val(i) <= 84);
            next += i.len() as u32;
            count += 1;
            for (pass, range) in passes.iter().enumerate() {
                if let Some(local) = local_range(&(start..next), range) {
                    restored[pass].extend(
                        i[local.start as usize..local.end as usize]
                            .iter()
                            .map(|&index| v[index as usize].position[0] as u32),
                    );
                }
            }
        });
        assert_eq!(next as usize, indices.len());
        assert_eq!(count, 3);
        for (pass, range) in passes.iter().enumerate() {
            assert_eq!(
                restored[pass],
                indices[range.start as usize..range.end as usize]
            );
        }
    }

    #[test]
    fn index_limit_splits_meshes_even_when_vertices_fit() {
        let vertices = vertices(3);
        let indices = [0, 1, 2].repeat(20);
        let mut sizes = Vec::new();
        for_each_batch(&vertices, &indices, 84, |_, v, i| {
            assert_eq!(v.len(), 3);
            sizes.push(i.len());
        });
        assert_eq!(sizes, [21, 21, 18]);
    }

    #[test]
    fn draw_ranges_clip_to_each_batch_without_overlap() {
        assert_eq!(local_range(&(6..12), &(0..9)), Some(0..3));
        assert_eq!(local_range(&(6..12), &(9..18)), Some(3..6));
        assert_eq!(local_range(&(6..12), &(0..6)), None);
        assert_eq!(local_range(&(6..12), &(12..18)), None);
        assert_eq!(local_range(&(6..12), &(9..9)), None);
    }
}
