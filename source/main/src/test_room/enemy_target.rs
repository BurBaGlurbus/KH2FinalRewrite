//! Stable live actor/shape targeting, the Scan gauge feed and the lock-on
//! reticle's inputs. The Scan gauge is the native GAUGE_ENEMY object
//! (visual::enemy_gauge) and the reticle the native dk::LockOn
//! (visual::lock_on).
use super::*;
use player::targeting::{self, Candidate, ManualCandidate, ManualInput, StableHolder, State};

pub const SCAN_ITEM: u16 = 138;
pub const SCAN_BIT: u16 = 19;

pub fn set_scan(game: &mut Game, camp: Option<&mut CampRes>, enabled: bool) -> Result<(), String> {
    if !game
        .ability_source
        .items
        .get(&SCAN_ITEM)
        .is_some_and(|&(bit, _)| bit == SCAN_BIT)
    {
        return Err("Selected-source Scan item/ability binding differs".into());
    }
    set_scan_slots(&mut game.ability_slots, enabled)?;
    if let Some(camp) = camp {
        camp.ability_slots.clone_from(&game.ability_slots);
    }
    drive_room::rebuild_abilities(game, None);
    Ok(())
}

fn set_scan_slots(slots: &mut [u16], enabled: bool) -> Result<(), String> {
    let mut found = false;
    for slot in slots.iter_mut().filter(|slot| **slot & 0x7fff == SCAN_ITEM) {
        *slot = SCAN_ITEM | if enabled { 0x8000 } else { 0 };
        found = true;
    }
    if found {
        Ok(())
    } else {
        Err("Scan is not present in the current learned ability inventory".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetId {
    Legacy(usize),
    Gallery(u64, usize),
}

#[derive(Resource, Default)]
pub struct Targeting {
    pub holder: StableHolder<TargetId>,
    pub limitation: Option<&'static str>,
    queued_cycle: Option<f32>,
}

impl Targeting {
    /// Preserve the native cycle direction if input is released before the
    /// next fixed simulation update consumes its queued request.
    pub fn queue_cycle(&mut self, logical_pressed: u64) {
        if let Some(direction) = cycle_direction(logical_pressed) {
            self.queued_cycle = Some(direction);
        }
    }
}

fn cycle_direction(logical: u64) -> Option<f32> {
    if logical & (1 << 36) != 0 {
        Some(-1.)
    } else if logical & (1 << 37) != 0 {
        Some(1.)
    } else {
        None
    }
}

pub struct Selection {
    pub shapes: Vec<player::ptya::Target>,
    pub target: Option<player::ptya::Target>,
    pub id: Option<TargetId>,
    pub locked: bool,
}

pub fn player_targetcentre(game: &Game) -> Option<[f32; 3]> {
    let Some(shape) = game.player_target_shape.as_ref() else {
        return Some(game.player.position);
    };
    let bone = usize::try_from(shape.bone).ok()?;
    let matrix = if bone == 0x4000 {
        Mat4::IDENTITY
    } else {
        let index = *game.poses.get(&(game.player.action, game.player.armed))?;
        let clip = game.clip_poses.get(index)?;
        let frame = game.player.motion_frame(&game.clips) as usize;
        *clip
            .body
            .get(frame.min(clip.body.len().saturating_sub(1)))?
            .get(&bone)?
    };
    let offset = Vec3::from_array(shape.position.map(|value| value as f32));
    let local = matrix.transform_point3(offset) + Vec3::Y * shape.position_height as f32;
    let centre =
        Vec3::from_array(game.player.position) + Quat::from_rotation_y(game.player.yaw) * local;
    centre.is_finite().then(|| centre.to_array())
}

pub fn candidates(game: &Game, gallery: &enemy_gallery::EnemyGallery) -> Vec<Candidate<TargetId>> {
    let mut out = Vec::new();
    if let Some(rt) = &game.enemy
        && rt.enabled
        && rt.enemy.targetable()
        && let Some(clip) = rt.clips.0.get(&rt.enemy.motion)
        && let Some(bones) = clip
            .bones
            .get((rt.enemy.motion_tick as usize).min(clip.bones.len().saturating_sub(1)))
    {
        let origin = Vec3::from_array(rt.enemy.position);
        let turn = Quat::from_rotation_y(rt.enemy.yaw);
        for (ordinal, &(bone, offset, radius, height)) in rt.targets.iter().enumerate() {
            if let Some(matrix) = bones.get(&bone) {
                out.push(Candidate {
                    key: TargetId::Legacy(ordinal),
                    shape: player::ptya::Target {
                        centre: (origin + turn * matrix.transform_point3(offset)).to_array(),
                        radius,
                        height,
                    },
                });
            }
        }
    }
    for actor in &gallery.actors {
        if let Some(battle) = &actor.battle
            && battle.error.is_none()
            && battle.brain.targetable()
        {
            out.extend(battle.pose.targets.iter().map(|shape| Candidate {
                key: TargetId::Gallery(actor.id, shape.ordinal),
                shape: player::ptya::Target {
                    centre: shape.centre.to_array(),
                    radius: shape.radius,
                    height: shape.height,
                },
            }));
        }
    }
    out
}

pub fn select(
    game: &Game,
    gallery: &enemy_gallery::EnemyGallery,
    state: &mut Targeting,
    toggle: bool,
    cycle: bool,
    dt: f32,
) -> Selection {
    let candidates = candidates(game, gallery);
    let cycle_direction = if cycle {
        state
            .queued_cycle
            .take()
            .or_else(|| cycle_direction(game.pad.pressed | game.pad.held))
    } else {
        None
    };
    state.limitation = None;
    if let Some(params) = &gallery.target_params {
        let mut manual = ManualInput {
            toggle,
            initial_choice: None,
            cycle_choice: None,
        };
        let initial = toggle && state.holder.state == State::None;
        if initial || (cycle && state.holder.state == State::Locked) {
            if let (Some(origin), Some(range)) = (player_targetcentre(game), gallery.lock_range) {
                let shapes = candidates
                    .iter()
                    .map(|&candidate| ManualCandidate {
                        candidate: Candidate {
                            key: candidate.key,
                            shape: player::ptya::Target {
                                centre: to_world(candidate.shape.centre),
                                ..candidate.shape
                            },
                        },
                        // Ordinary development-room actors are constructed without
                        // the native special range-bypass flag. Boss hosts differ.
                        bypass_range: false,
                    })
                    .collect::<Vec<_>>();
                if initial {
                    manual.initial_choice =
                        targeting::choose_manual(to_world(origin), range, &shapes);
                } else if let Some(current) = state.holder.target {
                    if let Some(direction) = cycle_direction {
                        manual.cycle_choice = targeting::cycle_manual(
                            to_world(origin),
                            range,
                            current,
                            direction,
                            &shapes,
                        );
                    } else {
                        state.limitation = Some("Manual cycle input producer unqualified");
                    }
                }
            } else {
                state.limitation = Some("Source manual target origin/range unavailable");
            }
        }
        state.holder.update(
            params,
            game.player.position,
            [game.player.yaw.sin(), game.player.yaw.cos()],
            &candidates,
            manual,
            dt,
        );
    } else {
        state.holder = StableHolder::default();
        state.limitation = Some("Selected-source player targeting parameters unavailable");
    }
    selection(state, &candidates)
}

fn selection(state: &Targeting, candidates: &[Candidate<TargetId>]) -> Selection {
    let index = state.holder.index(candidates);
    let target = index.map(|index| candidates[index].shape);
    let mut shapes = candidates
        .iter()
        .map(|candidate| candidate.shape)
        .collect::<Vec<_>>();
    if let Some(index) = index {
        shapes.swap(0, index);
    }
    Selection {
        shapes,
        target,
        id: state.holder.target,
        locked: state.holder.state == State::Locked,
    }
}

/// Source attack acquisition/hold, preserving a manual lock's stable identity.
pub fn acquire(
    game: &Game,
    gallery: &enemy_gallery::EnemyGallery,
    state: &mut Targeting,
) -> Selection {
    let candidates = candidates(game, gallery);
    if let Some(params) = &gallery.target_params {
        state.holder.acquire(
            params,
            game.player.position,
            [game.player.yaw.sin(), game.player.yaw.cos()],
            &candidates,
        );
        state.holder.hold(params);
    }
    selection(state, &candidates)
}

pub fn camera_anchor(
    game: &Game,
    gallery: &enemy_gallery::EnemyGallery,
    id: TargetId,
) -> Option<[f32; 3]> {
    match id {
        TargetId::Gallery(actor, ordinal) => {
            let battle = gallery
                .actors
                .iter()
                .find(|a| a.id == actor)?
                .battle
                .as_ref()?;
            // The lock reference is keyed by the held TARGET record's group:
            // the camera anchor is the kind-7 record with that group, else
            // the TARGET record itself.
            let target = battle.pose.targets.iter().find(|s| s.ordinal == ordinal)?;
            Some(
                battle
                    .pose
                    .camera
                    .iter()
                    .find(|shape| shape.group == target.group)
                    .unwrap_or(target)
                    .centre
                    .to_array(),
            )
        }
        TargetId::Legacy(ordinal) => game
            .enemy
            .as_ref()
            .and_then(|rt| enemy_targets(rt).get(ordinal).map(|s| s.centre)),
    }
}

/// The dynamic camera-collision spheres of the gallery's live actors
/// (FUN_140167c20 collects them from every actor each time the camera clips),
/// capped at the executable's list size.
pub fn camera_spheres(
    gallery: &enemy_gallery::EnemyGallery,
    to_world: impl Fn([f32; 3]) -> [f32; 3],
) -> Vec<follow_camera::Sphere> {
    gallery
        .actors
        .iter()
        .filter_map(|actor| actor.battle.as_ref())
        .flat_map(|battle| battle.pose.spheres.iter())
        .take(follow_camera::MAX_SPHERES)
        .map(|shape| follow_camera::Sphere {
            centre: to_world(shape.centre.to_array()),
            radius: shape.radius,
        })
        .collect()
}

/// The held target's actor still exists but can no longer be targeted: the
/// lock manager's release through FUN_1403ba260 (which plays se000 #8). A
/// vanished actor or a range release goes through FUN_1403dd280 silently.
pub fn untargetable_but_present(
    game: &Game,
    gallery: &enemy_gallery::EnemyGallery,
    id: TargetId,
) -> bool {
    match id {
        TargetId::Legacy(_) => game
            .enemy
            .as_ref()
            .is_some_and(|rt| rt.enabled && !rt.enemy.targetable()),
        TargetId::Gallery(actor, _) => gallery
            .actors
            .iter()
            .find(|a| a.id == actor)
            .and_then(|a| a.battle.as_ref())
            .is_some_and(|battle| !battle.brain.targetable()),
    }
}

/// A target's primary HP pool, live: current and maximum HP of an enabled,
/// targetable actor.
fn hp_pool(game: &Game, gallery: &enemy_gallery::EnemyGallery, id: TargetId) -> Option<(i32, i32)> {
    match id {
        TargetId::Legacy(_) => {
            let rt = game
                .enemy
                .as_ref()
                .filter(|rt| rt.enabled && rt.enemy.targetable())?;
            Some((rt.enemy.hp, rt.enemy.stats.max_hp))
        }
        TargetId::Gallery(id, _) => {
            let actor = gallery.actors.iter().find(|actor| actor.id == id)?;
            let battle = actor
                .battle
                .as_ref()
                .filter(|battle| battle.brain.targetable())?;
            Some((battle.brain.hp(), battle.assets.stats.max_hp))
        }
    }
}

/// Whether the shape's HP pool is the primary one (TARGET record byte 1 is 0).
fn primary_pool(gallery: &enemy_gallery::EnemyGallery, id: TargetId) -> bool {
    match id {
        TargetId::Gallery(actor, ordinal) => gallery
            .actors
            .iter()
            .find(|a| a.id == actor)
            .and_then(|a| a.battle.as_ref())
            .and_then(|battle| battle.pose.targets.iter().find(|s| s.ordinal == ordinal))
            .is_some_and(|shape| shape.parts == 0),
        TargetId::Legacy(_) => true,
    }
}

/// What FUN_1403dd440 would show for the current target: the owner has Scan
/// (ability word bit 19, equipped through the selected source's item table),
/// the target is a live actor, and the shape's HP pool is the primary one
/// (secondary pools of multi-part targets are not modelled and show nothing).
pub struct ScanView {
    pub id: TargetId,
    /// Manual lock (state 2) rather than the automatic target (state 1).
    pub manual: bool,
    pub hp: i32,
    pub max_hp: i32,
}

pub fn scan_view(
    game: &Game,
    gallery: &enemy_gallery::EnemyGallery,
    state: &Targeting,
) -> Option<ScanView> {
    let equipped = game
        .ability_source
        .items
        .get(&SCAN_ITEM)
        .is_some_and(|&(bit, _)| bit == SCAN_BIT)
        && game.player.abilities.has(SCAN_BIT);
    if !equipped {
        return None;
    }
    let id = state.holder.target?;
    if !primary_pool(gallery, id) {
        return None;
    }
    let (hp, max_hp) = hp_pool(game, gallery, id)?;
    Some(ScanView {
        id,
        manual: state.holder.state == State::Locked,
        hp,
        max_hp,
    })
}

/// The reticle's inputs (FUN_1403dd770): the held target's TARGET shape centre
/// in game world space and how it is held (auto or manual, primary pool).
pub fn reticle_view(
    game: &Game,
    gallery: &enemy_gallery::EnemyGallery,
    state: &Targeting,
) -> Option<(visual::lock_on::Held, [f32; 3])> {
    if state.holder.state == State::None {
        return None;
    }
    let id = state.holder.target?;
    hp_pool(game, gallery, id)?;
    let (centre, key) = match id {
        TargetId::Legacy(ordinal) => (
            game.enemy
                .as_ref()
                .and_then(|rt| enemy_targets(rt).get(ordinal).map(|s| s.centre))?,
            1 << 40 | ordinal as u64,
        ),
        TargetId::Gallery(actor, ordinal) => (
            gallery
                .actors
                .iter()
                .find(|a| a.id == actor)?
                .battle
                .as_ref()?
                .pose
                .targets
                .iter()
                .find(|s| s.ordinal == ordinal)
                .map(|s| s.centre.to_array())?,
            (actor & 0xff_ffff) << 16 | (ordinal as u64 & 0xffff),
        ),
    };
    Some((
        visual::lock_on::Held {
            key,
            manual: state.holder.state == State::Locked,
            primary_pool: primary_pool(gallery, id),
            // No supported actor carries the side-panel counters or the
            // manager's hide flag.
            ..visual::lock_on::Held::default()
        },
        to_world(centre),
    ))
}

/// Drives the gauge like the lock manager: a new target, or the same target
/// changing between the automatic and the manual lock (FUN_1403dcd70 releases
/// and re-creates it), re-creates it from the pool (FUN_1403dd440); the same target's
/// HP falling is a damage event (FUN_1403dcc10, with the trail) and rising a
/// heal (FUN_1403dcbb0).
#[derive(Default)]
pub struct ScanTracker {
    shown: Option<(TargetId, bool, i32)>,
}

impl ScanTracker {
    pub fn step(
        &mut self,
        gauge: &mut EnemyGauge,
        seq: &sprite_sequence::Sequence,
        slots: &visual::hud_slots::HudSlots,
        view: Option<ScanView>,
    ) {
        match (self.shown, view) {
            (_, None) => {
                if self.shown.take().is_some() {
                    gauge.hide();
                }
            }
            (Some((id, manual, hp)), Some(view)) if id == view.id && manual == view.manual => {
                match view.hp.cmp(&hp) {
                    std::cmp::Ordering::Less => gauge.damage(seq, view.hp),
                    std::cmp::Ordering::Greater => gauge.set_hp(seq, view.hp),
                    std::cmp::Ordering::Equal => {}
                }
                self.shown = Some((id, manual, view.hp));
            }
            (_, Some(view)) => {
                gauge.hide();
                gauge.show(seq, view.hp, view.max_hp);
                self.shown = Some((view.id, view.manual, view.hp));
            }
        }
        gauge.update(seq, slots);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_cycle_direction_survives_release_before_fixed_update() {
        let mut state = Targeting::default();
        state.queue_cycle(1 << 36);
        state.queue_cycle(0);
        assert_eq!(state.queued_cycle.take(), Some(-1.));
        assert_eq!(state.queued_cycle.take(), None);
        state.queue_cycle((1 << 36) | (1 << 37));
        assert_eq!(state.queued_cycle.take(), Some(-1.));
        state.queue_cycle(1 << 37);
        assert_eq!(state.queued_cycle.take(), Some(1.));
    }

    #[test]
    fn scan_toggle_preserves_other_slots_and_never_grants_unlearned_abilities() {
        let mut slots = [0x8001, SCAN_ITEM, 0x8123, SCAN_ITEM | 0x8000];
        set_scan_slots(&mut slots, false).unwrap();
        assert_eq!(slots, [0x8001, SCAN_ITEM, 0x8123, SCAN_ITEM]);
        set_scan_slots(&mut slots, true).unwrap();
        assert_eq!(
            slots,
            [0x8001, SCAN_ITEM | 0x8000, 0x8123, SCAN_ITEM | 0x8000]
        );
        let mut unlearned = [0x8001, 0, 0x8123];
        assert!(set_scan_slots(&mut unlearned, true).is_err());
        assert_eq!(unlearned, [0x8001, 0, 0x8123]);
    }

    fn flat_sequence() -> sprite_sequence::Sequence {
        let animation = sprite_sequence::Animation {
            flags: sprite_sequence::IS_ACTIVE | sprite_sequence::LINEAR,
            sprite_group: 0,
            frames: [0, 100],
            translate: [0; 4],
            pivot: [0; 4],
            rotation: [0.; 6],
            scale: [1.; 2],
            scale_x: [1.; 2],
            scale_y: [1.; 2],
            curve: [0.; 4],
            bounce: [0.; 4],
            bounce_count: [0; 2],
            blend: 0,
            color: [0x8080_8080; 2],
        };
        let group = sprite_sequence::AnimationGroup {
            animations: 0..1,
            play_count: 0,
            loop_frames: [0, 0],
            light_position_x: 0,
            text_position_y: 0,
            text_scale: 0,
            ui_padding: 0,
            text_position_x: 11,
        };
        sprite_sequence::Sequence {
            sprites: vec![sprite_sequence::Sprite {
                uv: [0, 0, 10, 10],
                scroll: [0.; 2],
                colors: [0x8080_8080; 4],
            }],
            parts: vec![sprite_sequence::SpritePart {
                rect: [0, 0, 10, 10],
                sprite: 0,
            }],
            sprite_groups: std::iter::once(0..1).collect(),
            animations: vec![animation],
            groups: vec![group; 47],
        }
    }

    fn view(actor: u64, hp: i32) -> Option<ScanView> {
        Some(ScanView {
            id: TargetId::Gallery(actor, 0),
            manual: false,
            hp,
            max_hp: 300,
        })
    }

    #[test]
    fn scan_tracker_follows_the_lock_managers_target_and_hp_events() {
        let seq = flat_sequence();
        let mut gauge = EnemyGauge::new(&seq, &mut visual::hud_slots::HudSlots::default());
        let mut tracker = ScanTracker::default();
        tracker.step(
            &mut gauge,
            &seq,
            &visual::hud_slots::HudSlots::default(),
            None,
        );
        assert!(!gauge.active());
        // A target appears: the gauge is created from its pool.
        tracker.step(
            &mut gauge,
            &seq,
            &visual::hud_slots::HudSlots::default(),
            view(1, 300),
        );
        assert!(gauge.active());
        assert!(!gauge.draw(&seq, 0).is_empty());
        // Damage to the same target is an event; the unchanged value is not.
        tracker.step(
            &mut gauge,
            &seq,
            &visual::hud_slots::HudSlots::default(),
            view(1, 100),
        );
        tracker.step(
            &mut gauge,
            &seq,
            &visual::hud_slots::HudSlots::default(),
            view(1, 100),
        );
        assert!(gauge.active());
        // A different target re-creates it, and no target releases it.
        tracker.step(
            &mut gauge,
            &seq,
            &visual::hud_slots::HudSlots::default(),
            view(2, 300),
        );
        assert!(gauge.active());
        tracker.step(
            &mut gauge,
            &seq,
            &visual::hud_slots::HudSlots::default(),
            None,
        );
        assert!(!gauge.active());
        assert!(gauge.draw(&seq, 0).is_empty());
    }
    #[test]
    fn switching_between_auto_and_manual_lock_recreates_the_gauge_without_a_trail() {
        let seq = flat_sequence();
        let slots = visual::hud_slots::HudSlots::default();
        let mut gauge = EnemyGauge::new(&seq, &mut visual::hud_slots::HudSlots::default());
        let mut tracker = ScanTracker::default();
        tracker.step(&mut gauge, &seq, &slots, view(1, 300));
        tracker.step(&mut gauge, &seq, &slots, view(1, 100));
        let trailing = gauge.draw(&seq, 0).len();
        let manual = |hp| view(1, hp).map(|v| ScanView { manual: true, ..v });
        tracker.step(&mut gauge, &seq, &slots, manual(100));
        assert_eq!(tracker.shown, Some((TargetId::Gallery(1, 0), true, 100)));
        // A fresh gauge at 100 HP draws what the tracker now draws, which is
        // fewer sprites than the damage trail's.
        let mut fresh = EnemyGauge::new(&seq, &mut visual::hud_slots::HudSlots::default());
        fresh.show(&seq, 100, 300);
        fresh.update(&seq, &slots);
        assert_eq!(gauge.draw(&seq, 0).len(), fresh.draw(&seq, 0).len());
        assert!(gauge.draw(&seq, 0).len() < trailing);
    }
}
