//! Metre-based kinematic character controller. Faces are two-sided, including
//! zero-thickness walls. A BVH limits collision queries to the local body box.
use ketchup_core::{DVec2, DVec3};

pub(super) const STEP: f64 = 0.18;
const SKIN: f64 = 0.002;
const GRAVITY: f64 = 9.81;
const MAX_SLOPE_Z: f64 = std::f64::consts::FRAC_1_SQRT_2;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub speed_kmh: f64,
    pub look_sensitivity: f64,
    pub eye_height: f64,
    pub width: f64,
    pub depth: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            speed_kmh: 6.,
            look_sensitivity: 0.65,
            eye_height: 1.65,
            width: 0.5,
            depth: 0.3,
        }
    }
}

impl Settings {
    pub fn normalize(&mut self) {
        let defaults = Self::default();
        for (value, default, min, max) in [
            (&mut self.speed_kmh, defaults.speed_kmh, 0.5, 12.),
            (
                &mut self.look_sensitivity,
                defaults.look_sensitivity,
                0.1,
                2.,
            ),
            (&mut self.eye_height, defaults.eye_height, 0.5, 2.3),
            (&mut self.width, defaults.width, 0.2, 1.2),
            (&mut self.depth, defaults.depth, 0.15, 1.2),
        ] {
            *value = if value.is_finite() {
                value.clamp(min, max)
            } else {
                default
            };
        }
    }

    fn eye(&self, crouched: bool, sitting: bool) -> f64 {
        self.eye_height
            * if sitting {
                0.55
            } else if crouched {
                0.6
            } else {
                1.
            }
    }
}

#[derive(Clone, Copy)]
struct Bounds {
    lo: DVec3,
    hi: DVec3,
}
impl Bounds {
    fn points(points: impl IntoIterator<Item = DVec3>) -> Self {
        points.into_iter().fold(
            Self {
                lo: DVec3::splat(f64::INFINITY),
                hi: DVec3::splat(f64::NEG_INFINITY),
            },
            |bounds, p| Self {
                lo: bounds.lo.min(p),
                hi: bounds.hi.max(p),
            },
        )
    }
    fn overlaps(self, other: Self) -> bool {
        self.lo.cmple(other.hi).all() && other.lo.cmple(self.hi).all()
    }
}

struct Surface {
    vertices: [DVec3; 3],
    bounds: Bounds,
    walkable: bool,
}
struct Node {
    bounds: Bounds,
    start: usize,
    end: usize,
    children: Option<(usize, usize)>,
}
#[derive(Default)]
pub(crate) struct World {
    surfaces: Vec<Surface>,
    nodes: Vec<Node>,
}

// Camera yaw points from the target toward the eye. These are its horizontal
// right and forward axes, matching Camera::right and WASD in every orientation.
fn basis(yaw: f64) -> (DVec3, DVec3) {
    (
        DVec3::new(-yaw.sin(), yaw.cos(), 0.),
        DVec3::new(-yaw.cos(), -yaw.sin(), 0.),
    )
}

impl World {
    pub fn new(triangles: impl IntoIterator<Item = [DVec3; 3]>) -> Self {
        let surfaces = triangles
            .into_iter()
            .filter_map(|vertices| {
                let normal = (vertices[1] - vertices[0]).cross(vertices[2] - vertices[0]);
                (normal.is_finite() && normal.length_squared() > 1e-20).then(|| Surface {
                    bounds: Bounds::points(vertices),
                    vertices,
                    walkable: normal.normalize().z.abs() + 1e-10 >= MAX_SLOPE_Z,
                })
            })
            .collect();
        let mut world = Self {
            surfaces,
            nodes: Vec::new(),
        };
        if !world.surfaces.is_empty() {
            world.build(0, world.surfaces.len());
        }
        world
    }

    fn build(&mut self, start: usize, end: usize) -> usize {
        let bounds = Bounds::points(
            self.surfaces[start..end]
                .iter()
                .flat_map(|s| [s.bounds.lo, s.bounds.hi]),
        );
        let index = self.nodes.len();
        self.nodes.push(Node {
            bounds,
            start,
            end,
            children: None,
        });
        if end - start > 8 {
            let extent = bounds.hi - bounds.lo;
            let axis = if extent.x > extent.y && extent.x > extent.z {
                0
            } else if extent.y > extent.z {
                1
            } else {
                2
            };
            let middle = (start + end) / 2;
            self.surfaces[start..end].select_nth_unstable_by(middle - start, |a, b| {
                (a.bounds.lo[axis] + a.bounds.hi[axis])
                    .total_cmp(&(b.bounds.lo[axis] + b.bounds.hi[axis]))
            });
            let a = self.build(start, middle);
            let b = self.build(middle, end);
            self.nodes[index].children = Some((a, b));
        }
        index
    }

    fn visit(&self, bounds: Bounds, mut visit: impl FnMut(&Surface) -> bool) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let mut pending = [0; usize::BITS as usize + 1];
        let mut len = 1;
        while len != 0 {
            len -= 1;
            let index = pending[len];
            let node = &self.nodes[index];
            if !node.bounds.overlaps(bounds) {
                continue;
            }
            if let Some((a, b)) = node.children {
                pending[len..len + 2].copy_from_slice(&[a, b]);
                len += 2;
            } else {
                for surface in &self.surfaces[node.start..node.end] {
                    if surface.bounds.overlaps(bounds) && visit(surface) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn body_bounds(feet: DVec3, yaw: f64, settings: &Settings, height: f64) -> Bounds {
        let (right, forward) = basis(yaw);
        let half = (right.abs() * settings.width + forward.abs() * settings.depth) * 0.5;
        Bounds {
            lo: feet - half,
            hi: feet + half + DVec3::Z * height,
        }
    }

    pub fn clear(&self, feet: DVec3, yaw: f64, settings: &Settings, height: f64) -> bool {
        let bounds = Self::body_bounds(feet, yaw, settings, height);
        let (right, forward) = basis(yaw);
        let center = feet + DVec3::Z * (height * 0.5);
        let half = DVec3::new(
            settings.width * 0.5 - SKIN,
            settings.depth * 0.5 - SKIN,
            height * 0.5 - SKIN,
        );
        !self.visit(bounds, |surface| {
            let triangle = surface.vertices.map(|p| {
                let p = p - center;
                DVec3::new(p.dot(right), p.dot(forward), p.z)
            });
            triangle_box(triangle, half)
        })
    }

    /// Clip walkable triangles against the whole footprint before finding the
    /// highest supporting point. This respects body depth, steps and face holes.
    pub fn floor(
        &self,
        feet: DVec3,
        yaw: f64,
        settings: &Settings,
        low: f64,
        high: f64,
    ) -> Option<f64> {
        let mut bounds = Self::body_bounds(feet, yaw, settings, 0.);
        bounds.lo.z = low;
        bounds.hi.z = high;
        let (right, forward) = basis(yaw);
        let mut best: Option<f64> = None;
        self.visit(bounds, |surface| {
            if !surface.walkable {
                return false;
            }
            // A triangle clipped to a rectangle has at most seven vertices.
            // Keep both buffers on the stack: this query runs every footstep.
            let mut polygon = [DVec3::ZERO; 12];
            for (out, p) in polygon.iter_mut().zip(surface.vertices) {
                let delta = p - feet;
                *out = DVec3::new(delta.dot(right), delta.dot(forward), p.z);
            }
            let mut len = 3;
            for (axis, sign, limit) in [
                (0, 1., settings.width * 0.5 - SKIN),
                (0, -1., settings.width * 0.5 - SKIN),
                (1, 1., settings.depth * 0.5 - SKIN),
                (1, -1., settings.depth * 0.5 - SKIN),
            ] {
                let mut clipped = [DVec3::ZERO; 12];
                let mut count = 0;
                if len != 0 {
                    let mut a = polygon[len - 1];
                    for &b in &polygon[..len] {
                        let da = a[axis] * sign - limit;
                        let db = b[axis] * sign - limit;
                        if (da <= 0.) != (db <= 0.) {
                            clipped[count] = a.lerp(b, da / (da - db));
                            count += 1;
                        }
                        if db <= 0. {
                            clipped[count] = b;
                            count += 1;
                        }
                        a = b;
                    }
                }
                polygon = clipped;
                len = count;
            }
            if let Some(z) = polygon[..len].iter().map(|p| p.z).reduce(f64::max)
                && z >= low - 1e-8
                && z <= high + 1e-8
            {
                best = Some(best.map_or(z, |old| old.max(z)));
            }
            false
        });
        best
    }
}

/// Separating axis test: the box axes, triangle normal, and all nine edge/box
/// cross products. Unlike face rays this also catches corners and thin walls.
fn triangle_box(triangle: [DVec3; 3], half: DVec3) -> bool {
    let edges = [
        triangle[1] - triangle[0],
        triangle[2] - triangle[1],
        triangle[0] - triangle[2],
    ];
    let separated = |axis: DVec3| {
        if axis.length_squared() < 1e-20 {
            return false;
        }
        let radius = half.dot(axis.abs());
        let values = triangle.map(|p| p.dot(axis));
        values.iter().copied().fold(f64::INFINITY, f64::min) > radius
            || values.iter().copied().fold(f64::NEG_INFINITY, f64::max) < -radius
    };
    for axis in [DVec3::X, DVec3::Y, DVec3::Z, edges[0].cross(edges[1])] {
        if separated(axis) {
            return false;
        }
    }
    for edge in edges {
        for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
            if separated(edge.cross(axis)) {
                return false;
            }
        }
    }
    true
}

#[derive(Clone, Debug)]
pub(crate) struct Person {
    pub feet: DVec3,
    pub yaw: f64,
    body_yaw: f64,
    pub sitting: bool,
    pub crouched: bool,
    seat_yaw: f64,
    pub grounded: bool,
    velocity_z: f64,
}

impl Person {
    pub fn place(
        world: &World,
        point: DVec3,
        yaw: f64,
        settings: &Settings,
        sitting: bool,
    ) -> Option<Self> {
        let z = world.floor(
            point,
            yaw,
            settings,
            point.z - SKIN,
            point.z + settings.width.hypot(settings.depth) * 0.5,
        )?;
        let feet = DVec3::new(point.x, point.y, z);
        world
            .clear(feet, yaw, settings, settings.eye(false, sitting) + 0.12)
            .then_some(Self {
                feet,
                yaw,
                body_yaw: yaw,
                seat_yaw: yaw,
                sitting,
                crouched: false,
                grounded: true,
                velocity_z: 0.,
            })
    }

    pub fn eye_height(&self, settings: &Settings) -> f64 {
        settings.eye(self.crouched, self.sitting)
    }
    fn height(&self, settings: &Settings) -> f64 {
        self.eye_height(settings) + 0.12
    }

    pub fn fits(&self, world: &World, settings: &Settings) -> bool {
        world.clear(self.feet, self.body_yaw, settings, self.height(settings))
    }

    pub fn turn(&mut self, delta: f64) {
        if self.sitting {
            self.yaw = (self.yaw + delta).clamp(
                self.seat_yaw - std::f64::consts::FRAC_PI_2,
                self.seat_yaw + std::f64::consts::FRAC_PI_2,
            );
        } else {
            // Looking never rotates the collision footprint into a wall. The
            // body follows during movement when its new orientation fits.
            self.yaw += delta;
        }
    }

    pub fn toggle_sit(&mut self, world: &World, settings: &Settings) -> bool {
        if !self.grounded {
            return false;
        }
        if self.sitting {
            if !world.clear(
                self.feet,
                self.body_yaw,
                settings,
                settings.eye(false, false) + 0.12,
            ) {
                return false;
            }
            self.sitting = false;
        } else {
            self.sitting = true;
            self.seat_yaw = self.yaw;
        }
        self.crouched = false;
        true
    }

    pub fn step(
        &mut self,
        world: &World,
        settings: &Settings,
        direction: DVec2,
        crouch: bool,
        jump: bool,
        dt: f64,
    ) {
        if self.sitting {
            return;
        }
        if crouch {
            self.crouched = true;
        } else if self.crouched
            && world.clear(
                self.feet,
                self.body_yaw,
                settings,
                settings.eye(false, false) + 0.12,
            )
        {
            self.crouched = false;
        }
        if jump && self.grounded && !self.crouched {
            self.velocity_z = (2. * GRAVITY * 0.25).sqrt();
            self.grounded = false;
        }
        if direction == DVec2::ZERO && self.grounded {
            return;
        }
        if direction != DVec2::ZERO
            && world.clear(self.feet, self.yaw, settings, self.height(settings))
        {
            self.body_yaw = self.yaw;
        }
        let dt = dt.clamp(0., 0.1);
        // Small bounded steps prevent tunnelling through thin faces, independent
        // of frame rate. Discard long pauses instead of teleporting on refocus.
        let count = (dt / (1. / 120.)).ceil().max(1.) as usize;
        let dt = dt / count as f64;
        let (right, forward) = basis(self.yaw);
        let direction = direction / direction.length().max(1.);
        let delta = (right * direction.x + forward * direction.y)
            * (settings.speed_kmh / 3.6)
            * if self.crouched { 0.45 } else { 1. }
            * dt;
        for _ in 0..count {
            if !self.move_horizontal(world, settings, delta) {
                self.move_horizontal(world, settings, DVec3::new(delta.x, 0., 0.));
                self.move_horizontal(world, settings, DVec3::new(0., delta.y, 0.));
            }
            self.velocity_z -= GRAVITY * dt;
            let fall = self.velocity_z * dt;
            if self.velocity_z <= 0.
                && let Some(z) = world.floor(
                    self.feet,
                    self.body_yaw,
                    settings,
                    self.feet.z + fall - SKIN,
                    self.feet.z + SKIN,
                )
            {
                self.feet.z = z;
                self.velocity_z = 0.;
                self.grounded = true;
            } else {
                let next = self.feet + DVec3::Z * fall;
                if world.clear(next, self.body_yaw, settings, self.height(settings)) {
                    self.feet = next;
                    self.grounded = false;
                } else {
                    self.velocity_z = 0.;
                }
            }
        }
    }

    fn move_horizontal(&mut self, world: &World, settings: &Settings, delta: DVec3) -> bool {
        if delta.length_squared() == 0. {
            return true;
        }
        let mut next = self.feet + delta;
        if self.grounded && self.velocity_z <= 0. {
            if let Some(z) = world.floor(
                next,
                self.body_yaw,
                settings,
                self.feet.z - STEP,
                self.feet.z + STEP,
            ) {
                next.z = z;
                // Check the upward leg of a step too: a low ceiling must not
                // be bypassed by raising and moving the body simultaneously.
                if z > self.feet.z
                    && !world.clear(
                        DVec3::new(self.feet.x, self.feet.y, z),
                        self.body_yaw,
                        settings,
                        self.height(settings),
                    )
                {
                    return false;
                }
            }
        }
        if world.clear(next, self.body_yaw, settings, self.height(settings)) {
            self.feet = next;
            true
        } else {
            false
        }
    }
}

/// A small downward eye dip tied to actual footsteps, never to held keys.
/// It does not move the body collider or raise the head into low ceilings.
#[derive(Default)]
pub(super) struct Gait {
    phase: f64,
    pub offset: f64,
}

impl Gait {
    pub fn update(
        &mut self,
        distance: f64,
        dt: f64,
        grounded: bool,
        crouched: bool,
        sitting: bool,
    ) {
        if sitting || !grounded {
            self.phase = 0.;
            self.offset = 0.;
            return;
        }
        let moving = distance > 1e-6;
        let target = if moving {
            self.phase =
                (self.phase + distance / 0.65 * std::f64::consts::TAU) % std::f64::consts::TAU;
            -(if crouched { 0.001 } else { 0.003 }) * (1. - self.phase.cos())
        } else {
            0.
        };
        self.offset += (target - self.offset) * (1. - (-dt.max(0.) * 16.).exp());
        if !moving && self.offset.abs() < 1e-5 {
            self.offset = 0.;
            self.phase = 0.;
        }
    }
}
