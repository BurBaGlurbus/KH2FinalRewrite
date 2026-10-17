//! A party member (Donald, Goofy) being hit, knocked down and revived: the
//! parts of the executable's friend object that differ from the enemy's.
//!
//! The hit pipeline is the shared BTLOBJ one (`FUN_1403d60c0`, reached
//! through `FUN_1401b0290` = FRIEND vt+0xc0): the record check, the HP
//! change and the reaction `FUN_1403d2fe0` (vt+0xc8). Only the motions
//! differ from Sora's: the flinches 10-13 (`DAT_14074c390`: front high,
//! front low, back high, back low), the air flinches 14/15
//! (`DAT_1405c63f8`), the blows 16 (front) and 18 (back) with the land
//! motions the flight ends on (blow + 1, `FUN_140422700`), the guard bounce
//! 41/43 (`FUN_1403d4650` with motion -1), the DEAD motion 54 and the
//! get-up 55.
//!
//! KO (`FUN_1403c33d0` through vt+0xb0 when the HP reaches 0): the dead bit
//! (`obj+0x9b8` bit 2) and the timer `obj+0xdc0` = PRTY word 6 (Donald 3600,
//! Goofy 2700) times the Hyper Healing factor. `FUN_1403c3660` counts it
//! down by SSTM+0x5c (8.0) a tick, or by 1.0 while the battle flag
//! (`DAT_142a11404`) is set; at 0 `FUN_1403c3bd0` heals `maxHP` times
//! SSTM+0x58 (0.5) through `FUN_1403d2eb0`, which for a dead party member
//! revives it (`FUN_1403c3c30`: the dead bit cleared, motion 55 from 54)
//! and scales the amount by SSTM+0x54 (0.5) unless the Hyper Healing flag
//! (`FUN_1403d7470(obj)+0x10` bit 3) is set. The DEAD motion is only
//! started once the member can act (`FUN_1403bfb30`: the motion's range 0-2
//! bits) and is on the ground.

use crate::actor_physics::{AIR_DRAG, Arc, Blow, KNOCK_SCALE, Knock, PHYSICS_STEP, Sampler, Sstm};
use crate::{Body, Hit, Vec3, World, combat};

/// The motion ids of the reactions.
pub mod motion {
    /// `DAT_14074c390`: front high, front low, back high, back low.
    pub const FLINCH: [u16; 4] = [10, 11, 12, 13];
    /// `DAT_1405c63f8`: air front, air back.
    pub const AIR: [u16; 2] = [14, 15];
    /// FUN_1403d2fe0 case 4: 0x10 for a hit from the front, 0x12 from
    /// behind; the flight lands on blow + 1 (FUN_140422700).
    pub const BLOW: [u16; 2] = [16, 18];
    /// The guard bounce (`FUN_1403d4650`, motion -1): ground and air.
    pub const BOUNCE: u16 = 41;
    pub const BOUNCE_AIR: u16 = 43;
    /// The land motion of the mode-1 landing (`obj+0x11c` = 5, set by
    /// `FUN_1403cbec0`), played after an air flinch.
    pub const LAND: u16 = 5;
    /// 0x36 and 0x37 (`FUN_1403c3660`, `FUN_1403c3c30`).
    pub const DEAD: u16 = 54;
    pub const REVIVE: u16 = 55;

    /// `FUN_1403d4880`: motions 10..=39.
    pub fn is_damage(motion: u16) -> bool {
        (10..40).contains(&motion)
    }

    /// `FUN_1403d48d0`: the reflect motions 0x8c..=0x96.
    pub fn is_reflect(motion: u16) -> bool {
        (0x8c..=0x96).contains(&motion)
    }

    /// `FUN_1403ca7c0` for a STDOBJ owner (`obj+0x6c8` bit 1, set by the
    /// STDOBJ constructor): the acts below priority 300 do not run.
    pub fn suspends_acts(motion: u16) -> bool {
        is_damage(motion) || is_reflect(motion)
    }
}

/// The SSTM rates of the KO and revive (`DAT_142ae5760` +0x54, +0x58,
/// +0x5c).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rates {
    /// +0x54: the share of a revive's heal that is kept without Hyper
    /// Healing.
    pub revive_keep: f32,
    /// +0x58: the share of the maximum HP the timer heals.
    pub revive_hp: f32,
    /// +0x5c: the KO timer's decrement per tick outside battle.
    pub ko_speed: f32,
}

impl Rates {
    pub fn read(record: &[u8]) -> Option<Self> {
        let f = |at: usize| {
            record
                .get(at..at + 4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        };
        Some(Rates {
            revive_keep: f(0x54)?,
            revive_hp: f(0x58)?,
            ko_speed: f(0x5c)?,
        })
    }

    /// `FUN_1403c3bd0`: `(int)(maxHP * SSTM+0x58)`.
    pub fn timer_heal(&self, max_hp: i32) -> i32 {
        (max_hp as f32 * self.revive_hp) as i32
    }

    /// `FUN_1403c3c30`: a heal on a dead member keeps SSTM+0x54 of its
    /// amount, all of it with Hyper Healing.
    pub fn revive_heal(&self, amount: i32, hyper_healing: bool) -> i32 {
        let k = if hyper_healing { 1. } else { self.revive_keep };
        (amount as f32 * k) as i32
    }

    /// `FUN_1403c3660`: what the KO timer loses in one tick.
    pub fn countdown(&self, battle: bool) -> f32 {
        if battle { 1. } else { self.ko_speed }
    }
}

/// `DAT_1405b532c`: the bounce's slide strength (-1 selects it).
const BOUNCE_SLIDE: f32 = 50.;

/// The step of a reaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// A ground flinch (class 2 or 3).
    Flinch,
    /// An air flinch and its hop (class 3 in the air).
    Air,
    /// A blow in flight (physics mode 4).
    Blow,
    /// The land motion after a blow.
    Land,
    /// The guard bounce (`FUN_1403d4650`).
    Bounce,
    /// Motion 55 after the revive.
    GetUp,
}

/// A reaction motion in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reaction {
    pub phase: Phase,
    pub motion: u16,
    pub tick: u32,
    knock: Option<Knock>,
    blow: Option<Blow>,
    sstm: Sstm,
    sampler: Sampler,
    arc: Option<Arc>,
    air: [f32; 2],
}

/// What a hit does to a member, from `FUN_1403d2fe0`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Started {
    pub reaction: Option<Reaction>,
    /// A knock-slide with no reaction motion (class 1).
    pub slide: Option<Knock>,
    /// The facing the reaction turns to (air flinch, blow).
    pub yaw: Option<f32>,
}

impl Started {
    fn nothing() -> Self {
        Started {
            reaction: None,
            slide: None,
            yaw: None,
        }
    }
}

/// What the room tells the reaction about the member's motion set and the
/// map.
pub struct Env<'a, W: World> {
    pub world: &'a W,
    pub body: Body,
    /// Ticks of a motion (None: not in the set).
    pub length: &'a dyn Fn(u16) -> Option<u32>,
    /// The motion's range 2 start, the tick the member can act again.
    pub actionable: &'a dyn Fn(u16) -> Option<u32>,
}

/// `FUN_1403d2fe0` for a friend object: which reaction a hit starts.
/// `forward` is the member's facing (sin, cos), `airborne` is
/// `FUN_1403ba9a0`, `has` is `FUN_1403c7d30` (the motion exists).
pub fn start(
    hit: &Hit,
    sstm: &Sstm,
    forward: [f32; 2],
    height: f32,
    airborne: bool,
    has: &dyn Fn(u16) -> bool,
) -> Started {
    let class = combat::reaction_class(hit.reaction);
    let original = combat::reaction_class(hit.original);
    let dir = hit.direction;
    // FUN_140140200 < 0: the hit travels against the facing, a hit from the
    // front.
    let back = forward[0] * dir[0] + forward[1] * dir[1] >= 0.;
    let facing = |back: bool| {
        let d = if back { dir } else { [-dir[0], -dir[1]] };
        d[0].atan2(d[1])
    };
    let slide = |strength: f32| Knock::new([dir[0], 0., dir[1]], strength, 0.);
    let strength = sstm.knock_strength(hit.knockback[0], original, hit.reaction);
    let base = Reaction {
        phase: Phase::Flinch,
        motion: 0,
        tick: 0,
        knock: None,
        blow: None,
        sstm: *sstm,
        sampler: Sampler::default(),
        arc: None,
        air: [0.; 2],
    };
    let flinch = |knock: Option<Knock>| {
        // FUN_1403d43b0: FRONT/BACK by the facing, HIGH/LOW by the sphere,
        // the partner motion when one is missing.
        let mut i = if back { 2 } else { 0 } + (!hit.high) as usize;
        if !has(motion::FLINCH[i]) {
            i ^= 1;
        }
        Started {
            reaction: Some(Reaction {
                motion: motion::FLINCH[i],
                knock,
                ..base
            }),
            slide: None,
            yaw: None,
        }
    };
    match class {
        1 => Started {
            slide: slide(strength),
            ..Started::nothing()
        },
        2 => flinch(None),
        3 if !airborne => flinch(slide(strength)),
        3 => {
            // FUN_1403d3840 + FUN_1403d4800: the air flinch, turned to the
            // attacker, and a hop.
            let mut i = back as usize;
            if !has(motion::AIR[i]) {
                i ^= 1;
            }
            let v = strength * sstm.hit_back_to_jump;
            Started {
                reaction: Some(Reaction {
                    phase: Phase::Air,
                    motion: motion::AIR[i],
                    arc: Some(Arc::launch(height, crate::AIR_HIT_HOP)),
                    air: [dir[0] * v, dir[1] * v],
                    ..base
                }),
                slide: None,
                yaw: Some(facing(back)),
            }
        }
        4 => {
            // KB1 up and KB2 along the hit, or SSTM BlowUp / BlowSpeed.
            let (up, side) = if original == 4 && (hit.knockback[0] != 0 || hit.knockback[1] != 0) {
                (hit.knockback[0] as f32, hit.knockback[1] as f32)
            } else {
                (sstm.blow_up, sstm.blow_speed)
            };
            let i = back as usize;
            if !has(motion::BLOW[i]) {
                return Started::nothing();
            }
            let v = [dir[0] * side, up, dir[1] * side];
            Started {
                reaction: Some(Reaction {
                    phase: Phase::Blow,
                    motion: motion::BLOW[i],
                    blow: Some(Blow::new(v, sstm)),
                    ..base
                }),
                slide: None,
                yaw: Some(facing(back)),
            }
        }
        _ => Started::nothing(),
    }
}

/// `FUN_1403d4650` (motion -1, strength -1 = 50): the guard bounce. The
/// member slides back along its own facing.
pub fn bounce(
    forward: [f32; 2],
    airborne: bool,
    sstm: &Sstm,
    has: &dyn Fn(u16) -> bool,
) -> Option<Reaction> {
    let motion = if airborne {
        motion::BOUNCE_AIR
    } else {
        motion::BOUNCE
    };
    if !has(motion) || airborne {
        return None;
    }
    Some(Reaction {
        phase: Phase::Bounce,
        motion,
        tick: 0,
        knock: Knock::new([-forward[0], 0., -forward[1]], BOUNCE_SLIDE, 0.),
        blow: None,
        sstm: *sstm,
        sampler: Sampler::default(),
        arc: None,
        air: [0.; 2],
    })
}

/// Motion 55 after a revive (`FUN_1403c3c30`).
pub fn get_up(sstm: &Sstm) -> Reaction {
    Reaction {
        phase: Phase::GetUp,
        motion: motion::REVIVE,
        tick: 0,
        knock: None,
        blow: None,
        sstm: *sstm,
        sampler: Sampler::default(),
        arc: None,
        air: [0.; 2],
    }
}

impl Reaction {
    /// One tick. Returns the new position and whether the reaction is over
    /// (the member can act again).
    pub fn step<W: World>(&mut self, mut position: Vec3, env: &Env<W>) -> (Vec3, bool) {
        self.tick += 1;
        match self.phase {
            Phase::Flinch | Phase::Bounce | Phase::Land | Phase::GetUp => {
                if let Some(k) = self.knock.as_mut() {
                    match k.step() {
                        Some(d) => {
                            position = env.world.collide(position, d, false, &env.body).position
                        }
                        None => self.knock = None,
                    }
                }
                let len = (env.length)(self.motion).unwrap_or(1);
                let ready = (env.actionable)(self.motion).unwrap_or(len).min(len);
                (position, self.tick >= ready)
            }
            Phase::Blow => {
                let fall_max = self.sstm.fall_max;
                if self.sampler.due(0.) {
                    let d = self
                        .blow
                        .as_mut()
                        .map_or([0.; 3], |b| b.step(&self.sstm, fall_max, PHYSICS_STEP));
                    self.sampler.store(d);
                }
                let d = self.sampler.displacement();
                let moved = env.world.collide(position, d, true, &env.body);
                position = moved.position;
                if moved.on_floor {
                    // Mode 4's land (FUN_140422700): blow + 1, no slide.
                    self.blow = None;
                    self.phase = Phase::Land;
                    self.motion += 1;
                    self.tick = 0;
                    self.sampler = Sampler::default();
                }
                (position, false)
            }
            Phase::Air => {
                let fall_max = self.sstm.fall_max;
                if self.sampler.due(0.) {
                    let dy = self
                        .arc
                        .as_mut()
                        .map_or(0., |a| a.step(position[1], fall_max, false, PHYSICS_STEP));
                    self.sampler.store([
                        self.air[0] * PHYSICS_STEP,
                        dy,
                        self.air[1] * PHYSICS_STEP,
                    ]);
                    self.air = self.air.map(|c| c * AIR_DRAG.powf(PHYSICS_STEP));
                }
                let d = self.sampler.displacement();
                let moved = env.world.collide(position, d, true, &env.body);
                position = moved.position;
                if moved.on_floor {
                    // The landing plays the land motion when the set has it.
                    if (env.length)(motion::LAND).is_some() {
                        self.phase = Phase::Land;
                        self.motion = motion::LAND;
                        self.tick = 0;
                        self.arc = None;
                        self.sampler = Sampler::default();
                        return (position, false);
                    }
                    return (position, true);
                }
                (position, false)
            }
        }
    }
}

/// The distance a ground knock-slide of `strength` covers: the speed
/// (strength x 0.1) decays by 0.9 a tick and stops below 1.
pub fn slide_distance(strength: f32) -> f32 {
    let mut v = strength * KNOCK_SCALE;
    let mut total = 0.;
    while v >= 1. {
        total += v;
        v *= 0.9;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat;
    impl World for Flat {
        fn floor(&self, _: f32, _: f32, top: f32) -> Option<f32> {
            (top >= 0.).then_some(0.)
        }
        fn resolve_walls(&self, p: [f32; 2], _: f32, _: f32, _: f32) -> [f32; 2] {
            p
        }
    }

    fn sstm() -> Sstm {
        let mut record = [0u8; 0x3c];
        let set =
            |r: &mut [u8; 0x3c], at: usize, v: f32| r[at..at + 4].copy_from_slice(&v.to_le_bytes());
        set(&mut record, 0x14, 16.);
        set(&mut record, 0x18, 0.8);
        set(&mut record, 0x1c, 0.1);
        set(&mut record, 0x20, 0.8);
        set(&mut record, 0x24, 24.);
        set(&mut record, 0x28, 40.);
        set(&mut record, 0x2c, 8.);
        set(&mut record, 0x30, 200.);
        set(&mut record, 0x34, 200.);
        set(&mut record, 0x38, 0.048);
        Sstm::read(&record).unwrap()
    }

    fn hit(reaction: u8, dir: [f32; 2], high: bool) -> Hit {
        Hit {
            reaction,
            original: reaction,
            direction: dir,
            high,
            knockback: [0x7fff, 0],
        }
    }

    const ALL: fn(u16) -> bool = |_| true;

    #[test]
    fn a_flinch_picks_front_or_back_and_high_or_low() {
        let s = sstm();
        // The member faces +z; a hit travelling toward -z comes from the
        // front.
        let motions = |dir: [f32; 2], high: bool| {
            start(&hit(3, dir, high), &s, [0., 1.], 0., false, &ALL)
                .reaction
                .unwrap()
                .motion
        };
        assert_eq!(motions([0., -1.], true), 10);
        assert_eq!(motions([0., -1.], false), 11);
        assert_eq!(motions([0., 1.], true), 12);
        assert_eq!(motions([0., 1.], false), 13);
    }

    #[test]
    fn a_missing_flinch_motion_falls_back_to_its_partner() {
        let s = sstm();
        let has = |m: u16| m != 10;
        let r = start(&hit(3, [0., -1.], true), &s, [0., 1.], 0., false, &has);
        assert_eq!(r.reaction.unwrap().motion, 11);
    }

    #[test]
    fn class_two_flinches_without_a_slide_and_class_three_slides() {
        let s = sstm();
        let two = start(&hit(2, [0., -1.], true), &s, [0., 1.], 0., false, &ALL);
        assert!(two.reaction.unwrap().knock.is_none());
        let three = start(&hit(3, [0., -1.], true), &s, [0., 1.], 0., false, &ALL);
        assert!(three.reaction.unwrap().knock.is_some());
        let one = start(&hit(1, [0., -1.], true), &s, [0., 1.], 0., false, &ALL);
        assert!(one.reaction.is_none() && one.slide.is_some());
    }

    #[test]
    fn a_blow_faces_the_attacker_and_flies_then_lands_on_the_next_motion() {
        let s = sstm();
        let h = Hit {
            reaction: 4,
            original: 4,
            direction: [0., -1.],
            high: true,
            knockback: [20, 2],
        };
        let started = start(&h, &s, [0., 1.], 0., false, &ALL);
        let mut r = started.reaction.unwrap();
        assert_eq!((r.phase, r.motion), (Phase::Blow, 16));
        // A front hit faces the negated hit direction: toward the attacker.
        assert!(started.yaw.unwrap().abs() < 1e-5);
        let env = Env {
            world: &Flat,
            body: Body {
                radius: 20.,
                height: 100.,
                step: 40.,
            },
            length: &|_| Some(60),
            actionable: &|_| Some(30),
        };
        let mut pos = [0., 0., 0.];
        let mut peak: f32 = 0.;
        for _ in 0..200 {
            let (p, _) = r.step(pos, &env);
            pos = p;
            peak = peak.max(pos[1]);
            if r.phase == Phase::Land {
                break;
            }
        }
        assert_eq!((r.phase, r.motion), (Phase::Land, 17));
        assert!(peak > 5.);
        assert!(pos[1].abs() < 1e-3);
        // The back blow lands on 19.
        let back = start(
            &Hit {
                direction: [0., 1.],
                ..h
            },
            &s,
            [0., 1.],
            0.,
            false,
            &ALL,
        );
        assert_eq!(back.reaction.unwrap().motion, 18);
    }

    #[test]
    fn an_air_flinch_hops_falls_and_lands_on_the_land_motion() {
        let s = sstm();
        let started = start(&hit(3, [0., -1.], true), &s, [0., 1.], 100., true, &ALL);
        let mut r = started.reaction.unwrap();
        assert_eq!((r.phase, r.motion), (Phase::Air, 14));
        // Turned to the attacker.
        assert!(started.yaw.is_some());
        let env = Env {
            world: &Flat,
            body: Body {
                radius: 20.,
                height: 100.,
                step: 40.,
            },
            length: &|_| Some(40),
            actionable: &|_| Some(20),
        };
        let mut pos = [0., 100., 0.];
        for _ in 0..300 {
            pos = r.step(pos, &env).0;
            if r.phase == Phase::Land {
                break;
            }
        }
        assert_eq!((r.phase, r.motion), (Phase::Land, 5));
        assert!(pos[1].abs() < 1e-3);
        // Without a land motion the reaction simply ends.
        let mut r = start(&hit(3, [0., -1.], true), &s, [0., 1.], 100., true, &ALL)
            .reaction
            .unwrap();
        let env = Env {
            length: &|m| (m != 5).then_some(40),
            ..env
        };
        let mut pos = [0., 100., 0.];
        let mut done = false;
        for _ in 0..300 {
            let (p, d) = r.step(pos, &env);
            pos = p;
            if d {
                done = true;
                break;
            }
        }
        assert!(done);
    }

    #[test]
    fn a_flinch_ends_at_the_actionable_tick() {
        let s = sstm();
        let mut r = start(&hit(3, [0., -1.], true), &s, [0., 1.], 0., false, &ALL)
            .reaction
            .unwrap();
        let env = Env {
            world: &Flat,
            body: Body {
                radius: 20.,
                height: 100.,
                step: 40.,
            },
            length: &|_| Some(70),
            actionable: &|_| Some(58),
        };
        let mut pos = [0.; 3];
        let mut done_at = None;
        for t in 1..=70 {
            let (p, done) = r.step(pos, &env);
            pos = p;
            if done && done_at.is_none() {
                done_at = Some(t);
            }
        }
        assert_eq!(done_at, Some(58));
        // The slide carries the member along the hit (toward -z).
        assert!(pos[2] < -30.);
    }

    #[test]
    fn the_bounce_slides_back_along_the_members_own_facing() {
        let s = sstm();
        let r = bounce([0., 1.], false, &s, &ALL).unwrap();
        assert_eq!((r.phase, r.motion), (Phase::Bounce, 41));
        let mut r = r;
        let env = Env {
            world: &Flat,
            body: Body {
                radius: 20.,
                height: 100.,
                step: 40.,
            },
            length: &|_| Some(60),
            actionable: &|_| Some(40),
        };
        let mut pos = [0.; 3];
        for _ in 0..40 {
            pos = r.step(pos, &env).0;
        }
        // Strength 50 x 0.1: about 41 units.
        assert!((pos[2] + slide_distance(50.)).abs() < 1e-3);
        assert!((slide_distance(50.) - 41.).abs() < 1.5);
        // No bounce in the air.
        assert!(bounce([0., 1.], true, &s, &ALL).is_none());
    }

    #[test]
    fn the_ko_timer_counts_down_eight_times_faster_outside_battle() {
        let rates = Rates {
            revive_keep: 0.5,
            revive_hp: 0.5,
            ko_speed: 8.,
        };
        let ticks = |battle: bool| (3600. / rates.countdown(battle)).ceil() as u32;
        assert_eq!(ticks(true), 3600);
        assert_eq!(ticks(false), 450);
    }

    #[test]
    fn a_revive_keeps_a_quarter_of_the_maximum_hp_without_hyper_healing() {
        let rates = Rates {
            revive_keep: 0.5,
            revive_hp: 0.5,
            ko_speed: 8.,
        };
        let amount = rates.timer_heal(80);
        assert_eq!(amount, 40);
        assert_eq!(rates.revive_heal(amount, false), 20);
        assert_eq!(rates.revive_heal(amount, true), 40);
    }

    #[test]
    fn rates_read_from_the_sstm_record() {
        let mut record = vec![0u8; 0x60];
        for (at, v) in [(0x54, 0.5f32), (0x58, 0.5), (0x5c, 8.)] {
            record[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        let rates = Rates::read(&record).unwrap();
        assert_eq!(
            (rates.revive_keep, rates.revive_hp, rates.ko_speed),
            (0.5, 0.5, 8.)
        );
        assert!(Rates::read(&record[..0x58]).is_none());
    }

    #[test]
    fn damage_motions_suspend_acts_and_others_do_not() {
        assert!(motion::suspends_acts(10) && motion::suspends_acts(39));
        assert!(motion::suspends_acts(140) && motion::suspends_acts(150));
        assert!(!motion::suspends_acts(40) && !motion::suspends_acts(54));
        assert!(!motion::suspends_acts(192));
    }
}
