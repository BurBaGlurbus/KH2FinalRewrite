//! The engine objects scripts act on, and the machinery that runs them.
//!
//! A [`World`] owns every script-driven object ([`Object`]) and the engine
//! state they share (random generator, frame time, the player). A trap runs
//! with a [`Ctx`]: the object whose script called it plus the world.

use crate::act;
use crate::env::Env;
use crate::script::{self, Callback, Exit, Fault, Host, Instance, Memory, Slot, Thread};
use crate::sysobj::Sysobj;
use std::collections::HashMap;

pub type ObjId = u32;

/// The scripts' shared random generator (DAT_1407535c0): an LCG with
/// multiplier 0x10dcd, initial value 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lcg(pub u32);

impl Default for Lcg {
    fn default() -> Self {
        Lcg(1)
    }
}

impl Lcg {
    pub fn step(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x10dcd);
        self.0
    }
}

/// A script-driven object: engine body plus script.
pub struct Object {
    pub id: ObjId,
    pub sys: Sysobj,
    pub acts: act::Manager,
    pub script: ScriptSpace,
    /// Callbacks traps queued for the interpreter to run (see [`Host::take_callbacks`]).
    pub pending: Vec<Callback>,
}

impl Object {
    /// An object with an empty script (tests and tools).
    pub fn bare(id: ObjId) -> Self {
        Self {
            id,
            sys: Sysobj::bare(),
            acts: act::Manager::default(),
            pending: Vec::new(),
            script: ScriptSpace {
                instance: Instance::placeholder(),
                stack_bytes: 0,
                temp_bytes: 0,
                main: None,
                main_ended: false,
                act_thread: None,
                act_generation: 0,
                faults: Vec::new(),
            },
        }
    }
}

/// The object's script instance and its threads.
pub struct ScriptSpace {
    pub instance: Instance,
    pub stack_bytes: u32,
    pub temp_bytes: u32,
    /// The persistent main thread (OBJ_MAIN) and whether it has ended.
    pub main: Option<Thread>,
    pub main_ended: bool,
    /// The running act thread.
    pub act_thread: Option<Thread>,
    /// Counts act thread replacements, so a run can tell its thread was
    /// discarded while it executed.
    pub act_generation: u32,
    /// Faults the interpreter reported (`pc` of the thread, fault), newest last.
    pub faults: Vec<(i32, Fault)>,
}

/// Side effects the presentation layer or other systems must carry out.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    EffectStart {
        object: ObjId,
        effect: i32,
        arguments: [i32; 2],
        handle: u32,
        /// Where the script placed it (`obj_effect_start_pos`); otherwise it
        /// follows the object.
        position: Option<[f32; 3]>,
        /// Bound to the object (`obj_effect_start_bind`).
        bound: bool,
        /// The object it is bound to when that is not the caller.
        other: Option<ObjId>,
    },
    /// A presentation request this build does not play (lights, screen fades,
    /// camera shots, command overrides): the trap's name and its operand words.
    Presentation {
        name: &'static str,
        words: [u32; 3],
    },
    /// A call into a subsystem this crate does not own (the camera, the
    /// screen, the mission HUD, sound, the party): the trap's name with its
    /// operands decoded as far as the executable does before it hands them
    /// on. Objects are resolved; `words` are the trap's argument words in
    /// order (floats as their bits); `vector` is the vector operand a trap
    /// takes, in the engine's +Y-up frame.
    Service {
        name: &'static str,
        object: Option<ObjId>,
        other: Option<ObjId>,
        words: [u32; 8],
        vector: Option<[f32; 4]>,
    },
    ScatterPrize {
        object: ObjId,
        count: i32,
    },
    /// A command on an effect handle: the first operand slots, and the vector
    /// the first one points at when it is readable.
    EffectCommand {
        handle: u32,
        words: [u32; 2],
        vector: Option<[f32; 3]>,
    },
    /// A motion event whose engine routine another subsystem owns (sound,
    /// effects, camera, physics modes).
    MotionEvent {
        object: ObjId,
        range: bool,
        kind: u8,
        params: Vec<u16>,
    },
    /// Frame event 1: an effect or sound with this id started.
    MotionEffect {
        object: ObjId,
        id: u16,
        handle: u32,
    },
    EffectDeadBlock {
        object: ObjId,
        handle: u32,
    },
    /// `magic_start_thread`: an enemy spell thread the magic system starts.
    MagicStart {
        object: ObjId,
        id: i32,
        arguments: [i32; 4],
        handle: u32,
    },
    /// `attack_strike`: a script applied an attack to a target directly.
    AttackStrike {
        /// The attack's handle and its slot in [`World::attacks`].
        attack: u32,
        slot: usize,
        /// The object struck (Sora is [`PLAYER_OBJECT`]); `None` when the
        /// script's AI object named nothing.
        target: Option<ObjId>,
        arguments: [i32; 2],
    },
    /// `obj_dead`: the object finished dying and is removed.
    ObjectDead {
        object: ObjId,
    },
    /// `obj_voice`: the object plays a voice line.
    Voice {
        object: ObjId,
        id: i32,
        arg: i32,
    },
    /// A service the executable provides that this build does not translate yet.
    Unsupported(&'static str),
}

/// A shout waiting for the frame's shout job (`act_shout`).
#[derive(Clone, Debug, PartialEq)]
pub struct Shout {
    pub name: Vec<u8>,
    pub shouter: ObjId,
    /// 0 or less: delivered at once; otherwise each receiver waits a random
    /// share of this many ticks.
    pub delay: f32,
}

/// DAT_142a23d50 holds 64 shouts.
pub const SHOUT_QUEUE: usize = 64;

/// A light a script made (`light_create`, FUN_1403f6f80): a 0x28-byte block
/// of the executable's light pool. The host draws what these say.
#[derive(Clone, Debug, PartialEq)]
pub struct Light {
    /// +0: the light's kind (the argument of `light_create`).
    pub kind: i32,
    /// +8: the fade, starting at full (1.0).
    pub fade: crate::sysobj::Fade,
    /// +0x10: flag bits (`light_set_flag`).
    pub flags: Vec<u32>,
    /// +0x14..+0x17: r, g, b and a (0x80 after `light_set_color`).
    pub color: [u8; 4],
    /// +0x18..+0x24: fog near, far, min and max.
    pub fog: [f32; 4],
}

/// Light handles are 16 bytes apart from here.
pub const LIGHT_HANDLES: u32 = script::EXTERN_SEGMENT + 0x0800_0000;

/// A vacuum a script made (`vacuum_create`, FUN_1403f8f20): a pull toward a
/// point. The host applies the pull; scripts shape it.
#[derive(Clone, Debug, PartialEq)]
pub struct Vacuum {
    /// +0: the point it pulls toward.
    pub position: [f32; 4],
    /// +0x10: flag bits (`vacuum_set_flag`).
    pub flags: u32,
    /// +0x14: the effective range (`vacuum_set_effective_range`).
    pub effective_range: f32,
    /// +0x18..+0x20: three values the constructor fills with its argument
    /// and `vacuum_set` 2:81 replaces.
    pub at_18: [f32; 3],
    /// +0x24, +0x28 (100.0 at creation) and +0x2c (0.01 at creation), set by
    /// 2:82 to 2:84.
    pub at_24: f32,
    pub at_28: f32,
    pub at_2c: f32,
}

/// Vacuum handles are 16 bytes apart from here.
pub const VACUUM_HANDLES: u32 = script::EXTERN_SEGMENT + 0x0900_0000;

/// Bits of the executable's global status word (DAT_142a11400) that scripts
/// set and read.
pub mod status {
    /// `status_no_leave`: enemies may not leave.
    pub const NO_LEAVE: u32 = 1 << 2;
    /// `status_prize_drain_start` / `_end`.
    pub const PRIZE_DRAIN: u32 = 1 << 3;
    /// `status_force_leave_start` / `_end`: every enemy leaves.
    pub const FORCE_LEAVE: u32 = 1 << 7;
}

/// An object search a script made (kept while `World::trace` is on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchRecord {
    /// `entry`, `serial`, `part` or `unit_arg`.
    pub kind: &'static str,
    pub value: i32,
    /// The unit argument index (`unit_arg` only).
    pub index: i32,
    pub found: bool,
}

/// A request to make an object of an entry appear (`sysobj_appear`,
/// FUN_1403df930).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AppearRequest {
    /// The object entry id (the script's word, flag bits included).
    pub entry: u32,
    /// The object whose script asked.
    pub by: ObjId,
}

/// An object the host built for an [`AppearRequest`].
pub struct Appeared {
    /// The object script (BAR type 3 payload); `None` builds a bare object.
    pub payload: Option<Vec<u8>>,
    pub sys: Sysobj,
}

/// The host side of `sysobj_appear`: it owns the object table (entry ids,
/// models, scripts) this crate does not hold.
pub trait Spawner: Send + Sync {
    fn appear(&mut self, request: &AppearRequest) -> Option<Appeared>;
}

/// Everything shared between objects.
pub struct World {
    /// `DAT_140717480`: 1.0 at 60 fps and normal speed.
    pub frame_time: f32,
    pub rng: Lcg,
    pub objects: Vec<Option<Box<Object>>>,
    /// Static result cells traps hand scripts addresses of (the executable's
    /// per-trap `static` buffers); allocated on first use.
    statics: HashMap<(u16, u16, u16), u32>,
    static_words: Vec<u32>,
    /// Traps a script called that nothing implements yet.
    pub missing: Vec<(u16, u16)>,
    /// Every trap called, newest last, when `trace` is on (development aid).
    pub trace: Option<Vec<(u16, u16)>>,
    /// The object searches scripts made, when `trace` is on.
    pub searches: Vec<SearchRecord>,
    /// The act names scripts started or queued, when tracing.
    pub act_log: Option<Vec<String>>,
    pub events: Vec<Event>,
    /// Keep running when a script calls a trap with no translation (the call
    /// returns zero and is listed in `missing`). Development aid.
    pub tolerate_missing: bool,
    next_effect: u32,
    /// Copies of target records (`trap_target_dup`), by slot.
    pub target_dups: Vec<Option<crate::sysobj::Target>>,
    /// `teamwork_alloc` blocks by key (DAT_142ae75d0).
    pub teamwork: HashMap<i32, u32>,
    next_magic: u32,
    /// The global control-off bits (DAT_142a171e8) and the hidden map parts.
    pub control_off: Vec<u32>,
    pub bg_hidden: u64,
    /// The stand-in traps reach when a script names Sora (see [`PLAYER_OBJECT`]).
    pub player_proxy: Option<Box<Object>>,
    /// The attack objects scripts created (`attack_new`), by slot.
    pub attacks: Vec<Option<crate::attack::Attack>>,
    /// `attack_new_combo_group`'s counter (DAT_142a24afc).
    pub combo_group: u32,
    /// Attack objects created so far (`Attack::serial`).
    pub attack_serial: u64,
    /// `status_set_lockon_ratio`: the lock-on ratio scripts last set.
    pub lockon_ratio: f32,
    /// How `sysobj_appear` makes objects (none: the script gets a null object).
    pub spawner: Option<Box<dyn Spawner>>,
    appear_depth: u32,
    /// The lights scripts made, by handle slot.
    pub lights: Vec<Option<Light>>,
    /// The vacuums scripts made, by handle slot.
    pub vacuums: Vec<Option<Vacuum>>,
    /// The shouts waiting for the frame's shout job.
    pub shouts: Vec<Shout>,
    /// The engine's own random generator (DAT_140783ca0), which the shout
    /// delay draws from; the scripts' generator is [`World::rng`].
    pub engine_rng: Lcg,
    /// Where the last map line check crossed the map (`bghit_get_cross_pos`).
    pub last_line_cross: Option<[f32; 3]>,
    /// The hit mark's two vectors (DAT_14074c0d8 / DAT_14074c0e8), engine
    /// frame: where the hit spark goes, and a second point.
    pub hit_mark: [[f32; 4]; 2],
    /// DAT_142a11400 bits scripts set (see [`status`]).
    pub status: u32,
    /// The mission timer's pause bit (DAT_140abb854 bit 1).
    pub mission_timer_paused: bool,
    /// Enemy units (spawn groups) scripts disabled (`unit_disable`).
    pub units_disabled: std::collections::BTreeSet<u32>,
}

/// How many `sysobj_appear` calls may be inside each other's OBJ_INIT.
const MAX_APPEAR_DEPTH: u32 = 4;

impl Default for World {
    fn default() -> Self {
        Self {
            frame_time: 1.0,
            rng: Lcg::default(),
            objects: Vec::new(),
            statics: HashMap::new(),
            static_words: Vec::new(),
            missing: Vec::new(),
            trace: None,
            searches: Vec::new(),
            act_log: None,
            events: Vec::new(),
            tolerate_missing: false,
            next_effect: 0,
            target_dups: Vec::new(),
            teamwork: HashMap::new(),
            next_magic: 0,
            control_off: Vec::new(),
            bg_hidden: 0,
            player_proxy: Some(Box::new(Object::bare(PLAYER_OBJECT))),
            attacks: Vec::new(),
            combo_group: 0,
            attack_serial: 0,
            lockon_ratio: 0.,
            spawner: None,
            appear_depth: 0,
            lights: Vec::new(),
            vacuums: Vec::new(),
            shouts: Vec::new(),
            engine_rng: Lcg::default(),
            last_line_cross: None,
            hit_mark: [[0.; 4]; 2],
            status: 0,
            mission_timer_paused: false,
            units_disabled: Default::default(),
        }
    }
}

impl World {
    /// FUN_1403df930 as the host sees it: build the object the spawner
    /// describes, run its OBJ_INIT and put it in the object list. Returns its
    /// id; `me` is the object whose script asked (never reused).
    pub fn appear(&mut self, env: &dyn Env, me: ObjId, entry: u32) -> Option<ObjId> {
        // An OBJ_INIT that makes objects of its own entry would recurse
        // without end; the executable's object table is finite.
        if self.appear_depth >= MAX_APPEAR_DEPTH {
            return None;
        }
        let mut spawner = self.spawner.take()?;
        let built = spawner.appear(&AppearRequest { entry, by: me });
        self.spawner = Some(spawner);
        let built = built?;
        self.appear_depth += 1;
        let made = self.appear_built(env, me, entry, built);
        self.appear_depth -= 1;
        made
    }

    fn appear_built(
        &mut self,
        env: &dyn Env,
        me: ObjId,
        entry: u32,
        built: Appeared,
    ) -> Option<ObjId> {
        let id = (self.objects.len() as u32).max(me + 1).max(1);
        let mut sys = built.sys;
        sys.entry_id = entry;
        let object = match built.payload {
            Some(payload) => Object::spawn(self, env, id, &payload, sys).ok()?,
            None => {
                let mut o = Object::bare(id);
                o.sys = sys;
                o
            }
        };
        if self.objects.len() <= id as usize {
            self.objects.resize_with(id as usize + 1, || None);
        }
        self.objects[id as usize] = Some(Box::new(object));
        Some(id)
    }

    /// Put every carried object on its holder (physics mode 3): at the
    /// holder's bone when the host knows it, else the holder's origin. `me` is
    /// the object the host runs itself, outside the list.
    pub fn follow_attachments(&mut self, env: &dyn Env, me: Option<&mut Object>) {
        let place = |holder: ObjId, bone: i32, world: &World, own: Option<&Object>| {
            if let Some(p) = env.attach_point(holder, bone) {
                return Some(p);
            }
            if holder == PLAYER_OBJECT {
                return Some(env.player_position());
            }
            world
                .objects
                .get(holder as usize)
                .and_then(|o| o.as_deref())
                .or(own.filter(|o| o.id == holder))
                .map(|o| o.sys.position)
        };
        // Carried objects of the list (the holder may be `me`).
        for id in 0..self.objects.len() {
            let Some(a) = self.objects[id].as_ref().and_then(|o| o.sys.attached) else {
                continue;
            };
            if let Some(p) = place(a.to, a.bone, self, me.as_deref())
                && let Some(o) = self.objects[id].as_mut()
            {
                o.sys.position = p;
            }
        }
        if let Some(object) = me
            && let Some(a) = object.sys.attached
            && let Some(p) = place(a.to, a.bone, self, None)
        {
            object.sys.position = p;
        }
    }

    /// The frame's shout job (FUN_1403c9d40): every queued shout goes to every
    /// object of the list in turn, `me` (the object the host runs itself,
    /// outside the list) included. With a delay each receiver draws a random
    /// share of it from the engine's generator and waits; without one the
    /// receiver's act of that name calls its timeout routine now.
    pub fn deliver_shouts(&mut self, env: &dyn Env, mut me: Option<&mut Object>) {
        for shout in std::mem::take(&mut self.shouts) {
            let mut ids: Vec<ObjId> = self.objects.iter().flatten().map(|o| o.id).collect();
            ids.sort_unstable();
            if let Some(object) = me.as_deref_mut() {
                self.shout_to(env, object, &shout);
            }
            for id in ids {
                if let Some(mut object) = self.objects[id as usize].take() {
                    self.shout_to(env, &mut object, &shout);
                    self.objects[id as usize] = Some(object);
                }
            }
        }
    }

    fn shout_to(&mut self, env: &dyn Env, object: &mut Object, shout: &Shout) {
        if object.sys.flags & SEARCH_SKIP_FLAGS != 0 {
            return;
        }
        if shout.delay > 0. {
            // (float)rng * 2^-32 * delay (DAT_1405b4d30).
            let draw = self.engine_rng.step() as f32 * 2.328_306_4e-10 * shout.delay;
            object.acts.pending_shout = Some(crate::act::PendingShout {
                name: shout.name.clone(),
                shouter: object_handle(shout.shouter),
                timer: draw,
            });
        } else {
            object.deliver_shout(self, env, &shout.name, object_handle(shout.shouter));
        }
    }

    /// One frame for every object in the list except `skip` (the one the host
    /// is running itself).
    pub fn tick_objects(&mut self, env: &dyn Env, skip: Option<ObjId>) {
        for id in 0..self.objects.len() {
            if skip == Some(id as ObjId) {
                continue;
            }
            if let Some(mut object) = self.objects[id].take() {
                object.tick(self, env);
                if self.objects.len() <= id {
                    self.objects.resize_with(id + 1, || None);
                }
                if !object.sys.gone {
                    self.objects[id] = Some(object);
                }
            }
        }
    }

    /// `trap_memory_alloc`: a zeroed block in the shared static area.
    pub fn alloc(&mut self, bytes: u32) -> u32 {
        let words = bytes.div_ceil(4).max(1) as usize;
        let at = self.static_words.len() as u32 * 4;
        self.static_words.extend(std::iter::repeat_n(0, words));
        script::EXTERN_SEGMENT + STATIC_BASE + at
    }

    /// A fresh handle for an effect the host will create.
    pub fn next_effect_handle(&mut self) -> u32 {
        self.next_effect += 1;
        EFFECT_HANDLES + self.next_effect * 16
    }

    /// `func_obj_control_off` / `_on`: set or clear control bit `index`.
    pub fn set_control_bit(&mut self, index: u32, off: bool) {
        let w = (index >> 5) as usize;
        if self.control_off.len() <= w {
            self.control_off.resize(w + 1, 0);
        }
        let bit = 1u32 << (index & 31);
        if off {
            self.control_off[w] |= bit;
        } else {
            self.control_off[w] &= !bit;
        }
    }

    /// `trap_bg_hide` / `trap_bg_show`: whether map part `index` shows.
    pub fn set_bg_part(&mut self, index: u32, shown: bool) {
        let bit = 1u64 << (index & 63);
        if shown {
            self.bg_hidden &= !bit;
        } else {
            self.bg_hidden |= bit;
        }
    }

    /// One frame for every attack object (FUN_1404376b0) and every light
    /// (its fade runs). An attack bound to an effect follows it and goes when
    /// the effect ends; a timed shape grows and goes when its time is up.
    pub fn tick_attacks(&mut self, env: &dyn Env) {
        let frame_time = self.frame_time;
        // DAT_142a11400 bit 5 holds the timers.
        let held = self.status & (1 << 5) != 0;
        for slot in self.attacks.iter_mut() {
            let Some(a) = slot.as_mut() else {
                continue;
            };
            if a.effect != 0 {
                if !env.effect_active(a.effect) {
                    a.flags |= crate::attack::flag::FREED;
                    *slot = None;
                    continue;
                }
                if let Some(p) = env.effect_position(a.effect) {
                    a.follow(p, frame_time);
                }
            }
            if !held && a.advance(frame_time) {
                a.flags |= crate::attack::flag::FREED;
                *slot = None;
            }
        }
        for light in self.lights.iter_mut().flatten() {
            light.fade.step(frame_time);
        }
    }

    /// The vacuum slot a handle names.
    pub fn vacuum_slot(&self, handle: u32) -> Option<usize> {
        let o = handle.checked_sub(VACUUM_HANDLES)?;
        let i = (o / 16) as usize;
        (o % 16 == 0 && self.vacuums.get(i)?.is_some()).then_some(i)
    }

    /// The light slot a handle names.
    pub fn light_slot(&self, handle: u32) -> Option<usize> {
        let o = handle.checked_sub(LIGHT_HANDLES)?;
        let i = (o / 16) as usize;
        (o % 16 == 0 && self.lights.get(i)?.is_some()).then_some(i)
    }

    /// A fresh handle for a magic thread the host will create.
    pub fn next_magic_handle(&mut self) -> u32 {
        self.next_magic += 1;
        MAGIC_HANDLES + self.next_magic * 16
    }

    /// The static cell if some trap already allocated it.
    pub fn static_lookup(&self, table: u16, index: u16, slot: u16) -> Option<u32> {
        self.statics
            .get(&(table, index, slot))
            .map(|at| script::EXTERN_SEGMENT + STATIC_BASE + at)
    }

    /// The handle of a four-word static cell owned by `(table, index, slot)`;
    /// the first use allocates it zeroed.
    pub fn static_cell(&mut self, table: u16, index: u16, slot: u16) -> u32 {
        let words = &mut self.static_words;
        let at = *self.statics.entry((table, index, slot)).or_insert_with(|| {
            let at = words.len() as u32 * 4;
            words.extend([0; 4]);
            at
        });
        script::EXTERN_SEGMENT + STATIC_BASE + at
    }

    pub fn static_read(&self, handle: u32) -> Option<u32> {
        let at = handle.checked_sub(script::EXTERN_SEGMENT + STATIC_BASE)?;
        self.static_words.get((at / 4) as usize).copied()
    }

    pub fn static_write(&mut self, handle: u32, value: u32) -> bool {
        let Some(at) = handle.checked_sub(script::EXTERN_SEGMENT + STATIC_BASE) else {
            return false;
        };
        match self.static_words.get_mut((at / 4) as usize) {
            Some(w) => {
                *w = value;
                true
            }
            None => false,
        }
    }
}

/// Engine handles: statics at 0x6000_0000, objects at 0x6100_0000 + id.
const STATIC_BASE: u32 = 0;
pub const OBJECT_HANDLES: u32 = script::EXTERN_SEGMENT + 0x0100_0000;

/// Sora's handle (the player is not one of the script-driven objects).
pub const PLAYER_HANDLE: u32 = script::EXTERN_SEGMENT + 0x00f0_0000;
/// Effect handles scripts keep, and copies of target records.
pub const EFFECT_HANDLES: u32 = script::EXTERN_SEGMENT + 0x0200_0000;
pub const ATTACK_HANDLES: u32 = script::EXTERN_SEGMENT + 0x0700_0000;
pub const MAGIC_HANDLES: u32 = script::EXTERN_SEGMENT + 0x0500_0000;
pub const TARGET_DUP_HANDLES: u32 = script::EXTERN_SEGMENT + 0x0300_0000;
/// Sora's lock-on target record (`trap_obj_lockon_target`: the record at +4 of
/// the player's lock-on block, FUN_14042b9c0).
pub const LOCKON_RECORD: u32 = TARGET_DUP_HANDLES + 0x00f0_0000;
/// The engine object's target record (sys+0xbf8) is this far into its handle range.
pub const TARGET_RECORD_OFFSET: u32 = 0x800;

/// FUN_1403baa80: objects with any of these +0x120 flag bits are invisible to
/// the engine's object searches.
pub const SEARCH_SKIP_FLAGS: u32 = 0x1008_0000;

/// The entry id of Sora's object (`trap_obj_search_by_entry` answers the
/// player for it without walking the list: FUN_1403b4110).
pub const PLAYER_ENTRY: u32 = 0x236;

/// The id the player proxy answers to. Sora is not a script-driven object, but
/// scripts call the same traps on her as on any object; those reach a stand-in
/// whose position and heading follow the host's player.
pub const PLAYER_OBJECT: ObjId = 0x00ff_ffff;

pub fn object_handle(id: ObjId) -> u32 {
    if id == PLAYER_OBJECT {
        return PLAYER_HANDLE;
    }
    OBJECT_HANDLES + id * 0x1000
}

/// A word inside an object's handle range: the object and its field offset.
pub fn object_field(address: u32) -> Option<(ObjId, u32)> {
    let o = address.checked_sub(OBJECT_HANDLES)?;
    (o < 0x0100_0000).then_some((o / 0x1000, o % 0x1000))
}

pub fn object_from_handle(handle: u32) -> Option<ObjId> {
    let o = handle.checked_sub(OBJECT_HANDLES)?;
    (o % 0x1000 == 0 && o < 0x0100_0000).then_some(o / 0x1000)
}

/// A word of a target record (sys+0xbf8): +0 and +8 hold the target's engine
/// handle (0 = nobody), +4 is zero and +0xc the search kind.
fn target_record_word(t: crate::sysobj::Target, offset: u32) -> u32 {
    match offset {
        0 | 8 => match t.id {
            0 => 0,
            crate::traps::target::PLAYER_TARGET => PLAYER_HANDLE,
            id => object_handle(id),
        },
        4 => t.group as u32,
        0xc => t.kind as u32,
        _ => 0,
    }
}

/// What a trap sees: the calling object and the shared world.
pub struct Ctx<'a> {
    pub world: &'a mut World,
    pub me: &'a mut Object,
    pub env: &'a dyn Env,
}

impl Ctx<'_> {
    /// The object with this id: the calling object or another in the world.
    pub fn object(&mut self, id: ObjId) -> Result<&mut Object, Fault> {
        if id == self.me.id {
            return Ok(self.me);
        }
        if id == PLAYER_OBJECT {
            let proxy = self
                .world
                .player_proxy
                .get_or_insert_with(|| Box::new(Object::bare(PLAYER_OBJECT)));
            proxy.sys.position = self.env.player_position();
            proxy.sys.yaw = self.env.player_yaw();
            return Ok(proxy);
        }
        self.world
            .objects
            .get_mut(id as usize)
            .and_then(|o| o.as_deref_mut())
            .ok_or(Fault::Memory(object_handle(id)))
    }

    /// FUN_1403bf4f0's walk of the engine's object list with a test (the
    /// first object it accepts, in creation order). The walk skips objects
    /// whose +0x120 flags (FUN_1403baa80: `flags & 0x10080000`) are set. The
    /// calling object is part of the list; Sora is not (searches by entry id
    /// answer her separately).
    pub fn search(&self, mut test: impl FnMut(&Object) -> bool) -> Option<ObjId> {
        let mut best: Option<ObjId> = None;
        let mut consider = |o: &Object| {
            if o.sys.flags & SEARCH_SKIP_FLAGS == 0 && best.is_none_or(|b| o.id < b) && test(o) {
                best = Some(o.id);
            }
        };
        consider(self.me);
        for o in self.world.objects.iter().flatten() {
            if o.id != self.me.id {
                consider(o);
            }
        }
        best
    }

    /// The next object of the list after `after` (the first when `None`)
    /// that passes `test`, in creation order (FUN_1403bf4f0 steps; the walk
    /// skips the objects FUN_1403baa80 hides).
    pub fn next_object(
        &self,
        after: Option<ObjId>,
        mut test: impl FnMut(&Object) -> bool,
    ) -> Option<ObjId> {
        let mut best: Option<ObjId> = None;
        let mut consider = |o: &Object| {
            if o.sys.flags & SEARCH_SKIP_FLAGS == 0
                && after.is_none_or(|a| o.id > a)
                && best.is_none_or(|b| o.id < b)
                && test(o)
            {
                best = Some(o.id);
            }
        };
        consider(self.me);
        for o in self.world.objects.iter().flatten() {
            if o.id != self.me.id {
                consider(o);
            }
        }
        best
    }

    /// Bind the AI object `ai` to what a search found (FUN_1404ad240 of the
    /// engine object into +4; a miss stores 0).
    pub fn bind_ai(
        &mut self,
        memory: &mut Memory<'_>,
        ai: u32,
        found: Option<ObjId>,
    ) -> Result<(), Fault> {
        let handle = found.map_or(0, object_handle);
        if self.write_u32(memory, ai + 4, handle) {
            Ok(())
        } else {
            Err(Fault::Memory(ai))
        }
    }

    /// An engine-object argument (the trap received the handle itself).
    pub fn sys_of(&self, handle: u32) -> Result<ObjId, Fault> {
        if handle == PLAYER_HANDLE {
            return Ok(PLAYER_OBJECT);
        }
        object_from_handle(handle).ok_or(Fault::Memory(handle))
    }

    /// An AI-object argument that may name Sora: the word at +4 is either an
    /// engine object or the player.
    pub fn sys_ref_ai(&self, memory: &Memory<'_>, ai: u32) -> Result<SysRef, Fault> {
        let handle = self.read_u32(memory, ai + 4).ok_or(Fault::Memory(ai))?;
        if handle == PLAYER_HANDLE {
            return Ok(SysRef::Player);
        }
        self.sys_of(handle).map(SysRef::Object)
    }

    /// An AI-object argument: the engine object handle is the word at +4.
    pub fn sys_of_ai(&self, memory: &Memory<'_>, ai: u32) -> Result<ObjId, Fault> {
        let handle = self.read_u32(memory, ai + 4).ok_or(Fault::Memory(ai))?;
        self.sys_of(handle)
    }

    /// Read a 32-bit cell a script handle names: script memory first, then the
    /// static result cells.
    pub fn read_u32(&self, memory: &Memory<'_>, handle: u32) -> Option<u32> {
        memory
            .read_u32(handle)
            .or_else(|| self.world.static_read(handle))
            .or_else(|| self.self_handle_word(handle))
    }

    /// Word 1 of an engine handle used as an AI object names the engine
    /// object, which is the handle itself.
    fn self_handle_word(&self, address: u32) -> Option<u32> {
        let base = address.checked_sub(4)?;
        (base == PLAYER_HANDLE || object_from_handle(base).is_some()).then_some(base)
    }

    pub fn read_f32(&self, memory: &Memory<'_>, handle: u32) -> Option<f32> {
        self.read_u32(memory, handle).map(f32::from_bits)
    }

    pub fn write_u32(&mut self, memory: &mut Memory<'_>, handle: u32, value: u32) -> bool {
        memory.write_u32(handle, value) || self.world.static_write(handle, value)
    }

    pub fn write_f32(&mut self, memory: &mut Memory<'_>, handle: u32, value: f32) -> bool {
        self.write_u32(memory, handle, value.to_bits())
    }
}

/// What a script-held object reference names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SysRef {
    Player,
    Object(ObjId),
}

pub type TrapFn = fn(&mut Ctx<'_>, &mut Memory<'_>, &mut [Slot; 16]) -> Result<(), Fault>;

/// The script [`Host`] for one object's thread: resolves traps through the
/// registry in [`crate::traps`].
pub struct Machine<'a> {
    pub ctx: Ctx<'a>,
    /// Keep going when a trap has no implementation (development).
    pub tolerate_missing: bool,
}

impl Host for Machine<'_> {
    fn frame_time(&self) -> f32 {
        self.ctx.world.frame_time
    }

    fn take_callbacks(&mut self) -> Vec<Callback> {
        std::mem::take(&mut self.ctx.me.pending)
    }

    fn patch_gate(&self) -> f32 {
        self.ctx
            .world
            .static_lookup(1, 0, 1)
            .and_then(|cell| self.ctx.world.static_read(cell + 12))
            .map_or(0.0, f32::from_bits)
    }

    fn peek(&self, address: u32) -> Option<u32> {
        if let Some(offset) = address.checked_sub(PLAYER_HANDLE).filter(|o| *o < 0x1000) {
            // An engine handle doubles as the AI object the scripts copy
            // around (`memcpy` of the 16-byte struct): word 1 names the engine
            // object, which here is the handle itself.
            if offset < 16 {
                return Some(if offset == 4 { PLAYER_HANDLE } else { 0 });
            }
            return self.ctx.world.player_proxy.as_ref()?.sys.peek_word(offset);
        }
        if let Some((id, offset)) = object_field(address) {
            if offset < 16 {
                return Some(if offset == 4 { object_handle(id) } else { 0 });
            }
            if (TARGET_RECORD_OFFSET..TARGET_RECORD_OFFSET + 16).contains(&offset) {
                let sys = if id == self.ctx.me.id {
                    &self.ctx.me.sys
                } else {
                    &self.ctx.world.objects.get(id as usize)?.as_deref()?.sys
                };
                return Some(target_record_word(
                    sys.target,
                    offset - TARGET_RECORD_OFFSET,
                ));
            }
            if id == self.ctx.me.id {
                return self.ctx.me.sys.peek_word(offset);
            }
            return self
                .ctx
                .world
                .objects
                .get(id as usize)?
                .as_deref()?
                .sys
                .peek_word(offset);
        }
        if let Some(offset) = address.checked_sub(LOCKON_RECORD).filter(|o| *o < 16) {
            let t = crate::sysobj::Target {
                id: self.ctx.env.lockon_target().unwrap_or(0),
                kind: -1,
                group: 0,
            };
            return Some(target_record_word(t, offset));
        }
        if let Some(d) = address
            .checked_sub(TARGET_DUP_HANDLES)
            .filter(|d| *d < 0x0100_0000)
        {
            // A copy of a target record (`trap_target_dup`): a slot is 16
            // bytes. A slot nobody holds reads as an empty record.
            let t = self
                .ctx
                .world
                .target_dups
                .get((d / 16) as usize)
                .copied()
                .flatten()
                .unwrap_or_default();
            return Some(target_record_word(t, d % 16));
        }
        self.ctx.world.static_read(address)
    }

    fn poke(&mut self, address: u32, value: u32) -> bool {
        if let Some(offset) = address.checked_sub(PLAYER_HANDLE).filter(|o| *o < 0x1000) {
            return self
                .ctx
                .world
                .player_proxy
                .as_mut()
                .is_some_and(|p| p.sys.poke_word(offset, value));
        }
        if let Some((id, offset)) = object_field(address) {
            if id == self.ctx.me.id {
                return self.ctx.me.sys.poke_word(offset, value);
            }
            return match self.ctx.world.objects.get_mut(id as usize) {
                Some(Some(o)) => o.sys.poke_word(offset, value),
                _ => false,
            };
        }
        self.ctx.world.static_write(address, value)
    }

    fn trap_flags(&self, table: u16, index: u16) -> Option<u32> {
        crate::trap_table::lookup(table, index).map(|t| t.flags)
    }

    fn trap(
        &mut self,
        memory: &mut Memory<'_>,
        table: u16,
        index: u16,
        args: &mut [Slot; 16],
    ) -> Result<(), Fault> {
        if let Some(t) = self.ctx.world.trace.as_mut() {
            t.push((table, index));
        }
        match crate::traps::lookup(table, index) {
            Some(f) => f(&mut self.ctx, memory, args),
            None => {
                self.ctx.world.missing.push((table, index));
                if self.tolerate_missing {
                    // Arguments are consumed; a returned value reads as zero.
                    args[0] = Slot::int(0);
                    Ok(())
                } else {
                    Err(Fault::Trap { table, index })
                }
            }
        }
    }
}

/// Run one thread of `obj`'s script to its next yield.
pub fn run_thread(world: &mut World, env: &dyn Env, obj: &mut Object, thread: &mut Thread) -> Exit {
    let mut instance = std::mem::replace(&mut obj.script.instance, Instance::placeholder());
    let world_tolerates = world.tolerate_missing;
    let exit = {
        let mut machine = Machine {
            ctx: Ctx {
                world,
                me: obj,
                env,
            },
            tolerate_missing: world_tolerates,
        };
        script::tick_tree(&mut instance, thread, &mut machine, 1_000_000)
    };
    obj.script.instance = instance;
    if let Exit::Fault(f) = exit {
        obj.script.faults.push((thread.pc, f));
    }
    exit
}
