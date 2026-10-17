//! Object lifecycle and the per-tick order of an enemy: creation and its
//! script triggers (FUN_1403b44d0), the script tick (FUN_1403b4460: the main
//! thread, then the act manager when the main thread has not ended) and the
//! act manager loop (FUN_1403ca550).

use crate::act::{self, Action};
use crate::env::Env;
use crate::motion::Effect;
use crate::script::{Callback, Instance, Slot, Thread};
use crate::sysobj::Sysobj;
use crate::world::{Event, ObjId, Object, ScriptSpace, World, object_handle, run_thread};

/// Script trigger keys (OBJ_INIT, OBJ_MAIN, OBJ_DAMAGE, OBJ_FALL ...).
pub const INIT: i32 = 0;
pub const MAIN: i32 = 1;
pub const DAMAGE: i32 = 6;
pub const FALL: i32 = 13;
pub const LAND: i32 = 14;
pub const ATTACK: i32 = 15;
/// Triggers the motion events fire: frame event 1 (effect or sound) and 7.
pub const EVENT_EFFECT: i32 = 16;
pub const EVENT_SCRIPT: i32 = 17;

/// Frame slots of the object's threads (see [`crate::script`]).
const MAIN_SLOT: u32 = 0;
const ACT_SLOT: u32 = 1;
const EVENT_SLOT: u32 = 2;

#[derive(Debug)]
pub enum SpawnError {
    Program,
}

impl Object {
    /// FUN_1403b44d0: build the object around a script. OBJ_INIT runs to its
    /// first yield on a throw-away thread with the object as its argument; then
    /// the persistent OBJ_MAIN thread is created.
    pub fn spawn(
        world: &mut World,
        env: &dyn Env,
        id: ObjId,
        payload: &[u8],
        sys: Sysobj,
    ) -> Result<Object, SpawnError> {
        let instance = Instance::new(id, payload).ok_or(SpawnError::Program)?;
        let init = instance.trigger(INIT);
        let main = instance.trigger(MAIN);
        let (stack_bytes, temp_bytes) = (instance.stack_bytes, instance.temp_bytes);
        let mut obj = Object::bare(id);
        obj.sys = sys;
        obj.script = ScriptSpace {
            instance,
            stack_bytes,
            temp_bytes,
            main: None,
            main_ended: false,
            act_thread: None,
            act_generation: 0,
            faults: Vec::new(),
        };
        if init > 0 {
            let mut t = Thread::with_sizes(
                stack_bytes,
                temp_bytes,
                EVENT_SLOT,
                init,
                &[Slot::address(object_handle(id))],
            );
            run_thread(world, env, &mut obj, &mut t);
        }
        if main > 0 {
            obj.script.main = Some(Thread::with_sizes(
                stack_bytes,
                temp_bytes,
                MAIN_SLOT,
                main,
                &[],
            ));
        } else {
            obj.script.main_ended = true;
        }
        Ok(obj)
    }

    /// FUN_1403e1840: fire a trigger on a throw-away thread, run once.
    pub fn fire(&mut self, world: &mut World, env: &dyn Env, key: i32, args: &[Slot]) {
        let entry = self.script.instance.trigger(key);
        if entry <= 0 {
            return;
        }
        let mut t = Thread::with_sizes(
            self.script.stack_bytes,
            self.script.temp_bytes,
            EVENT_SLOT,
            entry,
            args,
        );
        run_thread(world, env, self, &mut t);
    }

    /// Turn the act manager's requests into thread changes. Inside the
    /// interpreter callbacks are queued (it runs them after the trap); outside
    /// they run now.
    pub fn apply_actions(&mut self, world: Option<(&mut World, &dyn Env)>, actions: Vec<Action>) {
        let mut later = Vec::new();
        for action in actions {
            match action {
                Action::Kill => {
                    self.script.act_thread = None;
                    self.script.act_generation = self.script.act_generation.wrapping_add(1);
                }
                Action::Spawn { entry } => {
                    self.script.act_thread = Some(Thread::with_sizes(
                        self.script.stack_bytes,
                        self.script.temp_bytes,
                        ACT_SLOT,
                        i32::from(entry),
                        &[Slot::address(self.acts.ai_object)],
                    ));
                    self.script.act_generation = self.script.act_generation.wrapping_add(1);
                }
                Action::Callback { word, .. } => later.push(Callback {
                    entry: word,
                    args: vec![Slot::address(self.acts.ai_object)],
                }),
            }
        }
        match world {
            None => self.pending.extend(later),
            Some((world, env)) => {
                for cb in later {
                    let mut t = Thread::with_sizes(
                        self.script.stack_bytes,
                        self.script.temp_bytes,
                        EVENT_SLOT,
                        i32::from(cb.entry),
                        &cb.args,
                    );
                    run_thread(world, env, self, &mut t);
                }
            }
        }
    }

    /// Run a script entry on a throw-away thread of this object (the
    /// executable's FUN_1403e2000 with three AI object arguments).
    fn run_callback(&mut self, world: &mut World, env: &dyn Env, entry: i32, args: &[Slot]) {
        let mut t = Thread::with_sizes(
            self.script.stack_bytes,
            self.script.temp_bytes,
            EVENT_SLOT,
            entry,
            args,
        );
        run_thread(world, env, self, &mut t);
    }

    /// An attack this object's script made struck `victim` (FUN_1403d1730):
    /// the attack records the strike, and the script's hit callback runs with
    /// the AI object it was registered with, the victim and the attack. The
    /// callback runs on `self`; an attack registered by another object calls
    /// that object's script instead.
    pub fn attack_struck(&mut self, world: &mut World, env: &dyn Env, slot: usize, victim: ObjId) {
        let Some(Some(a)) = world.attacks.get_mut(slot) else {
            return;
        };
        a.flags |= crate::attack::flag::STRUCK;
        let (callback, by) = (a.hit_callback, a.callback_object);
        let Some((entry, ai)) = callback else {
            return;
        };
        let args = [
            Slot::address(ai),
            Slot::address(object_handle(victim)),
            Slot::address(crate::world::ATTACK_HANDLES + 0x100 * slot as u32),
        ];
        self.run_callback_of(world, env, by, entry, &args);
    }

    /// A guard turned the attack back (FUN_1403d2140): it is marked reflected
    /// by `reflecter`, and the reflect callback runs with the AI object it
    /// was registered with, the attack and the reflecter.
    pub fn attack_reflected(
        &mut self,
        world: &mut World,
        env: &dyn Env,
        slot: usize,
        reflecter: ObjId,
    ) {
        let Some(Some(a)) = world.attacks.get_mut(slot) else {
            return;
        };
        a.reflect_count = a.reflect_count.wrapping_add(1);
        a.flags |= crate::attack::flag::REFLECTED;
        a.reflecter = Some(reflecter);
        let (callback, by) = (a.reflect_callback, a.callback_object);
        let Some((entry, ai)) = callback else {
            return;
        };
        let args = [
            Slot::address(ai),
            Slot::address(crate::world::ATTACK_HANDLES + 0x100 * slot as u32),
            Slot::address(object_handle(reflecter)),
        ];
        self.run_callback_of(world, env, by, entry, &args);
    }

    /// Run a callback on the object that registered it: `self`, or one of the
    /// world's.
    fn run_callback_of(
        &mut self,
        world: &mut World,
        env: &dyn Env,
        by: Option<ObjId>,
        entry: i32,
        args: &[Slot],
    ) {
        match by {
            Some(id) if id != self.id => {
                if let Some(mut other) = world.objects.get_mut(id as usize).and_then(Option::take) {
                    other.run_callback(world, env, entry, args);
                    world.objects[id as usize] = Some(other);
                }
            }
            _ => self.run_callback(world, env, entry, args),
        }
    }

    /// One frame of this object's script (FUN_1403b4460).
    pub fn tick_script(&mut self, world: &mut World, env: &dyn Env) {
        let Some(mut main) = self.script.main.take() else {
            return;
        };
        run_thread(world, env, self, &mut main);
        if main.finished() {
            self.script.main_ended = true;
        } else {
            self.script.main = Some(main);
            self.tick_acts(world, env);
        }
    }

    /// A shout reached this object (FUN_1403ca950): if its act table has an
    /// act of that name, the act's timeout routine runs with this object's AI
    /// object and the shouter's.
    pub fn deliver_shout(&mut self, world: &mut World, env: &dyn Env, name: &[u8], shouter: u32) {
        let Some(index) = self.acts.find(name) else {
            return;
        };
        let timeout = self.acts.table[index].timeout;
        if timeout == 0 {
            return;
        }
        let args = [Slot::address(self.acts.ai_object), Slot::address(shouter)];
        self.run_callback(world, env, i32::from(timeout), &args);
    }

    /// FUN_1403ca550.
    fn tick_acts(&mut self, world: &mut World, env: &dyn Env) {
        if !self.acts.has_table {
            return;
        }
        // A shout that came with a delay arrives when its timer runs out.
        if let Some(pending) = self.acts.pending_shout.as_mut() {
            pending.timer -= world.frame_time;
            if pending.timer <= 0. {
                let done = self.acts.pending_shout.take().expect("pending shout");
                self.deliver_shout(world, env, &done.name, done.shouter);
            }
        }
        self.acts.flags |= act::JUST_STARTED;
        let mut highest = 0u16;
        loop {
            if !self.acts.locked() && self.sys.act_paused {
                return;
            }
            let mut actions = Vec::new();
            let started = self.acts.pop(&mut actions);
            self.apply_actions(Some((world, env)), actions);
            if !started && self.acts.flags & act::JUST_STARTED == 0 {
                return;
            }
            let flags = self.acts.flags;
            self.acts.flags &= !act::JUST_STARTED;
            if self.script.act_thread.is_none() {
                continue;
            }
            if flags & act::GATED != 0 {
                if self.sys.air_blocked() || self.sys.no_control() {
                    return;
                }
                self.acts.flags &= !act::GATED;
            }
            highest = highest.max(self.acts.priority);
            if highest < 300 && self.sys.hit_stunned() {
                return;
            }
            let generation = self.script.act_generation;
            let mut thread = self.script.act_thread.take().expect("act thread");
            run_thread(world, env, self, &mut thread);
            let replaced = generation != self.script.act_generation;
            let ended = thread.finished();
            if !replaced && !ended {
                self.script.act_thread = Some(thread);
            }
            if ended && self.acts.flags & act::JUST_STARTED == 0 {
                let mut actions = Vec::new();
                self.acts.thread_ended(&mut actions);
                self.apply_actions(Some((world, env)), actions);
            }
        }
    }

    /// One frame, in the executable's order: the motion clock and its events,
    /// the script (main thread, then acts), then root motion and physics.
    pub fn tick(&mut self, world: &mut World, env: &dyn Env) {
        self.sys.begin_tick(env);
        self.carry_out_motion_effects(world, env);
        self.tick_script(world, env);
        self.sys.end_tick(env);
    }

    /// What the motion events asked for: take-off, and the script triggers
    /// 0x10 (`id`, effect handle) and 0x11 (`p0`, `p1`).
    pub fn carry_out_motion_effects(&mut self, world: &mut World, env: &dyn Env) {
        let effects = std::mem::take(&mut self.sys.effects);
        for effect in effects {
            match effect {
                Effect::Takeoff => self.sys.take_off(env),
                Effect::EffectOrSound { id } => {
                    let handle = world.next_effect_handle();
                    world.events.push(Event::MotionEffect {
                        object: self.id,
                        id,
                        handle,
                    });
                    self.fire(
                        world,
                        env,
                        EVENT_EFFECT,
                        &[
                            Slot::address(object_handle(self.id)),
                            Slot::int(i32::from(id)),
                            Slot::address(handle),
                        ],
                    );
                }
                Effect::ScriptEvent { p0, p1 } => self.fire(
                    world,
                    env,
                    EVENT_SCRIPT,
                    &[
                        Slot::address(object_handle(self.id)),
                        Slot::int(i32::from(p0)),
                        Slot::int(i32::from(p1)),
                    ],
                ),
                Effect::SetSpeed { value } => self.sys.event_speed = f32::from(value) * 0.01,
                Effect::SetParameter { value } => self.sys.event_parameter = u32::from(value),
                Effect::PatternBit { word, bit } => {
                    if word == 0 {
                        self.sys.pattern_a |= 1 << bit;
                    } else {
                        self.sys.pattern_b |= 1 << bit;
                    }
                }
                Effect::Revenge => {
                    // [U] The revenge gauge compare (+0xd48 / +0xd4c) is not modeled.
                    world
                        .events
                        .push(Event::Unsupported("motion event revenge"));
                }
                Effect::Other {
                    range,
                    kind,
                    params,
                } => world.events.push(Event::MotionEvent {
                    object: self.id,
                    range,
                    kind,
                    params,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests;
