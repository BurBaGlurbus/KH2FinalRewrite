//! Test-room player controller: a fixed-tick state machine for locomotion,
//! jumping and grounded/aerial attack combos. It owns position, facing and
//! the active action/motion frame; presentation only reads this state.
//!
//! Movement parameters follow the executable's object init
//! (`actor_physics::ObjParams`: PRTY walk/run/jump/turn, SSTM fall max) and its
//! gravity (`actor_physics::G`); PTYA supplies the combo motions and ANB range
//! triggers the windows. Step height, body size and the collision response
//! are explicit test-room choices, not observed KH2 rules.

pub mod abilities;
pub mod actor_physics;
pub mod combat;
pub mod drive;
pub mod dusk;
pub mod enemy_prizes;
pub mod equipment_stats;
pub mod follow_camera;
pub mod forms;
pub mod growth;
pub mod limit_form;
pub mod magic;
pub mod party_hit;
pub mod party_members;
pub mod party_spell;
pub mod ptya;
pub mod shadow;
pub mod soldier;
pub mod targeting;
pub mod wisdom_form;

pub type Vec3 = [f32; 3];

/// One update is one 60 Hz global frame, matching the public ANB note that
/// gameplay motion runs against a 60-per-second global frame rate.
pub const TICKS_PER_SECOND: u32 = 60;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Units per tick.
    pub walk_speed: f32,
    pub run_speed: f32,
    /// Apex height in units.
    pub jump_height: f32,
    /// Radians per tick.
    pub turn_speed: f32,
    /// Maximum downward speed, units per tick.
    pub fall_max: f32,
    /// PRTY words 10/11 (record +0x28/+0x2c): inertia and drag of mode 0's
    /// range-42 movement (FUN_140422350 → FUN_1403b7ad0).
    pub land_inertia: f32,
    pub land_drag: f32,
    /// Test-room choice: highest ledge climbed without jumping.
    pub step_height: f32,
    /// Stick magnitude from which Sora runs (`DAT_140585c7c`, 0.8).
    pub walk_threshold: f32,
    /// Test-room choice: body radius/height for wall contact.
    pub radius: f32,
    pub height: f32,
    /// Test-room choice: ticks without attacking before the weapon is put
    /// away and field locomotion resumes.
    pub disarm_ticks: u32,
}

impl Params {
    pub fn validate(&self) -> Result<(), &'static str> {
        let positive = [
            self.walk_speed,
            self.run_speed,
            self.jump_height,
            self.turn_speed,
            self.fall_max,
            self.radius,
            self.height,
        ];
        if positive.iter().any(|v| !v.is_finite() || *v <= 0.)
            || !self.step_height.is_finite()
            || self.step_height < 0.
            || !(0. ..1.).contains(&self.walk_threshold)
        {
            return Err("player parameters must be finite and positive");
        }
        Ok(())
    }
}

/// Motion timing in ticks relative to the motion's FrameStart, with the ANB
/// range triggers the controller honours. `end` is FrameEnd and
/// `loop_start` FrameReturn; a return at or past the end holds the last frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Clip {
    pub end: u32,
    pub loop_start: u32,
    /// Range trigger 11, "Allow combo".
    pub combo_windows: Vec<(u32, u32)>,
    /// Range trigger 50: a finisher may chain (obj+0x194 bit 5).
    pub finisher_windows: Vec<(u32, u32)>,
    /// Frame trigger 17 (spell release): tick and the motion id it is
    /// filtered by (None releases for any motion).
    pub release: Vec<(u32, Option<u16>)>,
    /// Earliest range trigger 2, "Blend to idle".
    pub blend_to_idle: Option<u32>,
    /// Range trigger 3, "No gravity".
    pub no_gravity: Vec<(u32, u32)>,
    /// Range triggers 0, 1 and 2: commands are taken while one is active
    /// (obj+0x18c & 0x1c, FUN_14040d810 / FUN_1401b15f0).
    pub actionable: Vec<(u32, u32)>,
    /// Range trigger 42: mode 0 moves inertially (obj+0x190 bit 0,
    /// FUN_140422350 → FUN_1403b7ad0 with PRTY words 10/11).
    pub inertia: Vec<(u32, u32)>,
    /// Range trigger 28: turn toward the target by up to `deg` per tick.
    pub turn_to_target: Vec<(u32, u32, f32)>,
    /// Frame trigger 21: turn toward the target by up to `deg` at a tick.
    pub face_target: Vec<(u32, f32)>,
    /// Frame trigger 0: the pending physics mode starts (jump take-off,
    /// FUN_14040dd70).
    pub takeoff: Option<u32>,
    /// Range trigger 27: invulnerable (obj+0x18c bit 22, FUN_1403d3570).
    pub invulnerable: Vec<(u32, u32)>,
    /// Frame trigger 6: the item is used (FUN_14040dd70 → FUN_1403f6790).
    pub item_use: Option<u32>,
    /// Frame trigger 1: an object effect starts (tick, effect number);
    /// Wisdom's shots ride on it (FUN_14040dd70 case 1).
    pub effects: Vec<(u32, i16)>,
    /// Range triggers 16, 17 (script window bits, ctl+0x154), 20 (offer
    /// a reaction command every tick) and 26: (start, end, id, first
    /// parameter) (FUN_14040d810).
    pub windows: Vec<(u32, u32, u8, i16)>,
    /// Frame trigger 9: a one-frame script mark (ctl+0x155 bit 0).
    pub marks: Vec<u32>,
    /// Frame trigger 14: a battle voice (`vtbl` row n of the character).
    pub voices: Vec<(u32, i16)>,
    /// Frame trigger 2: a footstep (first parameter p0, which selects the
    /// step sound within the floor material's group of 16).
    pub steps: Vec<(u32, i16)>,
    /// Frame trigger 19 (FUN_140424c60): ground dust at the foot, by the
    /// trigger parameter (the foot id).
    pub dust: Vec<(u32, i16)>,
    /// Frame trigger 8: a sound request (`p0 * 1000 + p1`) on the actor's
    /// own motion (cloth, jump and item-use sounds, bank 0 and 999).
    pub sounds: Vec<(u32, u32)>,
    /// Frame trigger 7: a script event (0x11 → TR17) with its two
    /// parameters.
    pub events: Vec<(u32, i16, i16)>,
}

impl Clip {
    pub fn looping(end: u32, loop_start: u32) -> Self {
        Self {
            end,
            loop_start,
            ..Self::default()
        }
    }
    fn within(ranges: &[(u32, u32)], frame: u32) -> bool {
        ranges.iter().any(|&(a, b)| (a..=b).contains(&frame))
    }
    /// Frame shown for a running counter: plays to FrameEnd, then loops
    /// from FrameReturn (or holds FrameEnd when there is no loop span).
    pub fn looped_frame(&self, frame: u32) -> u32 {
        if frame <= self.end || self.loop_start >= self.end {
            frame.min(self.end)
        } else {
            let span = self.end - self.loop_start;
            self.loop_start + (frame - self.loop_start) % span
        }
    }
    pub fn held_frame(&self, frame: u32) -> u32 {
        frame.min(self.end)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttackStep {
    pub clip: Clip,
    /// PTYA +0xc "Jump": hop height when the step starts with no target
    /// (FUN_1403d78d0).
    pub hop: f32,
    /// PTYA +0x1c (speed maximum): horizontal speed of a hop without a
    /// target, and the cap toward one (FUN_1403d78d0).
    pub hop_speed: f32,
    /// PTYA +0x10 / +0x14: hop height limits toward a target.
    pub hop_max: f32,
    pub hop_min: f32,
    /// PTYA +0x18: minimum hop speed toward a target.
    pub hop_speed_min: f32,
    /// PTYA +0x3c: distance adjustment toward a target.
    pub hop_reach: f32,
    /// PTYA +0x38: blend into the next motion when this one ends
    /// (FUN_1403d78d0 → FUN_1403c8310).
    pub blend: f32,
    /// Root-position translation per tick in model space (+Z is facing).
    /// Its per-tick change moves the player; empty means no root motion.
    pub root: Vec<Vec3>,
}

/// A cast motion (magic.rs): its id, clip and root motion.
#[derive(Clone, Debug, PartialEq)]
pub struct SpellClip {
    pub motion: u16,
    pub clip: Clip,
    pub root: Vec<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Clips {
    /// Battle-mode (weapon drawn) locomotion.
    pub idle: Clip,
    pub walk: Clip,
    pub run: Clip,
    /// Field-mode (weapon put away) locomotion.
    pub field_idle: Clip,
    pub field_walk: Clip,
    pub field_run: Clip,
    pub jump: Clip,
    pub fall: Clip,
    pub land: Clip,
    /// The battler's PTYA records (obj+0xd50) and, at the same index,
    /// each record's motion as an attack step.
    pub records: Vec<ptya::Record>,
    pub attacks: Vec<AttackStep>,
    /// Cast motions by `Action::Cast(i)` / `AirCast(i)`.
    pub spells: Vec<SpellClip>,
    /// Sora's damage motions in [`HURT_MOTIONS`] order. Empty when not
    /// loaded.
    pub hurt: Vec<Clip>,
    /// Guard (motion 173) and its success recoil (motion 42), when loaded.
    pub guard: Option<Clip>,
    pub guard_recoil: Option<Clip>,
    /// The item-use motion (9), when loaded.
    pub item: Option<Clip>,
    /// The drive-start (0x34) and drive-in (0x35) motions, when the set
    /// holds them.
    pub drive: Option<Clip>,
    pub drive_in: Option<Clip>,
    /// The growth abilities' motions (`growth::MOTIONS`) with their root
    /// motion, when the set holds them.
    pub growth: Vec<(u16, Clip, Vec<Vec3>)>,
    /// Motions scripts start by id (the Limit Form limits), with root
    /// motion.
    pub scripted: Vec<(u16, Clip, Vec<Vec3>)>,
}

impl Clips {
    pub fn growth_clip(&self, motion: u16) -> Option<&(u16, Clip, Vec<Vec3>)> {
        self.growth.iter().find(|g| g.0 == motion)
    }

    pub fn scripted_clip(&self, motion: u16) -> Option<&(u16, Clip, Vec<Vec3>)> {
        self.scripted.iter().find(|g| g.0 == motion)
    }
}

/// Sora's damage-reaction motions in `Clips::hurt` order (P_EX100 slots
/// 4m+2): S_FRONT_HIGH, S_FRONT_LOW, S_BACK_HIGH, S_BACK_LOW (10..13),
/// AIR_FRONT, AIR_BACK (14, 15), BLOW_FRONT, BLOW_FRONT_LAND, BLOW_BACK,
/// BLOW_BACK_LAND (16..19), Aerial Recovery (40) and DEAD (54).
pub const HURT_MOTIONS: [u16; 12] = [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 40, 54];

/// Indices into [`HURT_MOTIONS`].
pub mod hurt {
    /// AIR_FRONT / AIR_BACK.
    pub const AIR: [usize; 2] = [4, 5];
    /// BLOW_FRONT / BLOW_BACK, and the land motions that follow them.
    pub const BLOW: [usize; 2] = [6, 8];
    pub const BLOW_LAND: [usize; 2] = [7, 9];
    pub const RECOVERY: usize = 10;
    pub const DEAD: usize = 11;
}

/// The hop an aerial hit gives (`DAT_1405b532c`, FUN_1403d4800).
pub const AIR_HIT_HOP: f32 = 50.;
/// Sora's `plyr` record (pref, id 1): Aerial Recovery's jump height (+0x50),
/// horizontal speed (+0x54) and the ticks a blow must have run (+0x58).
pub const RECOVERY_HEIGHT: f32 = 100.;
pub const RECOVERY_SPEED: f32 = 10.;
pub const RECOVERY_AFTER: f32 = 16.;
/// Retaliating Slash's PTYA record id (FUN_1403d6d50).
pub const RETALIATING_SLASH: u8 = 0x28;
/// The Square combo abilities in FUN_1403a81f0's order: Finishing Leap
/// (0x25, motion 167), Upper Slash (0x0c, motion 161), Horizontal Slash
/// (0x26, motion 193).
pub const SQUARE_ABILITIES: [u8; 3] = [0x25, 0x0c, 0x26];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    Idle,
    Walk,
    Run,
    Jump,
    Fall,
    Land,
    /// A grounded attack (index into `Clips::records` / `attacks`).
    Attack(usize),
    /// An attack in the air, including ground records that hop.
    AirAttack(usize),
    /// A spell cast on the ground / in the air (index into
    /// `Clips::spells`).
    Cast(usize),
    AirCast(usize),
    /// A damage reaction (index into `Clips::hurt`).
    Hurt(usize),
    /// Guard (PTYA 0x0b, motion 173): one press, actionable from its
    /// range-2 tick.
    Guard,
    /// Guard success recoil (motion 42).
    GuardRecoil,
    /// Using an item (motion 9): actionable again from its range-2 tick.
    Item,
    /// The Drive command's start (motion 0x34, FUN_14040ce60): physics
    /// mode 0 with no velocity, held until the form change swaps the
    /// player object.
    Drive,
    /// A Drive form's arrival (motion 0x35, FUN_140405630): locked until
    /// its range-2 tick, like an item.
    DriveIn,
    /// The High Jump's jump (motion 201, FUN_1403b59b0): JUMP with the
    /// growth row's height.
    HighJump,
    /// A growth ability's motion (`growth::motion`): Aerial Dodge (202),
    /// Quick Run (203, 204), Glide (205..208) or Dodge Roll (209).
    Growth(u16),
    /// A motion a script started by id (the Limit Form limits, BDX trap
    /// 1,11 motion_start): no commands unless its ranges allow them; it
    /// plays its queued motions when it ends (trap 1,13 motion_queue).
    Scripted(u16),
}

impl Action {
    pub fn airborne(self) -> bool {
        matches!(
            self,
            Self::Jump | Self::HighJump | Self::Fall | Self::AirAttack(_) | Self::AirCast(_)
        ) || matches!(self, Self::Growth(m) if m == growth::motion::AERIAL_DODGE
            || (growth::motion::GLIDE..=growth::motion::GLIDE_STAND).contains(&m))
            || matches!(self, Self::Hurt(i) if hurt::AIR.contains(&i)
            || hurt::BLOW.contains(&i)
            || i == hurt::RECOVERY)
    }
}

/// Camera-relative stick already converted to a world XZ direction, with
/// magnitude 0..=1, plus button edges for this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Input {
    pub direction: [f32; 2],
    pub jump: bool,
    /// The Jump button is up this tick (request byte +4 bit 0 clear): a
    /// jump still rising is cut (`JumpCut`).
    pub jump_up: bool,
    pub attack: bool,
    /// The Square button (request byte +4 bit 3): Retaliating Slash, the
    /// Square combo abilities, then Guard (FUN_1403d6d50, FUN_1403a81f0).
    pub guard: bool,
    /// Square is held (request byte +4 bit 2 [I]): Quick Run and Glide
    /// last while it is.
    pub guard_held: bool,
    /// XZ direction toward the current target, if any (Guard turns to it
    /// at its first tick: frame trigger 21, up to 360°).
    pub target: Option<[f32; 2]>,
    /// Target height above the player (Y-up) and the two bodies' summed
    /// radii, for aerial hops toward it (FUN_1403d78d0 / FUN_1403d7520).
    pub target_rise: f32,
    pub target_reach: f32,
    /// The current target's TARGET shape (the attack chooser's "target").
    pub target_shape: Option<ptya::Target>,
    /// Every attackable object's TARGET shapes near the player.
    pub nearby: [Option<ptya::Target>; 4],
    /// Grounded probe ahead hit the background (FUN_1403ba750) [U].
    pub probe_blocked: bool,
}

/// The player's collision body as the controller hands it to the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Body {
    pub radius: f32,
    pub height: f32,
    /// Highest ledge followed without leaving the ground (snap-down limit).
    pub step: f32,
}

/// The result of one tick's body-versus-map move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Collided {
    /// Feet position after the move.
    pub position: Vec3,
    /// Grounded move: a floor is still underfoot (false = walked off).
    /// Airborne move: the body reached a floor while not rising (landed).
    pub on_floor: bool,
}

/// Room collision as used by the controller.
pub trait World {
    /// Highest walkable surface at (x, z) whose height is at most `top`.
    fn floor(&self, x: f32, z: f32, top: f32) -> Option<f32>;
    /// Pushes a body circle out of walls overlapping the vertical span.
    fn resolve_walls(&self, position: [f32; 2], radius: f32, low: f32, high: f32) -> [f32; 2];

    /// One body-versus-map move per tick, as FUN_1403b89a0 hands the tick's
    /// whole displacement (mode velocity, root motion, knock-slide) to the
    /// collision solver: from the feet at `start` by `displacement` (Y-up).
    /// The default is the test room's solver built from `floor` and
    /// `resolve_walls`; a map collision port overrides it.
    fn collide(&self, start: Vec3, displacement: Vec3, airborne: bool, body: &Body) -> Collided {
        let [x, y, z] = start;
        let [dx, dy, dz] = displacement;
        let low = y + body.step;
        let mut position = start;
        if dx != 0. || dz != 0. {
            let moved = self.resolve_walls([x + dx, z + dz], body.radius, low, y + body.height);
            // A floor reachable from this height must exist under the new point.
            match self.floor(moved[0], moved[1], low) {
                Some(floor) => {
                    position[0] = moved[0];
                    position[2] = moved[1];
                    if !airborne {
                        if y - floor > body.step {
                            return Collided {
                                position,
                                on_floor: false,
                            };
                        }
                        position[1] = floor;
                    }
                }
                None if !airborne => {
                    return Collided {
                        position: start,
                        on_floor: true,
                    };
                }
                None => {}
            }
        }
        if !airborne {
            return Collided {
                position,
                on_floor: true,
            };
        }
        let next = y + dy;
        if dy <= 0.
            && let Some(floor) = self.floor(position[0], position[2], low)
            && next <= floor
        {
            position[1] = floor;
            return Collided {
                position,
                on_floor: true,
            };
        }
        position[1] = next;
        Collided {
            position,
            on_floor: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    pub position: Vec3,
    /// Facing angle; facing direction is (sin yaw, cos yaw) in XZ.
    pub yaw: f32,
    /// Last tick's vertical displacement (Y-up units per tick).
    pub vertical_speed: f32,
    /// Jump/fall curve while airborne (obj+0xf0..+0xfc).
    pub arc: actor_physics::Arc,
    /// Horizontal velocity while airborne (obj+0x20 x/z), units per tick.
    pub air_velocity: [f32; 2],
    /// Physics-mode velocity sampling (FUN_1403b89a0, obj+0xba8/+0xb98).
    pub sampler: actor_physics::Sampler,
    /// A jump waiting for its take-off trigger (obj+0x648 = 1): height and
    /// take-off velocity (+0xe0).
    pub pending_jump: Option<(f32, [f32; 2])>,
    /// The variable-height jump flags obj+0x120 bits 16–18.
    pub jump_cut: JumpCut,
    pub action: Action,
    /// Ticks since the current action started.
    pub frame: u32,
    /// Battle mode: weapon drawn, battle locomotion clips.
    pub armed: bool,
    /// Blend, in ticks, from the previous motion into the current one.
    pub blend: f32,
    idle_ticks: u32,
    /// Combo counters obj+0xd74 / +0xd75.
    pub combo: ptya::Combo,
    /// obj+0x120 bit 25: the last attack connected (set by a hit or a
    /// clash, cleared when an attack starts).
    pub connected: bool,
    /// Battle stats +0x1d0.
    pub abilities: abilities::Abilities,
    /// Stats +0x1c0 > 0 (MP charge).
    pub mp_charging: bool,
    /// MP and MP Charge (stats +0x180 / +0x1bc / +0x1c0).
    pub mp: magic::Mp,
    /// The Drive gauge (stats +0x1b0..+0x1b2).
    pub drive: drive::Gauge,
    /// The cast in progress and the spell it released this tick.
    cast: Option<CastState>,
    released: Option<Released>,
    /// Knock-slide (FUN_1403d4960 / FUN_1403d4570), for hits and the guard
    /// recoil alike.
    knock: Option<actor_physics::Knock>,
    /// A blow in flight (physics mode 4) and the ticks it has run.
    flight: Option<Flight>,
    /// Dead and still falling (physics mode 0x29).
    dead_air: bool,
    /// The item-use trigger fired this tick (frame trigger 6).
    item_fired: bool,
    /// 03system `pref/fmab`: the growth abilities' rows.
    pub fmab: growth::Fmab,
    /// obj+0x124 bit 1: an Aerial Dodge is available (set on leaving the
    /// ground, cleared by the dodge, a command or a hit).
    dodge_ready: bool,
    /// obj+0x124 bit 2: Square still held since Quick Run or Glide began.
    square_held: bool,
    /// obj+0xd70: invulnerability frames left (FUN_1403d7e40).
    pub invulnerable: f32,
    /// Quick Run's mode 0x10 state.
    quick_run: Option<growth::QuickRun>,
    /// Gliding (modes 0x11 / 0x12): the row in use (obj+0x108).
    glide: Option<growth::Row>,
    /// obj+0xe0 with +0xec set: the take-off velocity of the last arc
    /// started by FUN_1403cbb00 (a glide restarts from it).
    launch_velocity: Option<[f32; 2]>,
    /// Wisdom Form's Attack is "Shoot" (0x16d): FUN_1403d6fc0 always
    /// allows its combo count.
    pub shoot: bool,
    /// Motions queued after the current one (FUN_1403c8310): id and blend.
    pub queued: Vec<(u16, f32)>,
    /// A scripted motion started in the air keeps the airborne physics.
    scripted_air: bool,
}

/// A blow in flight: mode 4's velocity with the SSTM brakes it runs on, and
/// the time in the mode (obj+0x110).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Flight {
    blow: actor_physics::Blow,
    sstm: actor_physics::Sstm,
    time: f32,
}

/// A damaging hit as the reaction routine sees it (FUN_1403d2fe0).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// ATKP reaction after the patn override (dmg+0x26) and as authored
    /// (dmg+0x27).
    pub reaction: u8,
    pub original: u8,
    /// XZ unit vector from the attacker toward Sora.
    pub direction: [f32; 2],
    /// The sphere struck is a HIGH one (arg 0, 2..5).
    pub high: bool,
    /// ATKP KB1 / KB2.
    pub knockback: [i16; 2],
}

/// A cast waiting for its release tick (obj+0xda0).
#[derive(Clone, Copy, Debug, PartialEq)]
struct CastState {
    start: magic::Start,
    magic_type: u8,
    level: u8,
    cost: u8,
    form: u8,
    /// Releases so far (a form's finisher can release several times, each
    /// running trigger 21 again) and the frame of the last one.
    volleys: u32,
    last_frame: Option<u32>,
}

/// A spell released this tick (FUN_1403c6150): MP is already spent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Released {
    pub magic_type: u8,
    pub level: u8,
    /// The `magi` row and ATKP sub-table.
    pub row: usize,
    pub finisher: bool,
    pub airborne: bool,
    pub form: u8,
    /// Index of this release within the cast (0 for the first).
    pub volley: u32,
}

/// The variable-height jump (obj+0x120 bits 16–18). The jump command sets
/// `variable` (FUN_140422630 with its third argument, as FUN_1403d6fc0
/// passes for the Jump button); entering the jump mode moves it to
/// `held` (FUN_1403cbd30); each command pass turns `held` into `released`
/// once the button is up (FUN_1403d6fc0, request byte +4 bit 0 [I: the
/// "button held" bit, paired with bit 1 for the press]); the jump mode cuts
/// a released rise (FUN_1403cb680). Leaving the jump mode clears all three
/// (FUN_1403cb8a0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct JumpCut {
    pub variable: bool,
    pub held: bool,
    pub released: bool,
}

/// Guard recoil knock strength (DAT_140585efc).
pub const GUARD_KNOCK: f32 = 200.;
/// Horizontal speed cap kept when walking off a ledge (reported by the map
/// collision trace of FUN_1403b4780 / FUN_1403b9090).
pub const WALK_OFF_SPEED: f32 = 8.;

/// What changed during one tick, for logging and presentation.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Step {
    pub started: Option<Action>,
    pub landed: bool,
}

/// Motion blend, in ticks, when an action's motion starts: locomotion and
/// the fall motion blend over 8 (`DAT_140623bac`: FUN_140422520,
/// FUN_1403cb610); jump (FUN_140422630), land (FUN_1403cb940), attacks
/// (FUN_1403d78d0), damage (FUN_1403d43b0), guard (FUN_1403a9c70) and its
/// recoil (FUN_1403d61d0) start with blend 0.
pub fn blend_for(action: Action) -> f32 {
    match action {
        Action::Idle | Action::Walk | Action::Run | Action::Fall => actor_physics::MOTION_BLEND,
        _ => 0.,
    }
}

fn wrap_angle(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

impl Player {
    pub fn new(position: Vec3, yaw: f32) -> Self {
        Self {
            position,
            yaw,
            vertical_speed: 0.,
            arc: actor_physics::Arc::default(),
            air_velocity: [0.; 2],
            sampler: actor_physics::Sampler::default(),
            pending_jump: None,
            jump_cut: JumpCut::default(),
            action: Action::Idle,
            frame: 0,
            armed: false,
            blend: 0.,
            idle_ticks: 0,
            combo: ptya::Combo::default(),
            connected: false,
            abilities: abilities::Abilities::default(),
            mp_charging: false,
            mp: magic::Mp::new(100),
            drive: drive::Gauge::new(3),
            cast: None,
            released: None,
            knock: None,
            flight: None,
            dead_air: false,
            item_fired: false,
            fmab: growth::Fmab::default(),
            dodge_ready: false,
            square_held: false,
            invulnerable: 0.,
            quick_run: None,
            glide: None,
            launch_velocity: None,
            shoot: false,
            queued: Vec::new(),
            scripted_air: false,
        }
    }

    /// BDX trap 1,11 (FUN_1403c86a0): play `motion` now with `blend`;
    /// clears the queue. Motion 0 is the idle and 4 the fall.
    pub fn play_motion(&mut self, motion: u16, blend: f32, clips: &Clips) -> bool {
        self.queued.clear();
        self.start_motion_id(motion, blend, clips)
    }

    fn start_motion_id(&mut self, motion: u16, blend: f32, clips: &Clips) -> bool {
        let air = self.in_air();
        let action = match motion {
            0 if !air => Action::Idle,
            0 | 4 => Action::Fall,
            m if clips.scripted_clip(m).is_some() => Action::Scripted(m),
            _ => return false,
        };
        if action == Action::Fall && !matches!(self.action, Action::Fall) {
            self.arc = actor_physics::Arc::fall(self.position[1], 0.);
            self.sampler.reset();
        }
        self.scripted_air = air;
        self.pending_jump = None;
        self.cast = None;
        self.action = action;
        self.frame = 0;
        self.blend = blend;
        self.arm();
        true
    }

    /// BDX trap 1,13 (FUN_1403c8310): play `motion` after the current one.
    pub fn queue_motion(&mut self, motion: u16, blend: f32) {
        self.queued.push((motion, blend));
    }

    /// BDX trap 6,67 (FUN_14053ef20 → FUN_1403d78d0): PTYA record `id` of
    /// the set at once, with its hop toward the target.
    pub fn ptya_attack(&mut self, id: u8, input: &Input, clips: &Clips) -> bool {
        let Some(i) = ptya::find(&clips.records, id) else {
            return false;
        };
        if clips.attacks.len() <= i {
            return false;
        }
        let mut step = Step::default();
        let combo = self.combo;
        self.execute(i, combo, input, clips, &mut step);
        true
    }

    /// BDX trap 1,4 (FUN_1403b65c0) with the facing reversed: turn round.
    pub fn turn_round(&mut self) {
        self.yaw = wrap_angle(self.yaw + std::f32::consts::PI);
    }

    /// BDX trap 1,111 with speed 0 and trap 1,146 (FUN_1403d4960): the
    /// move request cleared and a knock-slide backwards.
    pub fn recoil_back(&mut self, strength: f32) {
        self.air_velocity = [0.; 2];
        let back = [-self.yaw.sin(), -self.yaw.cos()];
        self.recoil(back, strength);
    }

    /// FUN_140401680: a growth family's level (0 = none).
    pub fn growth_level(&self, family: usize) -> u8 {
        self.abilities.growth[family]
    }

    fn growth_row(&self, family: usize) -> Option<growth::Row> {
        match self.growth_level(family) {
            0 => None,
            l => self.fmab.row(l).copied(),
        }
    }

    /// Starts a grounded damage reaction with a knock-slide away along
    /// `direction` (XZ) at `strength` × 0.1 per tick, ×0.9 per tick until
    /// below 1 (FUN_1403d4960 / FUN_1403d4570). Airborne actions only take
    /// the slide.
    pub fn hurt(
        &mut self,
        index: usize,
        direction: [f32; 2],
        strength: f32,
        clips: &Clips,
    ) -> Option<Action> {
        self.knock = actor_physics::Knock::new([direction[0], 0., direction[1]], strength, 0.);
        // A hit (vt+0xc0) removes the Aerial Dodge and ends a dash.
        self.dodge_ready = false;
        self.quick_run = None;
        if self.in_air() || index >= clips.hurt.len() {
            return None;
        }
        self.pending_jump = None;
        self.cast = None;
        self.action = Action::Hurt(index);
        self.frame = 0;
        self.blend = 0.;
        self.arm();
        Some(self.action)
    }

    /// A knock-slide without a reaction motion (`obj_hitback`, FUN_1403d4960):
    /// away along `direction` (XZ) at `strength` x 0.1 per tick.
    pub fn recoil(&mut self, direction: [f32; 2], strength: f32) {
        self.knock = actor_physics::Knock::new([direction[0], 0., direction[1]], strength, 0.);
    }

    /// FUN_1403d2fe0 for Sora: the reaction class picks a knock-slide only
    /// (1), a flinch (2), a flinch with a slide on the ground or an air
    /// flinch with a hop in the air (3), or a blow (4). Returns the new
    /// action when a motion started. Facing: a ground flinch keeps it; the
    /// air flinch and the blow turn to the attacker when hit from the front.
    pub fn react(
        &mut self,
        hit: &Hit,
        sstm: &actor_physics::Sstm,
        clips: &Clips,
    ) -> Option<Action> {
        if self.dead() {
            return None;
        }
        let class = combat::reaction_class(hit.reaction);
        let original = combat::reaction_class(hit.original);
        let dir = hit.direction;
        let back = {
            let (s, c) = self.yaw.sin_cos();
            s * dir[0] + c * dir[1] >= 0.
        };
        let facing = |back: bool| {
            let d = if back { dir } else { [-dir[0], -dir[1]] };
            d[0].atan2(d[1])
        };
        let slide = |kb: f32| actor_physics::Knock::new([dir[0], 0., dir[1]], kb, 0.);
        let strength = sstm.knock_strength(hit.knockback[0], original, hit.reaction);
        match class {
            1 => {
                self.knock = slide(strength);
                None
            }
            2 | 3 if !self.in_air() => {
                // FUN_1403d43b0: FRONT/BACK by the facing, HIGH/LOW by the
                // sphere; the partner motion when one is missing.
                let mut i = if back { 2 } else { 0 } + (!hit.high) as usize;
                if i >= clips.hurt.len() {
                    i ^= 1;
                }
                if i >= clips.hurt.len() {
                    return None;
                }
                self.knock = if class == 3 { slide(strength) } else { None };
                self.begin_reaction(Action::Hurt(i));
                Some(self.action)
            }
            2 | 3 => {
                // FUN_1403d3840 + FUN_1403d4800: the air flinch and a hop
                // (class 2 in the air takes the same motion here: the
                // ground flinch is not a mode-1 motion).
                let mut i = hurt::AIR[back as usize];
                if i >= clips.hurt.len() {
                    i = hurt::AIR[!back as usize];
                }
                if i >= clips.hurt.len() {
                    return None;
                }
                self.yaw = facing(back);
                self.knock = None;
                self.flight = None;
                self.arc = actor_physics::Arc::launch(self.position[1], AIR_HIT_HOP);
                self.jump_cut = JumpCut::default();
                let v = strength * sstm.hit_back_to_jump;
                self.air_velocity = [dir[0] * v, dir[1] * v];
                self.sampler.reset();
                self.vertical_speed = 0.;
                self.begin_reaction(Action::Hurt(i));
                Some(self.action)
            }
            4 => {
                // The blow: KB1 up and KB2 along the hit, or SSTM BlowUp /
                // BlowSpeed (FUN_1403d2fe0 case 4, FUN_140409d30 → mode 4).
                let (up, side) =
                    if original == 4 && (hit.knockback[0] != 0 || hit.knockback[1] != 0) {
                        (hit.knockback[0] as f32, hit.knockback[1] as f32)
                    } else {
                        (sstm.blow_up, sstm.blow_speed)
                    };
                let i = hurt::BLOW[back as usize];
                if i >= clips.hurt.len() {
                    return None;
                }
                self.yaw = facing(back);
                self.knock = None;
                let v = [dir[0] * side, up, dir[1] * side];
                self.jump_cut = JumpCut::default();
                self.flight = Some(Flight {
                    blow: actor_physics::Blow::new(v, sstm),
                    sstm: *sstm,
                    time: 0.,
                });
                self.sampler.reset();
                self.vertical_speed = 0.;
                self.air_velocity = [0.; 2];
                self.begin_reaction(Action::Hurt(i));
                Some(self.action)
            }
            _ => None,
        }
    }

    /// The common start of a reaction motion: blend 0, commands dropped.
    fn begin_reaction(&mut self, action: Action) {
        self.pending_jump = None;
        self.cast = None;
        self.action = action;
        self.frame = 0;
        self.blend = 0.;
        self.arm();
    }

    /// The item command (FUN_1403d6fc0 execute 3): motion 9 with a 8-tick
    /// blend, no combo counting. Needs an active ANB range 0–2 or the
    /// unlocked tail of a reaction, the ground (FUN_1403b6cf0) and a living
    /// Sora; the use itself fires at the motion's frame trigger 6.
    pub fn use_item(&mut self, clips: &Clips) -> Option<Action> {
        if self.dead() || self.in_air() || clips.item.is_none() || !self.command_ready(clips) {
            return None;
        }
        self.begin_reaction(Action::Item);
        self.blend = actor_physics::MOTION_BLEND;
        Some(self.action)
    }

    /// The Drive command (FUN_14040ce60 for a form): physics mode 0, motion
    /// 0x34 with no blend, the velocity (obj+0x20..+0x2f) cleared. Any
    /// cast, knock-slide, blow or pending jump ends.
    pub fn start_drive(&mut self) -> Action {
        self.pending_jump = None;
        self.cast = None;
        self.knock = None;
        self.flight = None;
        self.air_velocity = [0.; 2];
        self.sampler = actor_physics::Sampler::default();
        self.jump_cut = JumpCut::default();
        self.vertical_speed = 0.;
        self.connected = false;
        self.combo = ptya::Combo::default();
        self.action = Action::Drive;
        self.frame = 0;
        self.blend = 0.;
        self.arm();
        self.action
    }

    /// Whether the drive-start motion has played to its end
    /// (FUN_1403c7e70 on motion 0x34).
    pub fn drive_done(&self, clips: &Clips) -> bool {
        self.action != Action::Drive || self.frame >= self.clip(clips).end
    }

    /// The new player object's first frame (FUN_140405630): on the ground
    /// a form plays its drive-in motion (0x35); in the air, and after a
    /// revert, it falls (motion 4) or idles (motion 0).
    pub fn arrive(&mut self, form: u8, airborne: bool, clips: &Clips) -> Action {
        self.combo = ptya::Combo::default();
        self.frame = 0;
        self.blend = 0.;
        if airborne {
            self.arc = actor_physics::Arc::fall(self.position[1], 0.);
            self.air_velocity = [0.; 2];
            self.sampler = actor_physics::Sampler::default();
            self.action = Action::Fall;
        } else if form != 0 && clips.drive_in.is_some() {
            self.action = Action::DriveIn;
        } else {
            self.action = Action::Idle;
        }
        self.arm();
        self.action
    }

    /// Whether the item-use trigger fired since the last call.
    pub fn take_item(&mut self) -> bool {
        std::mem::take(&mut self.item_fired)
    }

    /// Whether the DEAD motion is playing (obj+0x9b8 bit 2).
    pub fn dead(&self) -> bool {
        self.action == Action::Hurt(hurt::DEAD)
    }

    /// Sora's death (FUN_140404d60 → FUN_140405a00): the DEAD motion with
    /// no horizontal motion; in the air the body keeps falling (mode 0x29).
    pub fn die(&mut self, clips: &Clips) -> Option<Action> {
        if self.dead() || clips.hurt.len() <= hurt::DEAD {
            return None;
        }
        let air = self.in_air();
        self.knock = None;
        self.flight = None;
        self.air_velocity = [0.; 2];
        self.sampler = actor_physics::Sampler::default();
        if air {
            self.arc = actor_physics::Arc::fall(self.position[1], self.vertical_speed);
            self.jump_cut = JumpCut::default();
        }
        self.begin_reaction(Action::Hurt(hurt::DEAD));
        self.dead_air = air;
        Some(self.action)
    }

    /// Back on his feet after the DEAD motion (FUN_1403c3c30 plays 0x37,
    /// which this set does not hold): idle.
    pub fn revive(&mut self) {
        if !self.dead() {
            return;
        }
        self.dead_air = false;
        self.action = Action::Idle;
        self.frame = 0;
        self.blend = 0.;
        self.combo = ptya::Combo::default();
    }

    /// An ANB range 27 is active (obj+0x18c bit 22): no damage.
    pub fn range_invulnerable(&self, clips: &Clips) -> bool {
        self.invulnerable > 0. || Clip::within(&self.clip(clips).invulnerable, self.frame)
    }

    /// FUN_1403d6d50, a blow's recovery: Jump with Aerial Recovery plays
    /// motion 40 and hops; Square starts Retaliating Slash (PTYA
    /// 0x28), setting the counters to 1/1 first. Both wait for the blow to
    /// have run `RECOVERY_AFTER` ticks.
    fn recover(&mut self, input: &Input, clips: &Clips, stick: [f32; 2], step: &mut Step) {
        let Some(flight) = self.flight else {
            return;
        };
        if flight.time < RECOVERY_AFTER {
            return;
        }
        if input.jump
            && self.abilities.has(abilities::bit::AERIAL_RECOVERY)
            && clips.hurt.len() > hurt::RECOVERY
        {
            let h = stick[0].hypot(stick[1]);
            let (dir, speed) = if h > 0. {
                self.yaw = stick[0].atan2(stick[1]);
                ([stick[0] / h, stick[1] / h], RECOVERY_SPEED)
            } else {
                ([0.; 2], 0.)
            };
            self.flight = None;
            self.arc = actor_physics::Arc::launch(self.position[1], RECOVERY_HEIGHT);
            self.jump_cut = JumpCut::default();
            self.air_velocity = [dir[0] * speed, dir[1] * speed];
            self.sampler.reset();
            self.vertical_speed = 0.;
            self.begin_reaction(Action::Hurt(hurt::RECOVERY));
            step.started = Some(self.action);
            return;
        }
        // Square (request bit 3), not Attack, starts it.
        if input.guard {
            self.combo = ptya::Combo { step: 1, finish: 1 };
            let state = ptya::State {
                abilities: &self.abilities,
                airborne: true,
                mp_charging: self.mp_charging,
                combo_window: false,
                finisher_window: false,
                actionable: true,
            };
            if let Some(i) = ptya::find(&clips.records, RETALIATING_SLASH)
                && ptya::allowed(&clips.records[i], &state, self.combo)
                && let Some(attack) = clips.attacks.get(i)
            {
                self.flight = None;
                self.connected = false;
                self.begin_reaction(Action::AirAttack(i));
                step.started = Some(self.action);
                self.face_target(&attack.clip, input);
                self.lift(attack, input);
            }
        }
    }

    /// Mode 4 for one tick (FUN_140422780 every second frame).
    fn blow_flight(&mut self, p: &Params) -> [f32; 3] {
        let Some(flight) = self.flight.as_mut() else {
            return [0.; 3];
        };
        flight.time += 1.;
        if self.sampler.due(0.) {
            let dt = actor_physics::PHYSICS_STEP;
            let sstm = flight.sstm;
            let d = flight.blow.step(&sstm, p.fall_max, dt);
            self.sampler.store(d);
        }
        let d = self.sampler.displacement();
        self.vertical_speed = d[1];
        d
    }

    /// Mode 0x29 for one tick: no horizontal motion, the curve's fall.
    fn dead_fall(&mut self, p: &Params) -> [f32; 3] {
        if self.sampler.due(0.) {
            let dt = actor_physics::PHYSICS_STEP;
            let dy = self.arc.step(self.position[1], p.fall_max, false, dt);
            self.sampler.store([0., dy, 0.]);
        }
        let d = self.sampler.displacement();
        self.vertical_speed = d[1];
        d
    }

    /// Guard success (FUN_1403d61d0, grounded): face the attacker, play the
    /// recoil and knock-slide away at 200 × 0.1 per tick (×0.9 per tick,
    /// stopping below 1). `away` is the XZ unit vector from the attacker.
    pub fn guard_recoil(&mut self, away: [f32; 2], clips: &Clips) -> Option<Action> {
        if self.in_air() || clips.guard_recoil.is_none() {
            return None;
        }
        self.cast = None;
        self.yaw = (-away[0]).atan2(-away[1]);
        self.knock = actor_physics::Knock::new([away[0], 0., away[1]], GUARD_KNOCK, 0.);
        self.action = Action::GuardRecoil;
        self.frame = 0;
        self.blend = 0.;
        self.arm();
        Some(self.action)
    }

    fn start(&mut self, action: Action, step: &mut Step) {
        self.action = action;
        self.frame = 0;
        self.blend = blend_for(action);
        step.started = Some(action);
    }

    /// Mode 0's walk↔run switch (FUN_1403c6dc0): the new loop starts at the
    /// old one's phase, `fmod(len_new × (t + blend) / len_old, len_new)`,
    /// with the locomotion blend.
    fn switch_gait(&mut self, action: Action, clips: &Clips, step: &mut Step) {
        let old = self.clip(clips).end.max(1) as f32;
        let time = self.motion_frame(clips) as f32;
        self.start(action, step);
        let new = self.clip(clips).end.max(1) as f32;
        self.frame = ((new * ((time + actor_physics::MOTION_BLEND) / old)) % new) as u32;
    }

    pub fn clip<'a>(&self, clips: &'a Clips) -> &'a Clip {
        match (self.action, self.armed) {
            (Action::Idle, true) => &clips.idle,
            (Action::Walk, true) => &clips.walk,
            (Action::Run, true) => &clips.run,
            (Action::Idle, false) => &clips.field_idle,
            (Action::Walk, false) => &clips.field_walk,
            (Action::Run, false) => &clips.field_run,
            (Action::Jump, _) => &clips.jump,
            (Action::Fall, _) => &clips.fall,
            (Action::Land, _) => &clips.land,
            (Action::Attack(i) | Action::AirAttack(i), _) => &clips.attacks[i].clip,
            (Action::Cast(i) | Action::AirCast(i), _) => &clips.spells[i].clip,
            (Action::Hurt(i), _) => &clips.hurt[i],
            (Action::Guard, _) => clips.guard.as_ref().unwrap_or(&clips.idle),
            (Action::GuardRecoil, _) => clips.guard_recoil.as_ref().unwrap_or(&clips.idle),
            (Action::Item, _) => clips.item.as_ref().unwrap_or(&clips.idle),
            (Action::Drive, _) => clips.drive.as_ref().unwrap_or(&clips.idle),
            (Action::DriveIn, _) => clips.drive_in.as_ref().unwrap_or(&clips.idle),
            (Action::HighJump, _) => clips
                .growth_clip(growth::motion::HIGH_JUMP)
                .map_or(&clips.jump, |g| &g.1),
            (Action::Growth(m), _) => clips.growth_clip(m).map_or(&clips.idle, |g| &g.1),
            (Action::Scripted(m), _) => clips.scripted_clip(m).map_or(&clips.idle, |g| &g.1),
        }
    }

    /// Root-motion translation for the current attack frame, model space.
    pub fn root_offset(&self, clips: &Clips) -> Vec3 {
        let root = match self.action {
            Action::Attack(i) | Action::AirAttack(i) => &clips.attacks[i].root,
            Action::Cast(i) | Action::AirCast(i) => &clips.spells[i].root,
            Action::Growth(growth::motion::DODGE_ROLL) => {
                match clips.growth_clip(growth::motion::DODGE_ROLL) {
                    Some(g) => &g.2,
                    None => return [0.; 3],
                }
            }
            Action::Scripted(m) => match clips.scripted_clip(m) {
                Some(g) => &g.2,
                None => return [0.; 3],
            },
            _ => return [0.; 3],
        };
        root.get(self.motion_frame(clips) as usize)
            .copied()
            .unwrap_or([0.; 3])
    }

    fn arm(&mut self) {
        self.armed = true;
        self.idle_ticks = 0;
    }

    /// In the jump/fall mode: airborne actions once any pending jump has
    /// taken off.
    pub fn in_air(&self) -> bool {
        match self.action {
            Action::Hurt(hurt::DEAD) => self.dead_air,
            Action::Scripted(_) => self.scripted_air,
            a => a.airborne() && self.pending_jump.is_none(),
        }
    }

    /// An ANB range 0, 1 or 2 covers the current motion frame.
    fn actionable(&self, clips: &Clips) -> bool {
        Clip::within(&self.clip(clips).actionable, self.motion_frame(clips))
    }

    /// Motion frame to sample for the current action.
    pub fn motion_frame(&self, clips: &Clips) -> u32 {
        let clip = self.clip(clips);
        match self.action {
            Action::Idle | Action::Walk | Action::Run | Action::Jump | Action::Fall => {
                clip.looped_frame(self.frame)
            }
            Action::Growth(m)
                if (growth::motion::GLIDE..=growth::motion::GLIDE_STAND).contains(&m) =>
            {
                clip.looped_frame(self.frame)
            }
            // With nothing queued a script's motion keeps looping from its
            // return frame [I: the ANB loop field].
            Action::Scripted(_) if self.queued.is_empty() => clip.looped_frame(self.frame),
            // The blow and DEAD motions hold their loop span until the
            // flight or the revive ends them.
            Action::Hurt(i) if hurt::BLOW.contains(&i) || i == hurt::DEAD => {
                clip.looped_frame(self.frame)
            }
            _ => clip.held_frame(self.frame),
        }
    }

    fn turn_toward(&mut self, direction: [f32; 2], rate: f32) {
        let target = direction[0].atan2(direction[1]);
        let delta = wrap_angle(target - self.yaw);
        self.yaw = wrap_angle(self.yaw + delta.clamp(-rate, rate));
    }

    /// PTYA attack start with a positive Jump field (FUN_1403d78d0), then
    /// the mode's jump slot (FUN_140409ad0: FUN_1403cbb00 + mode 1). With a
    /// target the height is its rise clamped to [+0x14, +0x10] and the speed
    /// (distance − reach + PTYA +0x3c) / √(2h/g) clamped to [+0x18, +0x1c]
    /// toward it; without one, PTYA +0xc along the facing at +0x1c.
    fn lift(&mut self, step: &AttackStep, input: &Input) {
        if !(step.hop.is_finite() && step.hop > 0.) {
            return;
        }
        let finite = |v: f32| if v.is_finite() { v } else { 0. };
        let (height, dir, speed) = match input.target {
            Some(t) if t[0].hypot(t[1]) > 1e-4 => {
                let (lo, hi) = (finite(step.hop_min), finite(step.hop_max));
                let rise = input.target_rise;
                let height = if rise < lo {
                    lo
                } else if rise > hi {
                    hi
                } else {
                    rise
                };
                let len = t[0].hypot(t[1]);
                let reach = len - input.target_reach + finite(step.hop_reach);
                let per_tick = reach / actor_physics::rise_time(height);
                let (smin, smax) = (finite(step.hop_speed_min), finite(step.hop_speed));
                let speed = if smin <= per_tick {
                    per_tick.min(smax)
                } else {
                    smin
                };
                (height, [t[0] / len, t[1] / len], speed)
            }
            _ => (
                step.hop,
                [self.yaw.sin(), self.yaw.cos()],
                finite(step.hop_speed),
            ),
        };
        self.arc = actor_physics::Arc::launch(self.position[1], height);
        self.air_velocity = [dir[0] * speed, dir[1] * speed];
        self.sampler.reset();
        self.jump_cut = JumpCut::default();
    }

    /// Jump (mode 0 slot +0x48, FUN_140422630 → FUN_1403cbb00): plays JUMP
    /// and queues the arc of the PRTY jump height with the stick velocity
    /// (obj+0xe0); the jump mode starts at JUMP's take-off trigger. The
    /// Jump button's jump is variable (obj+0x120 bit 16).
    fn jump(&mut self, p: &Params, velocity: [f32; 2], step: &mut Step) {
        let (height, action) = self.jump_height(p);
        self.pending_jump = Some((height, velocity));
        self.jump_cut = JumpCut {
            variable: true,
            ..JumpCut::default()
        };
        self.vertical_speed = 0.;
        self.quick_run = None;
        self.start(action, step);
    }

    /// FUN_1403b59b0: High Jump's row height and motion 201, else the PRTY
    /// height and JUMP.
    fn jump_height(&self, p: &Params) -> (f32, Action) {
        match self.growth_row(growth::HIGH_JUMP) {
            Some(r) => (r.high_jump, Action::HighJump),
            None => (p.jump_height, Action::Jump),
        }
    }

    /// FUN_1403cb9f0 (an airborne mode's jump slot): Aerial Dodge, once per
    /// airtime (obj+0x124 bit 1) below level 5: a new arc of the row's
    /// height from here, at the row's speed along the stick (0 without
    /// one), motion 202. The arc cannot be cut.
    fn aerial_dodge(&mut self, moving: bool, direction: [f32; 2], step: &mut Step) -> bool {
        let level = self.growth_level(growth::AERIAL_DODGE);
        if !(level > 4 || (level != 0 && self.dodge_ready)) {
            return false;
        }
        let Some(row) = self.fmab.row(level).copied() else {
            return false;
        };
        let (dir, speed) = if moving {
            (direction, row.dodge_speed)
        } else {
            ([self.yaw.sin(), self.yaw.cos()], 0.)
        };
        if moving {
            self.yaw = direction[0].atan2(direction[1]);
        }
        self.arc = actor_physics::Arc::launch(self.position[1], row.dodge_height);
        self.air_velocity = [dir[0] * speed, dir[1] * speed];
        self.launch_velocity = Some(self.air_velocity);
        self.sampler.reset();
        self.jump_cut = JumpCut::default();
        self.glide = None;
        self.dodge_ready = false;
        self.vertical_speed = 0.;
        self.start(Action::Growth(growth::motion::AERIAL_DODGE), step);
        true
    }

    /// FUN_1403a81f0's growth branches and Square attacks, in its order:
    /// Glide (airborne, above the row's start height), the Square combo
    /// abilities, Guard with the stick idle, Dodge Roll, Quick Run, Guard.
    /// All growth branches need an active ANB range 0–2. `guard` allows
    /// the Guard branches (the test room's existing gate).
    #[allow(clippy::too_many_arguments)]
    fn square(
        &mut self,
        input: &Input,
        clips: &Clips,
        height: f32,
        moving: bool,
        direction: [f32; 2],
        guard: bool,
        step: &mut Step,
    ) -> bool {
        let actionable = self.actionable(clips);
        let air = self.in_air();
        if air
            && actionable
            && let Some(row) = self.growth_row(growth::GLIDE)
            && height > row.glide_start
        {
            self.start_glide(row, moving, step);
            return true;
        }
        if self.try_square(input, clips, step) {
            return true;
        }
        if guard && !moving && self.can_guard(clips) {
            self.start_guard(input, step);
            return true;
        }
        if !air && actionable {
            let dir = if moving {
                direction
            } else {
                [self.yaw.sin(), self.yaw.cos()]
            };
            if let Some(row) = self.growth_row(growth::DODGE_ROLL)
                && clips.growth_clip(growth::motion::DODGE_ROLL).is_some()
            {
                self.yaw = dir[0].atan2(dir[1]);
                self.invulnerable = row.roll_invulnerable;
                self.arm();
                self.start(Action::Growth(growth::motion::DODGE_ROLL), step);
                return true;
            }
            if self.start_quick_run(dir, clips, step) {
                return true;
            }
        }
        if guard && self.can_guard(clips) {
            self.start_guard(input, step);
            return true;
        }
        false
    }

    /// FUN_1403a9830: Quick Run along `dir` (mode 0x10, motion 203).
    fn start_quick_run(&mut self, dir: [f32; 2], clips: &Clips, step: &mut Step) -> bool {
        let Some(row) = self.growth_row(growth::QUICK_RUN) else {
            return false;
        };
        if clips.growth_clip(growth::motion::QUICK_RUN).is_none() {
            return false;
        }
        self.quick_run = Some(growth::QuickRun::start(dir, &row));
        self.yaw = dir[0].atan2(dir[1]);
        self.square_held = true;
        self.invulnerable = row.run_invulnerable;
        self.sampler.reset();
        self.arm();
        self.start(Action::Growth(growth::motion::QUICK_RUN), step);
        true
    }

    /// Square in the air: modes 0x11 (stick idle) / 0x12. The shared
    /// mode-1 enter (FUN_1403cbd30) restarts the velocity from the last
    /// arc's take-off velocity.
    fn start_glide(&mut self, row: growth::Row, moving: bool, step: &mut Step) {
        if let Some(v) = self.launch_velocity {
            self.air_velocity = v;
        }
        self.glide = Some(row);
        self.square_held = true;
        self.jump_cut = JumpCut::default();
        self.pending_jump = None;
        let m = if moving {
            growth::motion::GLIDE
        } else {
            growth::motion::GLIDE_STAND
        };
        self.start(Action::Growth(m), step);
        self.blend = growth::GLIDE_BLEND;
    }

    /// Leaving the ground without jumping (FUN_1403b4780 / FUN_1403b8150),
    /// keeping the ground velocity (obj+0xa38 → +0x20 [I: +0xa38 is the
    /// last ground velocity]).
    fn drop_off(&mut self, velocity: [f32; 2], step: &mut Step) {
        self.dodge_ready = true;
        self.launch_velocity = None;
        self.quick_run = None;
        self.arc = actor_physics::Arc::fall(self.position[1], 0.);
        self.pending_jump = None;
        self.jump_cut = JumpCut::default();
        self.air_velocity = velocity;
        self.sampler.reset();
        self.vertical_speed = 0.;
        self.start(Action::Fall, step);
    }

    /// FUN_1403a8fa0's move request: the stick direction at the walk speed
    /// below magnitude 0.8 and the run speed from it; zero without a stick.
    fn stick_velocity(direction: [f32; 2], magnitude: f32, p: &Params) -> [f32; 2] {
        if magnitude <= 0. {
            return [0.; 2];
        }
        let speed = if magnitude < p.walk_threshold {
            p.walk_speed
        } else {
            p.run_speed
        };
        [direction[0] * speed, direction[1] * speed]
    }

    pub fn tick(&mut self, input: &Input, p: &Params, clips: &Clips, world: &impl World) -> Step {
        let mut step = Step::default();
        self.frame = self.frame.saturating_add(1);
        if self.action == Action::Item
            && clips.item.as_ref().and_then(|c| c.item_use) == Some(self.frame)
        {
            self.item_fired = true;
        }
        // FUN_1403d6fc0: a rising variable jump whose button is up is
        // marked released (obj+0x120 bit 17 → bit 18).
        if self.jump_cut.held && input.jump_up {
            self.jump_cut.held = false;
            self.jump_cut.released = true;
        }
        // MP Charge runs down at 1 + the MP Haste terms (FUN_1403c0de0).
        self.mp.tick(1., self.abilities.mp_haste);
        self.mp_charging = self.mp.charging();
        // This tick's whole displacement: mode velocity, root motion and
        // knock-slide, collided once at the end (FUN_1403b89a0).
        let mut disp = [0f32; 3];
        // Mode 0 move request this tick (obj+0x10 × +0x1c): the stick, but
        // only while an ANB range 0–2 is active (the same gate as commands);
        // LAND before tick 12, attacks, damage and guard leave it idle [I:
        // the stick feed's gate was not traced; the original locks movement
        // on landing].
        let mut ground_request = [0f32; 2];
        let was_in_air = self.in_air();
        let magnitude = input.direction[0].hypot(input.direction[1]).min(1.);
        // Any stick past the pad's dead zone moves (FUN_1403a8fa0: speed is
        // set whenever the stick magnitude is above 0).
        let moving = magnitude > 0.;
        let direction = if moving {
            [
                input.direction[0] / magnitude,
                input.direction[1] / magnitude,
            ]
        } else {
            [0., 0.]
        };
        let stick = Self::stick_velocity(direction, magnitude, p);
        // FUN_1403d6fc0: Square released clears obj+0x124 bit 2; the
        // invulnerability timer runs down (FUN_1403d68b0).
        if self.square_held && !input.guard_held {
            self.square_held = false;
        }
        self.invulnerable = (self.invulnerable - 1.).max(0.);
        // Height above the ground under the player (FUN_1403b87e0; no
        // floor reads as 0).
        let height = world
            .floor(self.position[0], self.position[2], self.position[1] + 1.)
            .map_or(0., |f| (self.position[1] - f).max(0.));
        if matches!(self.action, Action::Hurt(i) if hurt::BLOW.contains(&i)) {
            // Aerial Recovery or Retaliating Slash may take over this tick.
            self.recover(input, clips, direction, &mut step);
        }
        match self.action {
            Action::Idle | Action::Walk | Action::Run | Action::Land => {
                // Commands need an active ANB range 0, 1 or 2 (obj+0x18c &
                // 0x1c, FUN_1401b15f0): always during locomotion, from
                // LAND's range-2 start (tick 12 installed) after a landing.
                let ready = self.actionable(clips);
                if (input.guard
                    && !input.attack
                    && self.square(input, clips, height, moving, direction, ready, &mut step))
                    || (ready
                        && input.attack
                        && self.try_attack(input, p, clips, ready, moving, direction, &mut step))
                {
                } else if ready && input.jump {
                    self.jump(p, stick, &mut step);
                } else {
                    let landing = self.action == Action::Land;
                    // FUN_1403a8fa0: run speed from stick magnitude 0.8
                    // (DAT_140585c7c), walk speed below; FUN_140422520 then
                    // plays RUN when speed ≥ (walk + run) / 2, else WALK, but
                    // only while ANB range 0 or 2 is active (obj+0x18c bits
                    // 2/4); LAND otherwise plays to its end, then idles.
                    let target = if !moving {
                        if landing && self.frame < clips.land.end {
                            Action::Land
                        } else {
                            Action::Idle
                        }
                    } else if !ready {
                        self.action
                    } else if magnitude < p.walk_threshold {
                        Action::Walk
                    } else {
                        Action::Run
                    };
                    self.idle_ticks = self.idle_ticks.saturating_add(1);
                    if self.armed && self.idle_ticks >= p.disarm_ticks && target != Action::Land {
                        // Weapon put away: restart the locomotion clip in field mode.
                        self.armed = false;
                        self.start(target, &mut step);
                    } else if target != self.action {
                        if matches!(
                            (self.action, target),
                            (Action::Walk, Action::Run) | (Action::Run, Action::Walk)
                        ) {
                            self.switch_gait(target, clips, &mut step);
                        } else {
                            self.start(target, &mut step);
                        }
                    }
                    if moving && ready {
                        // FUN_1403b4780 turns the facing at +0x134 per tick;
                        // FUN_140422350 moves along the stick direction at
                        // the full walk/run speed (inertially under range 42).
                        self.turn_toward(direction, p.turn_speed);
                        ground_request = stick;
                    }
                }
            }
            Action::Jump | Action::HighJump if self.pending_jump.is_some() => {
                // The jump mode is pending (obj+0x648 = 1) until the JUMP
                // motion's frame trigger 0 (tick 4 installed) switches to it
                // (FUN_14040dd70); until then mode 0 keeps going, with JUMP's
                // range 1 (tick 8) not yet feeding the stick.
                if self.actionable(clips) {
                    ground_request = stick;
                }
                if self.frame >= self.clip(clips).takeoff.unwrap_or(0)
                    && let Some((height, velocity)) = self.pending_jump.take()
                {
                    // Entering mode 1 from the ground readies the Aerial
                    // Dodge (FUN_1403cbd30, obj+0x124 bit 1).
                    self.dodge_ready = true;
                    self.launch_velocity = Some(velocity);
                    // FUN_1403cbd30 (mode 1 enter): the arc's base is the
                    // take-off height; the stored +0xe0 velocity applies;
                    // a variable jump becomes cuttable (bit 16 → bit 17).
                    self.arc = actor_physics::Arc::launch(self.position[1], height);
                    self.jump_cut = JumpCut {
                        held: self.jump_cut.variable,
                        ..JumpCut::default()
                    };
                    self.air_velocity = velocity;
                    self.sampler.reset();
                    disp = self.airborne(
                        input, moving, magnitude, direction, height, p, clips, &mut step,
                    );
                }
            }
            Action::Attack(i) => {
                let clip = &clips.attacks[i].clip;
                self.face_target(clip, input);
                // Root motion: the attack's root-curve change this tick.
                let root = &clips.attacks[i].root;
                let at = |f: u32| root.get(clip.held_frame(f) as usize).copied();
                if let (Some(now), Some(before)) = (at(self.frame), at(self.frame - 1)) {
                    let (s, c) = self.yaw.sin_cos();
                    let (dx, dz) = (now[0] - before[0], now[2] - before[2]);
                    disp[0] += dx * c + dz * s;
                    disp[2] += -dx * s + dz * c;
                }
                let recovery = self.frame >= clip.blend_to_idle.unwrap_or(clip.end);
                let ready = self.actionable(clips);
                // FUN_1403d6fc0 takes Square (FUN_1403a81f0) before the
                // attack command.
                if (input.guard
                    && self.square(input, clips, height, moving, direction, false, &mut step))
                    || (input.attack
                        && self.try_attack(input, p, clips, ready, moving, direction, &mut step))
                {
                } else if recovery && input.jump {
                    self.jump(p, stick, &mut step);
                } else if recovery && moving {
                    self.start(Action::Idle, &mut step);
                } else if self.frame >= clip.end {
                    self.start(Action::Idle, &mut step);
                    self.blend = clips.attacks[i].blend;
                }
            }
            Action::Cast(i) => {
                let clip = &clips.spells[i].clip;
                self.face_target(clip, input);
                self.release_if_due(clips, i);
                // A spell is a combo hit: the next press may chain from the
                // cast's combo window (range 11 / 35, finisher range 50).
                let ready = self.actionable(clips);
                if (input.guard
                    && self.square(input, clips, height, moving, direction, false, &mut step))
                    || (input.attack
                        && self.try_attack(input, p, clips, ready, moving, direction, &mut step))
                {
                } else if self.frame >= clip.end {
                    self.cast = None;
                    self.start(Action::Idle, &mut step);
                }
            }
            Action::Scripted(m) if !self.scripted_air => {
                // A script's motion: root motion along the facing, range-28
                // turns toward the target, then the queued motions.
                let empty = Vec::new();
                let (clip, root) = match clips.scripted_clip(m) {
                    Some(g) => (&g.1, &g.2),
                    None => (&clips.idle, &empty),
                };
                self.face_target(clip, input);
                let at = |f: u32| root.get(clip.held_frame(f) as usize).copied();
                if let (Some(now), Some(before)) = (at(self.frame), at(self.frame - 1)) {
                    let (s, c) = self.yaw.sin_cos();
                    let (dx, dz) = (now[0] - before[0], now[2] - before[2]);
                    disp[0] += dx * c + dz * s;
                    disp[2] += -dx * s + dz * c;
                }
                if self.frame >= clip.end && !self.queued.is_empty() {
                    let (next, blend) = self.queued.remove(0);
                    if !self.start_motion_id(next, blend, clips) {
                        self.start(Action::Idle, &mut step);
                    }
                    step.started = Some(self.action);
                }
            }
            Action::Growth(growth::motion::DODGE_ROLL) => {
                // Motion 209 moves by its root curve (mode 0, no request).
                let empty = Vec::new();
                let (clip, root) = match clips.growth_clip(growth::motion::DODGE_ROLL) {
                    Some(g) => (&g.1, &g.2),
                    None => (&clips.idle, &empty),
                };
                let at = |f: u32| root.get(clip.held_frame(f) as usize).copied();
                if let (Some(now), Some(before)) = (at(self.frame), at(self.frame - 1)) {
                    let (s, c) = self.yaw.sin_cos();
                    let (dx, dz) = (now[0] - before[0], now[2] - before[2]);
                    disp[0] += dx * c + dz * s;
                    disp[2] += -dx * s + dz * c;
                }
                let ready = self.actionable(clips);
                let end = clip.end;
                if self.frame == growth::ROLL_CHAIN_TICK && input.guard_held {
                    // Frame trigger 34: Square still held → Quick Run.
                    let facing = [self.yaw.sin(), self.yaw.cos()];
                    self.start_quick_run(facing, clips, &mut step);
                } else if ready
                    && input.attack
                    && self.try_attack(input, p, clips, ready, moving, direction, &mut step)
                {
                } else if ready && input.jump {
                    self.jump(p, stick, &mut step);
                } else if ready
                    && input.guard
                    && self.square(input, clips, height, moving, direction, true, &mut step)
                {
                } else if (ready && moving) || self.frame >= end {
                    self.start(Action::Idle, &mut step);
                }
            }
            Action::Growth(m @ (growth::motion::QUICK_RUN | growth::motion::QUICK_RUN_END)) => {
                // FUN_140423500: the hold runs down while Square is held;
                // then motion 204 and the after-decay.
                let held = self.square_held;
                if let Some(q) = self.quick_run.as_mut()
                    && q.frame(held, 1.)
                {
                    self.start(Action::Growth(growth::motion::QUICK_RUN_END), &mut step);
                } else if m == growth::motion::QUICK_RUN_END && self.frame >= self.clip(clips).end {
                    // The queued FALL (blend 8); on flat ground it lands at
                    // once [I: the hand-back to mode 0 is not traced].
                    self.quick_run = None;
                    self.arc = actor_physics::Arc::fall(self.position[1], 0.);
                    self.launch_velocity = None;
                    self.air_velocity = [0.; 2];
                    self.sampler.reset();
                    self.start(Action::Fall, &mut step);
                }
            }
            Action::Hurt(i) if hurt::BLOW.contains(&i) => {
                disp = self.blow_flight(p);
            }
            // Mode 0 with the velocity cleared; motion 0x34's range 3 holds
            // gravity off (an airborne start hovers) until the swap.
            Action::Drive => {}
            Action::Hurt(hurt::DEAD) => {
                if self.dead_air {
                    disp = self.dead_fall(p);
                }
            }
            Action::Jump
            | Action::HighJump
            | Action::Fall
            | Action::AirAttack(_)
            | Action::AirCast(_)
            | Action::Growth(_)
            | Action::Scripted(_) => {
                disp = self.airborne(
                    input, moving, magnitude, direction, height, p, clips, &mut step,
                );
            }
            Action::Hurt(_) if self.in_air() => {
                disp = self.airborne(
                    input, moving, magnitude, direction, height, p, clips, &mut step,
                );
            }
            Action::Hurt(_)
            | Action::Guard
            | Action::GuardRecoil
            | Action::Item
            | Action::DriveIn => {
                // Locked until the clip's range-2 tick (hurt 50, guard 52,
                // recoil 40 installed), then input is taken again
                // (FUN_1401b15f0 actionable bits).
                let clip = self.clip(clips);
                let (end, ready) = (clip.end, clip.blend_to_idle.unwrap_or(clip.end));
                let actionable = self.frame >= ready;
                if input.attack
                    && self.try_attack(input, p, clips, actionable, moving, direction, &mut step)
                {
                } else if actionable {
                    if input.guard && self.can_guard(clips) {
                        self.start_guard(input, &mut step);
                    } else if input.jump {
                        self.jump(p, stick, &mut step);
                    } else if moving || self.frame >= end {
                        self.start(Action::Idle, &mut step);
                    }
                }
            }
        }
        if !was_in_air && !self.in_air() {
            // Mode 0 (FUN_140422350) sampled by FUN_1403b89a0 every second
            // frame. Under ANB range 42 (obj+0x190 bit 0, e.g. all of LAND)
            // the velocity eases toward the request through FUN_1403b7ad0
            // with PRTY words 10/11 as inertia and drag; otherwise it is the
            // request itself.
            let speed = ground_request[0].hypot(ground_request[1]);
            if let Some(q) = self.quick_run.as_mut() {
                // Mode 0x10 (FUN_140423480): no gravity, the decaying dash.
                let v = q.velocity[0].hypot(q.velocity[1]);
                if self.sampler.due(v) {
                    let d = q.step(actor_physics::PHYSICS_STEP);
                    self.sampler.store(d);
                }
            } else if self.sampler.due(speed) {
                let dt = actor_physics::PHYSICS_STEP;
                let clip = self.clip(clips);
                if Clip::within(&clip.inertia, self.frame) {
                    let dir = if speed > 0. {
                        [ground_request[0] / speed, ground_request[1] / speed]
                    } else {
                        [0.; 2]
                    };
                    self.air_velocity = actor_physics::steer(
                        self.air_velocity,
                        dir,
                        speed,
                        p.land_inertia,
                        p.land_drag,
                    );
                } else {
                    self.air_velocity = ground_request;
                }
                self.sampler
                    .store([self.air_velocity[0] * dt, 0., self.air_velocity[1] * dt]);
            }
            let [vx, _, vz] = self.sampler.displacement();
            disp[0] += vx;
            disp[2] += vz;
        }
        if let Some(knock) = self.knock.as_mut() {
            match knock.step() {
                Some(d) => {
                    disp[0] += d[0];
                    disp[2] += d[2];
                }
                None => self.knock = None,
            }
        }
        let airborne = self.in_air();
        if airborne || disp[0] != 0. || disp[2] != 0. {
            let body = Body {
                radius: p.radius,
                height: p.height,
                step: p.step_height,
            };
            let moved = world.collide(self.position, disp, airborne, &body);
            self.position = moved.position;
            if airborne && moved.on_floor {
                // Landing switches to the ground mode and plays the land
                // motion (FUN_1403cb940: obj+0x11c = 5, which FUN_1403d78d0
                // also sets for aerial attacks), cutting an aerial swing
                // short. The air velocity stays in obj+0x20 for LAND's
                // inertial slide.
                self.vertical_speed = 0.;
                self.sampler = actor_physics::Sampler::default();
                self.jump_cut = JumpCut::default();
                self.glide = None;
                self.quick_run = None;
                self.launch_velocity = None;
                step.landed = true;
                match self.action {
                    Action::Hurt(i) if hurt::BLOW.contains(&i) => {
                        // Mode 4's land (FUN_140422700): the land motion
                        // after the blow, with no slide.
                        let back = i == hurt::BLOW[1];
                        self.flight = None;
                        self.air_velocity = [0.; 2];
                        self.start(Action::Hurt(hurt::BLOW_LAND[back as usize]), &mut step);
                    }
                    Action::Hurt(hurt::DEAD) => self.dead_air = false,
                    _ => self.start(Action::Land, &mut step),
                }
            } else if !airborne && !moved.on_floor {
                // Walked off a ledge: fall keeping this tick's horizontal
                // displacement, capped at 8 per tick, as the velocity.
                let h = disp[0].hypot(disp[2]);
                let k = if h > WALK_OFF_SPEED {
                    WALK_OFF_SPEED / h
                } else {
                    1.
                };
                self.drop_off([disp[0] * k, disp[2] * k], &mut step);
            }
        }
        step
    }

    /// The current motion id (obj+0x180) as far as the controller knows it.
    pub fn motion_id(&self, clips: &Clips) -> u16 {
        match self.action {
            Action::Attack(i) | Action::AirAttack(i) => clips.records[i].motion,
            Action::Cast(i) | Action::AirCast(i) => clips.spells[i].motion,
            Action::Idle => 0,
            Action::Walk => 1,
            Action::Run => 2,
            Action::Jump => 3,
            Action::Fall => 4,
            Action::Land => 5,
            Action::Hurt(i) => HURT_MOTIONS[i.min(HURT_MOTIONS.len() - 1)],
            Action::Guard => 173,
            Action::GuardRecoil => 42,
            Action::Item => 9,
            Action::Drive => 0x34,
            Action::DriveIn => 0x35,
            Action::HighJump => growth::motion::HIGH_JUMP,
            Action::Growth(m) | Action::Scripted(m) => m,
        }
    }

    /// The attack command (FUN_1403d6fc0 type 1 → FUN_1403d6ca0, then
    /// FUN_1403a8690 → FUN_1403d78d0). The press counts toward the combo
    /// only inside an "allow combo" (11) or finisher (50) range after an
    /// attack that connected (obj+0x120 bit 25) or with Combo Master;
    /// otherwise it needs an active range 0–2 and starts a new combo. The
    /// record comes from the PTYA chooser, or Counterguard (0x27) out of the
    /// guard recoil. Returns false when nothing starts. Requests last one
    /// frame (FUN_1403b2340 clears them), so there is no input buffer.
    #[allow(clippy::too_many_arguments)]
    fn try_attack(
        &mut self,
        input: &Input,
        p: &Params,
        clips: &Clips,
        actionable: bool,
        moving: bool,
        direction: [f32; 2],
        step: &mut Step,
    ) -> bool {
        if clips.records.is_empty() {
            return false;
        }
        let clip = self.clip(clips);
        let frame = self.motion_frame(clips);
        let state = ptya::State {
            abilities: &self.abilities,
            airborne: self.in_air(),
            mp_charging: self.mp_charging,
            combo_window: Clip::within(&clip.combo_windows, frame),
            finisher_window: Clip::within(&clip.finisher_windows, frame),
            actionable,
        };
        let chain =
            self.shoot || self.connected || self.abilities.has(abilities::bit::COMBO_MASTER);
        let mut combo = self.combo;
        if !ptya::advance(&mut combo, &state, chain) {
            return false;
        }
        let current = self.motion_id(clips);
        let counter = if current == 42 {
            ptya::find(&clips.records, 0x27)
                .filter(|&i| ptya::allowed(&clips.records[i], &state, combo))
        } else {
            None
        };
        // [test-room carry-over] The stick sets the facing an attack is
        // chosen and swung along.
        let facing_yaw = if moving {
            direction[0].atan2(direction[1])
        } else {
            self.yaw
        };
        let nearby: Vec<ptya::Target> = input.nearby.iter().flatten().copied().collect();
        let scene = ptya::Scene {
            origin: self.position,
            facing: [facing_yaw.sin(), facing_yaw.cos()],
            target: input.target_shape,
            others: &nearby,
            jump_height: self.jump_height(p).0,
            current_motion: current,
        };
        let Some(i) = counter
            .or_else(|| ptya::select(&clips.records, &state, combo, &scene, input.probe_blocked))
        else {
            return false;
        };
        self.yaw = facing_yaw;
        self.execute(i, combo, input, clips, step);
        true
    }

    /// FUN_1403d78d0: starts PTYA record `i` with the counters `combo`.
    fn execute(
        &mut self,
        i: usize,
        combo: ptya::Combo,
        input: &Input,
        clips: &Clips,
        step: &mut Step,
    ) {
        let airborne = self.in_air();
        self.combo = combo;
        self.connected = false;
        // FUN_1403a89a0: an executed command uses up the Aerial Dodge.
        self.dodge_ready = false;
        self.glide = None;
        let record = clips.records[i];
        // FUN_1403d78d0: a hop from the ground starts a fresh count.
        if record.jump > 0. && !airborne {
            self.combo = ptya::Combo { step: 1, finish: 1 };
        }
        self.arm();
        self.pending_jump = None;
        let action = if airborne || record.leaves_ground() {
            Action::AirAttack(i)
        } else {
            Action::Attack(i)
        };
        self.start(action, step);
        self.face_target(&clips.attacks[i].clip, input);
        self.lift(&clips.attacks[i], input);
    }

    /// The Square combo abilities (FUN_1403a81f0 → FUN_1403a9950, tried in
    /// this order): each needs its record allowed (ability, ground/air,
    /// combo limit), refuses a non-finisher once a finisher may play
    /// (FUN_1403d7780 at offset 0), and needs the "allow combo" (11) or
    /// finisher (50) range plus a counted press (FUN_1403d6ca0).
    fn try_square(&mut self, input: &Input, clips: &Clips, step: &mut Step) -> bool {
        if clips.records.is_empty() || clips.attacks.len() < clips.records.len() {
            return false;
        }
        let clip = self.clip(clips);
        let frame = self.motion_frame(clips);
        let state = ptya::State {
            abilities: &self.abilities,
            airborne: self.in_air(),
            mp_charging: self.mp_charging,
            combo_window: Clip::within(&clip.combo_windows, frame),
            finisher_window: Clip::within(&clip.finisher_windows, frame),
            actionable: self.actionable(clips),
        };
        // FUN_1403a9950 passes only the connected flag; Combo Master
        // applies to the main attack command, not these Square abilities.
        let chain = self.connected;
        for id in SQUARE_ABILITIES {
            let Some(i) = ptya::find(&clips.records, id) else {
                continue;
            };
            let record = &clips.records[i];
            if !ptya::allowed(record, &state, self.combo) {
                continue;
            }
            if record.flags & ptya::flag::FINISHER == 0
                && ptya::finisher_allowed(&state, self.combo, 0)
            {
                continue;
            }
            if !(state.combo_window || state.finisher_window) {
                continue;
            }
            let mut combo = self.combo;
            if !ptya::advance(&mut combo, &state, chain) {
                continue;
            }
            self.execute(i, combo, input, clips, step);
            return true;
        }
        false
    }

    /// Whether commands are taken now: an ANB range 0–2, or the unlocked
    /// tail of a reaction (FUN_1401b15f0).
    /// Whether a command could run now (FUN_1403ba070's motion gate: an
    /// active ANB range 0–2, or a reaction's unlocked tail).
    pub fn can_command(&self, clips: &Clips) -> bool {
        !self.dead() && self.action != Action::Drive && self.command_ready(clips)
    }

    fn command_ready(&self, clips: &Clips) -> bool {
        match self.action {
            Action::Hurt(_)
            | Action::Guard
            | Action::GuardRecoil
            | Action::Item
            | Action::DriveIn => {
                let clip = self.clip(clips);
                self.frame >= clip.blend_to_idle.unwrap_or(clip.end)
            }
            _ => self.actionable(clips),
        }
    }

    /// The spell command (FUN_1403d6fc0 Execute 2 → FUN_1403d6ca0 →
    /// FUN_1403a9dd0 → FUN_1403c5170): counts a combo hit, picks the
    /// ground / finisher / air motion and row, and starts the cast. MP is
    /// spent at the release tick. Returns the new action, or None when the
    /// command is refused or the cast motion is not loaded.
    pub fn cast(
        &mut self,
        rec: &magic::Record,
        cost: u8,
        form: u8,
        target_is_self: bool,
        clips: &Clips,
    ) -> Option<(Action, magic::Start)> {
        let ready = self.command_ready(clips);
        let clip = self.clip(clips);
        let frame = self.motion_frame(clips);
        let state = ptya::State {
            abilities: &self.abilities,
            airborne: self.in_air(),
            mp_charging: self.mp_charging,
            combo_window: Clip::within(&clip.combo_windows, frame),
            finisher_window: Clip::within(&clip.finisher_windows, frame),
            actionable: ready,
        };
        // Casting needs the command to be accepted (FUN_1403ba070) and an
        // MP state the menu allows; the colour check is the caller's.
        let mut combo = self.combo;
        // FUN_1403d6fc0 command type 2 counts a spell with chain=1,
        // independently of contact and Combo Master.
        if !ptya::advance(&mut combo, &state, true) {
            return None;
        }
        // FUN_1403a9dd0: the Lion form never finishes; otherwise
        // FUN_1403d7780(obj, 0) after the count.
        let finisher = form != 7 && ptya::finisher_allowed(&state, combo, 0);
        let airborne = self.in_air();
        let start = magic::start(rec, finisher, airborne, form, target_is_self);
        let motion = start.motion as u16;
        let i = clips.spells.iter().position(|c| c.motion == motion)?;
        self.combo = combo;
        self.connected = false;
        self.arm();
        self.pending_jump = None;
        let action = if airborne {
            // The air cast hop (physics vt+0x20 with SSTM +0x48 = 40).
            self.arc = actor_physics::Arc::launch(self.position[1], magic::AIR_HOP);
            self.jump_cut = JumpCut::default();
            self.air_velocity = [0.; 2];
            self.sampler.reset();
            self.vertical_speed = 0.;
            Action::AirCast(i)
        } else {
            Action::Cast(i)
        };
        self.action = action;
        self.frame = 0;
        self.blend = start.blend;
        self.cast = Some(CastState {
            start,
            magic_type: rec.magic_type,
            level: rec.level,
            cost,
            form,
            volleys: 0,
            last_frame: None,
        });
        // A release at tick 0 (Reflect) fires with the start.
        self.release_if_due_at(clips, i, 0);
        Some((action, start))
    }

    /// FUN_1403c6150 at a frame-trigger-17 tick: spends MP (FUN_1403d7e90)
    /// and reports the spell as released.
    fn release_if_due(&mut self, clips: &Clips, i: usize) {
        let frame = self.frame;
        self.release_if_due_at(clips, i, frame);
    }

    fn release_if_due_at(&mut self, clips: &Clips, i: usize, frame: u32) {
        let spell = &clips.spells[i];
        let Some(c) = self.cast.as_mut() else { return };
        if c.last_frame == Some(frame)
            || !spell
                .clip
                .release
                .iter()
                .any(|&(f, m)| f == frame && m.is_none_or(|m| m == spell.motion))
        {
            return;
        }
        c.last_frame = Some(frame);
        // 0xff is "all MP": it always takes MP below 0 and starts a charge.
        // The cost is taken once: FUN_1403d7e90 clears it (obj+0xd58 = 0).
        let cost = std::mem::take(&mut c.cost);
        if c.volleys == 0 {
            self.mp.change(-(cost as i32), magic::CHARGE_LENGTH);
        }
        self.mp_charging = self.mp.charging();
        self.released = Some(Released {
            magic_type: c.magic_type,
            level: c.level,
            row: c.start.row,
            finisher: c.start.finisher,
            airborne: c.start.airborne,
            form: c.form,
            volley: c.volleys,
        });
        c.volleys += 1;
    }

    /// What gates a command now: the combo window, the finisher window,
    /// whether the motion takes commands, and the combo counters.
    pub fn command_gate(&self, clips: &Clips) -> (bool, bool, bool, ptya::Combo) {
        let clip = self.clip(clips);
        let frame = self.motion_frame(clips);
        (
            Clip::within(&clip.combo_windows, frame),
            Clip::within(&clip.finisher_windows, frame),
            self.command_ready(clips),
            self.combo,
        )
    }

    /// Whether a cast is running (from the command to the end of its motion).
    pub fn casting(&self) -> bool {
        self.cast.is_some()
    }

    /// The spell released since the last call.
    pub fn take_released(&mut self) -> Option<Released> {
        self.released.take()
    }

    /// PTYA 0x0b: the guard motion, turning to the target at once.
    /// Guard (FUN_1403a9c70) needs its PTYA record's ability, Guard (bit 1),
    /// and the motion loaded.
    fn can_guard(&self, clips: &Clips) -> bool {
        self.abilities.has(abilities::bit::GUARD) && clips.guard.is_some()
    }

    fn start_guard(&mut self, input: &Input, step: &mut Step) {
        if let Some(t) = input.target
            && t[0].hypot(t[1]) > 1e-4
        {
            self.yaw = t[0].atan2(t[1]);
        }
        self.arm();
        self.start(Action::Guard, step);
    }

    /// Attack facing toward the target: ANB range 28 turns by up to its
    /// parameter in degrees per tick (FUN_14040d810 → FUN_1403d4b10 →
    /// FUN_1403d3a90, `DAT_1405b15b4` = π/180) and frame trigger 21 by up to
    /// its parameter at that tick, both only beyond 50 units horizontally
    /// (`DAT_1405b532c`).
    fn face_target(&mut self, clip: &Clip, input: &Input) {
        let Some(t) = input.target else { return };
        if t[0].hypot(t[1]) <= 50. {
            return;
        }
        let frame = self.frame;
        let ranges = clip
            .turn_to_target
            .iter()
            .filter(|&&(a, b, _)| (a..=b).contains(&frame))
            .map(|&(_, _, deg)| deg);
        let ticks = clip
            .face_target
            .iter()
            .filter(|&&(f, _)| f == frame)
            .map(|&(_, deg)| deg);
        let limit = ranges.chain(ticks).fold(0f32, f32::max);
        if limit > 0. {
            self.turn_toward(t, limit * std::f32::consts::PI / 180.);
        }
    }

    /// Modes 0x11 / 0x12 each frame (FUN_140423840 / FUN_140423a70): the
    /// standing glide becomes the moving one once the stick moves; the
    /// moving glide banks (206 / 207) when the facing and the stick differ;
    /// releasing Square or nearing the ground ends it with a fall from zero
    /// vertical speed (FUN_1404235b0, FUN_1403cbc30).
    fn glide_frame(&mut self, height: f32, moving: bool, direction: [f32; 2], step: &mut Step) {
        let Some(row) = self.glide else { return };
        if !self.square_held || height < row.glide_end {
            self.glide = None;
            self.arc = actor_physics::Arc::fall(self.position[1], 0.);
            self.launch_velocity = None;
            self.sampler.reset();
            self.start(Action::Fall, step);
            self.blend = growth::GLIDE_BLEND;
            return;
        }
        let Action::Growth(current) = self.action else {
            return;
        };
        let want = if current == growth::motion::GLIDE_STAND && !moving {
            growth::motion::GLIDE_STAND
        } else if !moving {
            growth::motion::GLIDE
        } else {
            let f = [self.yaw.sin(), self.yaw.cos()];
            let dot = f[0] * direction[0] + f[1] * direction[1];
            if dot >= growth::GLIDE_STRAIGHT {
                growth::motion::GLIDE
            } else {
                // The cross product's y in the game's Y-down space (scene
                // z is negated there).
                let cy = -(f[1] * direction[0] - f[0] * direction[1]);
                if cy <= 0. {
                    growth::motion::GLIDE_LEFT
                } else {
                    growth::motion::GLIDE_RIGHT
                }
            }
        };
        if want != current {
            // motion_keep: the new glide motion blends over 16.
            self.start(Action::Growth(want), step);
            self.blend = growth::GLIDE_BLEND;
        }
        if moving {
            let rate = if want == growth::motion::GLIDE_STAND {
                0.
            } else {
                row.glide_turn
            };
            self.turn_toward(direction, rate);
        }
    }

    /// The glide's two-frame velocity (FUN_140423650 / FUN_140423920):
    /// standing, the velocity keeps 0.9 a step; moving, the speed eases
    /// toward the row's along the facing. Either sinks by the row.
    fn glide_velocity(&mut self, row: &growth::Row, height: f32) -> [f32; 3] {
        if self.sampler.due(1.) {
            let dt = actor_physics::PHYSICS_STEP;
            if self.action == Action::Growth(growth::motion::GLIDE_STAND) {
                self.air_velocity = [
                    self.air_velocity[0] * growth::GLIDE_STAND_KEEP,
                    self.air_velocity[1] * growth::GLIDE_STAND_KEEP,
                ];
            } else {
                let a = row.glide_keep;
                let speed = (1. - a) * row.glide_speed
                    + self.air_velocity[0].hypot(self.air_velocity[1]) * a;
                self.air_velocity = [self.yaw.sin() * speed, self.yaw.cos() * speed];
            }
            let dy = -row.glide_sink(height) * dt;
            self.sampler
                .store([self.air_velocity[0] * dt, dy, self.air_velocity[1] * dt]);
        }
        let d = self.sampler.displacement();
        self.vertical_speed = d[1];
        d
    }

    /// Mode 1 for one tick: action changes in the air, then this tick's
    /// displacement (collided by `tick`).
    #[allow(clippy::too_many_arguments)]
    fn airborne(
        &mut self,
        input: &Input,
        moving: bool,
        magnitude: f32,
        direction: [f32; 2],
        height: f32,
        p: &Params,
        clips: &Clips,
        step: &mut Step,
    ) -> [f32; 3] {
        let glide_motion =
            |m: u16| (growth::motion::GLIDE..=growth::motion::GLIDE_STAND).contains(&m);
        // The dispatcher hands a Jump press to the airborne mode's jump
        // slot while actionable (FUN_1403d6fc0 → FUN_1403cb9f0).
        if input.jump
            && self.actionable(clips)
            && !matches!(self.action, Action::Hurt(_))
            && self.aerial_dodge(moving, direction, step)
        {
        } else if let Action::Growth(m) = self.action
            && glide_motion(m)
        {
            self.glide_frame(height, moving, direction, step);
        } else if let Action::Scripted(m) = self.action {
            let clip = clips.scripted_clip(m).map_or(&clips.idle, |g| &g.1);
            self.face_target(clip, input);
            if self.frame >= clip.end && !self.queued.is_empty() {
                let (next, blend) = self.queued.remove(0);
                if !self.start_motion_id(next, blend, clips) {
                    self.start(Action::Fall, step);
                }
            }
        } else if let Action::Hurt(_) = self.action {
            // The air flinch and Aerial Recovery play out, then fall;
            // Aerial Recovery takes commands from its range-1 tick.
            let ready = self.actionable(clips);
            if input.attack
                && ready
                && self.try_attack(input, p, clips, true, moving, direction, step)
            {
            } else if self.frame >= self.clip(clips).end {
                self.start(Action::Fall, step);
            }
        } else if let Action::AirCast(i) = self.action {
            let clip = &clips.spells[i].clip;
            self.face_target(clip, input);
            self.release_if_due(clips, i);
            let ready = self.actionable(clips);
            if (input.guard && self.square(input, clips, height, moving, direction, false, step))
                || (input.attack
                    && self.try_attack(input, p, clips, ready, moving, direction, step))
            {
            } else if self.frame >= clip.end {
                self.cast = None;
                self.start(Action::Fall, step);
            }
        } else if let Action::AirAttack(i) = self.action {
            let clip = &clips.attacks[i].clip;
            self.face_target(clip, input);
            let ready = self.actionable(clips);
            if (input.guard && self.square(input, clips, height, moving, direction, false, step))
                || (input.attack
                    && self.try_attack(input, p, clips, ready, moving, direction, step))
            {
            } else if self.frame >= clip.end {
                self.start(Action::Fall, step);
                self.blend = clips.attacks[i].blend;
            }
        } else if input.attack
            && self.actionable(clips)
            && self.try_attack(input, p, clips, true, moving, direction, step)
        {
            // Air attacks need an active ANB range 0–2 too: JUMP from its
            // range-1 start (tick 8 installed), FALL throughout.
        } else if input.guard && self.square(input, clips, height, moving, direction, false, step) {
        } else if matches!(
            self.action,
            Action::Jump | Action::HighJump | Action::Growth(growth::motion::AERIAL_DODGE)
        ) && self.arc.falling_motion()
        {
            // FUN_1403cb680 → FUN_1403cb610: FALL (motion 4) blended over 8
            // once the apex is within 5 ticks.
            self.start(Action::Fall, step);
        }
        // FUN_1403cb680: a released jump still rising (6 < t < rise − 5)
        // plays FALL (FUN_1403cb610) and is relaunched with a 5-tick rise
        // of g × 12.5 (mode vt+0x20, FUN_140409b00 → FUN_1403cbc30, then
        // the mode re-entry clears the flags); within 5 ticks of the apex
        // the jump stops being cuttable.
        if self.jump_cut.released && self.arc.cuttable() {
            if matches!(self.action, Action::Jump | Action::HighJump) {
                self.start(Action::Fall, step);
            }
            self.arc = actor_physics::Arc::launch(self.position[1], actor_physics::JUMP_CUT_HOP);
            self.sampler.reset();
            self.jump_cut = JumpCut::default();
        } else if self.arc.falling_motion() {
            self.jump_cut.held = false;
        }
        // ANB "No gravity" ranges (range 3 → obj+0x18c bit 5, FUN_14040d810)
        // hold the height while the curve's time keeps running
        // (FUN_1403cb500).
        if let Some(row) = self.glide {
            return self.glide_velocity(&row, height);
        }
        let floating = Clip::within(&self.clip(clips).no_gravity, self.frame);
        // Mode 1 horizontal (FUN_1403cb360 → FUN_1403b7ad0 with 0.9, 0.95):
        // the velocity keeps 90% of itself and takes 10% of the stick
        // velocity, or decays ×0.95 without a stick. The stick only feeds it
        // under an ANB range 0–2: JUMP from tick 8, FALL throughout, never
        // during aerial attacks [I, as on the ground].
        let steering = moving && self.actionable(clips);
        if steering {
            self.turn_toward(direction, p.turn_speed);
        }
        let (dir, speed) = if !steering {
            ([0.; 2], 0.)
        } else {
            let v = Self::stick_velocity(direction, magnitude, p);
            (direction, v[0].hypot(v[1]))
        };
        // FUN_1403b89a0: the mode routine runs every second frame with a
        // two-frame step; its displacement is spread over those frames.
        if self.sampler.due(speed) {
            let dt = actor_physics::PHYSICS_STEP;
            let dy = self.arc.step(self.position[1], p.fall_max, floating, dt);
            self.air_velocity = actor_physics::steer(
                self.air_velocity,
                dir,
                speed,
                actor_physics::AIR_INERTIA,
                actor_physics::AIR_DRAG,
            );
            self.sampler
                .store([self.air_velocity[0] * dt, dy, self.air_velocity[1] * dt]);
        }
        let d = self.sampler.displacement();
        self.vertical_speed = d[1];
        d
    }
}

/// Room collision triangles split into walkable floors and walls.
pub struct TriangleWorld {
    floors: Vec<[Vec3; 3]>,
    walls: Vec<[Vec3; 3]>,
}

impl TriangleWorld {
    /// Every floor and wall triangle (scene coordinates).
    pub fn triangles(&self) -> impl Iterator<Item = &[Vec3; 3]> {
        self.floors.iter().chain(&self.walls)
    }

    /// Triangles whose upward normal component is at least `min_floor_y`
    /// (after normalizing) are floors when `stand` is set; steeper ones are
    /// walls when `collide` is set. Winding is ignored because the selected
    /// collision data is not oriented here.
    pub fn new(
        triangles: impl IntoIterator<Item = ([Vec3; 3], bool, bool)>,
        min_floor_y: f32,
    ) -> Self {
        let mut floors = Vec::new();
        let mut walls = Vec::new();
        for (t, stand, collide) in triangles {
            let u = sub(t[1], t[0]);
            let v = sub(t[2], t[0]);
            let n = cross(u, v);
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if length.is_nan() || length <= 1e-6 || t.iter().flatten().any(|c| !c.is_finite()) {
                continue;
            }
            if (n[1] / length).abs() >= min_floor_y {
                if stand {
                    floors.push(t);
                }
            } else if collide {
                walls.push(t);
            }
        }
        Self { floors, walls }
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.floors.len(), self.walls.len())
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: Vec3, b: Vec3) -> Vec3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// Height of triangle `t` at (x, z), if the point lies inside its XZ shadow.
fn height_at(t: &[Vec3; 3], x: f32, z: f32) -> Option<f32> {
    let [a, b, c] = t;
    let d = (b[2] - c[2]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[2] - c[2]);
    if d.abs() < 1e-9 {
        return None;
    }
    let w0 = ((b[2] - c[2]) * (x - c[0]) + (c[0] - b[0]) * (z - c[2])) / d;
    let w1 = ((c[2] - a[2]) * (x - c[0]) + (a[0] - c[0]) * (z - c[2])) / d;
    let w2 = 1. - w0 - w1;
    let e = -1e-4;
    (w0 >= e && w1 >= e && w2 >= e).then(|| w0 * a[1] + w1 * b[1] + w2 * c[1])
}

fn closest_on_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let len = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if len > 0. {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / len).clamp(0., 1.)
    } else {
        0.
    };
    [a[0] + ab[0] * t, a[1] + ab[1] * t]
}

impl World for TriangleWorld {
    fn floor(&self, x: f32, z: f32, top: f32) -> Option<f32> {
        self.floors
            .iter()
            .filter_map(|t| height_at(t, x, z))
            .filter(|&h| h <= top + 1e-3)
            .reduce(f32::max)
    }

    fn resolve_walls(&self, position: [f32; 2], radius: f32, low: f32, high: f32) -> [f32; 2] {
        let mut p = position;
        // A few relaxation passes let the body slide along corners.
        for _ in 0..4 {
            let mut pushed = false;
            for t in &self.walls {
                let (lo, hi) = t
                    .iter()
                    .fold((f32::MAX, f32::MIN), |(l, h), v| (l.min(v[1]), h.max(v[1])));
                if hi <= low || lo >= high {
                    continue;
                }
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let q = closest_on_segment(p, [a[0], a[2]], [b[0], b[2]]);
                    let d = [p[0] - q[0], p[1] - q[1]];
                    let distance = d[0].hypot(d[1]);
                    if distance < radius && distance > 1e-5 {
                        let push = (radius - distance) / distance;
                        p = [p[0] + d[0] * push, p[1] + d[1] * push];
                        pushed = true;
                    }
                }
            }
            if !pushed {
                break;
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> Params {
        Params {
            walk_speed: 2.,
            run_speed: 8.,
            jump_height: 185.,
            turn_speed: 0.2618,
            fall_max: 16.,
            land_inertia: 0.9,
            land_drag: 0.8,
            step_height: 20.,
            walk_threshold: 0.5,
            radius: 30.,
            height: 140.,
            disarm_ticks: 120,
        }
    }
    fn attack(end: u32, window: (u32, u32), hop: f32) -> AttackStep {
        AttackStep {
            clip: Clip {
                end,
                loop_start: 0,
                combo_windows: vec![window],
                blend_to_idle: Some(end - 10),
                ..Clip::default()
            },
            hop,
            hop_speed: 0.,
            hop_max: 0.,
            hop_min: 0.,
            hop_speed_min: 0.,
            hop_reach: 0.,
            blend: 0.,
            root: Vec::new(),
        }
    }
    /// A cut of Sora's set: finisher 3, the basic swing 5 (alternate 6),
    /// the target dash 6, the air swings 9 (alternate 10) and 10,
    /// Retaliating Slash and Upper Slash.
    fn records() -> Vec<ptya::Record> {
        use ptya::{flag, kind};
        let r = |id, kind, alternate, flags, motion, jump| ptya::Record {
            id,
            kind,
            alternate,
            flags,
            motion,
            jump,
            cos_min: 0.99996,
            cos_max: 1.,
            ..ptya::Record::default()
        };
        vec![
            r(3, kind::ALWAYS, -1, flag::FINISHER, 155, 0.),
            r(5, kind::ALWAYS, 6, 0, 151, 0.),
            r(6, kind::TARGET, -1, 0, 153, 0.),
            r(9, kind::ALWAYS, 10, flag::AIR, 181, 21.),
            r(10, kind::TARGET, -1, flag::AIR, 182, 21.),
            ptya::Record {
                ability: 101,
                ..r(0x28, kind::MANUAL, -1, flag::AIR, 172, 0.)
            },
            ptya::Record {
                ability: 18,
                ..r(0x0c, kind::MANUAL, -1, 0, 161, 0.)
            },
        ]
    }
    /// A loop with ANB range 0 over its whole length (always actionable).
    fn open(end: u32) -> Clip {
        Clip {
            actionable: vec![(0, end)],
            ..Clip::looping(end, 0)
        }
    }
    fn clips() -> Clips {
        Clips {
            idle: open(100),
            walk: open(60),
            run: open(40),
            field_idle: open(90),
            field_walk: open(50),
            field_run: open(36),
            jump: open(30),
            fall: open(30),
            land: Clip::looping(12, 0),
            drive: Some(Clip::looping(40, 40)),
            drive_in: None,
            growth: Vec::new(),
            scripted: Vec::new(),
            records: records(),
            spells: vec![
                SpellClip {
                    motion: 56,
                    clip: Clip {
                        end: 40,
                        release: vec![(10, Some(56))],
                        combo_windows: vec![(30, 40)],
                        ..Clip::default()
                    },
                    root: Vec::new(),
                },
                SpellClip {
                    motion: 57,
                    clip: Clip {
                        end: 40,
                        release: vec![(12, Some(57))],
                        ..Clip::default()
                    },
                    root: Vec::new(),
                },
                SpellClip {
                    motion: 58,
                    clip: Clip {
                        end: 40,
                        release: vec![(12, Some(58))],
                        ..Clip::default()
                    },
                    root: Vec::new(),
                },
            ],
            attacks: vec![
                attack(50, (60, 60), 0.),
                attack(40, (10, 25), 0.),
                attack(40, (10, 25), 0.),
                attack(30, (8, 20), 20.),
                attack(30, (8, 20), 20.),
                attack(30, (8, 20), 0.),
                attack(40, (10, 25), 0.),
            ],
            hurt: {
                let flat = |end| Clip {
                    end,
                    ..Clip::default()
                };
                let mut v = vec![flat(24); 4];
                // AIR_FRONT, AIR_BACK; BLOW_FRONT (loop from 4) and its
                // land, BLOW_BACK (loop from 3) and its land; Aerial
                // Recovery (range 27 to tick 16, range 1 from 24); DEAD.
                v.extend([flat(36), flat(36)]);
                v.push(Clip::looping(28, 4));
                v.push(Clip {
                    blend_to_idle: Some(40),
                    ..flat(50)
                });
                v.push(Clip::looping(26, 3));
                v.push(Clip {
                    blend_to_idle: Some(40),
                    ..flat(50)
                });
                v.push(Clip {
                    actionable: vec![(24, 30)],
                    invulnerable: vec![(0, 16)],
                    ..flat(30)
                });
                v.push(Clip::looping(28, 4));
                v
            },
            guard: Some(Clip {
                end: 60,
                blend_to_idle: Some(52),
                ..Clip::default()
            }),
            guard_recoil: Some(Clip {
                end: 80,
                blend_to_idle: Some(40),
                ..Clip::default()
            }),
            item: Some(Clip {
                end: 60,
                blend_to_idle: Some(36),
                item_use: Some(18),
                ..Clip::default()
            }),
        }
    }
    /// A flat 2000-unit square floor with one wall along x = 500.
    fn room() -> TriangleWorld {
        let f = [
            [-1000., 0., -1000.],
            [1000., 0., -1000.],
            [1000., 0., 1000.],
            [-1000., 0., 1000.],
        ];
        let w = [
            [500., 0., -1000.],
            [500., 300., -1000.],
            [500., 300., 1000.],
            [500., 0., 1000.],
        ];
        TriangleWorld::new(
            [
                [f[0], f[1], f[2]],
                [f[0], f[2], f[3]],
                [w[0], w[1], w[2]],
                [w[0], w[2], w[3]],
            ]
            .map(|t| (t, true, true)),
            0.5,
        )
    }
    fn hold(dir: [f32; 2]) -> Input {
        Input {
            direction: dir,
            ..Input::default()
        }
    }

    #[test]
    fn splits_floors_from_walls() {
        assert_eq!(room().counts(), (2, 2));
        assert_eq!(room().floor(0., 0., 10.), Some(0.));
        assert_eq!(room().floor(0., 0., -10.), None);
        assert_eq!(room().floor(5000., 0., 10.), None);
    }

    #[test]
    fn walks_and_runs_by_stick_magnitude_and_turns_at_rate_limit() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        player.tick(&hold([0., 0.3]), &p, &c, &w);
        assert_eq!(player.action, Action::Walk);
        assert!((player.position[2] - 2.).abs() < 1e-5);
        // FUN_1403b89a0 samples the move request every second frame: the
        // run starts next frame, the walk velocity carries this one.
        player.tick(&hold([0., 1.]), &p, &c, &w);
        assert_eq!(player.action, Action::Run);
        assert!((player.position[2] - 4.).abs() < 1e-5);
        player.tick(&hold([0., 1.]), &p, &c, &w);
        assert!((player.position[2] - 12.).abs() < 1e-5);
        // Facing +X requires a quarter turn: limited per tick, while the
        // body moves along the stick (FUN_140422350) at the next sample.
        player.tick(&hold([1., 0.]), &p, &c, &w);
        assert!((player.yaw - 0.2618).abs() < 1e-5);
        assert!((player.position[2] - 20.).abs() < 1e-5);
        player.tick(&hold([1., 0.]), &p, &c, &w);
        assert!((player.position[0] - 8.).abs() < 1e-5);
        assert!((player.position[2] - 20.).abs() < 1e-5);
        player.tick(&Input::default(), &p, &c, &w);
        assert_eq!(player.action, Action::Idle);
    }

    #[test]
    fn walk_to_run_keeps_the_gait_phase_and_blends_eight() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        for _ in 0..20 {
            player.tick(&hold([0., 0.3]), &p, &c, &w);
        }
        assert_eq!((player.action, player.motion_frame(&c)), (Action::Walk, 19));
        assert_eq!(player.blend, 8.);
        player.tick(&hold([0., 1.]), &p, &c, &w);
        // Field walk 50 → field run 36: 36 × (20 + 8) / 50 = 20.16.
        assert_eq!((player.action, player.frame), (Action::Run, 20));
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        assert_eq!(player.blend, 0.);
    }

    #[test]
    fn running_jump_carries_momentum_then_drags() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([-900., 0., 0.], 0.);
        let run_jump = Input {
            direction: [0., 1.],
            jump: true,
            ..Input::default()
        };
        player.tick(&run_jump, &p, &c, &w);
        assert_eq!(player.pending_jump, Some((185., [0., 8.])));
        // Stick released: FUN_1403b7ad0 scales the velocity by 0.95 per
        // two-frame sample, moving 7.6 on each of those frames.
        let z = player.position[2];
        player.tick(&Input::default(), &p, &c, &w);
        player.tick(&Input::default(), &p, &c, &w);
        assert!((player.position[2] - z - 15.2).abs() < 1e-4);
        // Stick sideways: 10% of the stick velocity at the next sample.
        player.tick(&hold([1., 0.]), &p, &c, &w);
        assert!((player.air_velocity[0] - 0.8).abs() < 1e-5);
        assert!((player.air_velocity[1] - 7.6 * 0.9).abs() < 1e-4);
    }

    #[test]
    fn fall_motion_starts_five_ticks_before_the_apex() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        let mut fell = None;
        for tick in 1..40 {
            let step = player.tick(&Input::default(), &p, &c, &w);
            if step.started == Some(Action::Fall) {
                fell = Some(tick);
                break;
            }
        }
        // rise √(2·185/g) = 30.1 ticks; FALL starts when rise ≤ t + 5. The
        // curve's t advances by 2 every second frame (t = 26 on tick 26).
        assert_eq!(fell, Some(26));
        assert_eq!(player.blend, 8.);
        assert!(player.position[1] < 185.);
    }

    /// The installed JUMP/LAND trigger layout: take-off at JUMP tick 4,
    /// commands from JUMP tick 8; LAND locked until tick 12 and inertial
    /// throughout (range 42).
    fn installed_jump_land(c: &mut Clips) {
        c.jump = Clip {
            actionable: vec![(8, 26)],
            takeoff: Some(4),
            ..Clip::looping(26, 0)
        };
        c.land = Clip {
            actionable: vec![(12, 26)],
            inertia: vec![(0, 26)],
            blend_to_idle: Some(12),
            ..Clip::looping(26, 0)
        };
    }

    #[test]
    fn jump_takes_off_at_the_jump_trigger() {
        let (p, mut c, w) = (params(), clips(), room());
        installed_jump_land(&mut c);
        let mut player = Player::new([0., 0., 0.], 0.);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        assert_eq!(player.action, Action::Jump);
        for _ in 1..4 {
            player.tick(&Input::default(), &p, &c, &w);
            assert_eq!(player.position[1], 0.);
            assert!(player.pending_jump.is_some());
        }
        // FUN_14040dd70: frame trigger 0 switches to the jump mode.
        player.tick(&Input::default(), &p, &c, &w);
        assert!(player.pending_jump.is_none());
        assert!(player.position[1] > 0.);
        // No air attack before JUMP's range-1 start (tick 8).
        let press = Input {
            attack: true,
            ..Input::default()
        };
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::Jump);
        for _ in 0..3 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::AirAttack(3));
    }

    #[test]
    fn landing_locks_commands_and_slides_inertially() {
        let (p, mut c, w) = (params(), clips(), room());
        installed_jump_land(&mut c);
        let mut player = Player::new([-900., 0., 0.], 0.);
        let run_jump = Input {
            direction: [0., 1.],
            jump: true,
            ..Input::default()
        };
        player.tick(&run_jump, &p, &c, &w);
        let mut landed = false;
        for _ in 0..120 {
            if player.tick(&hold([0., 1.]), &p, &c, &w).landed {
                landed = true;
                break;
            }
        }
        assert!(landed);
        assert_eq!(player.action, Action::Land);
        // Locked until LAND tick 12: no jump, no switch to running and no
        // stick; range 42 eases the carried velocity by the PRTY drag.
        let push = Input {
            direction: [0., 1.],
            jump: true,
            ..Input::default()
        };
        for _ in 0..10 {
            player.tick(&push, &p, &c, &w);
            assert_eq!(player.action, Action::Land);
        }
        assert!(player.air_velocity[1] > 0.);
        player.tick(&hold([0., 1.]), &p, &c, &w);
        player.tick(&hold([0., 1.]), &p, &c, &w);
        assert_eq!(player.action, Action::Run);
    }

    #[test]
    fn aerial_hop_heads_for_the_target() {
        let (p, mut c, w) = (params(), clips(), room());
        c.attacks[3].hop_max = 71.;
        c.attacks[3].hop_min = 21.;
        c.attacks[3].hop_speed_min = -8.;
        c.attacks[3].hop_speed = 8.;
        let mut player = Player::new([0., 0., 0.], 0.);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        for _ in 0..10 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        let press = Input {
            attack: true,
            target: Some([400., 0.]),
            target_rise: -100.,
            ..Input::default()
        };
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::AirAttack(3));
        // Height clamps to the 21 minimum; speed 400 / √(2·21/g) > 8 caps
        // at 8 toward +X, then the first two-frame sample drags it ×0.95.
        assert!((player.arc.rise - actor_physics::rise_time(21.)).abs() < 1e-4);
        assert!((player.air_velocity[0] - 7.6).abs() < 1e-4);
    }

    /// A world that counts collide calls and moves freely on a flat floor.
    struct Counting(std::cell::Cell<u32>);
    impl World for Counting {
        fn floor(&self, _x: f32, _z: f32, top: f32) -> Option<f32> {
            (top >= -1e-3).then_some(0.)
        }
        fn resolve_walls(&self, p: [f32; 2], _r: f32, _l: f32, _h: f32) -> [f32; 2] {
            p
        }
        fn collide(&self, start: Vec3, d: Vec3, airborne: bool, _b: &Body) -> Collided {
            self.0.set(self.0.get() + 1);
            let y = if airborne {
                (start[1] + d[1]).max(0.)
            } else {
                0.
            };
            Collided {
                position: [start[0] + d[0], y, start[2] + d[2]],
                on_floor: !airborne || (d[1] <= 0. && y == 0.),
            }
        }
    }

    #[test]
    fn one_collide_call_per_tick_with_knock_and_movement() {
        let (p, c) = (params(), clips());
        let w = Counting(std::cell::Cell::new(0));
        let mut player = Player::new([0., 0., 0.], 0.);
        player.guard_recoil([0., -1.], &c);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        for t in 0..120u32 {
            let input = if t == 60 { jump } else { hold([1., 0.]) };
            let before = w.0.get();
            player.tick(&input, &p, &c, &w);
            assert!(w.0.get() - before <= 1, "tick {t}");
        }
        assert!(player.position[0] > 0.);
    }

    #[test]
    fn walls_stop_the_body_radius_away() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([400., 0., 0.], std::f32::consts::FRAC_PI_2);
        for _ in 0..60 {
            player.tick(&hold([1., 0.]), &p, &c, &w);
        }
        assert!((player.position[0] - (500. - p.radius)).abs() < 1e-3);
    }

    #[test]
    fn jump_reaches_parameter_height_then_lands() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        assert_eq!(player.tick(&jump, &p, &c, &w).started, Some(Action::Jump));
        let mut apex: f32 = 0.;
        let mut landed = None;
        for t in 0..200 {
            let step = player.tick(&Input::default(), &p, &c, &w);
            apex = apex.max(player.position[1]);
            if step.landed {
                landed = Some(t);
                break;
            }
        }
        // FUN_1403cb500: the double-sine rise reaches 185 at t ≈ 30.1 and
        // the parabola lands at t ≈ 60.2.
        assert!(apex > 184.9 && apex <= 185.001, "apex {apex}");
        assert_eq!(landed, Some(60));
        assert_eq!(player.action, Action::Land);
        assert_eq!(player.position[1], 0.);
        for _ in 0..12 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert_eq!(player.action, Action::Idle);
    }

    #[test]
    fn combo_chains_only_inside_allow_combo_windows() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        let press = Input {
            attack: true,
            ..Input::default()
        };
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(1));
        assert_eq!(player.combo, ptya::Combo { step: 1, finish: 1 });
        // Presses last one frame: before the window nothing happens, and a
        // swing that missed cannot chain inside it either.
        for _ in 0..9 {
            player.tick(&press, &p, &c, &w);
            assert_eq!(player.action, Action::Attack(1));
        }
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(1));
        // After a hit the press in the window chains: 5 would repeat 151,
        // so its alternate 6 (153) plays.
        player.connected = true;
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(2));
        assert_eq!(player.combo.step, 2);
        assert!(!player.connected);
        for _ in 0..10 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        // Third step reaches the limit 3: the finisher.
        player.connected = true;
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(0));
        // Without another press the step finishes and returns to idle.
        for _ in 0..60 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert_eq!(player.action, Action::Idle);
    }

    /// Authored records/windows isolate the native combo gates without
    /// reproducing a game table or relying on a recorded animation.
    fn modifier_clips() -> Clips {
        let mut c = clips();
        c.records = [
            (3, 2, ptya::flag::FINISHER),
            (2, 3, ptya::flag::FINISHER),
            (5, 6, 0),
            (6, 5, 0),
            (11, 12, ptya::flag::AIR | ptya::flag::FINISHER),
            (12, 11, ptya::flag::AIR | ptya::flag::FINISHER),
            (9, 10, ptya::flag::AIR),
            (10, 9, ptya::flag::AIR),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (id, alternate, flags))| ptya::Record {
            id,
            alternate,
            flags,
            kind: ptya::kind::ALWAYS,
            motion: 1000 + i as u16,
            ..ptya::Record::default()
        })
        .collect();
        c.attacks = c
            .records
            .iter()
            .map(|r| AttackStep {
                clip: Clip {
                    end: 12,
                    combo_windows: if r.flags & ptya::flag::FINISHER == 0 {
                        vec![(2, 10)]
                    } else {
                        Vec::new()
                    },
                    finisher_windows: if r.flags & ptya::flag::FINISHER != 0 {
                        vec![(2, 10)]
                    } else {
                        Vec::new()
                    },
                    ..Clip::default()
                },
                ..attack(12, (2, 10), 0.)
            })
            .collect();
        c
    }

    fn modifier_abilities(items: &[u16]) -> abilities::Abilities {
        use abilities::{bit, item};
        let mut bytes = vec![0u8; 0x178];
        bytes[0x11c..0x11e].copy_from_slice(&2u16.to_le_bytes());
        bytes[0x130..0x132].copy_from_slice(&10u16.to_le_bytes());
        let rates = abilities::Rates::read(&bytes).unwrap();
        let slots: Vec<_> = items.iter().map(|id| id | 0x8000).collect();
        let mut a = abilities::Abilities::default();
        a.equip(
            &slots,
            None,
            |id| {
                Some((
                    match id {
                        item::COMBO_BOOST => bit::COMBO_BOOST,
                        item::AIR_COMBO_BOOST => bit::AIR_COMBO_BOOST,
                        item::BERSERK_CHARGE => bit::BERSERK,
                        0x21b => bit::COMBO_MASTER,
                        // Synthetic bits for count adders: no record in
                        // this authored set gates on these particular bits.
                        _ => 40 + id % 8,
                    },
                    false,
                ))
            },
            &abilities::GROWTH,
            &rates,
        );
        a
    }

    fn modifier_press(player: &mut Player, c: &Clips, connected: bool) -> Step {
        if player.combo.step != 0 {
            player.tick(&Input::default(), &params(), c, &room());
        }
        player.connected = connected;
        player.tick(
            &Input {
                attack: true,
                ..Input::default()
            },
            &params(),
            c,
            &room(),
        )
    }

    fn modifier_damage(player: &Player, air: bool) -> i32 {
        let a = &player.abilities;
        combat::damage(
            &combat::AttackerStats {
                strength: 7,
                max: 100,
                min: 1,
                mods: combat::AttackerMods {
                    player_type: true,
                    mp_charging: player.mp_charging,
                    berserk_strength: a.berserk_strength as i32,
                    combo_step: player.combo.step as i32,
                    ground_limit: a.ground_combo as i32,
                    air_limit: a.air_combo as i32,
                    combo_boost: a.combo_boost as i32,
                    air_combo_boost: a.air_combo_boost as i32,
                    combo_boost_enabled: a.has(abilities::bit::COMBO_BOOST),
                    air_combo_boost_enabled: a.has(abilities::bit::AIR_COMBO_BOOST),
                    ..combat::AttackerMods::default()
                },
                ..combat::AttackerStats::default()
            },
            &combat::AttackParams {
                power: 200,
                combo_kind: if air { 2 } else { 1 },
                ..combat::AttackParams::default()
            },
            &combat::TargetStats {
                max_hp: 100,
                defense: 2,
                cap: 100,
                min: 1,
                weakness: [100; 7],
            },
        )
    }

    #[test]
    fn equipped_count_modifiers_finishers_and_boosts_drive_ground_and_air_attacks() {
        use abilities::item;
        let a = modifier_abilities(&[
            item::COMBO_PLUS,
            item::COMBO_PLUS,
            item::AIR_COMBO_PLUS,
            item::NEGATIVE_COMBO,
            item::FINISHING_PLUS,
            item::COMBO_BOOST,
            item::COMBO_BOOST,
            item::AIR_COMBO_BOOST,
        ]);
        assert_eq!((a.ground_combo, a.air_combo, a.finishers), (4, 3, 2));
        let c = modifier_clips();
        for (air, sequence, expected_damage) in [
            (false, vec![5, 6, 5, 3, 2], 18),
            (true, vec![9, 10, 11, 12], 13),
        ] {
            let mut player = Player::new([0., if air { 1000. } else { 0. }, 0.], 0.);
            player.abilities = a.clone();
            if air {
                player.action = Action::Fall;
                player.arc = actor_physics::Arc::fall(1000., 0.);
            }
            for (i, id) in sequence.into_iter().enumerate() {
                let started = modifier_press(&mut player, &c, true).started.unwrap();
                let index = match started {
                    Action::Attack(i) | Action::AirAttack(i) => i,
                    other => panic!("unexpected action {other:?}"),
                };
                assert_eq!(c.records[index].id, id);
                assert_eq!(player.combo.step as usize, i + 1);
                assert!(!player.connected);
                if player.combo.finish == 2 {
                    assert_eq!(modifier_damage(&player, air), expected_damage);
                }
            }
            assert_eq!(player.combo.finish, 2);
            let stopped = player.combo;
            assert!(modifier_press(&mut player, &c, true).started.is_none());
            assert_eq!(player.combo, stopped);
        }
    }

    #[test]
    fn combo_master_chains_misses_and_berserk_suppresses_finishers_only_during_charge() {
        use abilities::item;
        let c = modifier_clips();
        let mut without_master = Player::new([0.; 3], 0.);
        without_master.abilities = modifier_abilities(&[item::BERSERK_CHARGE]);
        without_master.mp.change(-without_master.mp.max - 1, 30.);
        assert!(
            modifier_press(&mut without_master, &c, false)
                .started
                .is_some()
        );
        assert!(
            modifier_press(&mut without_master, &c, false)
                .started
                .is_none()
        );
        assert_eq!(without_master.combo.step, 1);
        let mut player = Player::new([0.; 3], 0.);
        player.abilities = modifier_abilities(&[
            item::NEGATIVE_COMBO,
            item::NEGATIVE_COMBO,
            item::FINISHING_PLUS,
            item::BERSERK_CHARGE,
            0x21b,
        ]);
        assert_eq!(player.abilities.ground_combo, 1);
        player.mp.change(-player.mp.max - 1, 30.);
        for count in 1..=6 {
            let step = modifier_press(&mut player, &c, false);
            let Some(Action::Attack(index)) = step.started else {
                panic!("miss {count} did not chain");
            };
            assert_eq!(c.records[index].flags & ptya::flag::FINISHER, 0);
            assert_eq!(
                player.combo,
                ptya::Combo {
                    step: count,
                    finish: 1
                }
            );
            assert_eq!(modifier_damage(&player, false), 14);
        }
        player.mp.add(100);
        let Some(Action::Attack(index)) = modifier_press(&mut player, &c, false).started else {
            panic!("finisher did not return after charge");
        };
        assert_eq!(c.records[index].id, 3);
        assert_eq!(modifier_damage(&player, false), 10);
        // Count one makes Negative Combo select a finisher immediately
        // when the equipped Berserk ability has no active MP Charge.
        let mut fresh = Player::new([0.; 3], 0.);
        fresh.abilities = player.abilities.clone();
        assert_eq!(
            modifier_press(&mut fresh, &c, false).started,
            Some(Action::Attack(0))
        );
    }

    #[test]
    fn a_negative_signed_limit_selects_a_finisher_and_caps_boost_damage_below_zero() {
        use abilities::item;
        let c = modifier_clips();
        let mut player = Player::new([0.; 3], 0.);
        player.abilities = modifier_abilities(&[
            item::NEGATIVE_COMBO,
            item::NEGATIVE_COMBO,
            item::NEGATIVE_COMBO,
            item::NEGATIVE_COMBO,
            item::COMBO_BOOST,
            item::COMBO_BOOST,
        ]);
        assert_eq!(player.abilities.ground_combo, -1);
        assert_eq!(
            modifier_press(&mut player, &c, false).started,
            Some(Action::Attack(0))
        );
        assert_eq!(player.combo.step, 1);
        // The native capped count remains signed: 10 + (-1 × 20 × 10
        // + 99) / 100 = 9, with division truncating toward zero.
        assert_eq!(modifier_damage(&player, false), 9);
    }

    #[test]
    fn recovery_frames_allow_movement_cancel() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        player.tick(
            &Input {
                attack: true,
                ..Input::default()
            },
            &p,
            &c,
            &w,
        );
        for _ in 0..20 {
            player.tick(&hold([0., 1.]), &p, &c, &w);
        }
        assert_eq!(player.action, Action::Attack(1));
        assert_eq!(player.position, [0., 0., 0.]);
        for _ in 0..15 {
            player.tick(&hold([0., 1.]), &p, &c, &w);
        }
        assert!(matches!(player.action, Action::Run | Action::Idle));
    }

    #[test]
    fn air_attacks_hop_and_restart_from_the_fall() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        let press = Input {
            attack: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        for _ in 0..25 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        player.tick(&press, &p, &c, &w);
        assert_eq!(player.action, Action::AirAttack(3));
        assert!(player.vertical_speed > 0.);
        for _ in 0..40 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert!(!matches!(player.action, Action::AirAttack(_)));
        // FALL is actionable (range 0): a new air combo starts from step 1
        // (FUN_1403d6ca0), so the exe allows another swing while airborne.
        if player.action == Action::Fall {
            player.tick(&press, &p, &c, &w);
            assert_eq!(player.action, Action::AirAttack(3));
            assert_eq!(player.combo.step, 1);
        }
    }

    #[test]
    fn walking_off_a_ledge_falls() {
        let (p, c, _) = (params(), clips(), ());
        let high = [
            [-100., 100., -100.],
            [100., 100., -100.],
            [100., 100., 100.],
            [-100., 100., 100.],
        ];
        let low = [
            [-1000., 0., -1000.],
            [1000., 0., -1000.],
            [1000., 0., 1000.],
            [-1000., 0., 1000.],
        ];
        let w = TriangleWorld::new(
            [
                [high[0], high[1], high[2]],
                [high[0], high[2], high[3]],
                [low[0], low[1], low[2]],
                [low[0], low[2], low[3]],
            ]
            .map(|t| (t, true, true)),
            0.5,
        );
        let mut player = Player::new([0., 100., 0.], 0.);
        let mut fell = false;
        for _ in 0..80 {
            player.tick(&hold([0., 1.]), &p, &c, &w);
            fell |= player.action == Action::Fall;
        }
        assert!(fell);
        assert_eq!(player.position[1], 0.);
    }

    #[test]
    fn surfaces_without_flags_are_ignored() {
        let f = [[-10., 0., -10.], [10., 0., -10.], [10., 0., 10.]];
        let w = [[0., 0., -10.], [0., 99., -10.], [0., 99., 10.]];
        let world = TriangleWorld::new([(f, false, true), (w, true, false)], 0.5);
        assert_eq!(world.counts(), (0, 0));
    }

    #[test]
    fn attacking_draws_the_weapon_until_idle_timeout() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        assert!(!player.armed);
        assert_eq!(player.clip(&c).end, 90);
        player.tick(
            &Input {
                attack: true,
                ..Input::default()
            },
            &p,
            &c,
            &w,
        );
        assert!(player.armed);
        for _ in 0..60 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert_eq!(player.action, Action::Idle);
        assert!(player.armed);
        assert_eq!(player.clip(&c).end, 100);
        for _ in 0..p.disarm_ticks {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert!(!player.armed);
        assert_eq!(player.clip(&c).end, 90);
    }

    #[test]
    fn attack_root_motion_moves_along_facing() {
        let (p, mut c, w) = (params(), clips(), room());
        c.attacks[1].root = (0..40).map(|f| [0., 0., f as f32 * 2.]).collect();
        // Facing +X: model +Z root motion moves the player along +X.
        let mut player = Player::new([0., 0., 0.], std::f32::consts::FRAC_PI_2);
        player.tick(
            &Input {
                attack: true,
                ..Input::default()
            },
            &p,
            &c,
            &w,
        );
        for _ in 0..10 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert!(
            (player.position[0] - 20.).abs() < 1e-3,
            "{:?}",
            player.position
        );
        assert!(player.position[2].abs() < 1e-3);
    }

    #[test]
    fn rejects_invalid_parameters() {
        let mut p = params();
        assert!(p.validate().is_ok());
        p.run_speed = f32::NAN;
        assert!(p.validate().is_err());
    }

    #[test]
    fn guard_turns_to_target_locks_and_recoils_with_a_knock_slide() {
        let (p, c, world) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        let press = Input {
            guard: true,
            ..Input::default()
        };
        // Guard needs its ability (bit 1).
        assert_eq!(player.tick(&press, &p, &c, &world).started, None);
        player.abilities.set(abilities::bit::GUARD);
        let press = Input {
            guard: true,
            target: Some([1., 0.]),
            ..Input::default()
        };
        let step = player.tick(&press, &p, &c, &world);
        assert_eq!(step.started, Some(Action::Guard));
        assert!((player.yaw - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        // Locked: moving does nothing until tick 52.
        let stick = Input {
            direction: [0., 1.],
            ..Input::default()
        };
        for _ in 1..52 {
            player.tick(&stick, &p, &c, &world);
        }
        assert_eq!(player.action, Action::Guard);
        player.tick(&stick, &p, &c, &world);
        assert_eq!(player.action, Action::Idle);
        // Recoil: face the attacker and slide away 20, ×0.9, until below 1.
        assert_eq!(
            player.guard_recoil([0., -1.], &c),
            Some(Action::GuardRecoil)
        );
        assert!(player.yaw.abs() < 1e-5);
        let start = player.position[2];
        for _ in 0..60 {
            player.tick(&Input::default(), &p, &c, &world);
        }
        let moved = start - player.position[2];
        let expected: f32 = (0..29).map(|k| 20. * 0.9f32.powi(k)).sum();
        assert!((moved - expected).abs() < 1e-2, "{moved} vs {expected}");
    }

    #[test]
    fn hurt_plays_its_clip_slides_back_and_returns_to_idle() {
        let c = clips();
        let world = room();
        let mut player = Player::new([0.; 3], 0.);
        assert_eq!(player.hurt(1, [0., -1.], 100., &c), Some(Action::Hurt(1)));
        assert!(player.armed);
        let p = params();
        player.tick(&Input::default(), &p, &c, &world);
        assert!((player.position[2] + 10.).abs() < 1e-3);
        player.tick(&Input::default(), &p, &c, &world);
        assert!((player.position[2] + 19.).abs() < 1e-3);
        for _ in 0..30 {
            player.tick(&Input::default(), &p, &c, &world);
        }
        assert_eq!(player.action, Action::Idle);
    }

    fn sstm() -> actor_physics::Sstm {
        actor_physics::Sstm {
            fall_max: 16.,
            blow_brake_xz: 0.8,
            blow_min_xz: 0.1,
            blow_brake_up: 0.8,
            blow_up: 24.,
            blow_speed: 40.,
            blow_to_hit_back: 8.,
            hit_back: 200.,
            hit_back_small: 200.,
            hit_back_to_jump: 0.048,
        }
    }

    fn claw(direction: [f32; 2], reaction: u8) -> Hit {
        Hit {
            reaction,
            original: reaction,
            direction,
            high: true,
            // Claws carry KB1 0x7fff; a blow's KB1/KB2 are up and speed,
            // both 0 for the SSTM defaults.
            knockback: if reaction == 4 { [0, 0] } else { [0x7fff, 0] },
        }
    }

    #[test]
    fn a_class_three_hit_on_the_ground_flinches_by_facing_and_slides() {
        let (c, s) = (clips(), sstm());
        // Facing +Z; the attacker is in front (the hit travels toward -Z).
        let mut front = Player::new([0.; 3], 0.);
        let hit = claw([0., -1.], 3);
        assert_eq!(front.react(&hit, &s, &c), Some(Action::Hurt(0)));
        assert!(front.yaw.abs() < 1e-5, "a ground flinch keeps the facing");
        // From behind, with a LOW sphere: BACK_LOW.
        let mut back = Player::new([0.; 3], 0.);
        let hit = Hit {
            high: false,
            ..claw([0., 1.], 3)
        };
        assert_eq!(back.react(&hit, &s, &c), Some(Action::Hurt(3)));
        assert_eq!(back.motion_id(&c), 13);
        // Class 2 flinches without a slide.
        let mut plain = Player::new([0.; 3], 0.);
        plain.react(&claw([0., -1.], 2), &s, &c);
        let p = params();
        plain.tick(&Input::default(), &p, &c, &room());
        assert_eq!(plain.position, [0.; 3]);
        // Class 1 slides without a motion.
        let mut slide = Player::new([0.; 3], 0.);
        assert_eq!(slide.react(&claw([0., -1.], 1), &s, &c), None);
        slide.tick(&Input::default(), &p, &c, &room());
        assert!((slide.position[2] + 20.).abs() < 1e-3);
    }

    #[test]
    fn a_class_three_hit_in_the_air_flinches_and_hops() {
        let (p, c, s, w) = (params(), clips(), sstm(), room());
        let mut player = Player::new([0., 100., 0.], 0.);
        player.arc = actor_physics::Arc::fall(100., 0.);
        player.action = Action::Fall;
        // Hit from the front: AIR_FRONT, turned to face the attacker.
        let hit = claw([0., -1.], 3);
        assert_eq!(player.react(&hit, &s, &c), Some(Action::Hurt(hurt::AIR[0])));
        assert!(player.in_air());
        assert_eq!(player.motion_id(&c), 14);
        // 50 units up, 200 × 0.048 = 9.6 per tick away from the attacker.
        let mut top = 100f32;
        for _ in 0..30 {
            player.tick(&Input::default(), &p, &c, &w);
            top = top.max(player.position[1]);
        }
        assert!(top > 145. && top < 151., "{top}");
        assert!(player.position[2] < -50., "{}", player.position[2]);
        // The flinch ends into Fall, then Land.
        for _ in 0..200 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert!(!player.in_air());
    }

    #[test]
    fn a_blow_flies_lands_and_gets_up_after_its_land_motion() {
        let (p, c, s, w) = (params(), clips(), sstm(), room());
        let mut player = Player::new([0.; 3], 0.);
        // From behind: BLOW_BACK (motion 18), flying along the hit.
        let hit = claw([0., 1.], 4);
        assert_eq!(
            player.react(&hit, &s, &c),
            Some(Action::Hurt(hurt::BLOW[1]))
        );
        assert_eq!(player.motion_id(&c), 18);
        let mut peak = 0f32;
        let mut landed = None;
        for t in 1..400 {
            let step = player.tick(&Input::default(), &p, &c, &w);
            peak = peak.max(player.position[1]);
            if step.landed {
                landed = Some(t);
                break;
            }
        }
        let t = landed.expect("the blow lands");
        assert!(t > 20 && t < 120, "{t}");
        assert!(peak > 30., "{peak}");
        assert!(player.position[2] > 60., "{}", player.position[2]);
        assert_eq!(player.action, Action::Hurt(hurt::BLOW_LAND[1]));
        assert_eq!(player.motion_id(&c), 19);
        // Commands come back 40 ticks into the land motion.
        let at = player.position;
        for _ in 0..39 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert_eq!(player.position, at);
        assert_eq!(player.action, Action::Hurt(hurt::BLOW_LAND[1]));
        let go = Input {
            direction: [1., 0.],
            ..Input::default()
        };
        for _ in 0..3 {
            player.tick(&go, &p, &c, &w);
        }
        assert_ne!(player.action, Action::Hurt(hurt::BLOW_LAND[1]));
    }

    #[test]
    fn a_blow_uses_kb1_and_kb2_when_the_attack_has_them() {
        let (c, s) = (clips(), sstm());
        let mut a = Player::new([0.; 3], 0.);
        a.react(&claw([0., 1.], 4), &s, &c);
        let mut b = Player::new([0.; 3], 0.);
        b.react(
            &Hit {
                knockback: [60, 10],
                ..claw([0., 1.], 4)
            },
            &s,
            &c,
        );
        let (p, w) = (params(), room());
        let (mut a, mut b) = (a, b);
        for _ in 0..40 {
            a.tick(&Input::default(), &p, &c, &w);
            b.tick(&Input::default(), &p, &c, &w);
        }
        // KB1 60 rises higher than the default 24 and KB2 10 travels
        // slower than the default 40.
        assert!(b.position[2] < a.position[2]);
    }

    #[test]
    fn aerial_recovery_needs_the_ability_and_sixteen_ticks_of_blow() {
        let (p, c, s, w) = (params(), clips(), sstm(), room());
        let mut with = Player::new([0.; 3], 0.);
        with.abilities.set(abilities::bit::AERIAL_RECOVERY);
        with.react(&claw([0., 1.], 4), &s, &c);
        let mut without = Player::new([0.; 3], 0.);
        without.react(&claw([0., 1.], 4), &s, &c);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        // Too early: nothing.
        for _ in 0..10 {
            with.tick(&jump, &p, &c, &w);
        }
        assert_eq!(with.action, Action::Hurt(hurt::BLOW[1]));
        for _ in 0..10 {
            with.tick(&jump, &p, &c, &w);
            without.tick(&jump, &p, &c, &w);
        }
        assert_eq!(with.action, Action::Hurt(hurt::RECOVERY));
        assert_eq!(with.motion_id(&c), 40);
        assert_eq!(without.action, Action::Hurt(hurt::BLOW[1]));
        // Range 27 holds for 16 ticks of the recovery.
        assert!(with.range_invulnerable(&c));
        for _ in 0..20 {
            with.tick(&Input::default(), &p, &c, &w);
        }
        assert!(!with.range_invulnerable(&c));
    }

    /// A growth table whose one row is LV1's installed values.
    fn fmab() -> growth::Fmab {
        let mut t = vec![1u8, 0, 0, 0, 8, 0, 0, 0];
        for v in [
            235f32, 145., 18., 14., 30., 0.94, 0.93, 0., 16., 0.015, 0., 3., 0.91, 60., 10.,
            0.05236, 10.,
        ] {
            t.extend(v.to_le_bytes());
        }
        growth::Fmab::read(&t).unwrap()
    }

    fn growth_clips() -> Clips {
        let mut c = clips();
        let roll = Clip {
            end: 40,
            actionable: vec![(36, 40)],
            ..Clip::default()
        };
        c.growth = vec![
            (201, open(30), Vec::new()),
            (202, open(30), Vec::new()),
            (203, Clip::looping(40, 40), Vec::new()),
            (204, Clip::looping(30, 30), Vec::new()),
            (205, open(60), Vec::new()),
            (206, open(60), Vec::new()),
            (207, open(60), Vec::new()),
            (208, open(60), Vec::new()),
            (209, roll, Vec::new()),
        ];
        c
    }

    fn grown(family: usize) -> Player {
        let mut player = Player::new([0.; 3], 0.);
        player.fmab = fmab();
        player.abilities.growth[family] = 1;
        player
    }

    #[test]
    fn high_jump_uses_the_row_height_and_motion_201() {
        let (p, c, w) = (params(), growth_clips(), room());
        let mut player = grown(growth::HIGH_JUMP);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        assert_eq!(
            player.tick(&jump, &p, &c, &w).started,
            Some(Action::HighJump)
        );
        let mut apex: f32 = 0.;
        for _ in 0..200 {
            player.tick(&Input::default(), &p, &c, &w);
            apex = apex.max(player.position[1]);
        }
        assert!((apex - 235.).abs() < 0.5, "apex {apex}");
    }

    #[test]
    fn aerial_dodge_relaunches_once_per_airtime() {
        let (p, c, w) = (params(), growth_clips(), room());
        let mut player = grown(growth::AERIAL_DODGE);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        for _ in 0..20 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        let before = player.position[1];
        assert!(before > 50.);
        player.tick(&jump, &p, &c, &w);
        assert_eq!(player.action, Action::Growth(growth::motion::AERIAL_DODGE));
        let mut apex: f32 = 0.;
        for t in 0..30 {
            let input = if t == 10 { jump } else { Input::default() };
            player.tick(&input, &p, &c, &w);
            apex = apex.max(player.position[1]);
            if t == 10 {
                // A second press in the same airtime does nothing.
                assert_eq!(player.action, Action::Growth(growth::motion::AERIAL_DODGE));
                assert!(player.frame > 10);
            }
        }
        assert!(apex > before + 140., "{apex} from {before}");
    }

    #[test]
    fn glide_holds_above_the_floor_until_square_is_released() {
        let (p, c, w) = (params(), growth_clips(), room());
        let mut player = grown(growth::GLIDE);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        for _ in 0..30 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        let square = Input {
            guard: true,
            guard_held: true,
            ..Input::default()
        };
        player.tick(&square, &p, &c, &w);
        assert_eq!(player.action, Action::Growth(growth::motion::GLIDE_STAND));
        let held = Input {
            guard_held: true,
            direction: [0., 1.],
            ..Input::default()
        };
        let start = player.position;
        for _ in 0..40 {
            player.tick(&held, &p, &c, &w);
        }
        assert_eq!(player.action, Action::Growth(growth::motion::GLIDE));
        assert!(player.position[2] > start[2] + 100.);
        assert!(player.position[1] > 60.);
        player.tick(&Input::default(), &p, &c, &w);
        assert_eq!(player.action, Action::Fall);
    }

    #[test]
    fn dodge_roll_and_quick_run_start_from_square_on_the_ground() {
        let (p, c, w) = (params(), growth_clips(), room());
        let mut roll = grown(growth::DODGE_ROLL);
        let square = Input {
            guard: true,
            guard_held: true,
            direction: [1., 0.],
            ..Input::default()
        };
        roll.tick(&square, &p, &c, &w);
        assert_eq!(roll.action, Action::Growth(growth::motion::DODGE_ROLL));
        assert!(roll.range_invulnerable(&c));
        let mut run = grown(growth::QUICK_RUN);
        run.tick(&square, &p, &c, &w);
        assert_eq!(run.action, Action::Growth(growth::motion::QUICK_RUN));
        let held = Input {
            guard_held: true,
            ..Input::default()
        };
        for _ in 0..10 {
            run.tick(&held, &p, &c, &w);
        }
        assert!(run.position[0] > 200., "{:?}", run.position);
        run.tick(&Input::default(), &p, &c, &w);
        assert_eq!(run.action, Action::Growth(growth::motion::QUICK_RUN_END));
    }

    fn jump_peak(release_at: Option<u32>) -> f32 {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0.; 3], 0.);
        let press = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&press, &p, &c, &w);
        let mut peak: f32 = 0.;
        for t in 1..200 {
            let up = release_at.is_some_and(|r| t >= r);
            let input = Input {
                jump_up: up,
                ..Input::default()
            };
            player.tick(&input, &p, &c, &w);
            peak = peak.max(player.position[1]);
        }
        assert_eq!(player.position[1], 0.);
        peak
    }

    #[test]
    fn releasing_jump_cuts_the_rise_after_six_ticks_of_arc() {
        let full = jump_peak(None);
        assert!((full - params().jump_height).abs() < 1.5, "{full}");
        // Released at once: the cut waits for the arc's time to pass 6,
        // then a 5-tick rise of g × 12.5 replaces the rest.
        let tap = jump_peak(Some(1));
        assert!(tap < full * 0.75, "{tap} vs {full}");
        // Later releases go higher; past rise − 5 nothing is cut.
        let mid = jump_peak(Some(14));
        assert!(tap < mid && mid < full, "{tap} {mid} {full}");
        let late = jump_peak(Some(40));
        assert!((late - full).abs() < 1e-3, "{late} vs {full}");
    }

    #[test]
    fn cut_hop_is_five_ticks_of_rise() {
        assert!((actor_physics::rise_time(actor_physics::JUMP_CUT_HOP) - 5.).abs() < 1e-4);
    }

    #[test]
    fn upper_slash_takes_square_in_a_combo_window_after_a_hit() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0.; 3], 0.);
        player.abilities.set(18);
        let swing = Input {
            attack: true,
            ..Input::default()
        };
        player.tick(&swing, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(1));
        let square = Input {
            guard: true,
            ..Input::default()
        };
        // Outside the window, or before the swing connected: nothing.
        player.tick(&square, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(1));
        for _ in 2..12 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        player.tick(&square, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(1));
        player.connected = true;
        player.tick(&square, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(6));
        assert_eq!(player.motion_id(&c), 161);
        assert_eq!(player.combo, ptya::Combo { step: 2, finish: 1 });
    }

    #[test]
    fn combo_master_does_not_let_square_abilities_chain_a_miss() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0.; 3], 0.);
        player.abilities.set(18);
        player.abilities.set(abilities::bit::COMBO_MASTER);
        player.tick(
            &Input {
                attack: true,
                ..Input::default()
            },
            &p,
            &c,
            &w,
        );
        for _ in 1..12 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        let square = Input {
            guard: true,
            ..Input::default()
        };
        player.tick(&square, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(1));
        assert_eq!(player.combo, ptya::Combo { step: 1, finish: 1 });
        player.connected = true;
        player.tick(&square, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(6));
    }

    #[test]
    fn square_abilities_need_their_ability() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0.; 3], 0.);
        let swing = Input {
            attack: true,
            ..Input::default()
        };
        player.tick(&swing, &p, &c, &w);
        for _ in 1..12 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        player.connected = true;
        let square = Input {
            guard: true,
            ..Input::default()
        };
        player.tick(&square, &p, &c, &w);
        assert_eq!(player.action, Action::Attack(1));
    }

    #[test]
    fn retaliating_slash_starts_from_a_blow_with_its_ability() {
        let (p, c, s, w) = (params(), clips(), sstm(), room());
        let mut player = Player::new([0.; 3], 0.);
        player.abilities.set(101);
        player.react(&claw([0., 1.], 4), &s, &c);
        // The attack button does not start it; Square does.
        let attack = Input {
            attack: true,
            ..Input::default()
        };
        for _ in 0..17 {
            player.tick(&attack, &p, &c, &w);
        }
        assert!(matches!(player.action, Action::Hurt(_)));
        let swing = Input {
            guard: true,
            ..Input::default()
        };
        for _ in 0..3 {
            player.tick(&swing, &p, &c, &w);
        }
        assert_eq!(player.action, Action::AirAttack(5));
        assert_eq!(player.motion_id(&c), 172);
        assert_eq!(player.combo, ptya::Combo { step: 1, finish: 1 });
    }

    #[test]
    fn death_holds_the_dead_motion_and_falls_in_the_air() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 120., 0.], 0.);
        player.arc = actor_physics::Arc::fall(120., 0.);
        player.action = Action::Fall;
        assert_eq!(player.die(&c), Some(Action::Hurt(hurt::DEAD)));
        assert!(player.dead());
        let x = player.position[0];
        for _ in 0..200 {
            player.tick(
                &Input {
                    direction: [1., 0.],
                    attack: true,
                    jump: true,
                    ..Input::default()
                },
                &p,
                &c,
                &w,
            );
        }
        assert_eq!(player.position[1], 0.);
        assert_eq!(player.position[0], x);
        assert!(player.dead());
        // Hits and inputs do nothing; the revive stands him up.
        assert_eq!(player.react(&claw([0., -1.], 3), &sstm(), &c), None);
        player.revive();
        assert_eq!(player.action, Action::Idle);
    }

    #[test]
    fn using_an_item_fires_at_trigger_six_and_unlocks_at_range_two() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0.; 3], 0.);
        assert_eq!(player.use_item(&c), Some(Action::Item));
        assert_eq!(player.motion_id(&c), 9);
        // The combo counters are untouched.
        assert_eq!(player.combo, ptya::Combo::default());
        let mut fired = None;
        for t in 1..=40 {
            player.tick(&Input::default(), &p, &c, &w);
            if player.take_item() {
                fired = Some(t);
            }
            if t < 36 {
                assert_eq!(player.action, Action::Item, "locked at {t}");
            }
        }
        assert_eq!(fired, Some(18));
        // Not in the air, not while locked, not while dead.
        let mut air = Player::new([0., 50., 0.], 0.);
        air.arc = actor_physics::Arc::fall(50., 0.);
        air.action = Action::Fall;
        assert_eq!(air.use_item(&c), None);
        let mut busy = Player::new([0.; 3], 0.);
        busy.use_item(&c);
        assert_eq!(busy.use_item(&c), None);
    }

    fn fire(level: u8) -> magic::Record {
        magic::Record {
            magic_type: magic::FIRE,
            level,
            world: 0,
            file: String::new(),
            item: 0x15,
            command: 0x31,
            ground: (56, 8),
            finisher: (57, 8),
            air: (58, 8),
            voice: 7,
            voice_finisher: 11,
            voice_self: -1,
        }
    }

    #[test]
    fn a_cast_spends_mp_at_its_release_tick_and_counts_a_combo_hit() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        player.mp = magic::Mp::new(100);
        let (action, start) = player.cast(&fire(1), 12, 0, false, &c).unwrap();
        assert_eq!(action, Action::Cast(0));
        assert_eq!((start.motion, start.row, start.finisher), (56, 0, false));
        assert_eq!(player.combo, ptya::Combo { step: 1, finish: 1 });
        // No MP is spent before the release tick (10).
        for _ in 0..9 {
            player.tick(&Input::default(), &p, &c, &w);
            assert_eq!(player.mp.cur, 100);
            assert!(player.take_released().is_none());
        }
        player.tick(&Input::default(), &p, &c, &w);
        let r = player.take_released().unwrap();
        assert_eq!((r.magic_type, r.level, r.row, r.airborne), (0, 1, 0, false));
        assert_eq!(player.mp.cur, 88);
        // The motion ends in idle.
        for _ in 0..40 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert_eq!(player.action, Action::Idle);
        assert!(player.take_released().is_none());
    }

    #[test]
    fn a_form_finisher_releases_several_times_and_spends_mp_once() {
        let (p, w) = (params(), room());
        let mut c = clips();
        // A motion with three release frames (Final Form's Blizzard finisher
        // releases five times).
        c.spells[0].clip.release = vec![(4, None), (8, None), (12, None)];
        let mut player = Player::new([0., 0., 0.], 0.);
        player.mp = magic::Mp::new(100);
        player.cast(&fire(1), 12, 0, false, &c).unwrap();
        let mut volleys = Vec::new();
        for _ in 0..20 {
            player.tick(&Input::default(), &p, &c, &w);
            if let Some(r) = player.take_released() {
                volleys.push((r.volley, player.mp.cur));
            }
        }
        assert_eq!(volleys, vec![(0, 88), (1, 88), (2, 88)]);
    }

    #[test]
    fn spells_chain_without_contact_but_still_require_the_window_and_finisher_budget() {
        let mut c = clips();
        let mut player = Player::new([0.; 3], 0.);
        player.action = Action::Attack(1);
        player.combo = ptya::Combo { step: 1, finish: 1 };
        player.frame = 9;
        assert!(!player.connected);
        assert!(!player.abilities.has(abilities::bit::COMBO_MASTER));
        assert!(player.cast(&fire(1), 12, 0, false, &c).is_none());
        assert_eq!(player.combo.step, 1);
        player.frame = 12;
        assert_eq!(
            player.cast(&fire(1), 12, 0, false, &c).unwrap().0,
            Action::Cast(0)
        );
        assert_eq!(player.combo.step, 2);

        // A spell finisher's authored range 50 uses Finishing Plus's
        // budget even though command type 2 always supplies chain=1.
        c.spells[1].clip.finisher_windows = vec![(2, 8)];
        player.abilities = modifier_abilities(&[abilities::item::FINISHING_PLUS]);
        player.action = Action::Attack(1);
        player.frame = 12;
        assert_eq!(
            player.cast(&fire(1), 12, 0, false, &c).unwrap().0,
            Action::Cast(1)
        );
        assert_eq!(player.combo, ptya::Combo { step: 3, finish: 1 });
        player.frame = 2;
        assert_eq!(
            player.cast(&fire(1), 12, 0, false, &c).unwrap().0,
            Action::Cast(1)
        );
        assert_eq!(player.combo, ptya::Combo { step: 4, finish: 2 });
        player.frame = 2;
        assert!(player.cast(&fire(1), 12, 0, false, &c).is_none());
        assert_eq!(player.combo, ptya::Combo { step: 4, finish: 2 });
    }

    #[test]
    fn the_third_hit_of_a_combo_casts_the_finisher_row() {
        let (_, c, _) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        // Two attacks already counted (limit 3), a hit connected.
        player.combo = ptya::Combo { step: 2, finish: 1 };
        player.connected = true;
        player.action = Action::Attack(1);
        player.frame = 12;
        let (action, start) = player.cast(&fire(2), 12, 0, false, &c).unwrap();
        assert_eq!(action, Action::Cast(1));
        assert_eq!((start.motion, start.row, start.finisher), (57, 3, true));
        assert_eq!(player.combo.step, 3);
    }

    #[test]
    fn interrupted_casts_spend_nothing_and_cure_always_charges() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        player.cast(&fire(1), 12, 0, false, &c).unwrap();
        for _ in 0..5 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        player.hurt(0, [0., 1.], 125., &c);
        for _ in 0..20 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert_eq!(player.mp.cur, 100);
        assert!(player.take_released().is_none());
        // Cost 0xff always takes MP below 0: a full-bar cast starts a charge.
        let mut cure = Player::new([0., 0., 0.], 0.);
        cure.cast(&fire(1), 0xff, 0, true, &c).unwrap();
        for _ in 0..11 {
            cure.tick(&Input::default(), &p, &c, &w);
        }
        assert!(cure.take_released().is_some());
        assert!(cure.mp.charging());
        assert_eq!(cure.mp.cur, 0);
        // MP Haste runs the charge down faster.
        let before = cure.mp.remaining;
        cure.abilities.mp_haste = 1.5;
        cure.tick(&Input::default(), &p, &c, &w);
        assert_eq!(cure.mp.remaining, before - 1.5);
    }

    #[test]
    fn an_air_cast_hops_forty_and_lands() {
        let (p, c, w) = (params(), clips(), room());
        let mut player = Player::new([0., 0., 0.], 0.);
        let jump = Input {
            jump: true,
            ..Input::default()
        };
        player.tick(&jump, &p, &c, &w);
        for _ in 0..20 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        assert!(player.in_air());
        let (action, start) = player.cast(&fire(1), 12, 0, false, &c).unwrap();
        assert_eq!(action, Action::AirCast(2));
        assert!(start.airborne);
        assert_eq!(start.row, 0);
        for _ in 0..13 {
            player.tick(&Input::default(), &p, &c, &w);
        }
        let r = player.take_released().unwrap();
        assert!(r.airborne);
        assert_eq!(player.mp.cur, 88);
    }
}
