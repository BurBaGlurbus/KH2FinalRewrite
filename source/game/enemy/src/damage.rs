//! Damage handling around a script object: the engine part of a hit
//! (FUN_140410d60: damage, reaction motion and physics) and the script
//! triggers it fires. The damage formula and reaction classes are the player
//! crate's translations; here they are applied to an [`Object`].
//!
//! [U] Which of the engine's other hit effects apply to a given enemy (guard
//! breaks, element status, multi-pool HP, special reaction cases) is not yet
//! translated; scripts that need them lack the traps listed by the coverage
//! tools.

use crate::env::Env;
use crate::object::DAMAGE;
use crate::script::Slot;
use crate::sysobj::{Reaction, Vec3, hlen};
use crate::traps::target::PLAYER_TARGET;
use crate::world::{Object, World, object_handle};
use player::{actor_physics, combat};

/// The damage record's handle (the argument OBJ_DAMAGE receives).
pub fn damage_handle(id: u32) -> u32 {
    object_handle(id) + 0x400
}

fn horizontal(v: Vec3) -> Vec3 {
    let l = hlen(v);
    if l > 0. {
        [v[0] / l, 0., v[2] / l]
    } else {
        [0.; 3]
    }
}

impl Object {
    /// Can this object be hit now.
    pub fn targetable(&self) -> bool {
        !self.sys.gone
            && self.sys.hp > 0
            && !self.acts.locked()
            && !self.sys.shadow_move
            && !self.sys.left
    }

    /// A hit from Sora. Returns the damage dealt, or `None` when the object
    /// cannot be hit.
    #[allow(clippy::too_many_arguments)]
    pub fn take_hit(
        &mut self,
        world: &mut World,
        env: &dyn Env,
        direction: Vec3,
        high: bool,
        attack: &combat::AttackParams,
        attacker: &combat::AttackerStats,
        difficulty: combat::Difficulty,
    ) -> Option<i32> {
        if !self.targetable() {
            return None;
        }
        let state = combat::TargetState {
            hp: self.sys.hp,
            difficulty,
            ..combat::TargetState::full(self.sys.stats.max_hp)
        };
        let damage = combat::damage_with(attacker, attack, &self.sys.stats, &state);
        let sys = &mut self.sys;
        sys.hp = (sys.hp - damage).max(0);
        sys.damage_reaction = u32::from(attack.reaction);
        sys.damage_element = attack.element;
        sys.damage_attack_type = attack.attack_type;
        sys.last_attacker = Some(PLAYER_TARGET);
        let back = sys.yaw.sin() * direction[0] + sys.yaw.cos() * direction[2] >= 0.;
        let class = combat::reaction_class(attack.reaction);
        let clips = env.clips(sys.model);
        if class == 5 {
            // Reaction 5 only captures; the host calls `capture`.
        } else if class == 4 {
            let [mut up, mut speed] = attack.knockback.map(f32::from);
            if up == 0. && speed == 0. {
                up = sys.sstm.blow_up;
                speed = sys.sstm.blow_speed;
            }
            let d = horizontal(direction);
            sys.blow = Some(actor_physics::Blow::new(
                [d[0] * speed, up, d[2] * speed],
                &sys.sstm,
            ));
            sys.knock = None;
            sys.jump = None;
            sys.hop = None;
            sys.steer = None;
            sys.fly = None;
            sys.pending_fly = None;
            sys.sampler.reset();
            sys.airborne = true;
            sys.reaction = Reaction::Blow { back };
            sys.motion_start(if back { 18 } else { 16 }, 0., clips);
        } else {
            let strength = sys
                .sstm
                .knock_strength(attack.knockback[0], class, attack.reaction);
            sys.knock = actor_physics::Knock::new(horizontal(direction), strength, 0.);
            sys.reaction = Reaction::Flinch;
            sys.motion_start(10 + (back as usize) * 2 + (!high) as usize, 0., clips);
        }
        if self.sys.hp == 0 {
            // FUN_140410af0: the engine starts the locked `dead` act.
            let actions = self.acts.start(b"dead");
            self.apply_actions(Some((world, env)), actions);
        }
        self.fire(world, env, DAMAGE, &[Slot::address(damage_handle(self.id))]);
        Some(damage)
    }

    /// A guarded attack bounced off (FUN_140411060 on the ground): play the
    /// attack's reflect motion, slide back, clear the acts and start the
    /// revenge act.
    pub fn reflect(
        &mut self,
        world: &mut World,
        env: &dyn Env,
        reflect_motion: usize,
        hit_back: f32,
    ) {
        if !self.targetable() {
            return;
        }
        let sys = &mut self.sys;
        sys.motion_start(reflect_motion, 0., env.clips(sys.model));
        sys.reaction = Reaction::Flinch;
        let strength = if hit_back < 0. { 50. } else { hit_back };
        sys.knock = if strength > 0. {
            actor_physics::Knock::new([-sys.yaw.sin(), 0., -sys.yaw.cos()], strength, 0.)
        } else {
            None
        };
        let clear = self.acts.clear();
        self.apply_actions(Some((world, env)), clear);
        let start = self.acts.start(b"mode_revenge");
        self.apply_actions(Some((world, env)), start);
    }
}
