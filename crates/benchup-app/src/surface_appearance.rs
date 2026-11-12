//! Face-space material coordinates for retained GPU glue rendering.
use benchup_core::DVec3;
use glam::DMat3;

#[derive(Clone, Copy, Debug)]
pub(crate) struct FaceMaterial {
    pub params: [u8; 4],
    local: DMat3,
    origin: DVec3,
}

impl FaceMaterial {
    pub(crate) fn wood_glue(plane: benchup_core::faces::Plane) -> Self {
        Self {
            params: [255, 255, 255, 0],
            local: DMat3::from_cols(plane.u, plane.v, plane.normal).transpose(),
            origin: plane.origin,
        }
    }

    /// The model keeps vertices in world space. Sample in the inverse instance
    /// frame so moving, rotating, mirroring or scaling does not swim the grain.
    pub fn coordinates(self, point: DVec3) -> [f32; 3] {
        (self.local * (point - self.origin)).as_vec3().to_array()
    }
}
