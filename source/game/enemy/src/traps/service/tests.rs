//! Synthetic objects and programs only.
use super::*;
use crate::env::NullEnv;
use crate::script::Memory;
use crate::traps::test_support::{Frame, ai_object, call, frame, vector, world_with};
use crate::world::{AppearRequest, Appeared, Object, Spawner, World, object_handle};

fn setup() -> (World, Object, Frame) {
    (World::default(), Object::bare(0), frame())
}

#[test]
fn a_light_fades_in_and_out_and_keeps_its_fog_and_flags() {
    let (mut world, mut me, mut f) = setup();
    let create = lookup(1, 181).unwrap();
    let light = call(&mut world, &mut me, &mut f, create, &[Slot::int(1)]).value;
    let slot = world.light_slot(light).expect("a light");
    assert_eq!(world.lights[slot].as_ref().unwrap().fade.value, 1.0);
    assert_eq!(world.lights[slot].as_ref().unwrap().color, [0x80; 4]);
    // Colour: r, g, b and a fade in over 4 ticks.
    let color = lookup(1, 183).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        color,
        &[
            Slot::address(light),
            Slot::int(10),
            Slot::int(20),
            Slot::int(30),
            Slot::float(4.),
        ],
    );
    assert_eq!(
        world.lights[slot].as_ref().unwrap().color,
        [10, 20, 30, 0x80]
    );
    assert_eq!(world.lights[slot].as_ref().unwrap().fade.value, 0.0);
    for _ in 0..4 {
        world.tick_attacks(&NullEnv);
    }
    assert_eq!(world.lights[slot].as_ref().unwrap().fade.value, 1.0);
    let fadeout = lookup(1, 184).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        fadeout,
        &[Slot::address(light), Slot::float(4.)],
    );
    world.tick_attacks(&NullEnv);
    assert_eq!(world.lights[slot].as_ref().unwrap().fade.value, 0.75);
    let fog = lookup(1, 258).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        fog,
        &[Slot::address(light), Slot::float(900.)],
    );
    assert_eq!(world.lights[slot].as_ref().unwrap().fog[1], 900.);
    let flag = lookup(1, 182).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        flag,
        &[Slot::address(light), Slot::int(3)],
    );
    assert_eq!(world.lights[slot].as_ref().unwrap().flags[0], 8);
}

#[test]
fn a_vacuum_is_made_shaped_and_freed() {
    let (mut world, mut me, mut f) = setup();
    let at = vector(&mut f, 0, [1., 2., 3., 1.]);
    let create = lookup(2, 77).unwrap();
    let vac = call(
        &mut world,
        &mut me,
        &mut f,
        create,
        &[Slot::address(at), Slot::float(7.)],
    )
    .value;
    let slot = world.vacuum_slot(vac).expect("a vacuum");
    let v = world.vacuums[slot].clone().unwrap();
    assert_eq!(v.at_18, [7.; 3]);
    assert_eq!((v.at_28, v.at_2c), (100., 0.01));
    let range = lookup(7, 32).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        range,
        &[Slot::address(vac), Slot::float(250.)],
    );
    let flag = lookup(2, 79).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        flag,
        &[Slot::address(vac), Slot::int(2)],
    );
    let v = world.vacuums[slot].clone().unwrap();
    assert_eq!((v.effective_range, v.flags), (250., 4));
    let destroy = lookup(2, 78).unwrap();
    call(&mut world, &mut me, &mut f, destroy, &[Slot::address(vac)]);
    assert!(world.vacuum_slot(vac).is_none());
}

#[test]
fn the_status_bits_scripts_set_show_in_the_leave_queries() {
    let (mut world, mut me, mut f) = setup();
    let query = crate::traps::lookup(1, 206).unwrap();
    assert_eq!(call(&mut world, &mut me, &mut f, query, &[]).as_int(), 0);
    let start = lookup(1, 204).unwrap();
    call(&mut world, &mut me, &mut f, start, &[]);
    assert_eq!(call(&mut world, &mut me, &mut f, query, &[]).as_int(), 1);
    let end = lookup(1, 205).unwrap();
    call(&mut world, &mut me, &mut f, end, &[]);
    assert_eq!(call(&mut world, &mut me, &mut f, query, &[]).as_int(), 0);
}

struct BareSpawner;

impl Spawner for BareSpawner {
    fn appear(&mut self, request: &AppearRequest) -> Option<Appeared> {
        (request.entry != 999).then(|| Appeared {
            payload: None,
            sys: crate::sysobj::Sysobj::bare(),
        })
    }
}

#[test]
fn sysobj_appear_makes_an_object_the_searches_then_find() {
    let (mut world, mut me, mut f) = setup();
    world.spawner = Some(Box::new(BareSpawner));
    let appear = crate::traps::lookup(1, 0).unwrap();
    let handle = call(&mut world, &mut me, &mut f, appear, &[Slot::int(1895)]).value;
    assert_eq!(handle, object_handle(1));
    assert_eq!(world.objects[1].as_ref().unwrap().sys.entry_id, 1895);
    // A search by entry id reaches it.
    let ai = ai_object(&mut f, 0, 0);
    let search = crate::traps::lookup(1, 114).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        search,
        &[Slot::address(ai), Slot::int(1895)],
    );
    let memory = Memory::new(&mut f.inst, &mut f.th);
    assert_eq!(memory.read_u32(ai + 4), Some(object_handle(1)));
    // An entry the host cannot make gives the script a null object.
    let none = call(&mut world, &mut me, &mut f, appear, &[Slot::int(999)]).value;
    assert_eq!(none, 0);
}

#[test]
fn each_all_walks_the_object_list_in_order() {
    let (_, mut me, mut f) = setup();
    let mut world = world_with(&[0., 0., 0.]);
    world.objects[2].as_mut().unwrap().sys.flags = 0x0008_0000; // hidden from the walk
    let each = lookup(1, 331).unwrap();
    let mut seen = Vec::new();
    let mut at = Slot::address(0);
    for _ in 0..8 {
        let next = call(&mut world, &mut me, &mut f, each, &[at]);
        if next.value == 0 {
            break;
        }
        seen.push(crate::world::object_from_handle(next.value).unwrap());
        at = next;
    }
    assert_eq!(
        seen,
        [0, 1, 3],
        "the caller first, the hidden object skipped"
    );
}

#[test]
fn enemy_each_walks_only_the_enemy_class() {
    let (_, mut me, mut f) = setup();
    me.sys.class_flags = 0;
    let mut world = world_with(&[0., 0.]);
    world.objects[2].as_mut().unwrap().sys.class_flags = 0;
    let each = lookup(2, 4).unwrap();
    let first = call(&mut world, &mut me, &mut f, each, &[Slot::address(0)]);
    assert_eq!(crate::world::object_from_handle(first.value), Some(1));
    assert_eq!(call(&mut world, &mut me, &mut f, each, &[first]).value, 0);
}
