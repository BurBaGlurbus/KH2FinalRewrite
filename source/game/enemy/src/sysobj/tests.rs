//! Synthetic objects only.
use super::*;
use crate::env::{Clips, Env};

struct Thirty;

impl Clips for Thirty {
    fn length(&self, _: usize) -> u32 {
        30
    }
    fn loop_start(&self, _: usize) -> u32 {
        0
    }
    fn root_delta(&self, _: usize, _: u32) -> Vec3 {
        [0.; 3]
    }
    fn events(&self, _: usize) -> Option<&crate::motion::MotionEvents> {
        None
    }
}

/// A flat floor at zero.
struct Flat;

impl Env for Flat {
    fn floor(&self, _: f32, _: f32, _: f32) -> Option<f32> {
        Some(0.)
    }
    fn clips(&self, _: u32) -> &dyn Clips {
        &Thirty
    }
    fn player_position(&self) -> Vec3 {
        [0.; 3]
    }
    fn target_position(&self, _: u32) -> Option<Vec3> {
        None
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1.0e-3
}

#[test]
fn hover_runs_the_steering_mode_but_is_neither_a_jump_nor_a_fly() {
    let mut s = Sysobj::bare();
    s.start_hover([3., 4., 0., 5.], 0.5);
    assert!(!s.is_jump());
    assert!(!s.is_fly());
    assert_eq!(s.mode_flags(), 0x100);
    assert!(!s.airborne, "mode 6 does not mark the object airborne");
    assert!(close(s.velocity[0], 3.) && close(s.velocity[1], 4.));
    // Mode 6 hands its update to mode 5's: the first run keeps half.
    s.end_tick(&Flat);
    assert!(close(s.velocity[0], 1.5) && close(s.velocity[1], 2.));
    // Mode 5 is a fly and not a hover.
    let mut f = Sysobj::bare();
    f.start_steer([1., 0., 0.], 1., 0.5);
    assert_eq!(f.mode_flags(), 0x102);
    assert!(f.is_fly());
}

#[test]
fn a_hop_starts_at_the_take_off_event_and_rises_with_the_executables_integration() {
    let mut s = Sysobj::bare();
    // Apex 100 in 20 ticks: g = 2h / t^2 = 0.5, rising speed g * t = 10.
    s.arm_hop([0., 100., 0., 0.], 20., 0.9);
    assert_eq!(s.pending_mode, 2);
    assert!(!s.is_jump(), "armed, not yet airborne");
    s.take_off(&Flat);
    assert!(s.is_jump());
    assert_eq!(s.mode_flags(), 0x101);
    let mut peak = 0f32;
    let mut ticks = 0;
    while s.is_jump() && ticks < 400 {
        s.end_tick(&Flat);
        peak = peak.max(s.position[1]);
        ticks += 1;
    }
    // The vertical speed is updated before it is applied (FUN_1403cbdd0), so
    // ten two-tick steps of 9, 8, ... 0 rise 90 of the 100 asked for.
    assert!(close(peak, 90.), "peak {peak}");
    assert!(!s.is_jump(), "it lands");
    assert!(close(s.position[1], 0.));
    assert!(!s.airborne);
}

#[test]
fn a_hop_is_capped_by_the_spec_fall_speed() {
    let mut s = Sysobj::bare();
    // A script writes the spec's maximum fall speed (+0x138) as it does for
    // the hop.
    assert!(s.poke_word(0x138, 3f32.to_bits()));
    s.position = [0., 500., 0.];
    s.arm_hop([0., 1., 0., 0.], 2., 0.9);
    s.take_off(&Flat);
    for _ in 0..40 {
        s.end_tick(&Flat);
    }
    let fall = s.moved[1].abs();
    assert!(fall <= 3.01, "falling {fall} per tick");
}

#[test]
fn a_jump_plays_its_fall_motion_near_the_apex_and_its_land_motion_on_touching_down() {
    let mut s = Sysobj::bare();
    s.launch_jump([0., 0., 0.], 100., 0.);
    s.jump_motions = JumpMotions { fall: 7, land: 8 };
    s.take_off(&Flat);
    assert!(s.is_jump());
    let mut seen = Vec::new();
    for _ in 0..400 {
        s.end_tick(&Flat);
        if seen.last() != Some(&s.motion.slot) {
            seen.push(s.motion.slot);
        }
        if !s.is_jump() {
            break;
        }
    }
    assert!(!s.is_jump());
    assert_eq!(
        seen,
        [0, 7, 8],
        "idle, the fall motion once, then the land motion"
    );
}

#[test]
fn arming_a_jump_resets_the_fall_and_land_motions() {
    let mut s = Sysobj::bare();
    s.jump_motions = JumpMotions { fall: 7, land: 8 };
    s.launch_jump([0., 0., 1.], 50., 4.);
    assert_eq!(s.jump_motions, JumpMotions { fall: -1, land: 5 });
    // With no fall motion the arc plays none; the land motion is 5.
    s.take_off(&Flat);
    let mut slots = vec![s.motion.slot];
    for _ in 0..400 {
        s.end_tick(&Flat);
        if slots.last() != Some(&s.motion.slot) {
            slots.push(s.motion.slot);
        }
        if !s.is_jump() {
            break;
        }
    }
    assert_eq!(slots, [0, 5]);
}
