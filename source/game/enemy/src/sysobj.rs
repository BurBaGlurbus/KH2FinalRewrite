//! The engine object ("sysobj") an enemy script drives, and the per-tick engine
//! work around the script: motion clock, root motion, jump/blow/knock physics
//! and movement through the collision world. The motion and physics rules are
//! those the earlier Shadow/Dusk/Soldier translations established from the
//! executable (FUN_1403c6740 motion advance, FUN_1403b89a0 physics sampling,
//! FUN_1403cbb00 jumps, FUN_1403cb360/cb500 arcs, FUN_1403d4570 knock-slide);
//! offsets in comments are the executable's.

use crate::env::{Clips, Env};
use crate::motion::{self, Effect, Flags};
use player::{actor_physics, combat};

pub type Vec3 = [f32; 3];

pub fn sub(a: Vec3, b: Vec3) -> Vec3 {
    [0, 1, 2].map(|k| a[k] - b[k])
}
pub fn hlen(v: Vec3) -> f32 {
    v[0].hypot(v[2])
}

pub fn wrap(a: f32) -> f32 {
    crate::traps::math::wrap_angle(a)
}

/// One HP pool of the battle sheet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pool {
    pub hp: i32,
    pub max: i32,
    pub min: i32,
}

/// +0x158: the motion player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Motion {
    /// +0x28/+0x180: the playing motion slot.
    pub slot: usize,
    /// Clock in ticks.
    pub tick: u32,
    /// Increments on every motion start (attack-instance identity).
    pub serial: u64,
    /// Remaining blend, in motion-clock ticks (0 = none).
    pub blend: f32,
    blend_elapsed: u32,
    blend_duration: u32,
    /// +0x34.. : the event flags (see [`crate::motion`]).
    pub flags: Flags,
    /// Root displacement of this tick's clock step, in model space.
    pub root_step: Vec3,
}

/// Reaction states the engine holds between a hit and returning to the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reaction {
    None,
    Flinch,
    Blow { back: bool },
    BlowLand,
}

/// A request to leave the ground once a motion's take-off event fires
/// (`trap_obj_flyjump`, physics mode 11).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingFly {
    pub direction: Vec3,
    pub height: f32,
    pub speed: f32,
}

/// Physics mode 5 (`trap_obj_fly`): a free velocity that follows the object's
/// move request. The routine is FUN_140421c80 with FUN_1403b7ad0 as its
/// steering step. Mode 6 (`trap_obj_hover`) runs the very same routine
/// (FUN_140422900 hands over to mode 5's update) but its mode flags are 0x100,
/// so it counts neither as a jump nor as a fly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steer {
    /// +0x108: the share of the velocity kept each step.
    pub inertia: f32,
    /// Mode 6 instead of mode 5.
    pub hover: bool,
}

/// +0x118 / +0x11c: the motions a jump plays near its apex and on landing
/// (-1: none). Arming any jump sets (-1, 5): no fall motion, motion 5 on
/// landing (FUN_1403cbb00, FUN_1403cbd00).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JumpMotions {
    pub fall: i32,
    pub land: i32,
}

impl Default for JumpMotions {
    fn default() -> Self {
        Self { fall: -1, land: 5 }
    }
}

/// Physics mode 9 (`trap_obj_hop`): a ballistic hop whose vertical speed is
/// integrated every step (FUN_1403cbdd0) and whose horizontal velocity follows
/// the object's move request with one inertia (+0x108) for both the pull and
/// the decay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hop {
    /// Horizontal velocity (x, z), Y-up frame.
    pub velocity: [f32; 2],
    /// +0x24: vertical speed, Y-down (negative rises).
    pub vy_down: f32,
    /// +0x10c: acceleration downwards per tick squared.
    pub gravity: f32,
    /// +0x108.
    pub inertia: f32,
}

/// What `obj_hop` armed (FUN_1403cbec0): the hop begins when the playing
/// motion's take-off event (frame event 0) fires, like a jump.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PendingHop {
    pub hop: Hop,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fly {
    pub arc: actor_physics::Arc,
    pub velocity: [f32; 2],
    pub initial_height: f32,
    pub original_airborne: bool,
    pub fall_started: bool,
}

/// An alpha fade (sysobj+0xa08: value, rate per tick; FUN_1403b7810/77c0).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Fade {
    pub value: f32,
    pub rate: f32,
}

impl Fade {
    /// FUN_1403b7810: from `from` to `to` over `ticks` (<= 2 means one tick).
    pub fn start(&mut self, from: f32, to: f32, mut ticks: f32) {
        if ticks <= 2.0 {
            ticks = 1.0;
        }
        if ticks > 0.0 {
            self.value = from;
            self.rate = (to - from) / ticks;
        } else {
            self.value = to;
            self.rate = 0.0;
        }
    }

    /// FUN_1403b77c0: true while the fade runs.
    pub fn step(&mut self, frame_time: f32) -> bool {
        if self.rate == 0.0 {
            return false;
        }
        self.value += self.rate * frame_time;
        if self.value > 0.0 {
            if 1.0 <= self.value {
                self.value = 1.0;
                self.rate = 0.0;
            }
            return true;
        }
        self.value = 0.0;
        self.rate = 0.0;
        true
    }
}

/// The record at sysobj+0xbf8: the actor this one is aiming at.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Target {
    /// 0 = nobody.
    pub id: u32,
    /// Search kind it was made with (+0xc), -1 when unset.
    pub kind: i32,
    /// The record's group word (+4; `target_set_group` / `target_get_group`,
    /// FUN_1403bee60 writes it).
    pub group: i32,
}

#[derive(Clone, Debug)]
pub struct Sysobj {
    /// The model the host selects clips with.
    pub model: u32,
    /// +0x670: position.
    pub position: Vec3,
    pub spawn: Vec3,
    /// +0x68c: heading about Y.
    pub yaw: f32,
    /// +0x7a8: the heading `wish_dir` asks the engine to turn toward.
    pub wish_yaw: Option<f32>,
    /// +0x30: scale (y animated by scripts).
    pub scale: Vec3,
    /// Alpha as faded by +0xa08.
    pub alpha: f32,
    pub fade: Fade,
    pub hidden: bool,
    /// +0x9b8 bit 4: leaving; the object is removed once faded.
    pub left: bool,
    pub gone: bool,
    /// +0x120: script-settable flags (`obj_set_flag`).
    pub flags: u32,
    pub hp: i32,
    pub stats: combat::TargetStats,
    pub sstm: actor_physics::Sstm,
    pub motion: Motion,
    pub airborne: bool,
    pub reaction: Reaction,
    /// +0x10/+0x1c: the movement the script requested (direction, speed).
    pub move_direction: Vec3,
    pub move_speed: f32,
    /// +0xa28 / +0xa38: wished and actual displacement of the last tick.
    pub wished: Vec3,
    pub moved: Vec3,
    pub target: Target,
    /// +0x9f4: spawn arguments, and the spawn type byte at +0x1c.
    pub spawn_args: [i32; 8],
    pub appear_way: i32,
    /// Spawned flying (`obj_is_entry_fly`).
    pub entry_fly: bool,
    /// The attacker of the last hit (the damage record's source).
    pub last_attacker: Option<u32>,
    /// The reaction value of the damage record being handled (+0x26).
    pub damage_reaction: u32,
    /// +0xa58/+0xa64: the second movement request (`trap_obj_move_request`).
    pub request_direction: Vec3,
    pub request_speed: f32,
    /// `trap_sysobj_motion_push`: motions queued behind the playing one.
    pub motion_queue: Vec<(usize, f32)>,
    /// +0xd10: enabled attack/behaviour patterns.
    pub pattern_mask: u32,
    /// Pool 0's HP floor (sheet +8) and the pools above it (host-provided).
    pub min_hp: i32,
    pub extra_pools: Vec<Pool>,
    /// +0x1c4 / +0x1c8: prize range and hit-back addition set by scripts.
    pub prize_range: i32,
    pub hitback_addition: i32,
    /// The battle parameter sheet (ENMP) number the script bound.
    pub sheet: i32,
    /// +0x8c8 bit 6: sunk and slithering (Shadow).
    pub shadow_move: bool,
    /// +0x134: turn rate parameter (radians per tick; scripts pass 0 for it).
    pub turn_rate: f32,
    /// `obj_set_movement`'s `+0x118/+0x11c`: the fall and land motions of the
    /// current jump.
    pub jump_motions: JumpMotions,
    /// +0x648: the physics mode the next take-off event (frame event 0) enters
    /// (1 jump, 3 fly).
    pub pending_mode: u8,
    /// The jump a pending mode 1 launches: direction, height, speed.
    pub(crate) pending_jump: Option<(Vec3, f32, f32)>,
    /// Effects the motion events asked for this tick, for the object to carry out.
    pub effects: Vec<Effect>,
    /// Range event 0x18 (+0x108) and 0x19/0x1a (+0x638) values, and the
    /// pattern words range events 0x24/0x25 set (+0xd14 / +0xd18).
    pub event_speed: f32,
    pub event_parameter: u32,
    pub pattern_a: u32,
    pub pattern_b: u32,
    /// +0x4dc: the team (`obj_set_team`).
    pub team: i32,
    /// +0xd4c: the karma limit (`enemy_set_karma_limit`).
    pub karma_limit: i32,
    /// +0x5a8: the collision shape masks `obj_disable_collision` and
    /// `obj_reset_collision` edit (word `n` of shapes 32n..).
    pub collision: Vec<u32>,
    /// The object entry's id and part id (`obj_get_entry_id`, `obj_part`).
    pub entry_id: u32,
    pub entry_part: u16,
    /// The spawn record's serial (the ushort at +0x1e of the record +0x9f0
    /// points to; `trap_obj_search_by_serial` matches it). 0 when the object
    /// has no spawn record.
    pub spawn_serial: u16,
    /// +0xccc: routine option flags (`rtn_option_flag`), and +0xcd8..+0xcfc:
    /// the routine time parameters (`set_rtn_time_param`; word 7 is `touch_zone`).
    pub rtn_flags: Vec<u32>,
    pub rtn_time: [u32; 9],
    /// +0x20: the steering velocity (four lanes; mode 5 moves by it).
    pub velocity: [f32; 4],
    /// The entry's unit group (`obj_unit_group`) and the damage record's
    /// "reaction executed" flag (+0x18 bit 0).
    pub entry_group: u16,
    pub damage_exec: bool,
    /// The element of the attack being handled (`damage_element`).
    pub damage_element: u8,
    /// +0x2a8: the motion playback speed scripts set (`obj_motion_speed`).
    pub motion_speed: f32,
    /// The attack parameter id of the hit being handled (the damage record's
    /// attack, `damage_attack_param_id`).
    pub damage_param_id: u32,
    /// +0x3c: the fourth lane of the scale vector.
    pub scale_w: f32,
    pub(crate) pending_fly: Option<PendingFly>,
    pub(crate) fly: Option<Fly>,
    /// Physics mode 5 state, and +0x124 bit 27 (the last step moved).
    pub(crate) steer: Option<Steer>,
    pub steer_moving: bool,
    /// A plain jump (FUN_1403cbb00): the arc and its horizontal velocity.
    pub(crate) jump: Option<(actor_physics::Arc, [f32; 2])>,
    /// Physics mode 9 (hop), and the hop armed for the next take-off.
    pub(crate) hop: Option<Hop>,
    pub(crate) pending_hop: Option<PendingHop>,
    pub(crate) blow: Option<actor_physics::Blow>,
    pub(crate) knock: Option<actor_physics::Knock>,
    pub(crate) sampler: actor_physics::Sampler,
    pub record: combat::HitRecord,
    /// +0x18c bit 18: the act manager is paused (set by engine states).
    pub act_paused: bool,
    /// Words of the engine object a script wrote through its handle at
    /// offsets nothing else models (the spec block +0x128.., for one). They
    /// read back as written; an engine reader maps its own offset to a field
    /// above when it exists.
    pub raw_words: std::collections::BTreeMap<u32, u32>,
    /// +0x124: state bits (see [`state_flag`]) besides the steering bit.
    pub state_flags: u32,
    /// +0x6c8: the object class bits (see [`class_flag`]); the host sets them
    /// for objects it knows are Sora, summons and so on.
    pub class_flags: u32,
    /// +0x8c8 bit 5: the scissoring draw state (`obj_set_scissoring`).
    pub scissoring: bool,
    /// +0x7e9..+0x7eb: the stealth colour (`obj_set_stealth_color`).
    pub stealth_color: [u8; 3],
    /// +0xd1c: the medal count (`obj_set_medal`, `obj_get_medal`).
    pub medal: i32,
    /// +0xd24..+0xd33: the knock-back request (`obj_hitback` writes the
    /// direction and strength, `obj_clear_hitback` zeroes it).
    pub hitback: [f32; 4],
    /// +0x9d8: the object that rides this one (0 = none).
    pub rider: u32,
    /// +0xac0 == 1 with the entry's pirate bit: a pirate shade
    /// (`obj_is_pirate_shade`, FUN_1403baa90).
    pub pirate_entry: bool,
    pub shade_state: u8,
    /// Stop requests (+0x64c, `enemy_stop_all_start`): a stopped object does
    /// not advance.
    pub stop_requests: Vec<u32>,
    /// The attacker kept for friendly fire (+0xd40, `enemy_set_attacker`).
    pub friend_attacker: Option<u32>,
    /// The attack type byte (ATKP +4) and the hit part (record +0x25) of the
    /// hit being handled.
    pub damage_attack_type: u8,
    pub damage_parts: u8,
    /// +0x6a0..: carried by another object (`obj_attach`, physics mode 3).
    pub attached: Option<Attach>,
    /// FUN_1401a7cf0 / FUN_1401a8a90: the model is hidden (`obj_hide_part`
    /// and `obj_show_part` take a part but the routines ignore it).
    pub model_hidden: bool,
    /// FUN_1401a87a0 / FUN_1401a8570: the model's texture animation plays.
    pub texture_animation: bool,
    /// +0xaa4: the texture fade (`obj_tex_fade_start`).
    pub tex_fade: Fade,
}

/// An object carried by another (FUN_1403d9fe0): it follows the holder's
/// bone `bone` instead of moving on its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Attach {
    /// The holder.
    pub to: crate::world::ObjId,
    /// +0x6a4 and +0x6a6.
    pub bone: i32,
    pub flags: u16,
    /// The script entry and AI object the attach was given (`obj_attach`
    /// arguments 4 and 5).
    pub callback: Option<(i32, u32)>,
}

/// Bits of [`Sysobj::state_flags`] (+0x124).
pub mod state_flag {
    /// The jump mode's landing is armed (`FUN_1403cbd00` clears it).
    pub const GROUND_ARMED: u32 = 1;
    /// `obj_hide_shadow`.
    pub const HIDE_SHADOW: u32 = 1 << 9;
    /// The object `obj_search_by_glance` last picked, and one it passed over
    /// after it had been picked.
    pub const GLANCED: u32 = 1 << 12;
    pub const GLANCE_LOST: u32 = 1 << 13;
    /// `obj_is_limit` reads +0x126 bit 0.
    pub const LIMIT: u32 = 1 << 16;
}

/// Bits of [`Sysobj::class_flags`] (+0x6c8).
pub mod class_flag {
    /// The enemy class: the objects `enemy_each` and `enemy_stop_all_start`
    /// walk (+0x6c8 bit 4).
    pub const ENEMY: u32 = 1 << 4;
    /// `sysobj_is_zako` (+0x6c8 bit 11): an ordinary enemy rather than a
    /// boss; the host sets it from the object entry.
    pub const ZAKO: u32 = 1 << 11;
    /// `sysobj_is_summon`.
    pub const SUMMON: u32 = 1 << 22;
    /// `sysobj_is_sora` reads +0x6cb bit 0.
    pub const SORA: u32 = 1 << 24;
}

impl Sysobj {
    /// A zeroed object for tools and tests.
    pub fn bare() -> Self {
        let stats = combat::TargetStats {
            max_hp: 100,
            defense: 0,
            cap: 100,
            min: 0,
            weakness: [100; 7],
        };
        let sstm = actor_physics::Sstm {
            fall_max: 20.,
            blow_brake_xz: 0.,
            blow_min_xz: 0.,
            blow_brake_up: 0.,
            blow_up: 0.,
            blow_speed: 0.,
            blow_to_hit_back: 0.,
            hit_back: 0.,
            hit_back_small: 0.,
            hit_back_to_jump: 0.,
        };
        Self::new(0, [0.; 3], 0., stats, sstm)
    }

    pub fn new(
        model: u32,
        position: Vec3,
        yaw: f32,
        stats: combat::TargetStats,
        sstm: actor_physics::Sstm,
    ) -> Self {
        Self {
            model,
            position,
            spawn: position,
            yaw,
            wish_yaw: None,
            scale: [1.; 3],
            alpha: 1.,
            fade: Fade::default(),
            hidden: false,
            left: false,
            gone: false,
            flags: 0,
            hp: stats.max_hp,
            stats,
            sstm,
            motion: Motion::default(),
            airborne: false,
            reaction: Reaction::None,
            move_direction: [0.; 3],
            move_speed: 0.,
            wished: [0.; 3],
            moved: [0.; 3],
            target: Target::default(),
            spawn_args: [0; 8],
            appear_way: -1,
            entry_fly: false,
            last_attacker: None,
            damage_reaction: 0,
            request_direction: [0.; 3],
            request_speed: 0.,
            motion_queue: Vec::new(),
            pattern_mask: 0,
            min_hp: 0,
            extra_pools: Vec::new(),
            prize_range: 0,
            hitback_addition: 0,
            sheet: 0,
            shadow_move: false,
            turn_rate: 0.,
            jump_motions: JumpMotions::default(),
            pending_mode: 0,
            pending_jump: None,
            effects: Vec::new(),
            event_speed: 0.,
            event_parameter: 0,
            pattern_a: 0,
            pattern_b: 0,
            team: 0,
            karma_limit: 0,
            collision: Vec::new(),
            entry_id: 0,
            entry_part: 0,
            spawn_serial: 0,
            rtn_flags: Vec::new(),
            rtn_time: [0; 9],
            scale_w: 1.,
            velocity: [0.; 4],
            entry_group: 0,
            damage_exec: false,
            damage_element: 0,
            motion_speed: 1.,
            damage_param_id: 0,
            pending_fly: None,
            fly: None,
            steer: None,
            steer_moving: false,
            jump: None,
            hop: None,
            pending_hop: None,
            blow: None,
            knock: None,
            sampler: actor_physics::Sampler::default(),
            record: combat::HitRecord::new(false),
            act_paused: false,
            raw_words: std::collections::BTreeMap::new(),
            state_flags: 0,
            class_flags: class_flag::ENEMY,
            scissoring: false,
            stealth_color: [0; 3],
            medal: 0,
            hitback: [0.; 4],
            rider: 0,
            pirate_entry: false,
            shade_state: 0,
            stop_requests: Vec::new(),
            friend_attacker: None,
            damage_attack_type: 0,
            damage_parts: 0,
            attached: None,
            model_hidden: false,
            texture_animation: false,
            tex_fade: Fade::default(),
        }
    }

    /// The flag word of the current physics mode (+0xc of the mode object the
    /// executable keeps in the table at DAT_140750d30): bit 0 air, bit 1 fly,
    /// bit 6 blow, bit 7 ground, bit 8 always. The modes this crate runs are
    /// listed; the others are not entered here.
    pub fn mode_flags(&self) -> u32 {
        if self.blow.is_some() {
            0x1c1
        } else if self.jump.is_some() {
            0x181
        } else if self.hop.is_some() {
            0x101
        } else if self.fly.is_some() {
            0x103
        } else if let Some(steer) = &self.steer {
            if steer.hover { 0x100 } else { 0x102 }
        } else {
            0x180
        }
    }

    /// FUN_1403c86a0 / FUN_1403c88c0: begin a motion. A blend of at most two
    /// ticks takes the immediate path.
    pub fn motion_start(&mut self, slot: usize, blend: f32, clips: &dyn Clips) -> bool {
        if clips.length(slot) == 0 {
            return false;
        }
        let m = &mut self.motion;
        m.slot = slot;
        m.tick = 0;
        m.serial = m.serial.wrapping_add(1);
        m.blend = if blend <= 2. { 0. } else { blend };
        m.blend_duration = m.blend as u32;
        m.blend_elapsed = 0;
        m.root_step = [0.; 3];
        // FUN_1403c8a40 rewrites the flag word when a motion starts, so the end
        // pulse of the previous motion does not carry over to the new one.
        m.flags.word &= !motion::flag::ENDED;
        m.flags.pulses = 0;
        true
    }

    /// FUN_1403c6dc0: like start, but nothing happens if `slot` already plays.
    pub fn motion_change(&mut self, slot: usize, blend: f32, clips: &dyn Clips) -> bool {
        if self.motion.slot == slot {
            return true;
        }
        self.motion_start(slot, blend, clips)
    }

    /// +0x18c bit 1 (the motion ended on this tick's step).
    pub fn motion_ended(&self) -> bool {
        self.motion.flags.word & motion::flag::ENDED != 0
    }

    /// Hit-stun suspension (FUN_1403ca7c0): a damage (10..=39) or reflect
    /// (140..=150) motion still plays.
    pub fn hit_stunned(&self) -> bool {
        let s = self.motion.slot;
        (10..=39).contains(&s) || (140..=150).contains(&s)
    }

    /// FUN_1403b9f70 (`obj_turn_dir`): turn toward the direction's heading by
    /// at most `2 * rate` radians. Returns true while the heading differs.
    pub fn turn_dir(&mut self, direction: Vec3, rate: f32) -> bool {
        let rate = if rate == 0. { self.turn_rate } else { rate };
        let step = rate * actor_physics::PHYSICS_STEP;
        let delta = wrap(direction[0].atan2(direction[2]) - self.yaw);
        if delta > step {
            self.yaw = wrap(step + self.yaw);
            true
        } else if -step <= delta {
            self.yaw = wrap(self.yaw);
            false
        } else {
            self.yaw = wrap(self.yaw - step);
            true
        }
    }

    pub fn forward(&self) -> Vec3 {
        [self.yaw.sin(), 0., self.yaw.cos()]
    }

    /// Applies a requested displacement through the collision world.
    fn apply_movement(&mut self, wished: Vec3, env: &dyn Env) {
        let from = self.position;
        self.position = env.movement(from, wished);
        self.wished = wished;
        self.moved = sub(self.position, from);
    }

    /// The start of a frame (FUN_1403c6740): advance the motion clock, fire the
    /// frame events it passed, reset and re-evaluate the range flags. Runs
    /// before the script.
    pub fn begin_tick(&mut self, env: &dyn Env) {
        if self.gone {
            return;
        }
        let clips = env.clips(self.model);
        let m = &mut self.motion;
        m.flags.pulses = 0;
        m.root_step = [0.; 3];
        if m.flags.word & motion::flag::PAUSED != 0 {
            return;
        }
        m.flags.word &= !0x102;
        if clips.length(m.slot) == 0 {
            return;
        }
        let old = m.tick;
        let mut stepped = false;
        if m.blend > 0. {
            m.blend_elapsed = if m.blend_elapsed == 0 {
                2
            } else {
                m.blend_elapsed.saturating_add(1)
            };
            if m.blend_elapsed >= m.blend_duration {
                m.tick = m.blend_elapsed - m.blend_duration;
                m.blend = 0.;
                m.blend_elapsed = 0;
                stepped = true;
            } else {
                m.blend = (m.blend_duration - m.blend_elapsed) as f32;
            }
        } else {
            m.tick = clips.advance(m.slot, m.tick);
            m.root_step = clips.root_delta(m.slot, old);
            stepped = true;
        }
        if stepped {
            if clips.ended(m.slot, m.tick) {
                m.flags.word |= motion::flag::ENDED;
            }
            self.dispatch_frame_events(clips);
        }
        self.dispatch_ranges(clips);
    }

    /// Frame events whose frame the clock just passed.
    fn dispatch_frame_events(&mut self, clips: &dyn Clips) {
        let slot = self.motion.slot;
        let (t, duration, loop_top) =
            (self.motion.tick, clips.length(slot), clips.loop_start(slot));
        let Some(events) = clips.events(slot) else {
            return;
        };
        for e in &events.frames {
            if motion::frame_event_fires(e.frame, t, duration, loop_top) {
                self.motion.flags.frame_event(e, slot, &mut self.effects);
            }
        }
    }

    /// FUN_14040e5c0 / FUN_14040e740: reset, then the ranges the clock is in.
    fn dispatch_ranges(&mut self, clips: &dyn Clips) {
        let slot = self.motion.slot;
        self.motion.flags.reset();
        let clock =
            motion::native_clock(self.motion.tick, clips.length(slot), clips.loop_start(slot));
        let Some(events) = clips.events(slot) else {
            return;
        };
        for e in &events.ranges {
            if u32::from(e.start) <= clock && (e.end == u16::MAX || clock < u32::from(e.end)) {
                self.motion.flags.range_event(e, slot, &mut self.effects);
            }
        }
    }

    /// Take-off (frame event 0): enter the physics mode a script asked for.
    pub fn take_off(&mut self, env: &dyn Env) {
        match self.pending_mode {
            1 => {
                if let Some((direction, height, speed)) = self.pending_jump.take() {
                    self.start_jump(direction, height, speed);
                }
            }
            2 => {
                if let Some(PendingHop { hop }) = self.pending_hop.take() {
                    self.enter_hop(hop);
                }
            }
            3 => {
                if let Some(launch) = self.pending_fly.take() {
                    let floor = env.floor(self.position[0], self.position[2], self.position[1]);
                    self.steer = None;
                    self.fly = Some(Fly {
                        arc: actor_physics::Arc::launch(self.position[1], launch.height),
                        velocity: [
                            launch.direction[0] * launch.speed,
                            launch.direction[2] * launch.speed,
                        ],
                        initial_height: floor.map_or(0., |f| (self.position[1] - f).max(0.)),
                        original_airborne: self.airborne,
                        fall_started: false,
                    });
                    self.airborne = true;
                    self.sampler.reset();
                }
            }
            _ => {}
        }
        self.pending_mode = 0;
    }

    /// The rest of a frame after the script: heading, root motion, physics.
    pub fn end_tick(&mut self, env: &dyn Env) {
        if self.gone {
            return;
        }
        let clips = env.clips(self.model);
        let no_control = self.motion.flags.word & motion::flag::NO_CONTROL != 0;
        let reacting = self.reaction != Reaction::None && self.hp > 0;
        if reacting {
            self.update_reaction(clips, env);
        }
        if !reacting
            && !no_control
            && let Some(desired) = self.wish_yaw
        {
            let delta = wrap(desired - self.yaw);
            let step = self.turn_rate.max(0.) * 2.;
            self.yaw = wrap(self.yaw + delta.clamp(-step, step));
        }
        // The texture fade runs with the frame.
        self.tex_fade.step(1.0);
        // A carried object does not move by itself (physics mode 3); the
        // world places it on its holder.
        if self.attached.is_some() {
            return;
        }
        if self.fly.is_some() {
            self.update_fly(clips, env);
        } else if let Some(steer) = self.steer {
            self.update_steer(steer, env);
        } else if let Some(hop) = self.hop {
            self.update_hop(hop, clips, env);
        } else if let Some((arc, velocity)) = self.jump {
            self.update_jump(arc, velocity, clips, env);
        } else if !reacting {
            let root = self.motion.root_step;
            let (sin, cos) = self.yaw.sin_cos();
            let wished = if self.move_speed != 0. {
                self.move_direction.map(|v| v * self.move_speed)
            } else {
                [
                    root[0] * cos + root[2] * sin,
                    root[1],
                    -root[0] * sin + root[2] * cos,
                ]
            };
            self.apply_movement(wished, env);
        }
    }

    /// FUN_140421c80: one step of physics mode 5. Like every mode routine it
    /// runs every `PHYSICS_STEP` frames (FUN_1403b89a0) and its displacement is
    /// spread over them.
    fn update_steer(&mut self, steer: Steer, env: &dyn Env) {
        if self.sampler.due(self.move_speed) {
            let dt = actor_physics::PHYSICS_STEP;
            // FUN_1403b7ad0: toward the request keeping `inertia` of the
            // velocity, or the velocity scaled by `inertia` without one.
            let v = self.velocity;
            let v3 = if self.move_speed > 0. {
                let target = self.move_direction.map(|c| c * self.move_speed);
                [0, 1, 2].map(|k| target[k] * (1. - steer.inertia) + v[k] * steer.inertia)
            } else {
                [v[0], v[1], v[2]].map(|c| c * steer.inertia)
            };
            self.velocity = [v3[0], v3[1], v3[2], v[3]];
            let step = v3.map(|c| c * dt);
            self.steer_moving = step[0] * step[0] + step[1] * step[1] + step[2] * step[2] > 1.0e-4;
            self.sampler.store(step);
        }
        self.steer = Some(steer);
        let d = self.sampler.displacement();
        self.apply_movement(d, env);
    }

    /// `trap_obj_fly` (FUN_140421ce0): enter physics mode 5 with a velocity of
    /// `direction` (normalised) × `speed`.
    pub fn start_steer(&mut self, direction: Vec3, speed: f32, inertia: f32) {
        let len = (direction[0] * direction[0]
            + direction[1] * direction[1]
            + direction[2] * direction[2])
            .sqrt();
        let v = if len != 0. {
            direction.map(|c| c / len * speed)
        } else {
            direction.map(|c| c * speed)
        };
        self.velocity = [v[0], v[1], v[2], speed * speed];
        self.leave_air_modes();
        self.steer = Some(Steer {
            inertia,
            hover: false,
        });
        self.sampler.reset();
        self.airborne = true;
    }

    /// `trap_obj_hover` (FUN_140422950): physics mode 6, the steering mode
    /// with the velocity `normalize(v.xyz) * v.w`. Unlike `start_steer` it
    /// does not mark the object airborne (the mode flags are 0x100).
    pub fn start_hover(&mut self, v: [f32; 4], inertia: f32) {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let unit = if len != 0. {
            [v[0] / len, v[1] / len, v[2] / len]
        } else {
            [v[0], v[1], v[2]]
        };
        self.velocity = [unit[0] * v[3], unit[1] * v[3], unit[2] * v[3], v[3] * v[3]];
        self.leave_air_modes();
        self.steer = Some(Steer {
            inertia,
            hover: true,
        });
        self.sampler.reset();
    }

    /// Entering a physics mode leaves the others (FUN_1403b4da0).
    fn leave_air_modes(&mut self) {
        self.jump = None;
        self.hop = None;
        self.fly = None;
        self.pending_fly = None;
        self.pending_hop = None;
        self.pending_mode = 0;
    }

    /// FUN_1403cbec0 (`trap_obj_hop`): arm a hop that starts at the next
    /// take-off event. `v` is the script's vector: heading x/z, the apex
    /// height in y (script frame: positive rises here) and the horizontal
    /// speed in w; `time` is how long the rise takes.
    pub fn arm_hop(&mut self, v: [f32; 4], time: f32, inertia: f32) {
        let (x, z) = (v[0], v[2]);
        let len = (x * x + z * z).sqrt();
        let dir = if len != 0. {
            [x / len, z / len]
        } else {
            [x, z]
        };
        // FUN_1403cbec0: g = 2h / t^2, rising speed g * t (Y-down: negative).
        let gravity = (v[1] + v[1]) / (time * time);
        self.pending_mode = 2;
        self.pending_hop = Some(PendingHop {
            hop: Hop {
                velocity: [dir[0] * v[3], dir[1] * v[3]],
                vy_down: -(gravity * time),
                gravity,
                inertia,
            },
        });
        self.pending_jump = None;
        self.jump_motions = JumpMotions::default();
        self.state_flags &= !state_flag::GROUND_ARMED;
    }

    /// Enter the hop mode now (mode 9; FUN_1403cbd30 on entry).
    pub fn enter_hop(&mut self, hop: Hop) {
        self.leave_air_modes();
        self.steer = None;
        self.hop = Some(hop);
        self.state_flags |= state_flag::GROUND_ARMED;
        self.sampler.reset();
        self.airborne = true;
    }

    /// FUN_1403cbdd0, once per physics step: horizontal velocity by the
    /// steering step (the object's move request, one inertia for pull and
    /// decay), vertical speed by the hop's gravity capped at the spec's
    /// maximum fall speed (+0x138). The fall motion plays five ticks before
    /// the apex; landing plays the land motion.
    fn update_hop(&mut self, mut hop: Hop, clips: &dyn Clips, env: &dyn Env) {
        if self.sampler.due(self.move_speed) {
            let dt = actor_physics::PHYSICS_STEP;
            hop.velocity = actor_physics::steer(
                hop.velocity,
                [self.move_direction[0], self.move_direction[2]],
                self.move_speed,
                hop.inertia,
                hop.inertia,
            );
            // Y-down: positive falls. The cap is a spec word scripts write;
            // an unwritten one leaves the speed uncapped.
            let cap = self
                .raw_words
                .get(&0x138)
                .map_or(f32::INFINITY, |w| f32::from_bits(*w));
            hop.vy_down = (dt * hop.gravity + hop.vy_down).min(cap);
            // The vertical lane of the step is vy * dt (Y-up: negated).
            self.sampler.store([
                hop.velocity[0] * dt,
                -hop.vy_down * dt,
                hop.velocity[1] * dt,
            ]);
        }
        self.hop = Some(hop);
        let d = self.sampler.displacement();
        self.apply_movement(d, env);
        // FUN_1403cbe70 (mode slot 7): the fall motion, once, near the apex.
        if self.state_flags & state_flag::GROUND_ARMED != 0
            && self.jump_motions.fall >= 0
            && hop.vy_down + hop.gravity * actor_physics::FALL_MOTION_LEAD >= 0.
        {
            self.state_flags &= !state_flag::GROUND_ARMED;
            self.motion_change(
                self.jump_motions.fall as usize,
                actor_physics::MOTION_BLEND,
                clips,
            );
        }
        if let Some(floor) = env.floor(self.position[0], self.position[2], self.position[1] + 30.)
            && self.position[1] <= floor
            && d[1] <= 0.
        {
            self.position[1] = floor;
            self.hop = None;
            self.land(clips);
        }
    }

    /// FUN_1403cb940 (mode slot 6): touching down plays the land motion and
    /// returns to the ground mode.
    fn land(&mut self, clips: &dyn Clips) {
        self.airborne = false;
        self.sampler = actor_physics::Sampler::default();
        self.state_flags &= !(1 << 27);
        let land = self.jump_motions.land;
        if land >= 0 {
            let slot = land as usize;
            if land != 5 {
                self.motion_change(slot, 0., clips);
            } else if clips.length(slot) != 0 {
                self.motion_start(slot, 0., clips);
            }
        }
    }

    fn update_jump(
        &mut self,
        mut arc: actor_physics::Arc,
        mut velocity: [f32; 2],
        clips: &dyn Clips,
        env: &dyn Env,
    ) {
        if self.sampler.due(0.) {
            let dt = actor_physics::PHYSICS_STEP;
            // Without a move request the take-off velocity decays (FUN_1403b7ad0).
            velocity = actor_physics::steer(
                velocity,
                [0.; 2],
                0.,
                actor_physics::AIR_INERTIA,
                actor_physics::AIR_DRAG,
            );
            let dy = arc.step(self.position[1], self.sstm.fall_max, false, dt);
            self.sampler.store([velocity[0] * dt, dy, velocity[1] * dt]);
        }
        self.jump = Some((arc, velocity));
        let d = self.sampler.displacement();
        self.apply_movement(d, env);
        // FUN_1403cb680 (mode slot 7): the fall motion starts five ticks before
        // the apex, once.
        if self.state_flags & state_flag::GROUND_ARMED != 0
            && self.jump_motions.fall >= 0
            && arc.falling_motion()
        {
            self.state_flags &= !state_flag::GROUND_ARMED;
            self.motion_change(
                self.jump_motions.fall as usize,
                actor_physics::MOTION_BLEND,
                clips,
            );
        }
        if let Some(floor) = env.floor(self.position[0], self.position[2], self.position[1] + 30.)
            && self.position[1] <= floor
            && d[1] <= 0.
        {
            self.position[1] = floor;
            self.jump = None;
            self.land(clips);
        }
    }

    fn update_fly(&mut self, clips: &dyn Clips, env: &dyn Env) {
        let mut fly = self.fly.expect("active fly jump");
        if !fly.fall_started && fly.arc.falling_motion() && self.reaction == Reaction::None {
            if !self.motion_start(155 + 1, actor_physics::MOTION_BLEND, clips) {
                return;
            }
            fly.fall_started = true;
        }
        let height = env
            .floor(self.position[0], self.position[2], self.position[1])
            .map_or(0., |f| (self.position[1] - f).max(0.));
        if fly.arc.t >= fly.arc.rise && height <= fly.initial_height {
            self.airborne = fly.original_airborne;
            self.fly = None;
            self.sampler.reset();
            self.motion_start(155 + 2, 0., clips);
            return;
        }
        let frozen = self.gravity_frozen();
        if self.sampler.due(self.move_speed) {
            let dt = actor_physics::PHYSICS_STEP;
            let dy = fly
                .arc
                .step(self.position[1], self.sstm.fall_max, frozen, dt);
            fly.velocity = actor_physics::steer(
                fly.velocity,
                [self.move_direction[0], self.move_direction[2]],
                self.move_speed,
                actor_physics::AIR_INERTIA,
                actor_physics::AIR_DRAG,
            );
            self.sampler
                .store([fly.velocity[0] * dt, dy, fly.velocity[1] * dt]);
        }
        let root = self.motion.root_step;
        let (sin, cos) = self.yaw.sin_cos();
        let root = [
            root[0] * cos + root[2] * sin,
            root[1],
            -root[0] * sin + root[2] * cos,
        ];
        let base = self.sampler.displacement();
        let wished = [0, 1, 2].map(|axis| base[axis] + root[axis]);
        self.apply_movement(wished, env);
        fly.arc.base -= self.moved[1] - wished[1];
        self.fly = Some(fly);
    }

    fn update_reaction(&mut self, clips: &dyn Clips, env: &dyn Env) {
        if let Reaction::Blow { back } = self.reaction {
            if let Some(blow) = &mut self.blow {
                if self.sampler.due(0.) {
                    self.sampler.store(blow.step(
                        &self.sstm,
                        self.sstm.fall_max,
                        actor_physics::PHYSICS_STEP,
                    ));
                }
                let d = self.sampler.displacement();
                let from = self.position;
                self.position = env.movement(from, d);
                if d[1] <= 0.
                    && env
                        .floor(self.position[0], self.position[2], from[1])
                        .is_some_and(|y| self.position[1] <= y)
                {
                    self.blow = None;
                    self.airborne = false;
                    self.reaction = Reaction::BlowLand;
                    self.motion_start(if back { 19 } else { 17 }, 0., clips);
                }
            }
        } else {
            if let Some(knock) = &mut self.knock {
                if let Some(d) = knock.step() {
                    self.position = env.movement(self.position, d);
                } else {
                    self.knock = None;
                }
            }
            if clips.ended(self.motion.slot, self.motion.tick) {
                self.reaction = Reaction::None;
                self.motion_start(0, 0., clips);
            }
        }
    }

    /// FUN_1403cbb00 (`method_obj_jump`): arm a jump. The object leaves the
    /// ground when the playing motion's take-off event (frame event 0) fires
    /// (`+0x648` = 1). `height` is the Y-up apex height, `speed` the
    /// horizontal speed along `direction`.
    pub fn launch_jump(&mut self, direction: Vec3, height: f32, speed: f32) {
        self.pending_mode = 1;
        self.pending_jump = Some((direction, height, speed));
        self.pending_hop = None;
        // Arming resets the motions the jump plays near its apex and when it
        // lands (FUN_1403cbb00): none, and motion 5.
        self.jump_motions = JumpMotions::default();
        self.state_flags &= !state_flag::GROUND_ARMED;
    }

    /// Enter the jump physics mode now.
    fn start_jump(&mut self, direction: Vec3, height: f32, speed: f32) {
        let len = hlen(direction);
        let dir = if len > 0. {
            [direction[0] / len, direction[2] / len]
        } else {
            [0.; 2]
        };
        self.steer = None;
        self.hop = None;
        self.jump = Some((
            actor_physics::Arc::launch(self.position[1], height),
            [dir[0] * speed, dir[1] * speed],
        ));
        // FUN_1403cbd30 on entering the jump mode arms the fall motion.
        self.state_flags |= state_flag::GROUND_ARMED;
        self.sampler.reset();
        self.airborne = true;
    }

    /// Arc frozen by a gravity range (`+0x18c & 0x1000020` or bit 6).
    pub fn gravity_frozen(&self) -> bool {
        let w = self.motion.flags.word;
        w & (motion::flag::GRAVITY_FROZEN | motion::flag::GRAVITY_FROZEN_2) != 0
            || w & motion::flag::HOLD != 0
    }

    /// FUN_1403ba9a0: true while the object is airborne or in a special state;
    /// an act first run waits for it to clear.
    pub fn air_blocked(&self) -> bool {
        self.is_jump()
            || self.motion.flags.word
                & (motion::flag::GRAVITY_FROZEN | motion::flag::GRAVITY_FROZEN_2)
                != 0
            || self.flags >> 23 & 1 != 0
    }

    /// +0x18c bit 9: the playing clip holds the actor out of control.
    pub fn no_control(&self) -> bool {
        self.motion.flags.word & motion::flag::NO_CONTROL != 0
    }

    /// Physics mode entry for a fall (FUN_1403b8150): the parabola continues
    /// from the given Y-up vertical speed with no horizontal velocity.
    pub fn start_fall(&mut self, vertical_up: f32) {
        self.steer = None;
        self.hop = None;
        self.jump = Some((
            actor_physics::Arc::fall(self.position[1], vertical_up),
            [0.; 2],
        ));
        self.sampler.reset();
        self.airborne = true;
    }

    /// Read a word of the engine object that scripts access directly through
    /// its handle (`.d` addressing). Only the offsets found in installed
    /// scripts are mapped: +0x30..+0x38 is the scale, +0x134 the turn rate
    /// (word 12 of the spec `obj_spec` returns).
    pub fn peek_word(&self, offset: u32) -> Option<u32> {
        match offset {
            0x30 => Some(self.scale[0].to_bits()),
            0x34 => Some(self.scale[1].to_bits()),
            0x38 => Some(self.scale[2].to_bits()),
            0x3c => Some(self.scale_w.to_bits()),
            0x20..=0x2c if offset.is_multiple_of(4) => {
                Some(self.velocity[((offset - 0x20) / 4) as usize].to_bits())
            }
            0x134 => Some(self.turn_rate.to_bits()),
            _ => self.raw_words.get(&offset).copied(),
        }
    }

    /// Write a word of the engine object (see [`Sysobj::peek_word`]).
    pub fn poke_word(&mut self, offset: u32, value: u32) -> bool {
        match offset {
            0x30 => self.scale[0] = f32::from_bits(value),
            0x34 => self.scale[1] = f32::from_bits(value),
            0x38 => self.scale[2] = f32::from_bits(value),
            0x3c => self.scale_w = f32::from_bits(value),
            0x20..=0x2c if offset.is_multiple_of(4) => {
                self.velocity[((offset - 0x20) / 4) as usize] = f32::from_bits(value)
            }
            0x134 => self.turn_rate = f32::from_bits(value),
            _ if offset < 0x1000 && offset.is_multiple_of(4) => {
                self.raw_words.insert(offset, value);
            }
            _ => return false,
        }
        true
    }

    /// `obj_jump_direct`: enter the jump mode now.
    pub fn jump_now(&mut self, direction: Vec3, height: f32, speed: f32) {
        self.pending_mode = 0;
        self.pending_jump = None;
        self.start_jump(direction, height, speed);
    }

    /// `obj_blow` (FUN_1403d3e50): knocked away with this velocity.
    pub fn blow_now(&mut self, velocity: Vec3) {
        self.blow = Some(actor_physics::Blow::new(velocity, &self.sstm));
        self.knock = None;
        self.jump = None;
        self.hop = None;
        self.fly = None;
        self.steer = None;
        self.pending_fly = None;
        self.sampler.reset();
        self.airborne = true;
        self.reaction = Reaction::Blow { back: false };
    }

    /// FUN_140418800: set routine option flag `index`.
    pub fn set_rtn_flag(&mut self, index: u32) {
        let w = (index >> 5) as usize;
        if self.rtn_flags.len() <= w {
            self.rtn_flags.resize(w + 1, 0);
        }
        self.rtn_flags[w] |= 1 << (index & 31);
    }

    /// FUN_140417630.
    pub fn rtn_flag(&self, index: u32) -> bool {
        self.rtn_flags
            .get((index >> 5) as usize)
            .is_some_and(|w| w >> (index & 31) & 1 != 0)
    }

    /// `trap_obj_is_fly` (FUN_14042ef50): the physics mode has the fly bit
    /// (the flyjump and steering modes here), or the motion holds the arc
    /// (+0x18c bit 6).
    pub fn is_fly(&self) -> bool {
        self.fly.is_some()
            || self.steer.is_some_and(|s| !s.hover)
            || self.motion.flags.word & motion::flag::HOLD != 0
    }

    /// FUN_1403f1fa0: enable collision shape `index` (bit in the first word of
    /// its pair).
    pub fn enable_collision(&mut self, index: u32) {
        let w = (index >> 5) as usize;
        if self.collision.len() <= w + 1 {
            self.collision.resize(w + 2, 0);
        }
        self.collision[w] |= 1 << (index & 31);
    }

    /// FUN_1403f1f80: disable collision shape `index` (bit in the second word of
    /// its pair).
    pub fn disable_collision(&mut self, index: u32) {
        let w = (index >> 5) as usize + 1;
        if self.collision.len() <= w {
            self.collision.resize(w + 1, 0);
        }
        self.collision[w] |= 1 << (index & 31);
    }

    /// FUN_1403f2000: reset collision shape `index` (clears the bit in both words).
    pub fn reset_collision(&mut self, index: u32) {
        let w = (index >> 5) as usize;
        if self.collision.len() <= w + 1 {
            self.collision.resize(w + 2, 0);
        }
        self.collision[w] &= !(1 << (index & 31));
        self.collision[w + 1] &= !(1 << (index & 31));
    }

    /// Pool `index` of the battle sheet (0 is the object's HP).
    pub fn pool(&self, index: usize) -> Option<Pool> {
        if index == 0 {
            Some(Pool {
                hp: self.hp,
                max: self.stats.max_hp,
                min: self.min_hp,
            })
        } else {
            self.extra_pools.get(index - 1).copied()
        }
    }

    pub fn set_pool_min(&mut self, index: usize, value: i32) {
        if index == 0 {
            self.min_hp = value;
        } else if let Some(p) = self.extra_pools.get_mut(index - 1) {
            p.min = value;
        }
    }

    /// FUN_140431140: a value under the floor lands on the floor, one over the
    /// maximum on the maximum.
    pub fn set_pool_hp(&mut self, index: usize, value: i32) {
        let Some(p) = self.pool(index) else {
            return;
        };
        let hp = if value < p.min {
            p.min
        } else {
            value.min(p.max)
        };
        if index == 0 {
            self.hp = hp;
        } else if let Some(p) = self.extra_pools.get_mut(index - 1) {
            p.hp = hp;
        }
    }

    /// `trap_obj_idle`: back to the ground physics mode (FUN_1403b4da0 mode 0).
    pub fn to_ground_mode(&mut self) {
        self.jump = None;
        self.hop = None;
        self.fly = None;
        self.steer = None;
        self.pending_fly = None;
        self.pending_hop = None;
        self.airborne = false;
        self.sampler.reset();
    }

    /// `trap_obj_is_jump`: the physics state byte at +0xc bit 0 (jump, hop
    /// and flyjump have it; the steering modes do not).
    pub fn is_jump(&self) -> bool {
        self.jump.is_some() || self.hop.is_some() || self.fly.is_some()
    }
}

#[cfg(test)]
mod tests;
