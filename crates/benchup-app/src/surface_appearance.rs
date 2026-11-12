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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

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

}
