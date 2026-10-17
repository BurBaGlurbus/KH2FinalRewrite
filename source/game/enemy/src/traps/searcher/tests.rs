//! Synthetic objects only.
use super::*;
use crate::env::NullEnv;
use crate::traps::target::{OBJECT_TARGET, record_of};
use crate::traps::test_support::{Frame, call, frame, vector, world_with};
use crate::world::{Object, World};

#[test]
fn the_searcher_picks_the_nearest_enemy_in_range_and_remembers_the_pick() {
    let mut world = world_with(&[100., 300., 5000.]);
    let mut me = Object::bare(0);
    me.sys.class_flags = 0;
    let mut f = frame();
    let searcher = f.inst.work_base() + 64;
    let record = crate::world::TARGET_DUP_HANDLES;
    world
        .target_dups
        .push(Some(crate::sysobj::Target::default()));
    let origin = vector(&mut f, 200, [0., 0., 0., 1.]);
    let facing = vector(&mut f, 216, [1., 0., 0., 1.]);
    let init = lookup(6, 3).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        init,
        &[Slot::address(searcher), Slot::int(4)],
    );
    let search = lookup(6, 5).unwrap();
    let args = [
        Slot::address(searcher),
        Slot::address(record),
        Slot::address(origin),
        Slot::address(facing),
        Slot::int(1), // the nearest
        Slot::int(0),
        Slot::int(1), // remember the pick
    ];
    let pick = |world: &mut World, me: &mut Object, f: &mut Frame| {
        call(world, me, f, search, &args);
        let mut ctx = Ctx {
            world,
            me,
            env: &NullEnv,
        };
        record_of(&mut ctx, record).unwrap()
    };
    // Object 3 is past the searcher's range of 1000; object 1 is nearest.
    assert_eq!(pick(&mut world, &mut me, &mut f).id, OBJECT_TARGET | 1);
    // The pick is remembered, so the next search takes object 2.
    assert_eq!(pick(&mut world, &mut me, &mut f).id, OBJECT_TARGET | 2);
    // Nothing is left in range: the record is empty.
    assert_eq!(pick(&mut world, &mut me, &mut f).id, 0);
    // A reset forgets them.
    let reset = lookup(6, 4).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        reset,
        &[Slot::address(searcher)],
    );
    assert_eq!(pick(&mut world, &mut me, &mut f).id, OBJECT_TARGET | 1);
}

#[test]
fn the_farthest_pick_and_the_angle_limit() {
    let mut world = world_with(&[100., 300.]);
    let mut me = Object::bare(0);
    me.sys.class_flags = 0;
    let mut f = frame();
    let searcher = f.inst.work_base() + 64;
    let record = crate::world::TARGET_DUP_HANDLES;
    world
        .target_dups
        .push(Some(crate::sysobj::Target::default()));
    let origin = vector(&mut f, 200, [0., 0., 0., 1.]);
    // Facing +z, with both objects out along +x.
    let facing = vector(&mut f, 216, [0., 0., 1., 1.]);
    let init = lookup(6, 3).unwrap();
    call(
        &mut world,
        &mut me,
        &mut f,
        init,
        &[Slot::address(searcher), Slot::int(2)],
    );
    let search = lookup(6, 5).unwrap();
    let mut farthest = [
        Slot::address(searcher),
        Slot::address(record),
        Slot::address(origin),
        Slot::address(facing),
        Slot::int(0), // the farthest
        Slot::int(0),
        Slot::int(0),
    ];
    call(&mut world, &mut me, &mut f, search, &farthest);
    let mut ctx = Ctx {
        world: &mut world,
        me: &mut me,
        env: &NullEnv,
    };
    assert_eq!(record_of(&mut ctx, record).unwrap().id, OBJECT_TARGET | 2);
    // A half angle under a quarter turn around +z excludes both objects.
    let angle_at = searcher + 24;
    let mut memory = Memory::new(&mut f.inst, &mut f.th);
    assert!(memory.write_f32(angle_at, 0.5));
    farthest[4] = Slot::int(1);
    call(&mut world, &mut me, &mut f, search, &farthest);
    let mut ctx = Ctx {
        world: &mut world,
        me: &mut me,
        env: &NullEnv,
    };
    assert_eq!(record_of(&mut ctx, record).unwrap().id, 0);
}
