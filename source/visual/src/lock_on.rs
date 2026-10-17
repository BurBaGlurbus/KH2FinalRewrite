//! The lock-on reticle, translated from the executable's dk::LockOn object
//! (vtable 0x1405b24f8, a dk::Sprite3D, priority 4000, pool of 34 created by
//! FUN_14017c1e0): creation FUN_14017c570 / FUN_14017bc60, per-frame
//! position FUN_14017c520 and projection FUN_14016b350 / FUN_140166a20, scale
//! FUN_14017c630, and the lock manager's choice of group in FUN_1403dd770.
//!
//! The art is the `fiel` sequence's groups 1, 5, 23 and 27 (additive rings and
//! four corner marks). Auto target: group 5 for the primary HP pool, 1 for
//! others. Manual lock: 27 (blue) for the primary pool, 23 (red) for others.
//! A side panel (group 20, a second Sprite3D at the same point and scale) is
//! shown only for a flagged target (FUN_1403dd770 -> FUN_14017bf20).
use crate::gauge::Draw;
use data::sprite_sequence::{Modifiers, Player, Sequence};

/// The side panel's group (FUN_14017bc60 creates it with id 0x14).
const PANEL_GROUP: usize = 20;

/// Groups by (manual, primary pool): FUN_1403dd770 / FUN_1403dd650.
pub fn group_for(manual: bool, primary_pool: bool) -> usize {
    match (manual, primary_pool) {
        (false, false) => 1,
        (false, true) => 5,
        (true, false) => 0x17,
        (true, true) => 0x1b,
    }
}

/// Scale applied by FUN_14017c630: the projected depth times 1e-5 plus 0.4
/// (0x1405b2530, 0x1405b1d8c), kept within 0.5..=1.
fn scale_for(depth: i32) -> f32 {
    let scale = depth as f32 * 1.0e-5 + 0.4;
    let upper = if scale <= 1. { scale } else { 1. };
    if 0.5 <= upper { upper } else { 0.5 }
}

pub struct LockOn {
    group: usize,
    player: Player,
    /// Projected position truncated to integers, as the object stores it
    /// (+0x1c0, +0x1c4, +0x208).
    screen: [i32; 3],
    scale: f32,
}

impl LockOn {
    pub fn new(seq: &Sequence, group: usize) -> Self {
        Self {
            group,
            player: Player::new(seq, group),
            screen: [0, 0, 0],
            scale: 1.,
        }
    }

    pub fn group(&self) -> usize {
        self.group
    }

    /// FUN_14016b350: the target's projected screen position (layout units and
    /// depth) is truncated, the depth sets the scale, and the group plays one
    /// frame.
    pub fn update(&mut self, seq: &Sequence, screen: [f32; 3]) {
        self.screen = screen.map(|v| v as i32);
        self.scale = scale_for(self.screen[2]);
        self.player.update(seq, 1.);
    }

    /// Drawn only while the depth is within the projection's near and far
    /// bounds (FUN_14016abc0). The X origin is zero: the projection already
    /// is in layout units.
    pub fn draw(&self, seq: &Sequence, sheet: usize) -> Option<Draw> {
        let [x, y, depth] = self.screen;
        if !(1..=4_000_000).contains(&depth) {
            return None;
        }
        let modifiers = Modifiers {
            scale: self.scale,
            ..Modifiers::default()
        };
        Some(Draw::Quads {
            quads: seq.quads(&self.player, [x as f32, y as f32], &modifiers),
            z: 0,
            sheet,
        })
    }
}

/// The lock manager's reticle: created when a target is acquired, replaced
/// when the target or the lock state changes (the animation restarts), and
/// destroyed with the target. A release (FUN_14017bfe0) does not delete the
/// sprite: it restarts it on the group before its own (1 -> 0, 5 -> 4,
/// 0x17 -> 0x16, 0x1b -> 0x1a, ten frames each in the `fiel` sheet) and the
/// object deletes itself when that ends, still following its 3D point.
#[derive(Default)]
pub struct Reticle {
    shown: Option<Shown>,
    outros: Vec<Outro>,
}

struct Shown {
    key: u64,
    lock: LockOn,
    /// The side panel, created hidden and frozen: it plays only while shown.
    panel: LockOn,
    /// The target point in game space, for the outro.
    world: [f32; 3],
}

struct Outro {
    lock: LockOn,
    world: [f32; 3],
}

/// What the lock manager is holding this frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Held {
    /// Identity of the held target and shape; a change replaces the reticle.
    pub key: u64,
    pub manual: bool,
    pub primary_pool: bool,
    /// The target actor's flag +0x6c8 bit 1 with its counters +0xd1c or
    /// +0xd20 above zero (placement byte +0x31 of its spawn record): the
    /// side panel is shown. The test room's actors carry no such counters.
    pub flagged: bool,
    /// The manager's hide flag (+0x40 bit 0): no reticle is kept.
    pub hidden: bool,
}

impl Reticle {
    /// One frame: `held` carries the target shape centre in game space and
    /// `project` maps a game-space point through the camera as last applied
    /// to layout units and depth. Returns the drawable when anything shows.
    pub fn step(
        &mut self,
        seq: &Sequence,
        held: Option<(Held, [f32; 3])>,
        project: &dyn Fn([f32; 3]) -> [f32; 3],
        sheet: usize,
    ) -> Option<Draw> {
        let held = held.filter(|(held, _)| !held.hidden);
        let wanted = held.map(|(held, _)| {
            let group = group_for(held.manual, held.primary_pool);
            (held.key << 8 | group as u64, group)
        });
        // Same group, another target: FUN_14017bb80 re-creates the sprite at
        // once. Anything else retires the old one through its outro.
        let replaced = match (&self.shown, wanted) {
            (Some(shown), Some((key, _))) => shown.key != key,
            (Some(_), None) => true,
            _ => false,
        };
        if replaced && let Some(old) = self.shown.take() {
            let group = (old.key & 0xff) as usize;
            if wanted.is_none_or(|(_, new_group)| new_group != group) {
                let mut lock = LockOn::new(seq, group - 1);
                lock.screen = old.lock.screen;
                lock.scale = old.lock.scale;
                self.outros.push(Outro {
                    lock,
                    world: old.world,
                });
            }
        }
        let mut quads = Vec::new();
        let mut z = 0;
        let mut collect = |draw: Option<Draw>, sheet_out: &mut usize| {
            if let Some(Draw::Quads {
                quads: more,
                z: draw_z,
                sheet,
            }) = draw
            {
                z = draw_z;
                *sheet_out = sheet;
                quads.extend(more);
            }
        };
        let mut sheet_out = sheet;
        if let (Some((held, world)), Some((key, group))) = (held, wanted) {
            if self.shown.is_none() {
                self.shown = Some(Shown {
                    key,
                    lock: LockOn::new(seq, group),
                    panel: LockOn::new(seq, PANEL_GROUP),
                    world,
                });
            }
            if let Some(shown) = self.shown.as_mut() {
                shown.world = world;
                shown.lock.update(seq, project(world));
                // FUN_14017bf20: a flagged target shows and unfreezes the
                // panel, an unflagged one hides and freezes it.
                shown.panel.screen = shown.lock.screen;
                shown.panel.scale = shown.lock.scale;
                if held.flagged {
                    shown.panel.player.update(seq, 1.);
                }
                collect(shown.lock.draw(seq, sheet), &mut sheet_out);
                if held.flagged {
                    collect(shown.panel.draw(seq, sheet), &mut sheet_out);
                }
            }
        }
        // FUN_14016b3c0 deletes an outro sprite once its animation ends.
        self.outros.retain_mut(|outro| {
            outro.lock.update(seq, project(outro.world));
            if outro.lock.player.finished() {
                return false;
            }
            collect(outro.lock.draw(seq, sheet), &mut sheet_out);
            true
        });
        (!quads.is_empty()).then_some(Draw::Quads {
            quads,
            z,
            sheet: sheet_out,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use data::sprite_sequence::{Animation, AnimationGroup, Sprite, SpritePart};

    fn synthetic() -> Sequence {
        let animation = Animation {
            flags: data::sprite_sequence::IS_ACTIVE | data::sprite_sequence::LINEAR,
            sprite_group: 0,
            frames: [0, 69],
            translate: [0; 4],
            pivot: [0; 4],
            rotation: [0.; 6],
            scale: [1.; 2],
            scale_x: [1.; 2],
            scale_y: [1.; 2],
            curve: [0.; 4],
            bounce: [0.; 4],
            bounce_count: [0; 2],
            blend: 1,
            color: [0x8080_8080; 2],
        };
        let group = AnimationGroup {
            animations: 0..1,
            play_count: 0,
            loop_frames: [12, 0],
            light_position_x: 0,
            text_position_y: 0,
            text_scale: 0,
            ui_padding: 0,
            text_position_x: 0,
        };
        Sequence {
            sprites: vec![Sprite {
                uv: [0, 0, 10, 10],
                scroll: [0.; 2],
                colors: [0x8080_8080; 4],
            }],
            parts: vec![SpritePart {
                rect: [-14, -14, 14, 14],
                sprite: 0,
            }],
            sprite_groups: std::iter::once(0..1).collect(),
            animations: vec![animation],
            groups: vec![group; 40],
        }
    }

    #[test]
    fn groups_follow_the_lock_state_and_hp_pool() {
        assert_eq!(group_for(false, false), 1);
        assert_eq!(group_for(false, true), 5);
        assert_eq!(group_for(true, false), 23);
        assert_eq!(group_for(true, true), 27);
    }

    #[test]
    fn scale_grows_with_depth_between_one_half_and_one() {
        assert_eq!(scale_for(500), 0.5);
        assert_eq!(scale_for(10_000), 0.5);
        assert!((scale_for(40_000) - 0.8).abs() < 1e-6);
        assert_eq!(scale_for(1_000_000), 1.);
    }

    #[test]
    fn position_truncates_toward_zero_and_distant_points_are_not_drawn() {
        let seq = synthetic();
        let mut lock = LockOn::new(&seq, 27);
        lock.update(&seq, [300.9, 120.9, 800.9]);
        assert_eq!(lock.screen, [300, 120, 800]);
        let Some(Draw::Quads { quads, .. }) = lock.draw(&seq, 2) else {
            panic!("reticle quads")
        };
        // 28 wide at half scale, centred on the truncated position.
        assert!((quads[0].corners[0][0] - (300. - 7.)).abs() < 1e-3);
        lock.update(&seq, [0., 0., 0.5]);
        assert!(lock.draw(&seq, 2).is_none());
        lock.update(&seq, [0., 0., 5_000_000.]);
        assert!(lock.draw(&seq, 2).is_none());
    }

    #[test]
    fn the_reticle_restarts_on_a_new_target_and_vanishes_without_one() {
        let seq = synthetic();
        let mut reticle = Reticle::default();
        let held = |key| {
            Some((
                Held {
                    key,
                    manual: true,
                    primary_pool: true,
                    ..Held::default()
                },
                [256., 208., 600.],
            ))
        };
        assert!(reticle.step(&seq, held(1), &|p| p, 3).is_some());
        for _ in 0..30 {
            reticle.step(&seq, held(1), &|p| p, 3);
        }
        let time = reticle.shown.as_ref().unwrap().lock.player.time;
        assert!(time > 12.);
        reticle.step(&seq, held(2), &|p| p, 3);
        assert!(reticle.shown.as_ref().unwrap().lock.player.time < 2.);
        // A release retires the sprite through its outro (the synthetic outro
        // group loops, so it stays).
        reticle.step(&seq, None, &|p| p, 3);
        assert!(reticle.shown.is_none());
        assert_eq!(reticle.outros.len(), 1);
    }

    #[test]
    fn a_flagged_target_adds_the_side_panel_and_the_hide_flag_removes_the_reticle() {
        let seq = synthetic();
        let mut reticle = Reticle::default();
        let held = |flagged, hidden| {
            Some((
                Held {
                    key: 7,
                    manual: false,
                    primary_pool: true,
                    flagged,
                    hidden,
                },
                [256., 208., 600.],
            ))
        };
        let quads = |draw: Option<Draw>| match draw {
            Some(Draw::Quads { quads, .. }) => quads.len(),
            _ => 0,
        };
        let plain = quads(reticle.step(&seq, held(false, false), &|p| p, 3));
        assert_eq!(plain, 1);
        // The panel is frozen while hidden: it has not even started.
        assert_eq!(reticle.shown.as_ref().unwrap().panel.player.state, 1);
        // Shown, it starts on its first frame and then advances.
        assert_eq!(quads(reticle.step(&seq, held(true, false), &|p| p, 3)), 2);
        assert_eq!(quads(reticle.step(&seq, held(true, false), &|p| p, 3)), 2);
        assert!(reticle.shown.as_ref().unwrap().panel.player.time > 0.);
        let paused = reticle.shown.as_ref().unwrap().panel.player.time;
        assert_eq!(quads(reticle.step(&seq, held(false, false), &|p| p, 3)), 1);
        assert_eq!(reticle.shown.as_ref().unwrap().panel.player.time, paused);
        // The hide flag destroys the reticle, so the animation restarts after.
        reticle.step(&seq, held(false, true), &|p| p, 3);
        assert!(reticle.shown.is_none());
        // The restarted reticle and its panel draw next to the old outro.
        assert_eq!(quads(reticle.step(&seq, held(true, false), &|p| p, 3)), 3);
    }

    #[test]
    fn a_release_plays_the_group_before_and_a_same_class_retarget_does_not() {
        let mut seq = synthetic();
        // Group 26 (manual primary's outro) runs ten frames once.
        let mut short = seq.animations[0];
        short.frames = [0, 9];
        seq.animations.push(short);
        seq.groups[26] = AnimationGroup {
            animations: 1..2,
            play_count: 1,
            loop_frames: [0, 0],
            ..seq.groups[26].clone()
        };
        let mut reticle = Reticle::default();
        let held = |key| {
            Some((
                Held {
                    key,
                    manual: true,
                    primary_pool: true,
                    ..Held::default()
                },
                [256., 208., 600.],
            ))
        };
        let quads = |draw: Option<Draw>| match draw {
            Some(Draw::Quads { quads, .. }) => quads.len(),
            _ => 0,
        };
        reticle.step(&seq, held(1), &|p| p, 3);
        // Another target in the same class replaces the sprite at once.
        reticle.step(&seq, held(2), &|p| p, 3);
        assert!(reticle.outros.is_empty());
        // Releasing keeps drawing the outro for its ten frames, then removes it.
        let mut shown = 0;
        while quads(reticle.step(&seq, None, &|p| p, 3)) > 0 {
            shown += 1;
            assert!(shown < 30);
        }
        assert_eq!(shown, 10);
        assert!(reticle.outros.is_empty());
        // A change of class (auto -> manual) overlaps the outro with the new
        // sprite.
        let auto = Some((
            Held {
                key: 3,
                manual: false,
                primary_pool: true,
                ..Held::default()
            },
            [256., 208., 600.],
        ));
        reticle.step(&seq, auto, &|p| p, 3);
        assert_eq!(quads(reticle.step(&seq, held(3), &|p| p, 3)), 2);
    }
}
