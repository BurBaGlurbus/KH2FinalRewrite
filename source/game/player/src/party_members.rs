//! Party members (Donald, Goofy) as the executable's `kn::Friend` objects:
//! the follow behaviour of FUN_1401b41a0 / FUN_1401b4820, the slot
//! formation, and the attack choice of FUN_1401b1d40 with its action gate
//! FUN_1401b2880, on the shared movement rules.
//!
//! The thresholds are the personality resource's header floats read by
//! FUN_1401cc890 / FUN_1401cc8c0; the formation distance is not decoded
//! (`DAT_14071d760` is filled at run time) and is a test-room value. The
//! friend AI keeps no combo chain: each decision picks one eligible record
//! at random, never the same one three times in a row.
//!
//! Donald's four spells are records too (57-60, `FUN_1401cc650` /
//! `FUN_1401cc1a0`): they need the think loop's random MP-scaled permit
//! (`FUN_1401b53b0`) and an MP bar that is not charging, and play the act of
//! `party_spell`.

use crate::actor_physics::{Knock, Sstm};
use crate::party_hit;
use crate::{Body, Hit, Vec3, World};

/// Walk and run speeds per tick (PRTY words 0 and 1: 2.0 and 8.0).
pub const WALK_SPEED: f32 = 2.;
pub const RUN_SPEED: f32 = 8.;
/// Distances at which the gait steps up (from stand, from walk) and down
/// (to walk, to stand) [personality header pairs (0,200) (160,240)
/// (200,1000)].
const UP: [f32; 2] = [200., 240.];
const DOWN: [f32; 2] = [200., 160.];
/// Turn rate per tick (prty +0xc = 0.262).
const TURN: f32 = 0.262;
/// Slot yaw offset (the exe uses 0.349 × 2i − 0.349; widened here so the
/// two bodies do not overlap, as the formation table is not decoded) and
/// the formation distance behind the leader [test-room value].
const SLOT_YAW: f32 = 0.6;
const FORMATION: f32 = 150.;
/// Target search radius and the leash from the leader (personality +0x68,
/// +0x80).
pub const SEARCH_RADIUS: f32 = 800.;
pub const LEADER_LEASH: f32 = 1200.;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gait {
    Stand,
    Walk,
    Run,
}

/// The acts of the party's BDX scripts: Donald's `p_ex020` spells (`fire`,
/// `blizzard`, `thunder`, `cure`) and Goofy's `p_ex030` specials
/// (`tornado`, `turbo`; `blade`, his Bash, is not built).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spell {
    Fire,
    Blizzard,
    Thunder,
    Cure,
    Tornado,
    Turbo,
}

/// One action parameter entry (record of 0x80 bytes): the attack's motion
/// and its range window (FUN_1401b2880).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Record {
    pub id: u8,
    pub motion: u16,
    pub near: f32,
    pub far: f32,
    /// The target's height above the member must lie in
    /// `[low − r, high + r]` (the exe's Y-down window, negated).
    pub low: f32,
    pub high: f32,
    /// Air records jump first (+0x20 ≠ 0); not played yet.
    pub air: bool,
    /// MP cost (+8); 255 starts an MP Charge.
    pub cost: u8,
    /// The act this record starts instead of a weapon motion.
    pub spell: Option<Spell>,
    /// Cure: a member is healed while its HP is below this share of its
    /// maximum (+0x3c).
    pub heal_below: f32,
}

const fn ground(id: u8, motion: u16, far: f32) -> Record {
    Record {
        id,
        motion,
        near: 0.,
        far,
        low: -150.,
        high: 100.,
        air: false,
        cost: 0,
        spell: None,
        heal_below: 0.,
    }
}

/// A spell record: the act's motion, its range window (distances and the
/// Y-down height window negated into `low`/`high`) and MP cost.
const fn spell(id: u8, spell: Spell, motion: u16, window: [f32; 4], cost: u8) -> Record {
    Record {
        id,
        motion,
        near: window[0],
        far: window[1],
        low: window[2],
        high: window[3],
        air: false,
        cost,
        spell: Some(spell),
        heal_below: 0.,
    }
}

/// Donald's records (resource at 0x14071d850): the two ground attacks and
/// the spells 57-60 (Fire 0-350, Blizzard 250-750, Thunder 0-500 with
/// their height windows; Cure costs 255 and heals below 0.4 of the HP).
/// The act motions are 192 (Fire, Thunder, Cure) and 191 (Blizzard).
pub const DONALD: [Record; 6] = [
    ground(0, 151, 200.),
    ground(1, 152, 180.),
    spell(57, Spell::Fire, 192, [0., 350., -200., 100.], 10),
    spell(58, Spell::Blizzard, 191, [250., 750., -150., 100.], 10),
    spell(59, Spell::Thunder, 192, [0., 500., -1000., 500.], 10),
    Record {
        heal_below: 0.4,
        ..spell(60, Spell::Cure, 192, [0., 0., 0., 0.], 255)
    },
];
/// Goofy's records (personality 1): the two ground attacks and the
/// specials Tornado (id 92, range 0-500, height window -500/100 after the
/// negation) and Turbo (id 94, 0-500, -150/100), 10 MP each. Bash (id 93,
/// 250-750) needs the spawned shield object and is not built.
pub const GOOFY: [Record; 4] = [
    ground(4, 151, 220.),
    ground(5, 152, 300.),
    spell(92, Spell::Tornado, 191, [0., 500., -500., 100.], 10),
    spell(94, Spell::Turbo, 194, [0., 500., -150., 100.], 10),
];

/// The enemy a member fights: its centre and radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub position: Vec3,
    pub radius: f32,
}

/// An attack in progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attack {
    pub record: u8,
    pub motion: u16,
    pub tick: u32,
    /// The spell this attack is the cast of.
    pub spell: Option<Spell>,
    /// Cure: the party slot healed (0 Sora, 1 Donald, 2 Goofy).
    pub heal: u8,
}

/// The movement loop of Goofy's Tornado and Turbo (`p_ex030` L1967 and
/// L2924): `obj_set_movement(heading, speed)` each tick, the heading
/// turning toward the goal by at most 0.1745 per tick.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Special {
    heading: Vec3,
    /// Tornado's hover phase (+5 degrees a tick).
    spin: f32,
    /// Turbo's re-aim timer and count.
    timer: f32,
    count: u32,
    goal: Vec3,
    start_y: f32,
    started: bool,
}

impl Special {
    fn new() -> Self {
        Special {
            heading: [0., 1., 0.],
            spin: 0.,
            timer: 4.,
            count: 0,
            goal: [0.; 3],
            start_y: 0.,
            started: false,
        }
    }
}

/// A party member that follows the leader and fights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Follower {
    pub position: Vec3,
    pub yaw: f32,
    pub gait: Gait,
    /// Party slot 0 or 1 (obj+0xdd0).
    pub slot: u8,
    /// Ticks in the current motion.
    pub tick: u32,
    pub attack: Option<Attack>,
    /// The last three chosen records (obj+0xed0..0xee0).
    history: [u8; 3],
    seed: u32,
    /// The think loop's spell permit (obj+0xe18 bit 5), rolled each
    /// decision.
    pub permit: bool,
    special: Special,
    /// Downward speed while falling after a special.
    fall: f32,
    /// A hit reaction in progress (FUN_1403d2fe0): the member cannot act.
    pub reaction: Option<party_hit::Reaction>,
    /// A knock-slide that has no reaction motion (reaction class 1).
    slide: Option<Knock>,
    /// KO: the revive timer (obj+0xdc0) while the dead bit (obj+0x9b8 bit
    /// 2) is set.
    pub down: Option<f32>,
    /// The DEAD motion has started (`FUN_1403c3660`).
    laid: bool,
}

fn wrap(a: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    (a + std::f32::consts::PI).rem_euclid(tau) - std::f32::consts::PI
}

/// What a tick needs from the room.
pub struct Scene<'a, W: World> {
    pub leader: Vec3,
    pub leader_yaw: f32,
    /// The enemy to fight, when one can be hit and a fight is on.
    pub target: Option<Target>,
    pub world: &'a W,
    pub body: Body,
    /// Ticks of each attack motion (None: not loaded).
    pub length: &'a dyn Fn(u16) -> Option<u32>,
    /// Root-motion displacement of a motion at a tick (model space).
    pub root: &'a dyn Fn(u16, u32) -> Vec3,
    /// A spell act is still running (the act manager's lock, obj+0x4a8):
    /// no new decision.
    pub busy: bool,
    /// The AI may choose spells (the tour orders its casts instead).
    pub spells: bool,
    /// The enemy target's centre (`target_pos`), for the specials' goals.
    pub centre: Option<Vec3>,
    /// Sora's feet height (`trap_sysobj_player`), Tornado's hover reference.
    pub leader_y: f32,
    /// The motion's range 16 (`range_bit(obj, 1)`): the ticks the special
    /// moves in.
    pub range: &'a dyn Fn(u16) -> Option<(u32, u32)>,
    /// This member's MP: current, maximum and whether an MP Charge runs.
    pub mp: Option<(i32, i32, bool)>,
    /// HP and maximum HP of Sora, Donald and Goofy (None: absent).
    pub hp: [Option<(i32, i32)>; 3],
    /// The motion's range 2 start, the tick a reaction lets the member act
    /// again (`obj+0x18c` bits 2-4).
    pub actionable: &'a dyn Fn(u16) -> Option<u32>,
    /// SSTM (knock-back and blow constants) and the KO rates.
    pub sstm: Sstm,
    pub rates: party_hit::Rates,
    /// The battle flag (`DAT_142a11404`): the KO timer runs at 1.0 a tick
    /// instead of SSTM+0x5c.
    pub battle: bool,
}

impl Follower {
    /// FUN_1403c2fc0: at the leader's position, facing its direction.
    pub fn spawn(leader: Vec3, leader_yaw: f32, slot: u8) -> Self {
        Follower {
            position: leader,
            yaw: leader_yaw,
            gait: Gait::Stand,
            slot,
            tick: 0,
            attack: None,
            history: [255; 3],
            seed: 0x1234_5678 ^ (slot as u32 * 0x9e37_79b9),
            permit: false,
            special: Special::new(),
            fall: 0.,
            reaction: None,
            slide: None,
            down: None,
            laid: false,
        }
    }

    /// The motion id playing: an attack, or the gait (idle 0, walk 1,
    /// run 2).
    pub fn motion(&self) -> u16 {
        if let Some(r) = self.reaction {
            return r.motion;
        }
        if self.laid {
            return party_hit::motion::DEAD;
        }
        if let Some(a) = self.attack {
            return a.motion;
        }
        match self.gait {
            Gait::Stand => 0,
            Gait::Walk => 1,
            Gait::Run => 2,
        }
    }

    fn rand(&mut self, n: u32) -> u32 {
        // CRT-style LCG [the seed is unknown, U].
        self.seed = self.seed.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.seed >> 16) % n.max(1)
    }

    /// The goal point: behind the leader at the slot's yaw offset.
    pub fn goal(&self, leader: Vec3, leader_yaw: f32) -> Vec3 {
        let offset = SLOT_YAW * 2. * self.slot as f32 - SLOT_YAW;
        let a = leader_yaw + offset;
        [
            leader[0] - a.sin() * FORMATION,
            leader[1],
            leader[2] - a.cos() * FORMATION,
        ]
    }

    /// FUN_1401b2880's range test of one record against the target.
    pub fn eligible(&self, r: &Record, t: &Target) -> bool {
        if r.air {
            return false;
        }
        // Cure's record reads 0-0 as a range, which the original's check
        // would turn into "touching the enemy"; the cast waits on the
        // heal target instead [U: the record's flags were not decoded].
        if r.spell == Some(Spell::Cure) {
            return true;
        }
        let dx = t.position[0] - self.position[0];
        let dz = t.position[2] - self.position[2];
        let d = dx.hypot(dz);
        let rise = t.position[1] - self.position[1];
        let front = self.yaw.sin() * dx + self.yaw.cos() * dz >= 0.;
        d >= r.near - t.radius
            && d <= r.far + t.radius
            && rise >= r.low - t.radius
            && rise <= r.high + t.radius
            && front
    }

    /// Partner-in-need (obj+0xf78, FUN_1401b53b0): Sora when his HP is at
    /// most a quarter of the maximum, else the other member under it.
    fn partner_in_need(&self, hp: &[Option<(i32, i32)>; 3]) -> Option<usize> {
        let own = self.slot as usize + 1;
        [0, 3 - own]
            .into_iter()
            .find(|&i| hp[i].is_some_and(|(cur, max)| cur * 4 <= max))
    }

    /// FUN_1401cc1a0: the member Cure heals: the partner-in-need, else
    /// the first of itself, Sora and the other member whose HP is under
    /// `max × ratio` (FUN_1401b3310 truncates the product).
    pub fn heal_target(&self, ratio: f32, hp: &[Option<(i32, i32)>; 3]) -> Option<usize> {
        if let Some(i) = self.partner_in_need(hp) {
            return Some(i);
        }
        let own = self.slot as usize + 1;
        [own, 0, 3 - own]
            .into_iter()
            .find(|&i| hp[i].is_some_and(|(cur, max)| cur < (max as f32 * ratio) as i32))
    }

    /// The spell gate of FUN_1401b2880: a record with a cost needs the
    /// permit and an MP bar that is not charging; Cure also a member to
    /// heal.
    fn spell_ready<W: World>(&self, r: &Record, scene: &Scene<W>) -> bool {
        let Some(spell) = r.spell else { return true };
        if !scene.spells {
            return false;
        }
        if let Some((_, _, charging)) = scene.mp
            && r.cost >= 1
            && !(self.permit && !charging)
        {
            return false;
        }
        spell != Spell::Cure || self.heal_target(r.heal_below, &scene.hp).is_some()
    }

    /// FUN_1401b1d40 over the attack records: the random pick among the
    /// eligible ones, skipping a record chosen three times running.
    fn choose<W: World>(
        &mut self,
        records: &[Record],
        t: &Target,
        scene: &Scene<W>,
    ) -> Option<Record> {
        let candidates: Vec<Record> = records
            .iter()
            .copied()
            .filter(|r| {
                (scene.length)(r.motion).is_some()
                    && self.eligible(r, t)
                    && self.spell_ready(r, scene)
            })
            .collect();
        if candidates.is_empty() {
            return None;
        }
        let mut pick = candidates[self.rand(candidates.len() as u32) as usize];
        if self.history.iter().all(|&h| h == pick.id)
            && let Some(other) = candidates.iter().find(|r| r.id != pick.id)
        {
            pick = *other;
        }
        self.history = [self.history[1], self.history[2], pick.id];
        Some(pick)
    }

    /// Starts a record's motion toward the target (FUN_1401b7d30 for an
    /// attack, FUN_1403caa90 for a spell act).
    pub fn begin(&mut self, r: &Record, t: &Target, heal: u8) {
        let to = [
            t.position[0] - self.position[0],
            t.position[2] - self.position[2],
        ];
        if to[0].hypot(to[1]) > 1e-3 {
            self.yaw = to[0].atan2(to[1]);
        }
        self.attack = Some(Attack {
            record: r.id,
            motion: r.motion,
            tick: 0,
            spell: r.spell,
            heal,
        });
        self.gait = Gait::Stand;
        self.tick = 0;
        self.special = Special::new();
    }

    /// One tick of Tornado (`L1967`) or Turbo (`L2924`): the member waits for
    /// the motion's range 16, then moves along a heading it turns toward the
    /// goal; Tornado hovers over the target, Turbo dashes along the ground.
    fn special_step<W: World>(&mut self, a: &Attack, spell: Spell, scene: &Scene<W>) {
        let Some((lo, hi)) = (scene.range)(a.motion) else {
            return;
        };
        let sp = &mut self.special;
        let pos = self.position;
        if !sp.started {
            sp.started = true;
            sp.start_y = pos[1];
            if spell == Spell::Turbo {
                sp.heading = [self.yaw.sin(), 0., self.yaw.cos()];
                self.special.goal = self.random_ring(pos);
            }
        }
        let sp = &mut self.special;
        // The script waits for the range before it does anything.
        if a.tick < lo {
            if spell == Spell::Turbo
                && let Some(t) = scene.centre
            {
                // Range 22 turns the member toward the target.
                let d = [t[0] - pos[0], t[2] - pos[2]];
                if d[0].hypot(d[1]) > 1e-3 {
                    self.yaw = d[0].atan2(d[1]);
                }
            }
            return;
        }
        let ground = |x: f32, z: f32, top: f32| scene.world.floor(x, z, top);
        let mut home = true;
        let (goal, speed) = match spell {
            Spell::Tornado => {
                let hover = 65. + 35. * sp.spin.sin();
                match scene.centre {
                    Some(t) => {
                        let near = (t[0] - pos[0]).hypot(t[2] - pos[2]) < 150.;
                        if near {
                            ([t[0], scene.leader_y + hover, t[2]], 3.)
                        } else {
                            let g = ground(t[0], t[2], t[1] + 80.);
                            (
                                match g {
                                    Some(g) => [t[0], g + hover, t[2]],
                                    None => [t[0], t[1] + 80., t[2]],
                                },
                                10.,
                            )
                        }
                    }
                    None => (
                        [
                            pos[0] + self.yaw.sin() * 100.,
                            sp.start_y + hover,
                            pos[2] + self.yaw.cos() * 100.,
                        ],
                        2.,
                    ),
                }
            }
            _ => {
                // Turbo: the goal is the ground under the target; every 30
                // ticks a new search (one enemy here) and a new random point
                // 300-500 away, kept until the target is read
                // (FUN_1403f3fa0, drawn here from the member's own stream).
                if sp.timer >= 30. {
                    sp.count = if sp.count >= 5 { 0 } else { sp.count } + 1;
                    sp.timer = 0.;
                    self.special.goal = self.random_ring(pos);
                    home = false;
                } else if sp.timer >= 4. {
                    if let Some(t) = scene.centre {
                        let g = ground(t[0], t[2], t[1] + 80.);
                        self.special.goal = match g {
                            Some(g) => [t[0], g, t[2]],
                            None => t,
                        };
                    }
                } else {
                    home = false;
                }
                (self.special.goal, 15.)
            }
        };
        let sp = &mut self.special;
        let want = [goal[0] - pos[0], goal[1] - pos[1], goal[2] - pos[2]];
        if home {
            sp.heading = crate::magic::vector_homing(sp.heading, want, 0.174533);
        }
        if spell == Spell::Tornado {
            sp.spin = (sp.spin + 5f32.to_radians()).rem_euclid(std::f32::consts::TAU);
        } else {
            sp.timer += 1.;
        }
        if a.tick <= hi {
            let h = sp.heading;
            let flying = spell == Spell::Tornado;
            let disp = [
                h[0] * speed,
                if flying { h[1] * speed } else { 0. },
                h[2] * speed,
            ];
            let moved = scene
                .world
                .collide(self.position, disp, flying, &scene.body);
            self.position = moved.position;
            if !flying && h[0].hypot(h[2]) > 1e-3 {
                self.yaw = h[0].atan2(h[2]);
            }
        }
    }

    /// FUN_1403f3fa0: a point 300-500 from `pos` at a random bearing.
    fn random_ring(&mut self, pos: Vec3) -> Vec3 {
        let r = 300. + self.rand_unit() * 200.;
        let ang = self.rand_unit() * std::f32::consts::TAU;
        [pos[0] + ang.sin() * r, pos[1], pos[2] + ang.cos() * r]
    }

    fn rand_unit(&mut self) -> f32 {
        self.rand(10_000) as f32 / 10_000.
    }

    /// Whether the member is above the floor (`FUN_1403ba9a0`).
    pub fn airborne<W: World>(&self, world: &W) -> bool {
        world
            .floor(self.position[0], self.position[2], self.position[1])
            .is_some_and(|f| self.position[1] > f + 0.5)
    }

    /// The reaction a hit starts (`FUN_1403d2fe0` through FRIEND vt+0xc8):
    /// the member's attack or walk ends, its motion becomes the flinch or
    /// blow and the knock-slide begins. `has` tells which motions the
    /// member's set holds. Returns the motion that started.
    pub fn react<W: World>(
        &mut self,
        hit: &Hit,
        sstm: &Sstm,
        world: &W,
        has: &dyn Fn(u16) -> bool,
    ) -> Option<u16> {
        let started = party_hit::start(
            hit,
            sstm,
            [self.yaw.sin(), self.yaw.cos()],
            self.position[1],
            self.airborne(world),
            has,
        );
        if let Some(yaw) = started.yaw {
            self.yaw = yaw;
        }
        if started.slide.is_some() {
            self.slide = started.slide;
        }
        let r = started.reaction?;
        self.begin_reaction(r);
        Some(r.motion)
    }

    /// The guard bounce (`FUN_1403d4650` with motion -1).
    pub fn bounce<W: World>(
        &mut self,
        sstm: &Sstm,
        world: &W,
        has: &dyn Fn(u16) -> bool,
    ) -> Option<u16> {
        let r = party_hit::bounce(
            [self.yaw.sin(), self.yaw.cos()],
            self.airborne(world),
            sstm,
            has,
        )?;
        self.begin_reaction(r);
        Some(r.motion)
    }

    fn begin_reaction(&mut self, r: party_hit::Reaction) {
        self.attack = None;
        self.special = Special::new();
        self.gait = Gait::Stand;
        self.reaction = Some(r);
        self.tick = 0;
        self.laid = false;
    }

    /// HP reached 0 (`FUN_1403c33d0` through vt+0xb0): the dead bit and the
    /// revive timer (`prty[6]`).
    pub fn ko(&mut self, timer: f32) {
        self.down = Some(timer);
        self.attack = None;
        self.special = Special::new();
        self.laid = false;
    }

    /// The timer has run out (`FUN_1403c3660` calls `FUN_1403c3bd0`).
    pub fn revive_due(&self) -> bool {
        self.down.is_some_and(|t| t <= 0.)
    }

    /// A heal on the dead member (`FUN_1403c3c30`): the dead bit clears and
    /// motion 55 plays from 54.
    pub fn revive(&mut self, sstm: &Sstm) {
        if self.down.take().is_some() && self.laid {
            self.begin_reaction(party_hit::get_up(sstm));
        }
        self.laid = false;
    }

    /// One tick: finish or continue an attack, else choose one against the
    /// target, else approach the target or follow the leader.
    pub fn tick<W: World>(&mut self, records: &[Record], scene: &Scene<W>) {
        // The KO timer runs whatever the member is doing (FUN_1403c3660).
        if let Some(t) = self.down.as_mut() {
            *t -= scene.rates.countdown(scene.battle);
        }
        if let Some(k) = self.slide.as_mut() {
            match k.step() {
                Some(d) => {
                    self.position = scene
                        .world
                        .collide(self.position, d, false, &scene.body)
                        .position
                }
                None => self.slide = None,
            }
        }
        // A reaction locks the member until its motion lets it act.
        if let Some(mut r) = self.reaction {
            let env = party_hit::Env {
                world: scene.world,
                body: scene.body,
                length: scene.length,
                actionable: scene.actionable,
            };
            let (position, done) = r.step(self.position, &env);
            self.position = position;
            self.tick = r.tick;
            self.reaction = (!done).then_some(r);
            if done {
                self.tick = 0;
            }
            return;
        }
        // An attack plays out its motion with root motion, then ends.
        if let Some(mut a) = self.attack {
            a.tick += 1;
            if let Some(spell @ (Spell::Tornado | Spell::Turbo)) = a.spell {
                // The script moves the member; the motion's root does not.
                self.special_step(&a, spell, scene);
            } else {
                let (s, c) = self.yaw.sin_cos();
                let m = (scene.root)(a.motion, a.tick);
                let p = (scene.root)(a.motion, a.tick.saturating_sub(1));
                let (dx, dz) = (m[0] - p[0], m[2] - p[2]);
                let disp = [dx * c + dz * s, 0., -dx * s + dz * c];
                let moved = scene.world.collide(self.position, disp, false, &scene.body);
                self.position = moved.position;
            }
            self.tick = a.tick;
            let end = (scene.length)(a.motion).unwrap_or(0);
            self.attack = (a.tick < end).then_some(a);
            if self.attack.is_none() {
                self.tick = 0;
            }
            return;
        }
        // After a flying special the member falls to the floor (motion 4).
        if let Some(floor) = scene
            .world
            .floor(self.position[0], self.position[2], self.position[1])
            && self.position[1] > floor + 0.01
        {
            self.fall = (self.fall + 0.408).min(40.);
            let moved = scene
                .world
                .collide(self.position, [0., -self.fall, 0.], true, &scene.body);
            self.position = moved.position;
            return;
        }
        self.fall = 0.;
        // Down: the DEAD motion starts once the member is on the ground
        // and can act (FUN_1403c3660), and holds until the revive.
        if self.down.is_some() {
            self.attack = None;
            if self.laid {
                self.tick += 1;
            } else {
                self.laid = true;
                self.tick = 0;
            }
            return;
        }
        // A running spell act holds the member where it stands.
        if scene.busy {
            self.gait = Gait::Stand;
            self.tick += 1;
            return;
        }
        // The think loop's spell permit: MP share scaled (FUN_1401b53b0).
        if let Some((cur, max, _)) = scene.mp {
            let p = (cur as f32 / max.max(1) as f32) * 0.75 + 0.25;
            self.permit = self.rand(20001) as f32 / 20000. <= p;
        }
        // Decide: an eligible attack starts at once (FUN_1401b1d40).
        if let Some(t) = scene.target
            && let Some(r) = self.choose(records, &t, scene)
            && (scene.length)(r.motion).is_some()
        {
            let heal = if r.spell == Some(Spell::Cure) {
                self.heal_target(r.heal_below, &scene.hp).unwrap_or(0) as u8
            } else {
                0
            };
            self.begin(&r, &t, heal);
            return;
        }
        // Movement: toward the target in a fight, else behind the leader.
        let goal = match scene.target {
            Some(t) => t.position,
            None => self.goal(scene.leader, scene.leader_yaw),
        };
        let d = [goal[0] - self.position[0], goal[2] - self.position[2]];
        let dist = d[0].hypot(d[1]);
        let before = self.gait;
        self.gait = match self.gait {
            Gait::Stand if dist >= UP[0] => Gait::Walk,
            Gait::Walk if dist >= UP[1] => Gait::Run,
            Gait::Walk if dist <= DOWN[1] => Gait::Stand,
            Gait::Run if dist <= DOWN[0] => Gait::Walk,
            g => g,
        };
        // In a fight there is no standing at a distance: close in at a run.
        if scene.target.is_some() && self.gait != Gait::Run && dist > 60. {
            self.gait = Gait::Run;
        }
        if self.gait != before {
            self.tick = 0;
        } else {
            self.tick += 1;
        }
        let speed = match self.gait {
            Gait::Stand => 0.,
            Gait::Walk => WALK_SPEED,
            Gait::Run => RUN_SPEED,
        };
        if speed > 0. && dist > 1e-3 {
            let want = d[0].atan2(d[1]);
            let delta = wrap(want - self.yaw);
            self.yaw = wrap(self.yaw + delta.clamp(-TURN, TURN));
            let step = speed.min(dist);
            let disp = [self.yaw.sin() * step, 0., self.yaw.cos() * step];
            let moved = scene.world.collide(self.position, disp, false, &scene.body);
            self.position = moved.position;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Collided;

    struct Flat;
    impl World for Flat {
        fn floor(&self, _: f32, _: f32, _: f32) -> Option<f32> {
            Some(0.)
        }
        fn resolve_walls(&self, p: [f32; 2], _: f32, _: f32, _: f32) -> [f32; 2] {
            p
        }
        fn collide(&self, start: Vec3, d: Vec3, _: bool, _: &Body) -> Collided {
            Collided {
                position: [start[0] + d[0], 0., start[2] + d[2]],
                on_floor: true,
            }
        }
    }
    const BODY: Body = Body {
        radius: 30.,
        height: 140.,
        step: 24.,
    };
    fn length(m: u16) -> Option<u32> {
        matches!(m, 151 | 152).then_some(60)
    }
    fn no_root(_: u16, _: u32) -> Vec3 {
        [0.; 3]
    }
    fn no_actionable(_: u16) -> Option<u32> {
        None
    }
    pub(super) fn test_sstm() -> Sstm {
        let mut record = [0u8; 0x3c];
        for (at, v) in [
            (0x14usize, 16f32),
            (0x18, 0.8),
            (0x1c, 0.1),
            (0x20, 0.8),
            (0x24, 24.),
            (0x28, 40.),
            (0x2c, 8.),
            (0x30, 200.),
            (0x34, 200.),
            (0x38, 0.048),
        ] {
            record[at..at + 4].copy_from_slice(&v.to_le_bytes());
        }
        Sstm::read(&record).unwrap()
    }
    pub(super) fn test_rates() -> party_hit::Rates {
        party_hit::Rates {
            revive_keep: 0.5,
            revive_hp: 0.5,
            ko_speed: 8.,
        }
    }
    fn no_range(m: u16) -> Option<(u32, u32)> {
        match m {
            191 => Some((50, 380)),
            194 => Some((92, 330)),
            _ => None,
        }
    }
    fn scene(leader: Vec3, target: Option<Target>, world: &Flat) -> Scene<'_, Flat> {
        Scene {
            leader,
            leader_yaw: 0.,
            target,
            world,
            body: BODY,
            length: &length,
            root: &no_root,
            busy: false,
            spells: false,
            centre: None,
            leader_y: 0.,
            range: &no_range,
            mp: None,
            hp: [None; 3],
            actionable: &no_actionable,
            sstm: test_sstm(),
            rates: test_rates(),
            battle: true,
        }
    }

    #[test]
    fn a_distant_member_walks_then_runs_and_settles_at_the_goal() {
        let mut f = Follower::spawn([0., 0., 0.], 0., 0);
        let leader = [0., 0., 600.];
        let mut gaits = Vec::new();
        for _ in 0..400 {
            f.tick(&DONALD, &scene(leader, None, &Flat));
            if gaits.last() != Some(&f.gait) {
                gaits.push(f.gait);
            }
        }
        assert_eq!(gaits, vec![Gait::Walk, Gait::Run, Gait::Walk, Gait::Stand]);
        let goal = f.goal(leader, 0.);
        let d = (goal[0] - f.position[0]).hypot(goal[2] - f.position[2]);
        assert!(d < 200., "{d}");
    }

    #[test]
    fn the_two_slots_stand_on_either_side_behind_the_leader() {
        let a = Follower::spawn([0.; 3], 0., 0).goal([0.; 3], 0.);
        let b = Follower::spawn([0.; 3], 0., 1).goal([0.; 3], 0.);
        assert!(a[0] > 0. && b[0] < 0. || a[0] < 0. && b[0] > 0.);
        assert!(a[2] < 0. && b[2] < 0.);
    }

    #[test]
    fn a_member_closes_in_on_the_enemy_and_attacks_in_range() {
        let mut f = Follower::spawn([0.; 3], 0., 0);
        let t = Target {
            position: [0., 0., 500.],
            radius: 40.,
        };
        let mut started = None;
        for tick in 0..200 {
            f.tick(&DONALD, &scene([0.; 3], Some(t), &Flat));
            if f.attack.is_some() && started.is_none() {
                started = Some((tick, f.position[2]));
            }
        }
        let (_, z) = started.expect("an attack starts");
        // Within the longest record's reach plus the target's radius.
        assert!(500. - z <= 200. + 40. + 1., "{z}");
    }

    #[test]
    fn attacks_play_to_their_end_and_never_repeat_three_times() {
        let mut f = Follower::spawn([0., 0., 400.], 0., 0);
        let t = Target {
            position: [0., 0., 480.],
            radius: 40.,
        };
        let mut chosen = Vec::new();
        for _ in 0..3000 {
            let before = f.attack;
            f.tick(&DONALD, &scene([0.; 3], Some(t), &Flat));
            if before.is_none()
                && let Some(a) = f.attack
            {
                chosen.push(a.record);
            }
        }
        assert!(chosen.len() > 10, "{}", chosen.len());
        assert!(
            chosen
                .windows(4)
                .all(|w| !(w[0] == w[1] && w[1] == w[2] && w[2] == w[3]))
        );
        assert!(chosen.contains(&0) && chosen.contains(&1));
    }

    fn spell_length(m: u16) -> Option<u32> {
        matches!(m, 151 | 152 | 191 | 192).then_some(60)
    }

    fn spell_scene<'a>(
        t: Option<Target>,
        world: &'a Flat,
        mp: (i32, i32, bool),
        hp: [Option<(i32, i32)>; 3],
    ) -> Scene<'a, Flat> {
        Scene {
            leader: [0.; 3],
            leader_yaw: 0.,
            target: t,
            world,
            body: BODY,
            length: &spell_length,
            root: &no_root,
            busy: false,
            spells: true,
            centre: t.map(|t| t.position),
            leader_y: 0.,
            range: &no_range,
            mp: Some(mp),
            hp,
            actionable: &no_actionable,
            sstm: test_sstm(),
            rates: test_rates(),
            battle: true,
        }
    }

    #[test]
    fn spells_need_the_permit_and_an_mp_bar_that_is_not_charging() {
        let t = Target {
            position: [0., 0., 300.],
            radius: 40.,
        };
        // Full MP: the permit odds are 1; the member casts something.
        let mut f = Follower::spawn([0., 0., 100.], 0., 0);
        let mut spells = 0;
        for _ in 0..4000 {
            f.tick(
                &DONALD,
                &spell_scene(Some(t), &Flat, (100, 100, false), [Some((20, 20)); 3]),
            );
            if f.attack.is_some_and(|a| a.spell.is_some() && a.tick == 0) {
                spells += 1;
            }
        }
        assert!(spells > 5, "{spells}");
        // An MP Charge running: no spell is ever chosen.
        let mut f = Follower::spawn([0., 0., 100.], 0., 0);
        for _ in 0..4000 {
            f.tick(
                &DONALD,
                &spell_scene(Some(t), &Flat, (0, 100, true), [Some((20, 20)); 3]),
            );
            assert!(f.attack.is_none_or(|a| a.spell.is_none()));
        }
    }

    #[test]
    fn blizzard_waits_for_distance_and_fire_for_the_enemy_to_be_near() {
        // At 600 only Blizzard (250-750) is in range.
        let mut f = Follower::spawn([0., 0., 0.], 0., 0);
        f.permit = true;
        let t = Target {
            position: [0., 0., 600.],
            radius: 40.,
        };
        let scene = spell_scene(Some(t), &Flat, (100, 100, false), [Some((20, 20)); 3]);
        let ready: Vec<u8> = DONALD
            .iter()
            .filter(|r| f.eligible(r, &t) && f.spell_ready(r, &scene))
            .map(|r| r.id)
            .collect();
        assert_eq!(ready, vec![58]);
    }

    #[test]
    fn cure_heals_the_partner_in_need_then_itself_then_sora_then_the_other_member() {
        let f = Follower::spawn([0.; 3], 0., 0); // Donald: party slot 1
        // Sora at a quarter of his HP is the partner in need, whoever else
        // is hurt.
        assert_eq!(
            f.heal_target(0.4, &[Some((5, 20)), Some((1, 20)), Some((1, 20))]),
            Some(0)
        );
        // Else Donald himself under 0.4 (8 of 20).
        assert_eq!(
            f.heal_target(0.4, &[Some((20, 20)), Some((7, 20)), Some((20, 20))]),
            Some(1)
        );
        // Goofy at a quarter is the partner in need before Donald's 0.4.
        assert_eq!(
            f.heal_target(0.4, &[Some((20, 20)), Some((7, 20)), Some((5, 20))]),
            Some(2)
        );
        // Nobody under 0.4: no heal.
        assert_eq!(
            f.heal_target(0.4, &[Some((20, 20)), Some((20, 20)), Some((20, 20))]),
            None
        );
        // Sora alone under 0.4 but above a quarter (7 of 20).
        assert_eq!(
            f.heal_target(0.4, &[Some((7, 20)), Some((20, 20)), Some((20, 20))]),
            Some(0)
        );
    }

    #[test]
    fn turbo_waits_for_its_range_then_dashes_fifteen_a_tick_at_the_target() {
        let mut f = Follower::spawn([0.; 3], 0., 1);
        let t = Target {
            position: [0., 0., 6000.],
            radius: 40.,
        };
        f.begin(&GOOFY[3], &t, 0);
        let scene = |t: Target| Scene {
            centre: Some(t.position),
            length: &|m| matches!(m, 194).then_some(400),
            ..spell_scene(Some(t), &Flat, (100, 100, false), [None; 3])
        };
        let mut z_at = Vec::new();
        for _ in 0..400 {
            f.tick(&GOOFY, &scene(t));
            z_at.push(f.position[2]);
        }
        // Motionless before the range opens (attack tick 92 is index 91).
        assert!(z_at[..91].iter().all(|&z| z == 0.), "{}", z_at[90]);
        // Fifteen a tick inside it, straight along the target's bearing.
        assert!((z_at[92] - z_at[91] - 15.).abs() < 1e-3);
        // And it stops when the range ends (attack tick 330).
        assert_eq!(z_at[340], z_at[345]);
        assert!(z_at[345] > 3000.);
    }

    fn all_motions(_: u16) -> Option<u32> {
        Some(70)
    }
    fn ready_at_58(_: u16) -> Option<u32> {
        Some(58)
    }
    fn reaction_scene<'a>(
        world: &'a Flat,
        target: Option<Target>,
        battle: bool,
    ) -> Scene<'a, Flat> {
        Scene {
            leader: [0., 0., 0.],
            leader_yaw: 0.,
            target,
            world,
            body: BODY,
            length: &all_motions,
            root: &no_root,
            busy: false,
            spells: false,
            centre: None,
            leader_y: 0.,
            range: &no_range,
            mp: None,
            hp: [None; 3],
            actionable: &ready_at_58,
            sstm: test_sstm(),
            rates: test_rates(),
            battle,
        }
    }
    fn claw_hit() -> Hit {
        Hit {
            reaction: 3,
            original: 3,
            direction: [0., -1.],
            high: true,
            knockback: [0x7fff, 0],
        }
    }

    #[test]
    fn a_hit_ends_the_attack_and_locks_the_member_until_the_reaction_lets_it_act() {
        let t = Target {
            position: [0., 0., 100.],
            radius: 40.,
        };
        let mut f = Follower::spawn([0., 0., 0.], 0., 0);
        f.begin(&DONALD[0], &t, 0);
        assert!(f.attack.is_some());
        let sstm = test_sstm();
        let motion = f.react(&claw_hit(), &sstm, &Flat, &|m| m < 100);
        assert_eq!(motion, Some(10));
        assert!(f.attack.is_none());
        assert_eq!(f.motion(), 10);
        // An enemy stands in range, yet nothing is chosen while locked.
        for _ in 0..57 {
            f.tick(&DONALD, &reaction_scene(&Flat, Some(t), true));
            assert_eq!(f.motion(), 10);
            assert!(f.attack.is_none());
        }
        f.tick(&DONALD, &reaction_scene(&Flat, Some(t), true));
        assert!(f.reaction.is_none());
        // The slide carried the member along the hit, toward -z.
        assert!(f.position[2] < -30.);
        // Free again: the next tick decides.
        f.tick(&DONALD, &reaction_scene(&Flat, Some(t), true));
        assert!(f.attack.is_some() || f.gait != Gait::Stand);
    }

    #[test]
    fn a_ko_member_plays_its_reaction_then_lies_down_until_the_timer_and_a_heal() {
        let mut f = Follower::spawn([0., 0., 0.], 0., 0);
        let sstm = test_sstm();
        f.ko(3600.);
        f.react(&claw_hit(), &sstm, &Flat, &|m| m < 100);
        // The killing blow's reaction plays out first.
        for _ in 0..40 {
            f.tick(&DONALD, &reaction_scene(&Flat, None, false));
            assert_eq!(f.motion(), 10);
        }
        for _ in 0..40 {
            f.tick(&DONALD, &reaction_scene(&Flat, None, false));
        }
        assert_eq!(f.motion(), party_hit::motion::DEAD);
        // Outside battle the timer falls 8 a tick: 3600 lasts 450 ticks
        // from the KO.
        let mut ticks = 80;
        while !f.revive_due() {
            f.tick(&DONALD, &reaction_scene(&Flat, None, false));
            ticks += 1;
            assert!(ticks < 1000);
        }
        assert_eq!(ticks, 450);
        assert_eq!(f.motion(), party_hit::motion::DEAD);
        // The heal gets it up with motion 55, then it acts again.
        f.revive(&sstm);
        assert!(f.down.is_none());
        assert_eq!(f.motion(), party_hit::motion::REVIVE);
        for _ in 0..58 {
            f.tick(&DONALD, &reaction_scene(&Flat, None, false));
        }
        assert!(f.reaction.is_none());
        assert_eq!(f.motion(), 0);
    }

    #[test]
    fn the_ko_timer_runs_at_one_a_tick_in_battle() {
        let mut f = Follower::spawn([0., 0., 0.], 0., 0);
        f.ko(100.);
        for _ in 0..99 {
            f.tick(&DONALD, &reaction_scene(&Flat, None, true));
        }
        assert!(!f.revive_due());
        f.tick(&DONALD, &reaction_scene(&Flat, None, true));
        assert!(f.revive_due());
    }

    #[test]
    fn a_guard_bounce_slides_the_member_back_and_returns_it_to_the_ai() {
        let mut f = Follower::spawn([0., 0., 0.], 0., 0);
        let sstm = test_sstm();
        assert_eq!(f.bounce(&sstm, &Flat, &|_| true), Some(41));
        assert_eq!(f.motion(), 41);
        for _ in 0..70 {
            f.tick(&DONALD, &reaction_scene(&Flat, None, true));
        }
        assert!(f.reaction.is_none());
        // Facing +z, so the slide went toward -z.
        assert!(f.position[2] < -30.);
    }
}
