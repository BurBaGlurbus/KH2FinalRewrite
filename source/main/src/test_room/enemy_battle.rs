//! Ordinary source-bound enemies in the generated development arena.
//! Story flags, encounter scripts and unsupported variants stay outside this host.
use super::{
    Game, RoomWorld,
    enemy_battle_hits::ContactState,
    enemy_catalog::{Entry, Kind},
    enemy_combat::{AttackWindow, EnemyBinding, EnemyCombat, ShapePose},
    enemy_gallery::EnemyGallery,
};
use bevy::prelude::*;
use data::{motion::SelectedMotion, motion_eval, probe_source::ProbeSource};
use player::{
    World, actor_physics, combat, dusk,
    shadow::{self, ScriptRandom, Shadow},
    soldier,
};
use std::{collections::HashMap, sync::Arc};

pub enum Brain {
    /// The enemy's own object script, run by the translated interpreter.
    Script(Box<super::enemy_script::ScriptBrain>),
    Shadow(Box<Shadow>),
    Dusk(Box<dusk::Dusk>),
    Soldier(Box<soldier::Soldier>),
}
impl Brain {
    pub fn position(&self) -> [f32; 3] {
        match self {
            Self::Script(a) => a.position(),
            Self::Shadow(a) => a.position,
            Self::Dusk(a) => a.position,
            Self::Soldier(a) => a.position,
        }
    }
    pub fn yaw(&self) -> f32 {
        match self {
            Self::Script(a) => a.yaw(),
            Self::Shadow(a) => a.yaw,
            Self::Dusk(a) => a.yaw,
            Self::Soldier(a) => a.yaw,
        }
    }
    pub fn hp(&self) -> i32 {
        match self {
            Self::Script(a) => a.hp(),
            Self::Shadow(a) => a.hp,
            Self::Dusk(a) => a.hp,
            Self::Soldier(a) => a.hp,
        }
    }
    pub fn alive(&self) -> bool {
        match self {
            Self::Script(a) => a.alive(),
            Self::Shadow(a) => a.alive(),
            Self::Dusk(a) => a.alive(),
            Self::Soldier(a) => a.alive(),
        }
    }
    pub fn targetable(&self) -> bool {
        match self {
            Self::Script(a) => a.targetable(),
            Self::Shadow(a) => a.targetable(),
            Self::Dusk(a) => a.targetable(),
            Self::Soldier(a) => a.targetable(),
        }
    }
    pub fn record_mut(&mut self) -> &mut combat::HitRecord {
        match self {
            Self::Script(a) => a.record_mut(),
            Self::Shadow(a) => &mut a.record,
            Self::Dusk(a) => a.record(),
            Self::Soldier(a) => a.record(),
        }
    }
    pub fn hit(
        &mut self,
        d: [f32; 3],
        high: bool,
        p: &combat::AttackParams,
        s: &combat::AttackerStats,
        difficulty: combat::Difficulty,
    ) -> Option<i32> {
        match self {
            Self::Script(a) => a.hit(d, high, p, s, difficulty),
            Self::Shadow(a) => a.hit(d, high, p, s, difficulty),
            Self::Dusk(a) => a.hit(d, high, p, s, difficulty),
            Self::Soldier(a) => a.hit(d, high, p, s, difficulty),
        }
    }
    pub fn reflect(&mut self, motion: usize, back: f32) -> bool {
        match self {
            Self::Script(a) => {
                a.reflect(motion, back);
                true
            }
            Self::Soldier(a) => {
                a.reflect(motion, back);
                true
            }
            Self::Shadow(a) => {
                a.reflect(motion, back);
                true
            }
            Self::Dusk(a) => {
                a.reflect(motion, back);
                true
            }
        }
    }
    pub fn motion(&self) -> usize {
        match self {
            Self::Script(a) => a.motion(),
            Self::Shadow(a) => a.motion,
            Self::Dusk(a) => a.motion,
            Self::Soldier(a) => a.motion,
        }
    }
    pub fn elapsed(&self) -> u32 {
        match self {
            Self::Script(a) => a.elapsed(),
            Self::Shadow(a) => a.motion_tick,
            Self::Dusk(a) => a.motion_tick,
            Self::Soldier(a) => a.motion_tick,
        }
    }
    pub fn serial(&self) -> u64 {
        match self {
            Self::Script(a) => a.serial(),
            Self::Shadow(a) => a.motion_serial as u64,
            Self::Dusk(a) => a.motion_serial,
            Self::Soldier(a) => a.motion_serial,
        }
    }
    pub fn scale_y(&self) -> f32 {
        match self {
            Self::Script(a) => a.scale_y(),
            Self::Shadow(a) => a.scale_y,
            Self::Dusk(a) => a.scale_y,
            Self::Soldier(_) => 1.,
        }
    }
    pub fn blend(&self) -> f32 {
        match self {
            Self::Script(a) => a.blend(),
            Self::Shadow(_) => 0.,
            Self::Dusk(a) => a.blend,
            Self::Soldier(a) => a.blend,
        }
    }
    pub fn gone(&self) -> bool {
        match self {
            Self::Script(a) => a.gone(),
            Self::Shadow(a) => a.gone,
            Self::Dusk(a) => a.gone,
            Self::Soldier(a) => a.gone,
        }
    }
    pub fn fault(&self) -> Option<String> {
        match self {
            Self::Script(a) => a.fault.clone(),
            Self::Shadow(_) => None,
            Self::Dusk(a) => a.fault.map(|f| format!("{f:?}")),
            Self::Soldier(a) => a.fault.map(|f| format!("{f:?}")),
        }
    }
    pub fn description(&self) -> String {
        match self {
            Self::Script(a) => a.description(),
            Self::Shadow(a) => format!("{:?}", a.current_act()),
            Self::Dusk(a) => format!("{:?}", a.current_act()),
            Self::Soldier(a) => format!("{:?}", a.current_act()),
        }
    }
    pub fn defeat(&mut self) {
        match self {
            Self::Script(a) => a.defeat(),
            Self::Shadow(a) => a.defeat(),
            Self::Dusk(a) => a.defeat(),
            Self::Soldier(a) => a.defeat(),
        }
    }
    pub fn respawn(&mut self) {
        match self {
            Self::Script(a) => a.respawn(),
            Self::Shadow(a) => a.respawn(),
            Self::Dusk(a) => a.respawn(),
            Self::Soldier(a) => a.respawn(),
        }
    }
    pub fn capture(&mut self, key: u64, centre: [f32; 3], radius: f32, spin: f32) -> bool {
        match self {
            Self::Script(_) => false,
            Self::Shadow(a) => {
                a.capture(key, centre, radius, spin);
                true
            }
            Self::Dusk(_) | Self::Soldier(_) => false,
        }
    }
    /// The attack objects a script brain placed (none for the other brains).
    pub fn script_attacks(&self) -> Vec<super::enemy_script::ScriptAttack> {
        match self {
            Self::Script(a) => a.attacks(),
            _ => Vec::new(),
        }
    }
    /// A script attack struck Sora.
    pub fn mark_script_attack(&mut self, slot: usize) {
        if let Self::Script(a) = self {
            a.mark_attack_hit(slot);
        }
    }
    /// Sora's guard turned a script attack back.
    pub fn mark_script_attack_reflected(&mut self, slot: usize) {
        if let Self::Script(a) = self {
            a.mark_attack_reflected(slot);
        }
    }
    /// The attacks a script applied to Sora directly this frame
    /// (`attack_strike`): slot, parameter id and extra.
    pub fn take_script_strikes(&mut self) -> Vec<(usize, i32, i32)> {
        match self {
            Self::Script(a) => a.take_strikes(),
            _ => Vec::new(),
        }
    }
    /// The effects a script brain started this frame (none for other brains).
    pub fn take_script_effects(&mut self) -> Vec<super::enemy_script::ScriptEffect> {
        match self {
            Self::Script(a) => std::mem::take(&mut a.effects),
            _ => Vec::new(),
        }
    }
    pub fn follow_capture(&mut self, volumes: &[player::magic::Volume]) {
        if let Self::Shadow(a) = self
            && let Some(h) = a.held
        {
            if let Some(v) = volumes.iter().find(|v| v.key == h.key) {
                if let Some(held) = a.held.as_mut() {
                    held.centre = v.centre;
                }
            } else {
                a.release_held();
            }
        }
    }
}

pub(super) struct Clip {
    choice: usize,
    motion: Arc<SelectedMotion>,
    duration: u32,
    loop_top: u32,
    start: f32,
    fps: f32,
    roots: Vec<[f32; 3]>,
    windows: Vec<AttackWindow>,
    /// The clip's trigger tables as the enemy engine reads them.
    events: enemy::motion::MotionEvents,
}
fn native_clock(t: u32, duration: u32, loop_top: u32) -> u32 {
    if t < duration {
        t
    } else if loop_top < duration {
        loop_top + (t - duration) % (duration - loop_top)
    } else {
        duration
    }
}
fn native_frame_event(frame: u16, t: u32, duration: u32, loop_top: u32) -> bool {
    let frame = frame as u32;
    if t == 0 {
        return frame == 0;
    }
    let clock = native_clock(t, duration, loop_top);
    let step = if t == 2 || (loop_top == 0 && clock == 2) {
        2
    } else {
        1
    };
    let previous = native_clock(t.saturating_sub(step), duration, loop_top);
    if previous < clock {
        previous < frame && frame <= clock
    } else if previous > clock {
        (previous < frame && frame <= duration) || (loop_top < frame && frame <= clock)
    } else {
        false
    }
}
impl Clip {
    fn clock(&self, t: u32) -> u32 {
        native_clock(t, self.duration, self.loop_top)
    }
    fn key(&self, t: u32) -> f32 {
        self.clock(t) as f32 + self.start * 60. / self.fps
    }
    fn root(&self, t: u32) -> [f32; 3] {
        self.roots[self.clock(t) as usize]
    }
}
pub(super) struct Clips(HashMap<usize, Clip>);
impl shadow::Clips for Clips {
    fn advance(&self, _s: usize, t: u32) -> u32 {
        if t == 0 { 2 } else { t.saturating_add(1) }
    }
    fn root_step(&self, s: usize, from: u32, to: u32) -> [f32; 3] {
        let Some(c) = self.0.get(&s) else {
            return [0.; 3];
        };
        if to < from {
            [0, 1, 2].map(|i| {
                c.roots[c.duration as usize][i] - c.root(from)[i] + c.root(to)[i]
                    - c.roots[c.loop_top as usize][i]
            })
        } else {
            [0, 1, 2].map(|i| c.root(to)[i] - c.root(from)[i])
        }
    }
    fn length(&self, s: usize) -> u32 {
        self.0.get(&s).map_or(0, |c| c.duration)
    }
    fn loop_start(&self, s: usize) -> u32 {
        self.0.get(&s).map_or(0, |c| c.loop_top)
    }
    fn root(&self, s: usize, t: u32) -> [f32; 3] {
        self.0.get(&s).map_or([0.; 3], |c| c.root(t))
    }
}
impl enemy::env::Clips for Clips {
    fn length(&self, slot: usize) -> u32 {
        shadow::Clips::length(self, slot)
    }
    fn loop_start(&self, slot: usize) -> u32 {
        shadow::Clips::loop_start(self, slot)
    }
    fn root_delta(&self, slot: usize, tick: u32) -> [f32; 3] {
        dusk::Clips::root_delta(self, slot, tick)
    }
    fn advance(&self, slot: usize, tick: u32) -> u32 {
        dusk::Clips::advance(self, slot, tick)
    }
    fn events(&self, slot: usize) -> Option<&enemy::motion::MotionEvents> {
        self.0.get(&slot).map(|c| &c.events)
    }
    fn ended(&self, slot: usize, tick: u32) -> bool {
        dusk::Clips::ended(self, slot, tick)
    }
    fn in_loop(&self, slot: usize, tick: u32) -> bool {
        dusk::Clips::in_loop(self, slot, tick)
    }
}

impl dusk::Clips for Clips {
    fn takeoff(&self, s: usize, t: u32) -> Option<bool> {
        let c = self.0.get(&s)?;
        Some(
            c.motion
                .triggers
                .iter()
                .flat_map(|tr| &tr.records.frames)
                .any(|e| {
                    e.trigger == 0 && native_frame_event(e.frame as u16, t, c.duration, c.loop_top)
                }),
        )
    }
    fn gravity_frozen(&self, s: usize, t: u32) -> Option<bool> {
        soldier::Clips::no_gravity(self, s, t)
    }
    fn advance(&self, s: usize, t: u32) -> u32 {
        let at = self.0.get(&s).map_or(t, |c| c.clock(t));
        t.saturating_add(if at == 0 { 2 } else { 1 })
    }
    fn length(&self, s: usize) -> u32 {
        shadow::Clips::length(self, s)
    }
    fn loop_start(&self, s: usize) -> u32 {
        shadow::Clips::loop_start(self, s)
    }
    fn root(&self, s: usize, t: u32) -> [f32; 3] {
        shadow::Clips::root(self, s, t)
    }
    fn root_delta(&self, s: usize, t: u32) -> [f32; 3] {
        let Some(c) = self.0.get(&s) else {
            return [0.; 3];
        };
        let at = c.clock(t);
        let next = (if at == 0 { 2 } else { at + 1 }).min(c.duration);
        [0, 1, 2].map(|i| c.roots[next as usize][i] - c.roots[at as usize][i])
    }
    fn range(&self, s: usize, t: u32, r: u8) -> Option<bool> {
        let c = self.0.get(&s)?;
        let clock = c.clock(t);
        Some(
            c.motion
                .triggers
                .iter()
                .flat_map(|t| &t.records.ranges)
                .any(|e| {
                    e.trigger == 15 + r
                        && e.start_frame as u16 as u32 <= clock
                        && (e.end_frame as u16 == u16::MAX || clock < e.end_frame as u16 as u32)
                }),
        )
    }
    fn ended(&self, s: usize, t: u32) -> bool {
        self.0
            .get(&s)
            .is_some_and(|c| t > 0 && (c.clock(t - 1) + 1 >= c.duration))
    }
    fn in_loop(&self, s: usize, t: u32) -> bool {
        self.0.get(&s).is_some_and(|c| c.clock(t) >= c.loop_top)
    }
}

struct Arena<'a>(&'a RoomWorld);
impl soldier::Clips for Clips {
    fn no_gravity(&self, s: usize, t: u32) -> Option<bool> {
        let c = self.0.get(&s)?;
        let clock = c.clock(t);
        Some(
            c.motion
                .triggers
                .iter()
                .flat_map(|t| &t.records.ranges)
                .any(|r| {
                    matches!(r.trigger, 3 | 24 | 30)
                        && r.start_frame as u16 as u32 <= clock
                        && (r.end_frame as u16 == u16::MAX || clock < r.end_frame as u16 as u32)
                }),
        )
    }
}
impl shadow::Env for Arena<'_> {
    fn floor(&self, x: f32, z: f32, top: f32) -> Option<f32> {
        self.0.floor(x, z, top)
    }
}
impl dusk::Env for Arena<'_> {
    fn floor(&self, x: f32, z: f32, top: f32) -> Option<f32> {
        self.0.floor(x, z, top)
    }
    // The generated room has a flat floor and no step obstacles. Source
    // encounter step probes are not admitted by this development-room host.
    fn check_step(&self, _: [f32; 3], _: [f32; 3]) -> Option<u8> {
        Some(0)
    }
}

pub struct BattleActor {
    pub brain: Brain,
    pub assets: EnemyCombat,
    pub pose: ShapePose,
    pub motion: Arc<SelectedMotion>,
    pub frame: f32,
    pub serial: u64,
    pub windows: Vec<AttackWindow>,
    pub contacts: ContactState,
    clips: Arc<Clips>,
    pub paused: bool,
    pub error: Option<String>,
}
/// The supported enemies' objentry ids and model names.
pub const SUPPORTED: [(u32, &str); 3] = [(301, "M_EX010"), (302, "M_EX020"), (318, "M_EX990")];
pub fn supported(e: &Entry) -> bool {
    matches!(
        (e.object_id, e.model_name.as_str(), e.character),
        (301, "M_EX010", 1000) | (302, "M_EX020", 1001) | (318, "M_EX990", 3000)
    )
}
pub fn target_params(source: &ProbeSource) -> Result<player::targeting::Params, String> {
    source.verify_session().map_err(|e| e.to_string())?;
    let mut lib = data::path_lookup::Library::open(source).map_err(|e| e.to_string())?;
    let system = lib.read("03system.bin").map_err(|e| e.to_string())?;
    let pref = super::bar_entry(&system, b"pref").map_err(|e| e.to_string())?;
    let plyr = super::bar_entry(pref, b"plyr").map_err(|e| e.to_string())?;
    let offset = u32::from_le_bytes(
        plyr.get(8..12)
            .ok_or("Sora PLYR offset absent")?
            .try_into()
            .map_err(|_| "PLYR offset invalid")?,
    ) as usize;
    let params =
        player::targeting::Params::read(plyr.get(offset..).ok_or("PLYR record outside table")?)
            .ok_or("Sora PLYR record truncated")?;
    if [
        params.range,
        params.dy_min,
        params.dy_max,
        params.hold,
        params.height_weight,
        params.angle_weight,
    ]
    .iter()
    .any(|v| !v.is_finite())
    {
        return Err("PLYR target parameters nonfinite".into());
    }
    source.verify_session().map_err(|e| e.to_string())?;
    Ok(params)
}
pub fn lock_range(source: &ProbeSource) -> Result<f32, String> {
    let mut lib = data::path_lookup::Library::open(source).map_err(|e| e.to_string())?;
    let system = lib.read("03system.bin").map_err(|e| e.to_string())?;
    let pref = super::bar_entry(&system, b"pref").map_err(|e| e.to_string())?;
    let sstm = super::bar_entry(pref, b"sstm").map_err(|e| e.to_string())?;
    let offset = u32::from_le_bytes(
        sstm.get(4..8)
            .ok_or("SSTM record offset absent")?
            .try_into()
            .map_err(|_| "SSTM offset invalid")?,
    ) as usize;
    let at = offset
        .checked_add(0x4c)
        .ok_or("SSTM lock offset overflow")?;
    let range = f32::from_le_bytes(
        sstm.get(at..at + 4)
            .ok_or("SSTM lock range absent")?
            .try_into()
            .map_err(|_| "SSTM lock range invalid")?,
    );
    if !range.is_finite() || range <= 0. {
        return Err("SSTM lock range invalid".into());
    }
    Ok(range)
}
impl BattleActor {
    pub fn load(
        source: &ProbeSource,
        entry: &Entry,
        kind: &Kind,
        position: Vec3,
        yaw: f32,
        level: u16,
        difficulty: combat::Difficulty,
    ) -> Result<Self, String> {
        if !supported(entry) {
            return Err(
                "AI translation is available for ordinary Soldier, Shadow and Dusk only".into(),
            );
        }
        if kind.variants {
            return Err("contextual motion variants are not qualified for battle".into());
        }
        let binding = if entry.character == 3000 {
            EnemyBinding {
                enmp_id: 41,
                prize_id: Some(6),
            }
        } else if entry.character == 1000 {
            EnemyBinding {
                enmp_id: 33,
                prize_id: Some(1),
            }
        } else {
            EnemyBinding {
                enmp_id: 34,
                prize_id: Some(2),
            }
        };
        let assets = EnemyCombat::load(source, entry, kind, binding, level, difficulty)?;
        let mut clips = HashMap::new();
        for (choice, m) in kind
            .motions
            .iter()
            .enumerate()
            .filter(|(_, m)| m.status.is_none())
        {
            let motion = kind.load_motion(choice).map_err(|e| e.to_string())?;
            if motion.motion.raw.is_some() {
                return Err("raw native combat pose sampling is not qualified".into());
            }
            // An absent type-16 entry means no events, including the ordinary
            // Shadow/Soldier idle clips. It is not a malformed event table.
            if motion.triggers.len() > 1
                || motion.triggers.iter().any(|t| {
                    t.records.declared_frame_offset < 0
                        || t.records.declared_frame_offset as usize
                            != t.records.sequential_frame_offset
                })
            {
                return Err("native event table offset differs from admitted decoder".into());
            }
            let (start, _, fps, repeat) =
                motion.motion.frame_window().map_err(|e| e.to_string())?;
            let duration = motion.motion.raw.as_ref().map_or_else(
                || {
                    motion
                        .motion
                        .interpolated_header_words
                        .get(1)
                        .copied()
                        .unwrap_or(0)
                },
                |r| r.raw_header_words[4],
            );
            let loop_top = (repeat - start) * 60. / fps;
            if duration == 0
                || duration > 4096
                || !loop_top.is_finite()
                || loop_top < 0.
                || loop_top > duration as f32
                || loop_top.fract() != 0.
                || !start.is_finite()
                || start < 0.
                || !fps.is_finite()
                || fps <= 0.
            {
                return Err(format!(
                    "slot {} native clock outside admitted profile",
                    m.slot
                ));
            }
            let mut roots = Vec::new();
            for t in 0..=duration {
                let pose = motion_eval::evaluate(
                    &kind.model.skinning.bones,
                    &motion.motion,
                    t as f32 + start * 60. / fps,
                )
                .map_err(|e| e.to_string())?;
                if pose.root_translation.iter().any(|v| !v.is_finite()) {
                    return Err("nonfinite root motion".into());
                }
                roots.push(pose.root_translation);
            }
            let windows = assets.windows(&motion)?;
            let events = enemy::motion::MotionEvents {
                ranges: motion
                    .triggers
                    .iter()
                    .flat_map(|t| &t.records.ranges)
                    .map(|r| enemy::motion::RangeEvent {
                        start: r.start_frame as u16,
                        end: r.end_frame as u16,
                        kind: r.trigger,
                        params: r.parameters.iter().map(|&v| v as u16).collect(),
                    })
                    .collect(),
                frames: motion
                    .triggers
                    .iter()
                    .flat_map(|t| &t.records.frames)
                    .map(|f| enemy::motion::FrameEvent {
                        frame: f.frame as u16,
                        kind: f.trigger,
                        params: f.parameters.iter().map(|&v| v as u16).collect(),
                    })
                    .collect(),
            };
            clips.insert(
                m.slot,
                Clip {
                    choice,
                    motion,
                    duration,
                    loop_top: loop_top.round() as u32,
                    start,
                    fps,
                    roots,
                    windows,
                    events,
                },
            );
        }
        let clips = Arc::new(Clips(clips));
        let brain = if std::env::var("KH2RW_ENEMY_AI").is_ok_and(|v| v == "script") {
            Brain::Script(Box::new(super::enemy_script::ScriptBrain::load(
                source,
                &entry.model_name,
                position.to_array(),
                yaw,
                assets.stats,
                assets.sstm,
                clips.clone(),
            )?))
        } else if entry.character == 1000 {
            Brain::Soldier(Box::new(soldier::Soldier::new(
                position.to_array(),
                yaw,
                assets.stats,
                assets.sstm,
            )))
        } else if entry.character == 3000 {
            Brain::Dusk(Box::new(dusk::Dusk::new(
                position.to_array(),
                yaw,
                assets.stats,
                assets.sstm,
            )))
        } else {
            Brain::Shadow(Box::new(Shadow::new(
                position.to_array(),
                yaw,
                assets.stats,
                assets.sstm,
            )))
        };
        let c = clips
            .0
            .get(&brain.motion())
            .ok_or("initial source motion absent")?;
        let frame = c.key(0);
        let pose = assets.sample_shapes(kind, &c.motion, frame, position, yaw, false)?;
        Ok(Self {
            brain,
            motion: c.motion.clone(),
            windows: c.windows.clone(),
            assets,
            pose,
            frame,
            serial: 0,
            contacts: ContactState::default(),
            clips,
            paused: false,
            error: None,
        })
    }
    pub fn tick(
        &mut self,
        world: &RoomWorld,
        player: [f32; 3],
        kind: &Kind,
        rng: &mut ScriptRandom,
    ) -> Result<(usize, f32), String> {
        if !self.paused && !self.brain.gone() {
            match &mut self.brain {
                Brain::Script(a) => a.tick(world, player, rng),
                Brain::Shadow(a) => a.tick(rng, &*self.clips, &Arena(world), player, player),
                Brain::Dusk(a) => a.tick(
                    dusk::Input::arena(
                        player,
                        player,
                        actor_physics::ObjParams::defaults(self.assets.sstm.fall_max).turn,
                    ),
                    &*self.clips,
                    &Arena(world),
                    rng,
                ),
                Brain::Soldier(a) => a.tick(
                    dusk::Input::arena(
                        player,
                        player,
                        actor_physics::ObjParams::defaults(self.assets.sstm.fall_max).turn,
                    ),
                    &*self.clips,
                    &Arena(world),
                    rng,
                ),
            }
        }
        if let Some(f) = self.brain.fault() {
            return Err(f);
        }
        let c = self.clips.0.get(&self.brain.motion()).ok_or_else(|| {
            format!(
                "AI requested unsupported source motion {}",
                self.brain.motion()
            )
        })?;
        self.motion = c.motion.clone();
        self.serial = self.brain.serial();
        self.frame = c.clock(self.brain.elapsed()) as f32;
        self.windows = if self.brain.blend() > 0. {
            Vec::new()
        } else {
            c.windows.clone()
        };
        let key = c.key(if self.brain.blend() > 0. {
            0
        } else {
            self.brain.elapsed()
        });
        self.pose = self.assets.sample_shapes(
            kind,
            &self.motion,
            key,
            Vec3::from_array(self.brain.position()),
            self.brain.yaw(),
            false,
        )?;
        Ok((c.choice, key))
    }
}

pub fn tick(mut game: ResMut<Game>, mut gallery: ResMut<EnemyGallery>) {
    let tick = game.tick.saturating_sub(1);
    let basic_fk = gallery.basic_fk;
    let Some(rt) = game.enemy.as_mut() else {
        return;
    };
    // Scripts consume one shared stream. Summoning a new actor must not
    // restart that stream or give every actor an identical private seed.
    let mut rng = std::mem::take(&mut rt.rng);
    for actor in &mut gallery.actors {
        let Some(battle) = actor.battle.as_mut() else {
            continue;
        };
        if battle.error.is_some() {
            continue;
        }
        battle.paused = actor.frozen || game.cheats.enemy_frozen;
        battle
            .brain
            .follow_capture(game.magic.as_ref().map_or(&[], |m| m.volumes.as_slice()));
        if let Brain::Script(a) = &mut battle.brain {
            a.player_yaw = game.player.yaw;
        }
        let prior = (battle.brain.motion(), battle.brain.serial());
        match battle.tick(&game.world, game.player.position, &actor.kind, &mut rng) {
            Ok((choice, key)) => {
                actor.position = Vec3::from_array(battle.brain.position());
                actor.yaw = battle.brain.yaw();
                if let Err(e) = actor.battle_pose(choice, key, basic_fk) {
                    actor.error = Some(e)
                }
                super::enemy_motion_fx::scan(&mut game, actor);
                if let Some(battle) = actor.battle.as_mut() {
                    let object = battle.assets.object_id;
                    for e in battle.brain.take_script_effects() {
                        game.effect_queue.push(match e.at {
                            Some(at) => super::EffectRequest::ActorElementAt {
                                object,
                                id: e.id,
                                at: Vec3::from_array(at),
                            },
                            None => super::EffectRequest::ActorElement {
                                object,
                                actor: actor.id,
                                id: e.id,
                            },
                        });
                    }
                }
            }
            Err(e) => {
                battle.error = Some(e.clone());
                actor.error = Some(e.clone());
                game.event(serde_json::json!({"event":"enemy_battle_fault","tick":tick,"actor":actor.id,"reason":e}));
            }
        }
        if let Some(battle) = &actor.battle
            && prior != (battle.brain.motion(), battle.brain.serial())
        {
            game.event(serde_json::json!({"event":"enemy_battle_action","tick":tick,"actor":actor.id,"motion":battle.brain.motion(),"serial":battle.brain.serial(),"action":battle.brain.description(),"hp":battle.brain.hp(),"position":battle.brain.position()}));
        }
    }
    game.enemy.as_mut().expect("combat session retained").rng = rng;
    super::enemy_battle_hits::tick(&mut game, &mut gallery.actors, tick);
    super::enemy_battle_hits::retire(&mut game, &mut gallery.actors, tick);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_wraps_at_duration_into_the_source_return_window() {
        assert_eq!(native_clock(63, 64, 8), 63);
        assert_eq!(native_clock(64, 64, 8), 8);
        assert_eq!(native_clock(65, 64, 8), 9);
        assert_eq!(native_clock(120, 64, 8), 8);
        assert_eq!(native_clock(65, 64, 64), 64);
    }

    #[test]
    fn frame_events_cover_the_initial_skip_and_loop_end_once() {
        assert!(native_frame_event(0, 0, 10, 4));
        assert!(native_frame_event(1, 2, 10, 4));
        assert!(native_frame_event(2, 2, 10, 4));
        assert!(!native_frame_event(1, 3, 10, 4));
        assert!(native_frame_event(10, 10, 10, 4));
        assert!(!native_frame_event(4, 10, 10, 4));
        assert!(native_frame_event(5, 11, 10, 4));
        assert!(!native_frame_event(10, 11, 10, 4));
    }
}
