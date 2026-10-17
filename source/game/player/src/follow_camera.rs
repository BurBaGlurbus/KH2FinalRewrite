//! Field follow camera (mode 0) and lock-on camera (mode 2), from the installed
//! PC executable's camera update: the per-mode table (field of view, radius,
//! lock radius, radius limits, height curve), the yaw follow that swings
//! behind a moving player, the radius return, and the spring that eases the
//! current eye and look-at points toward their targets. The lock-on mode
//! translates FUN_140176c70 (mode 2 update), FUN_140176ec0 (look point,
//! screen-edge tests and yaw), FUN_140176710 (screen flags), FUN_140176bc0
//! (zoom), FUN_140176a70 (bearing), FUN_140164520 (yaw ease and eye target),
//! FUN_140164290 (radius return) and FUN_140176e40 (eye collision); the
//! spring is FUN_140165700 / FUN_140165bd0.
//!
//! Positions here are in the game's own world space: Y points down, so
//! "above the player" is negative Y. One `tick` is one 60 Hz frame (the
//! game's frame-time scale of 1). Camera collision uses the map's camera
//! collision mesh: the eye is pulled to the first hit on the segment from the
//! look-at point, 5 units off the surface.

pub type Vec3 = [f32; 3];

/// One row of the camera mode table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mode {
    /// Horizontal field of view in radians across a 640-unit-wide screen.
    pub fov: f32,
    pub radius: f32,
    pub lock_radius: f32,
    pub radius_min: f32,
    pub radius_max: f32,
    /// Eye height grows with the square of (radius - radius_min) times this.
    pub up_curve: f32,
}

/// Mode 0 (default/outdoor) row as stored in the executable's table.
pub const OUTDOOR: Mode = Mode {
    fov: 1.5,
    radius: 420.,
    lock_radius: 380.,
    radius_min: 250.,
    radius_max: 500.,
    up_curve: 0.008,
};

/// Look-at point height above the player's position.
pub const LOOK_AT_HEIGHT: f32 = 170.;
/// Eye target sits this far below the look-at point before the radius curve.
pub const EYE_DROP: f32 = 150.;
/// Yaw easing toward the yaw target, radians per frame.
pub const YAW_EASE: f32 = 0.174_532_9;
/// Behind-the-player follow speed, radians per frame, and its per-unit
/// change with the player's speed parameter (relative to 8).
pub const FOLLOW_SPEED: f32 = 0.004_363_32;
pub const FOLLOW_SPEED_PER_UNIT: f32 = 0.000_174_533;
pub const SPEED_REFERENCE: f32 = 8.;
/// Stick "toward the camera" component at or above which follow pauses.
pub const FOLLOW_STICK_LIMIT: f32 = 0.9;
/// Radius return toward the mode radius, units per frame.
pub const RADIUS_RETURN: f32 = 0.5;
/// Manual rotation, radians per frame at full input (default setting).
pub const MANUAL_TURN: f32 = 0.052_359_88;
/// Manual zoom, units per frame at full input.
pub const MANUAL_ZOOM: f32 = 8.;
/// Spring gains (default camera setting) and their per-unit speed change.
pub const SPRING_GAIN: f32 = 0.07;
pub const SPRING_GAIN_PER_UNIT: f32 = 0.0006;
pub const LOOK_AT_EXTRA_GAIN: f32 = 0.03;
pub const SPRING: f32 = 2.5;
pub const DAMPING: f32 = 0.5;
pub const REST_LENGTH: f32 = 1.;
/// Distance the eye is kept off a camera collision surface.
pub const COLLISION_OFFSET: f32 = 5.;

/// Lock-on look point: the fraction of the way from the player's upper body
/// toward the target's camera anchor (0x140623a28).
pub const LOCK_LOOK_BLEND: f32 = 0.3;
/// Height of the player's head point used for the off-screen test and the
/// far-and-above test (0x1405b22e8, Y down).
pub const HEAD_HEIGHT: f32 = 100.;
/// The look point stays on the player when the head is farther than this from
/// it (0x140623bf8) ...
pub const FAR_LOOK_DISTANCE: f32 = 300.;
/// ... and its direction points down more steeply than this (0x140623a4c).
pub const FAR_LOOK_SLOPE: f32 = 0.5;
/// Radius lost per frame for each tested point below the screen (0x140623b18).
pub const EDGE_ZOOM: f32 = 2.;
/// Yaw bias when the target leaves the left or right edge: 20 degrees
/// (0x1405b22ec / 0x1405b22f0).
pub const EDGE_YAW: f32 = 0.349_065_85;
/// The screen test's layout-unit bounds: x, y (0x14071a478) and depth
/// (near 1, far 4,000,000 as truncated from 0x1405b2bf0 / 0x1405b2bf4).
pub const SCREEN_X: [f32; 2] = [32., 480.];
pub const SCREEN_Y: [f32; 2] = [8., 408.];
pub const SCREEN_DEPTH: [f32; 2] = [1., 4_000_000.];
/// Screen test flags (FUN_140176710).
pub const OFF_TOP: u32 = 1;
pub const OFF_BOTTOM: u32 = 2;
pub const OFF_LEFT: u32 = 4;
pub const OFF_RIGHT: u32 = 8;
pub const OFF_NEAR: u32 = 0x10;
pub const OFF_FAR: u32 = 0x20;

/// A camera segment hit: the point, the polygon's unit normal facing the
/// segment's start and its surface nibble (5 marks the re-cast surface type).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    pub point: Vec3,
    pub normal: Vec3,
    pub surface: u32,
}

/// First hit on the segment `from` -> `to` (see `triangle_hit`, or the COCT
/// query for a map's camera collision).
pub type Cast<'a> = &'a dyn Fn(Vec3, Vec3) -> Option<RayHit>;

fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Nearest intersection of a segment with a triangle soup.
pub fn triangle_hit(triangles: &[[Vec3; 3]], from: Vec3, to: Vec3) -> Option<RayHit> {
    let dir = sub(to, from);
    let mut best: Option<(f32, Vec3)> = None;
    for [a, b, c] in triangles {
        let (e1, e2) = (sub(*b, *a), sub(*c, *a));
        let p = cross(dir, e2);
        let det = dot(e1, p);
        if det.abs() < 1e-8 {
            continue;
        }
        let inv = 1. / det;
        let t0 = sub(from, *a);
        let u = dot(t0, p) * inv;
        if !(0. ..=1.).contains(&u) {
            continue;
        }
        let q = cross(t0, e1);
        let v = dot(dir, q) * inv;
        if v < 0. || u + v > 1. {
            continue;
        }
        let t = dot(e2, q) * inv;
        if (0. ..=1.).contains(&t) && best.is_none_or(|(bt, _)| t < bt) {
            best = Some((t, cross(e1, e2)));
        }
    }
    let (t, n) = best?;
    let len = dot(n, n).sqrt();
    let mut n = n.map(|c| c / len);
    if dot(n, dir) > 0. {
        n = n.map(|c| -c);
    }
    Some(RayHit {
        point: [0, 1, 2].map(|k| from[k] + dir[k] * t),
        normal: n,
        surface: 0,
    })
}

/// A dynamic camera-collision sphere: one live kind-18 shape of an actor in
/// game world space (the 0x28-byte record FUN_1403c9c00 builds: the shape's
/// position through the actor or bone matrix, its stored height subtracted
/// from Y, and its radius).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    pub centre: Vec3,
    pub radius: f32,
}

/// The executable's sphere list holds at most 0x40 entries (FUN_140167c20).
pub const MAX_SPHERES: usize = 0x40;

fn length(a: Vec3) -> f32 {
    dot(a, a).sqrt()
}

/// FUN_140179200: where the segment `start` -> `end` enters the sphere. The
/// line's closest approach to the centre must be within the radius; when that
/// closest point lies beyond an end of the segment, that end must be inside
/// the sphere. The entry point is the closest point minus the half-chord
/// along the segment.
fn sphere_entry(sphere: Sphere, start: Vec3, end: Vec3) -> Option<Vec3> {
    let mid = [0, 1, 2].map(|i| (start[i] + end[i]) * 0.5);
    let half = sub(end, mid);
    let centre = sub(sphere.centre, mid);
    let length_squared = dot(half, half);
    if length_squared < f32::MIN_POSITIVE {
        return None;
    }
    let along = dot(half, centre);
    let radius_squared = sphere.radius * sphere.radius;
    let miss_squared = dot(centre, centre) - (along * along) / length_squared;
    if radius_squared < miss_squared {
        return None;
    }
    let scaled = half.map(|c| c * along * (1. / length_squared));
    let closest = [0, 1, 2].map(|i| mid[i] + scaled[i]);
    if !(-length_squared <= along && along <= length_squared) {
        let nearer = if along > length_squared { end } else { start };
        let off = sub(sphere.centre, nearer);
        if radius_squared < dot(off, off) {
            return None;
        }
    }
    let chord = ((radius_squared - miss_squared) / length_squared)
        .abs()
        .sqrt();
    Some([0, 1, 2].map(|i| closest[i] - half[i] * chord))
}

/// FUN_140167a50: the segment's entry point into the sphere list. A sphere
/// that contains `start` is skipped, and the executable's loop does not step
/// to the next entry after a skip, so every later iteration meets the same
/// skipped sphere. Each accepted entry point becomes the end of the segment
/// the later spheres are tested against, while the reach it is compared with
/// stays the original length.
pub fn sphere_hit(spheres: &[Sphere], start: Vec3, end: Vec3) -> Option<Vec3> {
    let reach = length(sub(end, start));
    let mut end = end;
    let mut hit = None;
    for sphere in spheres.iter().take(MAX_SPHERES) {
        if length(sub(sphere.centre, start)) <= sphere.radius {
            break;
        }
        if let Some(point) = sphere_entry(*sphere, start, end)
            && length(sub(point, start)) < reach
        {
            hit = Some(point);
            // The scan works on a copy of the segment whose end becomes the
            // entry point, so later spheres only count inside it.
            end = point;
        }
    }
    hit
}

/// FUN_140164940 with the default push-off: the segment `from` -> `to` is
/// first cut at the dynamic spheres (empty when no actor has any), then cast
/// against the camera collision; on a hit the end moves to the hit point plus
/// the polygon normal times 5. A surface-5 polygon re-casts from `from` to the
/// end's horizontal position at the hit's height (the end offset by the
/// previous target `prev`), and the re-cast point replaces the hit. A sphere
/// cut with no polygon behind it ends exactly on the sphere. Returns the
/// clipped end.
pub fn clip_with(prev: Vec3, from: Vec3, to: Vec3, cast: Cast, spheres: &[Sphere]) -> Option<Vec3> {
    let cut = sphere_hit(spheres, from, to);
    let to = cut.unwrap_or(to);
    let Some(hit) = cast(from, to) else {
        return cut;
    };
    let mut end = hit.point;
    if hit.surface == 5 {
        let delta = sub(to, prev);
        let probe = [delta[0] + prev[0], hit.point[1], delta[2] + prev[2]];
        end = cast(from, probe).map_or(probe, |second| second.point);
    }
    Some([0, 1, 2].map(|k| end[k] + hit.normal[k] * COLLISION_OFFSET))
}

/// `clip_with` against the camera collision alone.
pub fn clip(prev: Vec3, from: Vec3, to: Vec3, cast: Cast) -> Option<Vec3> {
    clip_with(prev, from, to, cast, &[])
}

/// Per-frame inputs. Stick axes follow the pad convention: x right, y
/// toward the player (down on the stick); `magnitude` is the stick's
/// analog strength.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraInput {
    /// Native mode-2 camera anchor, in Y-down world coordinates.
    pub lock_target: Option<Vec3>,
    pub player_position: Vec3,
    /// Player facing as a horizontal direction in world space.
    pub player_forward: Vec3,
    /// The player's horizontal position changed this frame.
    pub moved: bool,
    pub stick: [f32; 2],
    pub magnitude: f32,
    /// Manual rotation (-1..1, positive increases yaw).
    pub turn: f32,
    /// Manual zoom (-1..1, positive moves away).
    pub zoom: f32,
    /// The right stick's analog magnitude (pad +0x2c); any deflection stops
    /// the automatic swing behind the player.
    pub right_stick: f32,
    /// Swing directly behind the player and restore the mode radius.
    pub recenter: bool,
    /// The follow target's zoom lock (FUN_1403babf0: bit 3 of the object the
    /// follow reference's +0xc handle names). While set, manual zoom is
    /// ignored unless `zoom_override` is. No supported actor sets it.
    pub zoom_locked: bool,
    /// Pad bit 32 (R2) held: zoom stays allowed through the lock.
    pub zoom_override: bool,
    /// The player's speed parameter (PRTY run speed in this test room).
    pub speed: f32,
}

impl CameraInput {
    /// The test every zoom routine makes (FUN_140165110, FUN_140176c70, ...):
    /// zoom unless the lock is set and the override is not held.
    fn zoom_allowed(&self) -> bool {
        !self.zoom_locked || self.zoom_override
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowCamera {
    pub mode: Mode,
    pub yaw: f32,
    pub yaw_target: f32,
    pub radius: f32,
    pub eye: Vec3,
    pub look_at: Vec3,
    eye_target: Vec3,
    eye_target_prev: Vec3,
    look_at_target: Vec3,
    look_at_target_prev: Vec3,
}

/// The executable's angle wrap into [-pi, pi].
pub fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    if a > 0. {
        (a + PI) % TAU - PI
    } else {
        PI - (PI - a) % TAU
    }
}

/// Moves `from` toward `to` by at most `step` along the shorter arc.
fn approach(from: f32, to: f32, step: f32) -> f32 {
    let d = wrap(to - from);
    wrap(from + d.clamp(-step, step))
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn normalize_or(a: Vec3, fallback: Vec3) -> Vec3 {
    let length = dot(a, a).sqrt();
    if length > 0. && length.is_finite() {
        a.map(|c| c / length)
    } else {
        fallback
    }
}

fn dot(a: Vec3, b: Vec3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Eases `current` toward `target`: a spring on the distance with damping
/// against the target's own motion (`target_prev` is last frame's target).
pub fn spring(current: Vec3, target: Vec3, target_prev: Vec3, gain: f32) -> Vec3 {
    spring_with(
        current,
        target,
        target_prev,
        gain,
        [SPRING, DAMPING, REST_LENGTH],
    )
}

/// FUN_140165bd0 with explicit constants `[spring, damping, rest length]`
/// (the camera uses 2.5, 0.5, 1; the HUD slot stack 1.2, 0.5, 0.1).
pub fn spring_with(
    current: Vec3,
    target: Vec3,
    target_prev: Vec3,
    gain: f32,
    [spring_k, damping, rest]: [f32; 3],
) -> Vec3 {
    let d = sub(current, target);
    let length = dot(d, d).sqrt();
    if length == 0. {
        return current;
    }
    let v = sub(target_prev, target).map(|c| c * gain);
    let f = dot(d, v);
    let scale = ((f / length) * damping + (rest - length) * spring_k) * gain / length;
    let out = [
        current[0] + d[0] * scale,
        current[1] + d[1] * scale,
        current[2] + d[2] * scale,
    ];
    out.map(|c| if c.is_nan() { 0. } else { c })
}

/// Yaw that places the eye behind a player facing `forward`.
pub fn behind(forward: Vec3) -> f32 {
    (-forward[0]).atan2(-forward[2])
}

impl FollowCamera {
    /// A camera snapped behind the player at the mode radius.
    pub fn new(mode: Mode, input: &CameraInput) -> Self {
        let yaw = behind(input.player_forward);
        let mut camera = Self {
            mode,
            yaw,
            yaw_target: yaw,
            radius: mode.radius,
            eye: [0.; 3],
            look_at: [0.; 3],
            eye_target: [0.; 3],
            eye_target_prev: [0.; 3],
            look_at_target: [0.; 3],
            look_at_target_prev: [0.; 3],
        };
        camera.place_targets(input.player_position, true);
        camera
    }

    /// Vertical field of view for a 640x480 screen at the mode's horizontal
    /// field of view.
    pub fn vertical_fov(&self) -> f32 {
        let focal = 320. / (self.mode.fov * 0.5).tan();
        2. * (240. / focal).atan()
    }

    fn place_targets(&mut self, player: Vec3, snap: bool) {
        self.look_at_target = [player[0], player[1] - LOOK_AT_HEIGHT, player[2]];
        self.yaw = if snap {
            self.yaw_target
        } else {
            approach(self.yaw, self.yaw_target, YAW_EASE)
        };
        self.place_eye_target(self.look_at_target);
        if snap {
            self.eye = self.eye_target;
            self.look_at = self.look_at_target;
            self.eye_target_prev = self.eye_target;
            self.look_at_target_prev = self.look_at_target;
        }
    }

    /// FUN_140164520's eye target: the radius back along the yaw from the
    /// anchor, 150 below it less the radius curve.
    fn place_eye_target(&mut self, at: Vec3) {
        let d = self.radius - self.mode.radius_min;
        let (s, c) = self.yaw.sin_cos();
        self.eye_target = [
            at[0] + self.radius * s,
            at[1] + EYE_DROP - d * d * self.mode.up_curve,
            at[2] + self.radius * c,
        ];
    }

    /// A world point on the 512x416 layout screen and its depth, from the
    /// camera as last applied (eye, look-at, field of view): the view and
    /// projection matrices (FUN_14019cfc0 / FUN_14019ce40) then FUN_140166a20,
    /// which divides X and Y by depth and re-centres them. The focal scales
    /// follow the 640x480 virtual screen: 0.8 wide, 0.8667 high.
    pub fn project(&self, point: Vec3) -> Vec3 {
        let forward = normalize_or(sub(self.look_at, self.eye), [0., 0., 1.]);
        let down = normalize_or(
            sub([0., 1., 0.], forward.map(|c| c * forward[1])),
            [0., 1., 0.],
        );
        let right = cross(down, forward);
        let offset = sub(point, self.eye);
        let (x, y, z) = (dot(offset, right), dot(offset, down), dot(offset, forward));
        let focal = 320. / (self.mode.fov * 0.5).tan();
        let (x, y) = if z != 0. {
            (x * (1. / z), y * (1. / z))
        } else {
            (x, y)
        };
        [x * focal * 0.8 + 256., y * focal * 0.866_666_7 + 208., z]
    }

    /// FUN_140176710: which screen edges a world point is beyond.
    pub fn screen_flags(&self, point: Vec3) -> u32 {
        let [x, y, z] = self.project(point);
        let mut flags = 0;
        if x < SCREEN_X[0] {
            flags |= OFF_LEFT;
        }
        if SCREEN_X[1] < x {
            flags |= OFF_RIGHT;
        }
        if y < SCREEN_Y[0] {
            flags |= OFF_TOP;
        }
        if SCREEN_Y[1] < y {
            flags |= OFF_BOTTOM;
        }
        if z < SCREEN_DEPTH[0] {
            flags |= OFF_NEAR;
        }
        if SCREEN_DEPTH[1] < z {
            flags |= OFF_FAR;
        }
        flags
    }

    /// FUN_140164290: the radius moves toward the mode's lock radius by one
    /// frame's step, within the radius limits.
    fn return_to_lock_radius(&mut self) {
        let mode = self.mode;
        if (self.radius - mode.lock_radius).abs() >= 1. {
            if self.radius <= mode.lock_radius {
                self.radius += 1.;
                if self.radius >= mode.radius_max {
                    self.radius = mode.radius_max;
                }
            } else {
                self.radius -= 1.;
                if self.radius <= mode.radius_min {
                    self.radius = mode.radius_min;
                }
            }
        }
    }

    /// FUN_140166430 / FUN_140166450: clamp to the mode's radius limits.
    fn clamp_radius(&mut self) {
        if self.radius <= self.mode.radius_min {
            self.radius = self.mode.radius_min;
        }
        if self.mode.radius_max <= self.radius {
            self.radius = self.mode.radius_max;
        }
    }

    /// One frame. `cast` returns the first camera-collision hit on a segment
    /// (see `triangle_hit`).
    pub fn tick(&mut self, input: &CameraInput, cast: Cast) {
        self.tick_with(input, cast, &[]);
    }

    /// `tick` with the dynamic camera-collision spheres of this frame.
    pub fn tick_with(&mut self, input: &CameraInput, cast: Cast, spheres: &[Sphere]) {
        self.look_at_target_prev = self.look_at_target;
        self.eye_target_prev = self.eye_target;
        let speed_offset = input.speed - SPEED_REFERENCE;
        // At 60 Hz the gain is halved; the look-at point adds its extra gain
        // afterwards.
        let gain = (SPRING_GAIN + speed_offset * SPRING_GAIN_PER_UNIT) * 0.5;
        if let Some(anchor) = input.lock_target {
            self.tick_lock(input, anchor, cast, spheres);
            let feet_up = [
                input.player_position[0],
                input.player_position[1] - LOOK_AT_HEIGHT,
                input.player_position[2],
            ];
            self.spring_to_targets(gain);
            // FUN_140176e40: mode 2 clips only the eye, from the player's
            // upper body, and does not feed the clip back to the target.
            if let Some(eye) = clip_with(self.eye_target_prev, feet_up, self.eye, cast, spheres) {
                self.eye = eye;
            }
            return;
        }
        let mode = self.mode;
        let mut snap = false;
        // FUN_140165110 tests the recenter press (pad bit 17) first; the
        // manual turn is its else branch.
        if input.recenter {
            self.radius = mode.radius;
            self.yaw_target = behind(input.player_forward);
            self.eye = self.eye_target;
            snap = input.magnitude != 0.;
        } else if input.turn != 0. {
            self.yaw = wrap(self.yaw + input.turn * MANUAL_TURN);
            self.yaw_target = self.yaw;
        }
        let zoom = if input.zoom_allowed() { input.zoom } else { 0. };
        if zoom > 0. {
            self.radius = (self.radius + zoom * MANUAL_ZOOM).min(mode.radius_max);
        } else if zoom < 0. {
            self.radius = (self.radius + zoom * MANUAL_ZOOM).max(mode.radius_min);
        }
        if !snap
            && input.moved
            && input.magnitude != 0.
            // The follow branch also needs the right stick idle (pad +0x2c,
            // the magnitude behind both turn and zoom), whether or not the
            // zoom lock discards the zoom.
            && input.right_stick == 0.
            && input.turn == 0.
            && input.zoom == 0.
            && input.stick[1] < FOLLOW_STICK_LIMIT
        {
            let step = 2.
                * (input.stick[0].abs() + 1.)
                * (input.stick[1].abs() + 1.)
                * input.magnitude
                * (speed_offset * FOLLOW_SPEED_PER_UNIT + FOLLOW_SPEED);
            self.yaw = approach(self.yaw, behind(input.player_forward), step);
            self.yaw_target = self.yaw;
            if (self.radius - mode.radius).abs() >= 1. {
                self.radius = if self.radius <= mode.radius {
                    (self.radius + RADIUS_RETURN).min(mode.radius_max)
                } else {
                    (self.radius - RADIUS_RETURN).max(mode.radius_min)
                };
            }
        }
        self.place_targets(input.player_position, snap);
        if let Some(end) = clip_with(
            self.eye_target_prev,
            self.look_at_target,
            self.eye_target,
            cast,
            spheres,
        ) {
            self.eye_target = end;
        }
        self.spring_to_targets(gain);
        if let Some(eye) = clip_with(self.eye_target_prev, self.look_at, self.eye, cast, spheres) {
            self.eye = eye;
            self.eye_target = eye;
        }
    }

    /// FUN_140165700: the current look-at and eye ease toward their targets.
    fn spring_to_targets(&mut self, gain: f32) {
        self.look_at = spring(
            self.look_at,
            self.look_at_target,
            self.look_at_target_prev,
            gain + LOOK_AT_EXTRA_GAIN,
        );
        self.eye = spring(self.eye, self.eye_target, self.eye_target_prev, gain);
    }

    /// Mode 2 (FUN_140176c70): the look point blends toward the locked
    /// target's camera anchor, the radius and yaw react to the target and the
    /// player leaving the screen, and the eye target stays on the player.
    fn tick_lock(&mut self, input: &CameraInput, anchor: Vec3, cast: Cast, spheres: &[Sphere]) {
        let player = input.player_position;
        let feet_up = [player[0], player[1] - LOOK_AT_HEIGHT, player[2]];
        // FUN_140176ec0 with k = 0.3.
        let old_look_target = self.look_at_target;
        let toward = sub(anchor, feet_up).map(|c| LOCK_LOOK_BLEND * c);
        let look = [0, 1, 2].map(|i| toward[i] + feet_up[i]);
        let head = [player[0], player[1] - HEAD_HEIGHT, player[2]];
        let target_flags = self.screen_flags(anchor);
        let head_flags = self.screen_flags(head);
        let target_low = target_flags & (OFF_TOP | OFF_BOTTOM) == OFF_BOTTOM;
        let head_low = head_flags & (OFF_TOP | OFF_BOTTOM) == OFF_BOTTOM;
        if target_low {
            self.radius -= EDGE_ZOOM;
        }
        if head_low {
            self.radius -= EDGE_ZOOM;
        }
        self.clamp_radius();
        let to_head = sub(head, look);
        let distance = dot(to_head, to_head).sqrt();
        let slope = if distance != 0. {
            to_head[1] * (1. / distance)
        } else {
            to_head[1]
        };
        // The bearing from the old look target to the player (FUN_140176a70).
        let bearing = (player[0] - old_look_target[0]).atan2(player[2] - old_look_target[2]);
        if distance > FAR_LOOK_DISTANCE && slope > FAR_LOOK_SLOPE {
            self.look_at_target = feet_up;
        } else {
            if !target_low && !head_low {
                self.return_to_lock_radius();
            }
            self.look_at_target = look;
            let flags = if target_flags != 0 {
                target_flags
            } else {
                head_flags
            };
            if flags != 0 {
                let edge = if flags & OFF_LEFT != 0 {
                    EDGE_YAW
                } else if flags & OFF_RIGHT != 0 {
                    -EDGE_YAW
                } else {
                    0.
                };
                self.yaw_target = wrap(edge + bearing);
            }
        }
        if input.turn != 0. {
            self.yaw = wrap(self.yaw + MANUAL_TURN * input.turn);
            self.yaw_target = self.yaw;
        }
        // FUN_140176bc0: zoom with the right stick, one step per frame,
        // unless the follow target's zoom lock holds it.
        let zoom = if input.zoom_allowed() { input.zoom } else { 0. };
        if zoom < 0. {
            self.radius += zoom * MANUAL_ZOOM;
            if self.radius <= self.mode.radius_min {
                self.radius = self.mode.radius_min;
            }
        } else if zoom > 0. {
            self.radius += zoom * MANUAL_ZOOM;
            if self.mode.radius_max <= self.radius {
                self.radius = self.mode.radius_max;
            }
        }
        if input.recenter {
            self.yaw_target = bearing;
            self.radius = self.mode.lock_radius;
        }
        // FUN_140164520 with the player's upper body as the anchor, then the
        // eye-target collision from that point (FUN_140164910).
        let look_target = self.look_at_target;
        self.yaw = approach(self.yaw, self.yaw_target, YAW_EASE);
        self.place_eye_target(feet_up);
        if let Some(end) = clip_with(
            self.eye_target_prev,
            feet_up,
            self.eye_target,
            cast,
            spheres,
        ) {
            self.eye_target = end;
        }
        self.look_at_target = look_target;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(position: Vec3, forward: Vec3) -> CameraInput {
        CameraInput {
            player_position: position,
            player_forward: forward,
            speed: 8.,
            ..Default::default()
        }
    }

    fn locked(position: Vec3, forward: Vec3, anchor: Vec3) -> (FollowCamera, CameraInput) {
        let mut step = input(position, forward);
        let camera = FollowCamera::new(OUTDOOR, &step);
        step.lock_target = Some(anchor);
        (camera, step)
    }

    #[test]
    fn lock_looks_between_actors_but_keeps_the_eye_on_the_player_anchor() {
        let (mut camera, step) = locked([0.; 3], [0., 0., 1.], [100., -50., 200.]);
        camera.tick(&step, &|_, _| None);
        // On screen: the radius returns toward the lock radius one unit a frame.
        assert_eq!(camera.radius, 419.);
        for (got, want) in camera.look_at_target.into_iter().zip([30., -134., 60.]) {
            assert!((got - want).abs() < 1e-3);
        }
        assert!(camera.eye_target[0].abs() < 0.001);
        assert!((camera.eye_target[2] + 419.).abs() < 0.001);
        for _ in 0..60 {
            camera.tick(&step, &|_, _| None);
        }
        assert_eq!(camera.radius, 380.);
    }

    #[test]
    fn a_distant_target_far_above_the_player_keeps_the_player_look_anchor() {
        // Head to look point farther than 300 and pointing down: the target is
        // high above, so the look point stays on the player's upper body.
        let (mut camera, step) = locked([0.; 3], [0., 0., 1.], [0., -1500., 100.]);
        camera.tick(&step, &|_, _| None);
        assert_eq!(camera.look_at_target, [0., -170., 0.]);
        // A target far below does not trigger the rule.
        let (mut camera, step) = locked([0.; 3], [0., 0., 1.], [0., 600., 100.]);
        camera.tick(&step, &|_, _| None);
        assert_ne!(camera.look_at_target, [0., -170., 0.]);
    }

    #[test]
    fn projection_matches_the_virtual_screen() {
        let camera = FollowCamera::new(OUTDOOR, &input([0.; 3], [0., 0., 1.]));
        // The look-at point is on the optical axis: the layout centre.
        let [x, y, z] = camera.project(camera.look_at);
        assert!((x - 256.).abs() < 1e-2 && (y - 208.).abs() < 1e-2);
        assert!(z > 0.);
        // 1.5 radians span the 512-wide layout: the edge is at tan(0.75).
        let forward = normalize_or(sub(camera.look_at, camera.eye), [0., 0., 1.]);
        let down = normalize_or(
            sub([0., 1., 0.], forward.map(|c| c * forward[1])),
            [0., 1., 0.],
        );
        let right = cross(down, forward);
        let edge = camera.project(
            [0, 1, 2].map(|i| camera.eye[i] + forward[i] * 500. + right[i] * 500. * 0.931_596_3),
        );
        assert!((edge[0] - 512.).abs() < 0.1, "{edge:?}");
        assert!((edge[1] - 208.).abs() < 0.1, "{edge:?}");
        assert_eq!(camera.screen_flags(camera.look_at), 0);
        assert_eq!(camera.screen_flags([5000., -170., 0.]), OFF_RIGHT);
        assert_eq!(camera.screen_flags([-5000., -170., 0.]), OFF_LEFT);
        assert_eq!(
            camera.screen_flags([0., -170., -2000.]) & OFF_NEAR,
            OFF_NEAR
        );
    }

    #[test]
    fn a_target_beyond_a_side_edge_swings_the_yaw_target_with_a_bias() {
        let (mut camera, step) = locked([0.; 3], [0., 0., 1.], [3000., -100., 0.]);
        let flags = camera.screen_flags([3000., -100., 0.]);
        assert_eq!(flags, OFF_RIGHT);
        let look_before = camera.look_at_target;
        camera.tick(&step, &|_, _| None);
        // Bearing from the old look target to the player, 20 degrees off.
        let bearing = (0. - look_before[0]).atan2(0. - look_before[2]);
        assert!((camera.yaw_target - wrap(bearing - EDGE_YAW)).abs() < 1e-5);
        // The yaw follows by at most 0.1745 a frame.
        assert!(
            (wrap(camera.yaw - camera.yaw_target)).abs() >= 0. && camera.yaw != camera.yaw_target
        );
    }

    #[test]
    fn a_target_below_the_screen_zooms_in_two_units_a_frame_down_to_the_minimum() {
        // Straight below the optical axis, near the player: off the bottom.
        let (mut camera, step) = locked([0.; 3], [0., 0., 1.], [0., 3000., -300.]);
        assert_eq!(camera.screen_flags([0., 3000., -300.]) & 3, OFF_BOTTOM);
        camera.tick(&step, &|_, _| None);
        assert_eq!(camera.radius, 418.);
        for _ in 0..200 {
            camera.tick(&step, &|_, _| None);
            assert!((OUTDOOR.radius_min..=OUTDOOR.radius_max).contains(&camera.radius));
        }
    }

    #[test]
    fn lock_accepts_manual_turn_zoom_and_recenter() {
        let (mut camera, mut step) = locked([0.; 3], [0., 0., 1.], [0., -50., 300.]);
        step.turn = 1.;
        let yaw = camera.yaw;
        camera.tick(&step, &|_, _| None);
        assert!((camera.yaw - wrap(yaw + MANUAL_TURN)).abs() < 1e-6);
        assert_eq!(camera.yaw_target, camera.yaw);
        step.turn = 0.;
        step.zoom = -1.;
        camera.radius = 420.;
        camera.tick(&step, &|_, _| None);
        assert_eq!(camera.radius, 420. - 1. - MANUAL_ZOOM);
        step.zoom = 0.;
        step.recenter = true;
        camera.tick(&step, &|_, _| None);
        assert_eq!(camera.radius, 380.);
    }

    #[test]
    fn lock_clips_the_eye_from_the_players_upper_body_without_moving_its_target() {
        let wall = [[
            [-1000., -1000., -100.],
            [1000., -1000., -100.],
            [0., 1000., -100.],
        ]];
        let (mut camera, step) = locked([0.; 3], [0., 0., 1.], [0., -50., 400.]);
        camera.tick(&step, &|a, b| triangle_hit(&wall, a, b));
        assert!((camera.eye[2] - (-95.)).abs() < 1e-3);
        // The eye target was clipped from the same point, 5 units off the
        // wall; the post-spring clip never rewrites it.
        assert!((camera.eye_target[2] - (-95.)).abs() < 1e-3);
        camera.eye_target[2] = -300.;
        camera.tick(&step, &|_, _| None);
        assert!((camera.eye_target[2] + 419.).abs() < 1.5);
    }

    #[test]
    fn snapped_camera_sits_behind_and_above() {
        let camera = FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.]));
        // Facing +Z puts the eye at -Z, 420 back; Y is down, so above is
        // negative: look-at 170 up, eye 20 + 170^2 * 0.008 up.
        assert!((camera.eye[2] + 420.).abs() < 1e-3);
        assert!(camera.eye[0].abs() < 1e-3);
        assert!((camera.look_at[1] + 170.).abs() < 1e-3);
        assert!((camera.eye[1] + (20. + 170. * 170. * 0.008)).abs() < 1e-2);
        assert!((camera.vertical_fov() - 1.2197).abs() < 1e-3);
    }

    #[test]
    fn wrap_matches_the_executable_branches() {
        use std::f32::consts::PI;
        assert!((wrap(PI + 0.5) - (-PI + 0.5)).abs() < 1e-5);
        assert!((wrap(-PI - 0.5) - (PI - 0.5)).abs() < 1e-5);
        assert_eq!(wrap(0.25), 0.25);
        assert!((approach(3.0, -3.0, 0.1) - 3.1).abs() < 1e-5);
    }

    #[test]
    fn spring_eases_toward_a_still_target() {
        let out = spring([100., 0., 0.], [0., 0., 0.], [0., 0., 0.], 0.035);
        // (1 - 100) * 2.5 * 0.035 = -8.6625 along the offset.
        assert!((out[0] - (100. - 8.6625)).abs() < 1e-3);
        assert_eq!(
            spring([1., 2., 3.], [1., 2., 3.], [0.; 3], 0.5),
            [1., 2., 3.]
        );
    }

    #[test]
    fn a_surface_five_hit_is_recast_at_the_hit_height_toward_the_clipped_end() {
        let second = std::cell::Cell::new(None);
        let cast = |from: Vec3, to: Vec3| {
            if to[1] == -50. {
                Some(RayHit {
                    point: [0., -100., -50.],
                    normal: [0., 0., 1.],
                    surface: 5,
                })
            } else {
                second.set(Some((from, to)));
                Some(RayHit {
                    point: [3., -100., -80.],
                    normal: [1., 0., 0.],
                    surface: 0,
                })
            }
        };
        // Previous target (1, -40, 2) and new end (10, -50, -300).
        let end = clip([1., -40., 2.], [0., -170., 0.], [10., -50., -300.], &cast).unwrap();
        // The re-cast runs from the origin to the end's X and Z at the hit's Y.
        assert_eq!(second.get(), Some(([0., -170., 0.], [10., -100., -300.])));
        // The re-cast point replaces the hit; the first polygon's normal pushes it.
        assert_eq!(end, [3. + 0., -100., -80. + 5.]);
        // Other surfaces use the first hit directly.
        let plain = |_: Vec3, _: Vec3| {
            Some(RayHit {
                point: [0., -10., -50.],
                normal: [0., 0., 1.],
                surface: 3,
            })
        };
        assert_eq!(
            clip([0.; 3], [0.; 3], [0., 0., -300.], &plain),
            Some([0., -10., -45.])
        );
    }

    fn ball(z: f32, radius: f32) -> Sphere {
        Sphere {
            centre: [0., 0., z],
            radius,
        }
    }

    #[test]
    fn a_segment_enters_a_sphere_at_its_near_surface() {
        let hit = sphere_hit(&[ball(-100., 20.)], [0.; 3], [0., 0., -300.]).unwrap();
        assert!((hit[2] + 80.).abs() < 1e-3 && hit[0].abs() < 1e-3);
        // Off the line by more than the radius, and beyond the end.
        let aside = Sphere {
            centre: [50., 0., -100.],
            radius: 20.,
        };
        assert_eq!(sphere_hit(&[aside], [0.; 3], [0., 0., -300.]), None);
        assert_eq!(
            sphere_hit(&[ball(-400., 20.)], [0.; 3], [0., 0., -300.]),
            None
        );
        // The segment ends inside the sphere: the closest point is past the
        // end, the end is within the radius, and the entry is still found.
        let ending = sphere_hit(&[ball(-310., 20.)], [0.; 3], [0., 0., -300.]).unwrap();
        assert!((ending[2] + 290.).abs() < 1e-3);
        // A zero-length segment never hits.
        assert_eq!(sphere_hit(&[ball(0., 20.)], [0.; 3], [0.; 3]), None);
    }

    #[test]
    fn a_sphere_holding_the_start_stops_the_scan_and_the_nearest_entry_wins() {
        let start = [0.; 3];
        let end = [0., 0., -300.];
        // The first sphere contains the start: the executable never steps past
        // it, so the second sphere is not tested.
        assert_eq!(
            sphere_hit(&[ball(-10., 20.), ball(-100., 20.)], start, end),
            None
        );
        // Each accepted entry shortens the segment the later spheres are
        // tested against, so the nearer entry wins in either list order.
        for order in [
            [ball(-100., 20.), ball(-200., 20.)],
            [ball(-200., 20.), ball(-100., 20.)],
        ] {
            let hit = sphere_hit(&order, start, end).unwrap();
            assert!((hit[2] + 80.).abs() < 1e-3);
        }
        // At most 0x40 spheres are read.
        let mut many = vec![ball(-5000., 1.); MAX_SPHERES];
        many.push(ball(-100., 20.));
        assert_eq!(sphere_hit(&many, start, end), None);
    }

    #[test]
    fn a_sphere_cut_ends_the_clip_with_or_without_a_polygon_behind() {
        let start = [0.; 3];
        let end = [0., 0., -300.];
        let sphere = [ball(-100., 20.)];
        // No polygon: the end sits exactly on the sphere.
        let none = |_: Vec3, _: Vec3| None;
        let cut = clip_with([0.; 3], start, end, &none, &sphere).unwrap();
        assert!((cut[2] + 80.).abs() < 1e-3);
        // A polygon behind the sphere is out of reach of the shortened segment.
        let behind = |from: Vec3, to: Vec3| {
            (to[2] <= -250.).then_some(RayHit {
                point: [from[0], from[1], -250.],
                normal: [0., 0., 1.],
                surface: 0,
            })
        };
        let cut = clip_with([0.; 3], start, end, &behind, &sphere).unwrap();
        assert!((cut[2] + 80.).abs() < 1e-3);
        // A polygon in front of the sphere wins and is pushed off by 5.
        let front = |_: Vec3, _: Vec3| {
            Some(RayHit {
                point: [0., 0., -40.],
                normal: [0., 0., 1.],
                surface: 0,
            })
        };
        let cut = clip_with([0.; 3], start, end, &front, &sphere).unwrap();
        assert_eq!(cut, [0., 0., -35.]);
        // Without spheres the plain clip is unchanged.
        assert_eq!(clip([0.; 3], start, end, &none), None);
    }

    #[test]
    fn the_eye_stops_at_a_dynamic_sphere_behind_the_player() {
        let mut camera = FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.]));
        // The follow camera sits behind +Z facing, so a sphere on the way
        // from the look point to the eye target cuts the eye target short.
        let eye_target = camera.eye_target;
        let look = camera.look_at_target;
        let toward = sub(eye_target, look);
        let centre = [0, 1, 2].map(|i| look[i] + toward[i] * 0.5);
        let sphere = [Sphere {
            centre,
            radius: 20.,
        }];
        camera.tick_with(&input([0., 0., 0.], [0., 0., 1.]), &|_, _| None, &sphere);
        let reach = length(sub(camera.eye_target, look));
        assert!(reach < length(toward) * 0.6, "eye target cut at the sphere");
    }

    #[test]
    fn zoom_obeys_the_follow_targets_lock_unless_the_override_is_held() {
        // The radius after one frame; the lock camera also drifts toward its
        // own radius, so a gated zoom must equal no zoom at all.
        let radius = |zoom: f32, locked: bool, held: bool, lock: bool| {
            let mut step = input([0.; 3], [0., 0., 1.]);
            step.zoom = zoom;
            step.zoom_locked = locked;
            step.zoom_override = held;
            if lock {
                step.lock_target = Some([100., -50., 200.]);
            }
            let mut camera = FollowCamera::new(OUTDOOR, &step);
            camera.tick(&step, &|_, _| None);
            camera.radius
        };
        for lock in [false, true] {
            let rest = radius(0., false, false, lock);
            assert_eq!(radius(-1., true, false, lock), rest);
            assert!(radius(-1., true, true, lock) < rest);
            assert!(radius(-1., false, false, lock) < rest);
        }
    }

    #[test]
    fn follow_needs_the_right_stick_idle_and_recenter_beats_a_manual_turn() {
        let mut moving = input([0., 0., 0.], [1., 0., 0.]);
        moving.moved = true;
        moving.magnitude = 1.;
        let yaw_after = |step: &CameraInput| {
            let mut camera = FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.]));
            camera.tick(step, &|_, _| None);
            camera.yaw
        };
        let start = FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.])).yaw;
        let idle = yaw_after(&moving);
        let mut deflected = moving;
        deflected.right_stick = 0.5;
        assert_ne!(
            idle,
            FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.])).yaw
        );
        // Any right-stick deflection, even vertical zoom, stops the swing.
        assert!(wrap(yaw_after(&deflected) - start).abs() < 1e-5);
        let mut zooming = moving;
        zooming.zoom = 0.5;
        assert!(wrap(yaw_after(&zooming) - start).abs() < 1e-5);
        // Recenter and turn together: the recenter wins, so the yaw target is
        // behind the player and the manual turn is not applied.
        let mut both = input([0., 0., 0.], [1., 0., 0.]);
        both.recenter = true;
        both.turn = 1.;
        let mut camera = FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.]));
        camera.tick(&both, &|_, _| None);
        assert!((camera.yaw_target - behind([1., 0., 0.])).abs() < 1e-5);
    }

    #[test]
    fn wall_between_look_at_and_eye_pulls_the_eye_in() {
        // A wall at z = -100 across the path behind a player facing +Z.
        let wall = [[
            [-1000., -1000., -100.],
            [1000., -1000., -100.],
            [0., 1000., -100.],
        ]];
        let mut camera = FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.]));
        camera.tick(&input([0., 0., 0.], [0., 0., 1.]), &|a, b| {
            triangle_hit(&wall, a, b)
        });
        assert!((camera.eye[2] - (-95.)).abs() < 1e-3);
        assert!(triangle_hit(&wall, [0., 0., 0.], [0., 0., -50.]).is_none());
    }

    #[test]
    fn follow_swings_behind_a_moving_player_and_pauses_when_running_at_it() {
        let mut camera = FollowCamera::new(OUTDOOR, &input([0., 0., 0.], [0., 0., 1.]));
        let mut step = input([0., 0., 0.], [1., 0., 0.]);
        step.moved = true;
        step.magnitude = 1.;
        camera.tick(&step, &|_, _| None);
        // Behind +Z is -pi; behind +X is -pi/2. Full stick, no tilt turns
        // 2 * 1 * 1 * 1 * FOLLOW_SPEED per frame toward it.
        let expected = -std::f32::consts::PI + 2. * FOLLOW_SPEED;
        assert!((camera.yaw - expected).abs() < 1e-5);
        let before = camera.yaw;
        step.stick = [0., 1.];
        camera.tick(&step, &|_, _| None);
        assert_eq!(camera.yaw, before);
    }
}
