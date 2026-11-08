use super::*;

fn quad(a: DVec3, b: DVec3, c: DVec3, d: DVec3) -> [[DVec3; 3]; 2] {
    [[a, b, c], [a, c, d]]
}
fn floor(z: f64) -> [[DVec3; 3]; 2] {
    quad(
        DVec3::new(-10., -10., z),
        DVec3::new(10., -10., z),
        DVec3::new(10., 10., z),
        DVec3::new(-10., 10., z),
    )
}
fn wall_x(x: f64, low: f64, high: f64) -> [[DVec3; 3]; 2] {
    quad(
        DVec3::new(x, -5., low),
        DVec3::new(x, 5., low),
        DVec3::new(x, 5., high),
        DVec3::new(x, -5., high),
    )
}
fn move_for(
    person: &mut Person,
    world: &World,
    settings: &Settings,
    direction: DVec2,
    seconds: f64,
) {
    for _ in 0..(seconds * 120.).ceil() as usize {
        person.step(world, settings, direction, false, false, 1. / 120.);
    }
}

#[test]
fn walking_at_six_kmh_is_frame_rate_independent_and_diagonals_are_normalized() {
    let settings = Settings::default();
    let world = World::new(floor(0.));
    let start = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    for fps in [20, 30, 60, 120] {
        let mut person = start.clone();
        for _ in 0..fps {
            person.step(&world, &settings, DVec2::Y, false, false, 1. / fps as f64);
        }
        assert!((person.feet.x + 5. / 3.).abs() < 1e-8);
        assert!(person.feet.z.abs() < 1e-8);
    }
    let mut diagonal = start;
    move_for(&mut diagonal, &world, &settings, DVec2::ONE, 1.);
    assert!((diagonal.feet.length() - 5. / 3.).abs() < 1e-8);
}

#[test]
fn configured_speed_controls_walking_and_crouching_at_different_frame_rates() {
    let world = World::new(floor(0.));
    for (speed_kmh, meters_per_second) in [(3., 5. / 6.), (6., 5. / 3.), (9., 2.5)] {
        let settings = Settings {
            speed_kmh,
            ..Settings::default()
        };
        for fps in [30, 60, 120] {
            for crouch in [false, true] {
                let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
                for _ in 0..fps {
                    person.step(&world, &settings, DVec2::Y, crouch, false, 1. / fps as f64);
                }
                let expected = meters_per_second * if crouch { 0.45 } else { 1. };
                assert!((person.feet.x + expected).abs() < 1e-8);
            }
        }
    }
}

#[test]
fn zero_thickness_walls_block_both_sides_and_allow_sliding() {
    let settings = Settings::default();
    let world = World::new(floor(0.).into_iter().chain(wall_x(-1., 0., 3.)));
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    move_for(&mut person, &world, &settings, DVec2::ONE, 3.);
    assert!(person.feet.x > -0.86 && person.feet.x < -0.8);
    assert!(person.feet.y > 2.);
    let mut other = Person::place(
        &world,
        DVec3::new(-2., 0., 0.),
        std::f64::consts::PI,
        &settings,
        false,
    )
    .unwrap();
    move_for(&mut other, &world, &settings, DVec2::Y, 2.);
    assert!(other.feet.x < -1.14);
}

#[test]
fn body_width_depth_and_height_determine_clearance() {
    let world = World::new(
        floor(0.)
            .into_iter()
            .chain(wall_x(-0.3, 0., 3.))
            .chain(wall_x(0.3, 0., 3.))
            .chain(floor(1.9)),
    );
    let settings = Settings::default();
    assert!(Person::place(&world, DVec3::ZERO, 0., &settings, false).is_some());
    assert!(
        Person::place(
            &world,
            DVec3::ZERO,
            0.,
            &Settings {
                depth: 0.7,
                ..settings.clone()
            },
            false
        )
        .is_none()
    );
    assert!(
        Person::place(
            &world,
            DVec3::ZERO,
            std::f64::consts::FRAC_PI_2,
            &Settings {
                width: 0.7,
                ..settings.clone()
            },
            false
        )
        .is_none()
    );
    assert!(
        Person::place(
            &world,
            DVec3::ZERO,
            0.,
            &Settings {
                eye_height: 1.85,
                ..settings
            },
            false
        )
        .is_none()
    );
}

#[test]
fn low_edges_are_stepped_over_but_tall_edges_block() {
    let settings = Settings::default();
    for (height, crosses) in [(0.12, true), (0.18, true), (0.3, false)] {
        let top = quad(
            DVec3::new(-4., -5., height),
            DVec3::new(-1., -5., height),
            DVec3::new(-1., 5., height),
            DVec3::new(-4., 5., height),
        );
        let world = World::new(
            floor(0.)
                .into_iter()
                .chain(wall_x(-1., 0., height))
                .chain(top),
        );
        let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
        move_for(&mut person, &world, &settings, DVec2::Y, 2.);
        assert_eq!(
            person.feet.x < -2.,
            crosses,
            "step {height}: {:?}",
            person.feet
        );
        if crosses {
            assert!((person.feet.z - height).abs() < 1e-8);
        }
    }
}

#[test]
fn gentle_ramps_work_and_steep_ramps_block() {
    let settings = Settings::default();
    for (rise, crosses) in [(0.8, true), (2., true), (4., false)] {
        let ramp = quad(
            DVec3::new(-1., -5., 0.),
            DVec3::new(-1., 5., 0.),
            DVec3::new(-3., 5., rise),
            DVec3::new(-3., -5., rise),
        );
        let world = World::new(floor(0.).into_iter().chain(ramp));
        let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
        move_for(&mut person, &world, &settings, DVec2::Y, 2.);
        assert_eq!(person.feet.x < -2., crosses);
        if crosses {
            assert!(person.feet.z > 0.6);
        }
    }
}

#[test]
fn crouching_fits_under_low_ceiling_and_cannot_stand_into_it() {
    let settings = Settings::default();
    let ceiling = quad(
        DVec3::new(-4., -5., 1.3),
        DVec3::new(-1., -5., 1.3),
        DVec3::new(-1., 5., 1.3),
        DVec3::new(-4., 5., 1.3),
    );
    let world = World::new(floor(0.).into_iter().chain(ceiling));
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    for _ in 0..360 {
        person.step(&world, &settings, DVec2::Y, true, false, 1. / 120.);
    }
    assert!(person.feet.x < -1.5);
    person.step(&world, &settings, DVec2::ZERO, false, false, 1. / 120.);
    assert!(person.crouched);
    move_for(&mut person, &world, &settings, -DVec2::Y, 3.);
    person.step(&world, &settings, DVec2::ZERO, false, false, 1. / 120.);
    assert!(!person.crouched);
}

#[test]
fn jump_is_small_has_gravity_lands_and_cannot_repeat_in_midair() {
    let settings = Settings::default();
    let world = World::new(floor(0.));
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    let mut highest = 0_f64;
    for frame in 0..120 {
        person.step(
            &world,
            &settings,
            DVec2::ZERO,
            false,
            frame == 0 || frame == 10,
            1. / 120.,
        );
        highest = highest.max(person.feet.z);
    }
    assert!((0.23..=0.26).contains(&highest));
    assert!(person.grounded);
    assert!(person.feet.z.abs() < 1e-8);
}

#[test]
fn jump_stops_at_ceiling() {
    let settings = Settings::default();
    let world = World::new(floor(0.).into_iter().chain(floor(1.88)));
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    let mut highest = 0_f64;
    for frame in 0..120 {
        person.step(&world, &settings, DVec2::ZERO, false, frame == 0, 1. / 120.);
        highest = highest.max(person.feet.z);
    }
    assert!(highest > 0.05 && highest < 0.12);
    assert!(person.grounded);
}

#[test]
fn stepping_up_cannot_bypass_head_clearance() {
    let settings = Settings::default();
    let step = quad(
        DVec3::new(-4., -5., 0.15),
        DVec3::new(-1., -5., 0.15),
        DVec3::new(-1., 5., 0.15),
        DVec3::new(-4., 5., 0.15),
    );
    let world = World::new(
        floor(0.)
            .into_iter()
            .chain(wall_x(-1., 0., 0.15))
            .chain(step)
            .chain(floor(1.85)),
    );
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    move_for(&mut person, &world, &settings, DVec2::Y, 3.);
    assert!(person.feet.x > -1.);
    assert!(person.feet.z.abs() < 1e-8);
}

#[test]
fn sitting_locks_translation_and_limits_head_yaw_to_180_degrees() {
    let settings = Settings::default();
    let world = World::new(floor(0.));
    let mut person = Person::place(&world, DVec3::ZERO, 0.7, &settings, false).unwrap();
    assert!(person.toggle_sit(&world, &settings));
    let start = person.feet;
    person.step(&world, &settings, DVec2::ONE, true, true, 1.);
    assert_eq!(person.feet, start);
    assert!(person.eye_height(&settings) < 1.);
    person.turn(100.);
    assert!((person.yaw - 0.7 - std::f64::consts::FRAC_PI_2).abs() < 1e-8);
    person.turn(-200.);
    assert!((person.yaw - 0.7 + std::f64::consts::FRAC_PI_2).abs() < 1e-8);
    assert!(person.toggle_sit(&world, &settings));
    assert_eq!(person.eye_height(&settings), settings.eye_height);
}

#[test]
fn sitting_cannot_stand_through_ceiling() {
    let settings = Settings::default();
    let world = World::new(floor(0.).into_iter().chain(floor(1.2)));
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, true).unwrap();
    assert!(!person.toggle_sit(&world, &settings));
    assert!(person.sitting);
}

#[test]
fn walking_off_a_platform_falls_and_floor_holes_are_not_filled() {
    let settings = Settings::default();
    let platform = quad(
        DVec3::new(-1., -2., 2.),
        DVec3::new(1., -2., 2.),
        DVec3::new(1., 2., 2.),
        DVec3::new(-1., 2., 2.),
    );
    let world = World::new(platform.into_iter().chain(floor(0.)));
    let mut person = Person::place(&world, DVec3::Z * 2., 0., &settings, false).unwrap();
    move_for(&mut person, &world, &settings, DVec2::Y, 3.);
    assert!(person.grounded);
    assert!(person.feet.z.abs() < 1e-8);
    assert!(
        Person::place(
            &World::new(platform),
            DVec3::new(-3., 0., 2.),
            0.,
            &settings,
            false
        )
        .is_none()
    );
    let mut ring = Vec::new();
    for (x0, x1, y0, y1) in [
        (-5., -1., -5., 5.),
        (1., 5., -5., 5.),
        (-1., 1., -5., -1.),
        (-1., 1., 1., 5.),
    ] {
        ring.extend(quad(
            DVec3::new(x0, y0, 2.),
            DVec3::new(x1, y0, 2.),
            DVec3::new(x1, y1, 2.),
            DVec3::new(x0, y1, 2.),
        ));
    }
    let world = World::new(ring);
    assert!(Person::place(&world, DVec3::Z * 2., 0., &settings, false).is_none());
    assert!(Person::place(&world, DVec3::new(3., 0., 2.), 0., &settings, false).is_some());
}

#[test]
fn looking_near_walls_is_unrestricted_and_keeps_the_body_clear() {
    let settings = Settings {
        width: 0.8,
        depth: 0.2,
        ..Settings::default()
    };
    let world = World::new(
        floor(0.)
            .into_iter()
            .chain(wall_x(-0.3, 0., 3.))
            .chain(wall_x(0.3, 0., 3.)),
    );
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    let feet = person.feet;
    person.turn(std::f64::consts::FRAC_PI_2);
    assert_eq!(person.yaw, std::f64::consts::FRAC_PI_2);
    assert_eq!(person.feet, feet);
    assert!(person.fits(&world, &settings));
    // Movement can continue along the corridor while the collision footprint
    // stays in the orientation that fits. Looking does not bypass the walls.
    move_for(&mut person, &world, &settings, DVec2::Y, 2.);
    assert!(person.feet.y < -2.);
    assert!(person.fits(&world, &settings));
    person.turn(std::f64::consts::TAU * 3.);
    assert!(person.yaw > std::f64::consts::TAU * 3.);
    assert!(person.fits(&world, &settings));
}

#[test]
fn gait_is_subtle_tracks_travel_and_settles_when_still() {
    let mut gait = physics::Gait::default();
    let mut lowest = 0_f64;
    for _ in 0..240 {
        gait.update((5. / 3.) / 120., 1. / 120., true, false, false);
        lowest = lowest.min(gait.offset);
        assert!((-0.006..=0.).contains(&gait.offset));
    }
    assert!(lowest < -0.004);
    for _ in 0..240 {
        gait.update(0., 1. / 120., true, false, false);
    }
    assert_eq!(gait.offset, 0.);
    // Held keys against an obstacle produce no bob without actual travel.
    for _ in 0..120 {
        gait.update(0., 1. / 120., true, false, false);
    }
    assert_eq!(gait.offset, 0.);
    gait.update(0.2, 0.1, true, false, false);
    assert!(gait.offset < 0.);
    gait.update(0.1, 0.1, false, false, false);
    assert_eq!(gait.offset, 0.);
    gait.update(0.2, 0.1, true, false, false);
    gait.update(0.1, 0.1, true, false, true);
    assert_eq!(gait.offset, 0.);
}

#[test]
fn spatial_hierarchy_finds_local_walls_and_long_frame_stalls_are_bounded() {
    let settings = Settings::default();
    let mut triangles = floor(0.).to_vec();
    for x in -100..=100 {
        if x != 0 {
            triangles.extend(wall_x(x as f64, 0., 3.));
        }
    }
    let world = World::new(triangles);
    let mut person = Person::place(&world, DVec3::ZERO, 0., &settings, false).unwrap();
    person.step(&world, &settings, DVec2::Y, false, false, 5.);
    assert!((person.feet.x + 1. / 6.).abs() < 1e-8);
    move_for(&mut person, &world, &settings, DVec2::Y, 3.);
    assert!(person.feet.x > -0.86 && person.feet.x < -0.8);
}

#[test]
fn invalid_settings_normalize_and_old_data_gets_defaults() {
    assert_eq!(
        serde_json::from_str::<Settings>("{}").unwrap(),
        Settings::default()
    );
    let mut settings = Settings {
        speed_kmh: f64::NAN,
        look_sensitivity: f64::NAN,
        eye_height: f64::NAN,
        width: -1.,
        depth: f64::INFINITY,
    };
    settings.normalize();
    assert_eq!(settings.eye_height, Settings::default().eye_height);
    assert_eq!(settings.width, 0.2);
    assert_eq!(settings.depth, Settings::default().depth);
    assert_eq!(settings.speed_kmh, 6.);
    assert_eq!(settings.look_sensitivity, 0.65);
    assert!(Settings::default().look_sensitivity <= 0.67);
    settings.speed_kmh = -1.;
    settings.normalize();
    assert_eq!(settings.speed_kmh, 0.5);
    settings.speed_kmh = 100.;
    settings.normalize();
    assert_eq!(settings.speed_kmh, 12.);
    settings.look_sensitivity = -1.;
    settings.normalize();
    assert_eq!(settings.look_sensitivity, 0.1);
    settings.look_sensitivity = 10.;
    settings.normalize();
    assert_eq!(settings.look_sensitivity, 2.);
}
