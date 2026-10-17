//! Synthetic programs only: every byte is authored here.
use super::*;
use crate::act::Entry;
use crate::attack::{Attack, flag};
use crate::env::NullEnv;
use crate::world::{ATTACK_HANDLES, PLAYER_OBJECT, Shout};

const CODE_START: usize = 14;

/// A program whose OBJ_INIT ends at once and whose callback (word `entry`)
/// stores its `arguments` in work words 0, 4, ... (the last argument pops
/// first).
fn callback_program(arguments: u16) -> (Vec<u8>, i32) {
    let mut words: Vec<u16> = Vec::new();
    // OBJ_INIT: exit.
    words.push(0x0049);
    let entry = CODE_START + words.len();
    for k in (0..arguments).rev() {
        words.extend([0x0041, 4 * k]);
    }
    words.push(0x0049);
    let mut b = vec![0u8; 16];
    b[..4].copy_from_slice(b"test");
    for v in [64u32, 64, 64] {
        b.extend(v.to_le_bytes());
    }
    for v in [0u32, CODE_START as u32, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    for w in words {
        b.extend(w.to_le_bytes());
    }
    (b, entry as i32)
}

fn work_word(obj: &Object, at: usize) -> u32 {
    u32::from_le_bytes(obj.script.instance.work[at..at + 4].try_into().unwrap())
}

fn spawn(arguments: u16) -> (World, Object, i32) {
    let (program, entry) = callback_program(arguments);
    let mut world = World::default();
    let obj = Object::spawn(&mut world, &NullEnv, 0, &program, Sysobj::bare()).unwrap();
    (world, obj, entry)
}

#[test]
fn a_struck_attack_calls_its_hit_callback_with_the_ai_object_victim_and_attack() {
    let (mut world, mut obj, entry) = spawn(3);
    let mut attack = Attack::new(Some(0), 7, 0, 0, 2);
    attack.hit_callback = Some((entry, 0x1234));
    attack.callback_object = Some(0);
    world.attacks.push(Some(attack));
    obj.attack_struck(&mut world, &NullEnv, 0, PLAYER_OBJECT);
    assert_eq!(
        work_word(&obj, 0),
        0x1234,
        "the AI object it was registered with"
    );
    assert_eq!(
        work_word(&obj, 4),
        crate::world::PLAYER_HANDLE,
        "the victim"
    );
    assert_eq!(work_word(&obj, 8), ATTACK_HANDLES, "the attack");
    let a = world.attacks[0].as_ref().unwrap();
    assert_ne!(a.flags & flag::STRUCK, 0);
}

#[test]
fn a_reflected_attack_calls_its_reflect_callback_with_the_attack_and_reflecter() {
    let (mut world, mut obj, entry) = spawn(3);
    let mut attack = Attack::new(Some(0), 7, 0, 0, 2);
    attack.reflect_callback = Some((entry, 0x77));
    attack.callback_object = Some(0);
    world.attacks.push(Some(attack));
    obj.attack_reflected(&mut world, &NullEnv, 0, PLAYER_OBJECT);
    assert_eq!(work_word(&obj, 0), 0x77);
    assert_eq!(work_word(&obj, 4), ATTACK_HANDLES);
    assert_eq!(work_word(&obj, 8), crate::world::PLAYER_HANDLE);
    let a = world.attacks[0].as_ref().unwrap();
    assert_ne!(a.flags & flag::REFLECTED, 0);
    assert_eq!(a.reflecter, Some(PLAYER_OBJECT));
    assert_eq!(a.reflect_count, 1);
}

#[test]
fn an_attack_without_a_callback_only_records_the_strike() {
    let (mut world, mut obj, _) = spawn(3);
    world.attacks.push(Some(Attack::new(Some(0), 7, 0, 0, 2)));
    obj.attack_struck(&mut world, &NullEnv, 0, 3);
    assert_ne!(world.attacks[0].as_ref().unwrap().flags & flag::STRUCK, 0);
    assert_eq!(work_word(&obj, 0), 0);
}

/// An object whose act table has `avoid`, with the callback as its timeout.
fn with_avoid(arguments: u16) -> (World, Object) {
    let (mut world, mut obj, entry) = spawn(arguments);
    obj.acts.init_table();
    obj.acts.ai_object = 0x55;
    obj.acts.add(Entry {
        name: b"avoid".to_vec(),
        timeout: entry as u16,
        ..Entry::default()
    });
    world.shouts.clear();
    (world, obj)
}

#[test]
fn a_shout_calls_the_timeout_routine_of_the_act_of_that_name_for_the_shouter_too() {
    let (mut world, mut obj) = with_avoid(2);
    world.shouts.push(Shout {
        name: b"avoid".to_vec(),
        shouter: 0,
        delay: 0.,
    });
    world.deliver_shouts(&NullEnv, Some(&mut obj));
    assert_eq!(work_word(&obj, 0), 0x55, "the receiver's AI object");
    assert_eq!(work_word(&obj, 4), object_handle(0), "the shouter");
    assert!(world.shouts.is_empty());
    // An act of another name hears nothing.
    let (mut world, mut other) = with_avoid(2);
    world.shouts.push(Shout {
        name: b"leave".to_vec(),
        shouter: 0,
        delay: 0.,
    });
    world.deliver_shouts(&NullEnv, Some(&mut other));
    assert_eq!(work_word(&other, 0), 0);
}

#[test]
fn a_delayed_shout_waits_a_random_share_of_its_delay() {
    let (mut world, mut obj) = with_avoid(2);
    world.shouts.push(Shout {
        name: b"avoid".to_vec(),
        shouter: 0,
        delay: 40.,
    });
    world.deliver_shouts(&NullEnv, Some(&mut obj));
    let pending = obj.acts.pending_shout.clone().expect("it waits");
    assert!((0. ..=40.).contains(&pending.timer));
    assert_eq!(work_word(&obj, 0), 0, "not yet");
    // The act loop counts the timer down and then calls the routine.
    for _ in 0..41 {
        obj.tick_acts(&mut world, &NullEnv);
    }
    assert!(obj.acts.pending_shout.is_none());
    assert_eq!(work_word(&obj, 0), 0x55);
}
