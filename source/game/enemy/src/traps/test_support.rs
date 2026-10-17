//! Helpers for the trap tests: a script frame to run traps in and a way to
//! call one with argument slots. Synthetic programs only.
use crate::env::NullEnv;
use crate::script::{Instance, Memory, Slot, Thread};
use crate::world::{Ctx, Object, TrapFn, World, object_handle};

pub struct Frame {
    pub inst: Instance,
    pub th: Thread,
}

pub fn frame() -> Frame {
    let mut header = vec![0u8; 44];
    header[..4].copy_from_slice(b"test");
    for at in [16, 20, 24] {
        header[at..at + 4].copy_from_slice(&256u32.to_le_bytes());
    }
    let inst = Instance::new(0, &header).unwrap();
    let th = Thread::new(&inst, 0, 0, &[]);
    Frame { inst, th }
}

/// Run a trap with these argument slots; the result is slot 0 afterwards.
pub fn call(
    world: &mut World,
    me: &mut Object,
    f: &mut Frame,
    trap: TrapFn,
    args: &[Slot],
) -> Slot {
    let mut slots = [Slot::default(); 16];
    slots[..args.len()].copy_from_slice(args);
    let mut memory = Memory::new(&mut f.inst, &mut f.th);
    let mut ctx = Ctx {
        world,
        me,
        env: &NullEnv,
    };
    trap(&mut ctx, &mut memory, &mut slots).unwrap();
    slots[0]
}

/// An AI object (word 1 = the engine object's handle) in the work area at
/// `at`; returns its address.
pub fn ai_object(f: &mut Frame, at: u32, id: u32) -> u32 {
    let base = f.inst.work_base() + at;
    let mut memory = Memory::new(&mut f.inst, &mut f.th);
    assert!(memory.write_u32(base + 4, object_handle(id)));
    base
}

/// A four-lane vector in the work area at `at`; returns its address.
pub fn vector(f: &mut Frame, at: u32, v: [f32; 4]) -> u32 {
    let base = f.inst.work_base() + at;
    let mut memory = Memory::new(&mut f.inst, &mut f.th);
    for (k, lane) in v.iter().enumerate() {
        assert!(memory.write_f32(base + 4 * k as u32, *lane));
    }
    base
}

/// A world holding bare objects `1..=n` at these x positions.
pub fn world_with(xs: &[f32]) -> World {
    let mut world = World::default();
    for (k, x) in xs.iter().enumerate() {
        let id = k as u32 + 1;
        let mut o = Object::bare(id);
        o.sys.position = [*x, 0., 0.];
        if world.objects.len() <= id as usize {
            world.objects.resize_with(id as usize + 1, || None);
        }
        world.objects[id as usize] = Some(Box::new(o));
    }
    world
}
