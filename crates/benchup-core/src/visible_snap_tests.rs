use crate::inference::{Inference, MoveSnapCache, SnapTargets, infer_with_targets};
use crate::{Camera, DVec2, DVec3, Model, Projection};

const SIZE: DVec2 = DVec2::new(1000., 800.);

fn camera() -> Camera {
    Camera {
        target: DVec3::new(3., 3., 0.),
        yaw: -std::f64::consts::FRAC_PI_2,
        pitch: std::f64::consts::FRAC_PI_2,
        projection: Projection::Parallel,
        ..Default::default()
    }
}

fn world(camera: &Camera, x: f64, y: f64, z: f64) -> DVec3 {
    camera.ray(DVec2::new(x, y), SIZE).plane_z(z).unwrap()
}

fn rectangle(model: &mut Model, camera: &Camera, lo: DVec2, hi: DVec2, z: f64) {
    let points = [lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)]
        .map(|p| world(camera, p.x, p.y, z));
    for i in 0..4 {
        model.add_edge(points[i], points[(i + 1) % 4]).unwrap();
    }
}

#[test]
fn only_visible_prefers_hovered_edge_over_a_nearby_unrelated_endpoint() {
    let camera = camera();
    let mut model = Model::default();
    model
        .add_edge(
            world(&camera, 200., 300., 0.),
            world(&camera, 800., 300., 0.),
        )
        .unwrap();
    let unrelated = world(&camera, 402., 302., 0.);
    model
        .add_edge(unrelated, world(&camera, 402., 500., 0.))
        .unwrap();
    for only_visible in [false, true] {
        let snap = infer_with_targets(
            &model,
            &camera,
            SIZE,
            DVec2::new(400., 300.),
            None,
            None,
            true,
            false,
            None,
            SnapTargets {
                only_visible,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            snap.kind,
            if only_visible {
                Inference::OnEdge
            } else {
                Inference::Endpoint
            }
        );
        let expected = if only_visible {
            world(&camera, 400., 300., 0.)
        } else {
            unrelated
        };
        assert!(snap.point.distance(expected) < 1e-7);
    }
}

#[test]
fn only_visible_rejects_a_visible_corner_belonging_to_an_object_behind_the_cursor() {
    let camera = camera();
    let mut model = Model::default();
    // The back edge is visible just outside the foreground silhouette, but the
    // actual cursor is over the foreground face.
    let corner = world(&camera, 405., 300., 0.);
    model
        .add_edge(corner, world(&camera, 405., 500., 0.))
        .unwrap();
    rectangle(
        &mut model,
        &camera,
        DVec2::new(200., 200.),
        DVec2::new(402., 500.),
        2.,
    );
    for only_visible in [false, true] {
        let snap = infer_with_targets(
            &model,
            &camera,
            SIZE,
            DVec2::new(400., 300.),
            None,
            None,
            true,
            false,
            None,
            SnapTargets {
                only_visible,
                ..Default::default()
            },
        )
        .unwrap();
        if only_visible {
            assert!((snap.point.z - 2.).abs() < 1e-7);
        } else {
            assert!(snap.point.distance(corner) < 1e-7);
        }
    }
}

#[test]
fn face_centers_require_hovering_their_face_and_hidden_faces_do_not_occlude() {
    let camera = camera();
    let mut model = Model::default();
    rectangle(
        &mut model,
        &camera,
        DVec2::new(403., 298.),
        DVec2::new(407., 302.),
        0.,
    );
    let targets = SnapTargets {
        edges: false,
        faces: false,
        face_centers: true,
        only_visible: true,
    };
    let infer = |model: &Model, cursor, targets| {
        infer_with_targets(
            model, &camera, SIZE, cursor, None, None, true, false, None, targets,
        )
        .unwrap()
    };
    assert_eq!(
        infer(
            &model,
            DVec2::new(400., 300.),
            SnapTargets {
                only_visible: false,
                ..targets
            }
        )
        .kind,
        Inference::FaceCenter
    );
    assert_eq!(
        infer(&model, DVec2::new(400., 300.), targets).kind,
        Inference::Plane
    );
    assert_eq!(
        infer(&model, DVec2::new(405., 300.), targets).kind,
        Inference::FaceCenter
    );
    rectangle(
        &mut model,
        &camera,
        DVec2::new(200., 200.),
        DVec2::new(600., 500.),
        2.,
    );
    let front = *model.faces.keys().next_back().unwrap();
    assert_ne!(
        infer(&model, DVec2::new(405., 300.), targets).kind,
        Inference::FaceCenter
    );
    model.hidden.insert(front);
    assert_eq!(
        infer(&model, DVec2::new(405., 300.), targets).kind,
        Inference::FaceCenter
    );
}

#[test]
fn visible_only_cached_moves_keep_occlusion_in_x_ray_and_respect_axis_locks() {
    let camera = camera();
    let mut model = Model::default();
    model
        .add_edge(
            world(&camera, 400., 300., 0.),
            world(&camera, 600., 300., 0.),
        )
        .unwrap();
    rectangle(
        &mut model,
        &camera,
        DVec2::new(200., 200.),
        DVec2::new(700., 500.),
        2.,
    );
    let mut cache = MoveSnapCache::new(&model).with_x_ray(true);
    let start = world(&camera, 300., 300., 2.);
    for only_visible in [false, true] {
        for lock in [None, Some(crate::inference::Axis::X)] {
            let inferred = cache
                .infer_with_targets(
                    &camera,
                    SIZE,
                    DVec2::new(400., 300.),
                    start,
                    lock,
                    true,
                    false,
                    SnapTargets {
                        only_visible,
                        ..Default::default()
                    },
                )
                .unwrap();
            if only_visible {
                assert!((inferred.snap.point.z - 2.).abs() < 1e-7);
            } else if lock.is_none() {
                assert_eq!(inferred.snap.kind, Inference::Endpoint);
                assert!(inferred.snap.point.z.abs() < 1e-7);
            }
        }
    }
}
