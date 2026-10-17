//! Table 1 traps on the script's own object: acts, motion, movement, facing,
//! flags and the other small state the scripts query. Arguments named `ai` are
//! the script's AI object (a work-area struct whose +4 holds the engine
//! object); `sys` arguments are the engine object itself.
//!
//! Each function cites the executable function it translates. Engine services
//! the executable reaches that are not translated yet are marked [U].

use super::math::{read_native, read_sanitized, wrap_angle};
use crate::act::Entry;
use crate::script::{Fault, Memory, Slot};
use crate::sysobj::{JumpMotions, PendingFly};
use crate::world::{Ctx, Event, SysRef, TrapFn};

type Args = [Slot; 16];

pub fn lookup(table: u16, index: u16) -> Option<TrapFn> {
    if table != 1 {
        return None;
    }
    Some(match index {
        3 => sysobj_player,
        4 => wish_dir,
        5 => act_table_init,
        6 => act_table_add,
        7 => set_act_table,
        8 => act_start,
        9 => act_push,
        10 => is_act_exec,
        11 => motion_start,
        12 => motion_change,
        13 => motion_push,
        14 => motion_is_end,
        15 => motion_id,
        19 => fade_out,
        20 => fade_in,
        23 => area_world,
        28 => leave,
        34 => event_is_exec,
        39 => unit_arg,
        55 => jump,
        56 => is_culling,
        57 => is_jump,
        58 => fly,
        60 => is_air,
        61 => motion_frame_start,
        62 => get_moved,
        64 => get_wish_movement,
        65 => exec_fall,
        66 => exec_land,
        29 => motion_capture,
        63 => motion_is_in_loop,
        67 => motion_length,
        68 => motion_loop_top,
        69 => motion_time,
        70 => set_flag,
        71 => reset_flag,
        74 => idle,
        78 => motion_is_no_control,
        79 => set_dir,
        80 => turn_dir,
        87 => effect_start_bind,
        90 => act_shout,
        94 => is_exist,
        98 => step_pos,
        92 => scatter_prize,
        104 => shadow_move_start,
        105 => shadow_move_end,
        110 => appear_way,
        111 => set_movement,
        89 => move_request,
        119 => check_step,
        124 => is_entry_fly,
        128 => sys_jump,
        127 => act_clear,
        133 => hide,
        134 => show,
        147 => position,
        150 => motion_check_range,
        151 => motion_check_trigger,
        168 => fly_jump,
        171 => status_no_leave,
        201 => direction,
        211 => pattern_enable,
        212 => pattern_disable,
        238 => motion_capture_id,
        206 => status_force_leave,
        301 => tutorial_is_open,
        309 => sysobj_is_player,
        _ => return None,
    })
}

/// The shared result vector (`DAT_140756b48`) position/direction traps return.
fn shared_vector(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    v: [f32; 4],
    args: &mut Args,
) -> Result<(), Fault> {
    let cell = ctx.world.static_cell(1, 0, 1);
    for (k, lane) in v.iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    args[0] = Slot::address(cell);
    Ok(())
}

/// A distinct result cell for one trap.
fn own_vector(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    index: u16,
    v: [f32; 3],
    args: &mut Args,
) -> Result<(), Fault> {
    let cell = ctx.world.static_cell(1, index, 0);
    for (k, lane) in v.iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    ctx.write_f32(m, cell + 12, 1.0);
    args[0] = Slot::address(cell);
    Ok(())
}

fn int(args: &mut Args, v: i32) {
    args[0] = Slot::int(v);
}

/// 1:3 FUN_14042b230: Sora's object.
fn sysobj_player(_: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    args[0] = Slot::address(crate::world::PLAYER_HANDLE);
    Ok(())
}

/// 1:4 FUN_14042bfa0 -> FUN_1403b65c0: the heading the engine should turn to.
fn wish_dir(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = read_sanitized(ctx, m, args[1].value)?;
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.wish_yaw = Some(v[0].atan2(v[2]));
    Ok(())
}

/// 1:5 FUN_14042e200 -> FUN_1403ca790: clear the act table.
fn act_table_init(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, _: &mut Args) -> Result<(), Fault> {
    ctx.me.acts.init_table();
    Ok(())
}

/// 1:6 FUN_14042e220 -> FUN_1403ca0e0: register an act.
fn act_table_add(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let name = m
        .string(args[1].value, 128)
        .ok_or(Fault::Memory(args[1].value))?;
    let flags = args[2].value;
    let word = |k: usize| args[k].value as u16;
    ctx.me.acts.add(Entry {
        name,
        priority: flags as u16,
        flag_bits: (flags >> 16) as u16,
        entry: word(3),
        end: word(4),
        damage: word(7),
        timeout: word(9),
        start: word(10),
        event: word(11),
        unused: [word(5), word(6), word(8)],
    });
    Ok(())
}

/// 1:7 FUN_14042c620: bind the AI object and its act table to the engine object.
fn set_act_table(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let obj = ctx.object(id)?;
    obj.acts.ai_object = args[0].value;
    obj.acts.has_table = true;
    Ok(())
}

fn act_name(m: &Memory<'_>, handle: u32) -> Result<Vec<u8>, Fault> {
    m.string(handle, 128).ok_or(Fault::Memory(handle))
}

/// 1:8 FUN_14042c6b0 -> FUN_1403cae60.
fn act_start(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let name = act_name(m, args[1].value)?;
    if let Some(log) = ctx.world.act_log.as_mut() {
        log.push(format!("start {}", String::from_utf8_lossy(&name)));
    }
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let obj = ctx.object(id)?;
    let actions = obj.acts.start(&name);
    obj.apply_actions(None, actions);
    Ok(())
}

/// 1:9 FUN_14042c6f0 -> FUN_1403cab00.
fn act_push(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let name = act_name(m, args[1].value)?;
    if let Some(log) = ctx.world.act_log.as_mut() {
        log.push(format!("push {}", String::from_utf8_lossy(&name)));
    }
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.acts.push(&name);
    Ok(())
}

/// 1:10 FUN_14042c770.
fn is_act_exec(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let running = ctx.object(id)?.acts.is_exec();
    int(args, running as i32);
    Ok(())
}

/// 1:127 FUN_14042c680 -> FUN_1403ca440.
fn act_clear(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let obj = ctx.object(id)?;
    let actions = obj.acts.clear();
    obj.apply_actions(None, actions);
    Ok(())
}

/// 1:11 FUN_14042b320 -> FUN_1403c86a0: restart a motion.
fn motion_start(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let (slot, blend) = (args[1].as_int(), args[2].as_float());
    let env = ctx.env;
    let obj = ctx.object(id)?;
    let _ = m;
    obj.sys
        .motion_start(slot.max(0) as usize, blend, env.clips(obj.sys.model));
    Ok(())
}

/// 1:12 FUN_14042b3b0 -> FUN_1403c6dc0: change motion unless it already plays.
fn motion_change(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let (slot, blend) = (args[1].as_int(), args[2].as_float());
    let env = ctx.env;
    let obj = ctx.object(id)?;
    obj.sys
        .motion_change(slot.max(0) as usize, blend, env.clips(obj.sys.model));
    Ok(())
}

/// 1:61 FUN_14042b360: start a motion at a frame.
fn motion_frame_start(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let (slot, frame, blend) = (args[1].as_int(), args[2].as_float(), args[3].as_float());
    let env = ctx.env;
    let obj = ctx.object(id)?;
    let clips = env.clips(obj.sys.model);
    if obj.sys.motion_start(slot.max(0) as usize, blend, clips) {
        // [U] FUN_1403c88c0's start frame: the clock begins at `frame`.
        obj.sys.motion.tick = frame.max(0.0) as u32;
    }
    Ok(())
}

/// 1:14 FUN_14042b440: bit 1 of +0x18c.
fn motion_is_end(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let ended = ctx.object(id)?.sys.motion_ended();
    int(args, ended as i32);
    Ok(())
}

/// 1:15 FUN_14042b480: +0x180.
fn motion_id(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let slot = ctx.object(id)?.sys.motion.slot;
    int(args, slot as i32);
    Ok(())
}

/// 1:19 FUN_14042b4b0: fade the alpha out unless leaving.
fn fade_out(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let ticks = args[1].as_float();
    let obj = ctx.object(id)?;
    if !obj.sys.left {
        obj.sys.fade.start(1.0, 0.0, ticks);
    }
    Ok(())
}

/// 1:20 FUN_14042b500.
fn fade_in(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let ticks = args[1].as_float();
    let obj = ctx.object(id)?;
    if !obj.sys.left {
        obj.sys.fade.start(0.0, 1.0, ticks);
    }
    Ok(())
}

/// 1:23 FUN_14042e330: the current world id.
fn area_world(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    int(args, ctx.env.area_world());
    Ok(())
}

/// 1:28 FUN_14042c810: alpha out at once and leave.
fn leave(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let obj = ctx.object(id)?;
    if !obj.sys.left {
        obj.sys.fade.start(1.0, 0.0, 0.0);
    }
    obj.sys.left = true;
    Ok(())
}

/// 1:34 FUN_140430630.
fn event_is_exec(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    int(args, ctx.env.event_running() as i32);
    Ok(())
}

/// 1:39 FUN_14042db00 -> FUN_1403b5df0: spawn argument `n` (+0x9f4 + 4n).
fn unit_arg(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let n = args[1].as_int();
    let v = usize::try_from(n)
        .ok()
        .and_then(|n| ctx.object(id).ok()?.sys.spawn_args.get(n).copied())
        .unwrap_or(0);
    int(args, v);
    Ok(())
}

/// 1:55 FUN_14042cc80: `method_obj_jump(ai, v, motion, blend, auto)`. The jump
/// (FUN_1403cbb00) takes the apex height from `v.y` and a horizontal speed from
/// `v.w` along `v.xz`; then the motion starts; a nonzero `auto` makes the next
/// two motion slots the fall and land motions.
fn jump(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = read_sanitized(ctx, m, args[1].value)?;
    let (slot, blend, auto) = (args[2].as_int(), args[3].as_float(), args[4].as_int());
    let env = ctx.env;
    let obj = ctx.object(id)?;
    obj.sys.launch_jump([v[0], 0., v[2]], v[1], v[3]);
    obj.sys
        .motion_start(slot.max(0) as usize, blend, env.clips(obj.sys.model));
    if auto != 0 {
        obj.sys.jump_motions = JumpMotions {
            fall: slot + 1,
            land: slot + 2,
        };
    }
    Ok(())
}

/// 1:56 FUN_14042ee40 -> FUN_1403ba4a0: off-screen test.
fn is_culling(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let pos = ctx.object(id)?.sys.position;
    int(args, ctx.env.is_culled(pos) as i32);
    Ok(())
}

/// 1:57 FUN_14042ee80: physics state bit 0.
fn is_jump(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let jumping = ctx.object(id)?.sys.is_jump();
    int(args, jumping as i32);
    Ok(())
}

/// 1:58 FUN_14042eec0 -> FUN_140421ce0: `obj_fly(ai, v, inertia)` enters physics
/// mode 5 with the velocity `normalize(v.xyz) * v.w`.
fn fly(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = read_native(ctx, m, args[1].value)?;
    let inertia = args[2].as_float();
    ctx.object(id)?
        .sys
        .start_steer([v[0], v[1], v[2]], v[3], inertia);
    Ok(())
}

/// 1:60 FUN_14042efc0.
fn is_air(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &ctx.object(id)?.sys;
    let air = sys.air_blocked() || sys.airborne;
    int(args, air as i32);
    Ok(())
}

/// 1:62 FUN_14042f060: a copy of +0xa38.
fn get_moved(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.moved;
    own_vector(ctx, m, 62, [v[0], -v[1], v[2]], args)
}

/// 1:64 FUN_14042f160: a copy of +0xa28.
fn get_wish_movement(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.wished;
    own_vector(ctx, m, 64, [v[0], -v[1], v[2]], args)
}

/// 1:65 FUN_14042f220: flag 0x4000000 asks the engine to start a fall.
fn exec_fall(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.flags |= 0x400_0000;
    Ok(())
}

/// 1:66 FUN_14042f250: flag 0x8000000 asks the engine to land.
fn exec_land(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.flags |= 0x800_0000;
    Ok(())
}

/// 1:67 FUN_14042f280 -> FUN_1403c78f0: a clip length (-1 = the current one).
/// [U] The unit of the executable's value (motion-clock key time) is taken
/// to be the tick length the host reports.
fn motion_length(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let slot = args[1].as_int();
    let env = ctx.env;
    let obj = ctx.object(id)?;
    let slot = if slot < 0 {
        obj.sys.motion.slot
    } else {
        slot as usize
    };
    let length = env.clips(obj.sys.model).length(slot);
    args[0] = Slot::float(length as f32);
    Ok(())
}

/// 1:68 FUN_14042f2d0 -> FUN_1403c7950: a clip's loop start.
fn motion_loop_top(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let slot = args[1].as_int();
    let env = ctx.env;
    let obj = ctx.object(id)?;
    let slot = if slot < 0 {
        obj.sys.motion.slot
    } else {
        slot as usize
    };
    let top = env.clips(obj.sys.model).loop_start(slot);
    args[0] = Slot::float(top as f32);
    Ok(())
}

/// 1:70 FUN_14042f350: set bit `n` of the flag words from +0x120.
fn set_flag(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let n = args[1].value;
    if n >> 5 == 0 {
        ctx.object(id)?.sys.flags |= 1 << (n & 31);
    }
    Ok(())
}

/// 1:71 FUN_14042f390: clear bit `n` (only for a live object).
fn reset_flag(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let n = args[1].value;
    if n >> 5 == 0 {
        ctx.object(id)?.sys.flags &= !(1 << (n & 31));
    }
    Ok(())
}

/// 1:78 FUN_14042f500: +0x18c bit 9.
fn motion_is_no_control(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let blocked = ctx.object(id)?.sys.no_control();
    int(args, blocked as i32);
    Ok(())
}

/// 1:79 FUN_14042f540 -> FUN_1403b6420: face along `v` at once.
fn set_dir(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = read_sanitized(ctx, m, args[1].value)?;
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &mut ctx.object(id)?.sys;
    sys.yaw = wrap_angle(v[0].atan2(v[2]));
    sys.wish_yaw = Some(sys.yaw);
    Ok(())
}

/// 1:80 FUN_14042f5c0 -> FUN_1403b9f70.
fn turn_dir(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = read_sanitized(ctx, m, args[1].value)?;
    let rate = args[2].as_float();
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let turning = ctx.object(id)?.sys.turn_dir([v[0], v[1], v[2]], rate);
    int(args, turning as i32);
    Ok(())
}

/// 1:87 FUN_14042d540 -> FUN_1403b6670: start an effect bound to the object.
/// [U] The effect system is the host's; the request is recorded as an event.
fn effect_start_bind(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let (a, b, c) = (args[1].as_int(), args[2].as_int(), args[3].as_int());
    let handle = ctx.world.next_effect_handle();
    ctx.world.events.push(Event::EffectStart {
        object: id,
        effect: a,
        arguments: [b, c],
        handle,
        position: None,
        bound: true,
        other: None,
    });
    args[0] = Slot::address(handle);
    Ok(())
}

/// 1:92 FUN_14042c960 -> FUN_1403b6170: scatter prize drops.
/// [U] The prize tables are the host's; the request is recorded.
fn scatter_prize(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let n = args[1].as_int();
    ctx.world.events.push(Event::ScatterPrize {
        object: id,
        count: n,
    });
    Ok(())
}

/// 1:104 FUN_14042fbf0: +0x8c8 |= 0x40.
fn shadow_move_start(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.shadow_move = true;
    Ok(())
}

/// 1:105 FUN_14042fc10.
fn shadow_move_end(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.shadow_move = false;
    Ok(())
}

/// 1:110 FUN_14042fd70 -> FUN_1403b50d0: the spawn record's appear way.
fn appear_way(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let way = ctx.object(id)?.sys.appear_way;
    int(args, way);
    Ok(())
}

/// 1:111 FUN_14042fda0 -> FUN_1403b9da0: movement request (direction
/// normalised in place, speed).
fn set_movement(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let handle = args[1].value;
    let speed = args[2].as_float();
    let v = read_native(ctx, m, handle)?;
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let unit = if len != 0.0 {
        let inv = 1.0 / len;
        [v[0] * inv, v[1] * inv, v[2] * inv]
    } else {
        [v[0], v[1], v[2]]
    };
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &mut ctx.object(id)?.sys;
    sys.move_direction = unit;
    sys.move_speed = speed;
    Ok(())
}

/// 1:124 FUN_140430590 -> FUN_1403ba700: spawned flying.
fn is_entry_fly(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let fly = ctx.object(id)?.sys.entry_fly;
    int(args, fly as i32);
    Ok(())
}

/// 1:133 FUN_14042b640: class hide.
fn hide(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.hidden = true;
    Ok(())
}

/// 1:134 FUN_14042b680: class show.
fn show(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.hidden = false;
    Ok(())
}

/// 1:147 FUN_14042b7b0: the position.
fn position(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let p = match ctx.sys_ref_ai(m, args[0].value)? {
        SysRef::Player => ctx.env.player_position(),
        SysRef::Object(id) => ctx.object(id)?.sys.position,
    };
    let w = ctx
        .world
        .static_lookup(1, 0, 1)
        .and_then(|c| ctx.world.static_read(c + 12));
    let w = w.map_or(1.0, f32::from_bits);
    shared_vector(ctx, m, [p[0], -p[1], p[2], w], args)
}

/// 1:168 FUN_14042cf50: `obj_flyjump`, a jump that waits for the motion's
/// take-off event (physics mode 11).
fn fly_jump(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = read_sanitized(ctx, m, args[1].value)?;
    let (slot, blend, auto) = (args[2].as_int(), args[3].as_float(), args[4].as_int());
    let env = ctx.env;
    let obj = ctx.object(id)?;
    obj.sys.pending_fly = Some(PendingFly {
        direction: [v[0], 0., v[2]],
        height: v[1],
        speed: v[3],
    });
    obj.sys
        .motion_start(slot.max(0) as usize, blend, env.clips(obj.sys.model));
    if auto != 0 {
        obj.sys.jump_motions = JumpMotions {
            fall: slot + 1,
            land: slot + 2,
        };
    }
    Ok(())
}

/// 1:171 FUN_140430820: `DAT_142a11400` bit 2.
fn status_no_leave(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let set = ctx.env.no_leave() || ctx.world.status & crate::world::status::NO_LEAVE != 0;
    int(args, set as i32);
    Ok(())
}

/// 1:206 FUN_140430880: `DAT_142a11400` bit 7.
fn status_force_leave(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let set = ctx.env.force_leave() || ctx.world.status & crate::world::status::FORCE_LEAVE != 0;
    int(args, set as i32);
    Ok(())
}

/// 1:201 FUN_14042ba00 -> FUN_1403b56e0: the facing vector (w = heading).
fn direction(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let (v, yaw) = match ctx.sys_ref_ai(m, args[0].value)? {
        SysRef::Player => {
            let yaw = ctx.env.player_yaw();
            ([yaw.sin(), 0., yaw.cos()], yaw)
        }
        SysRef::Object(id) => {
            let sys = &ctx.object(id)?.sys;
            (sys.forward(), sys.yaw)
        }
    };
    shared_vector(ctx, m, [v[0], 0., v[2], yaw], args)
}

/// 1:301 FUN_140431730.
fn tutorial_is_open(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    int(args, ctx.env.tutorial_open() as i32);
    Ok(())
}

/// 1:309 FUN_14042b260: status bit 7 of the object (the damage source is Sora).
fn sysobj_is_player(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    int(args, (args[0].value == crate::world::PLAYER_HANDLE) as i32);
    let _ = ctx;
    Ok(())
}

/// 1:89 FUN_14042f8d0 -> FUN_1403b9a70: a second movement request vector
/// (+0xa58 direction, +0xa64 speed) that no translated consumer reads yet.
fn move_request(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = read_native(ctx, m, args[1].value)?;
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &mut ctx.object(id)?.sys;
    sys.request_direction = [v[0], v[1], v[2]];
    sys.request_speed = args[2].as_float();
    Ok(())
}

/// 1:119 FUN_1404302a0 -> FUN_1403b7f10: the ground probe ahead of the
/// object (0 level, 1 higher, 2 lower, 3 no ground).
fn check_step(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = read_native(ctx, m, args[1].value)?;
    let (up, down) = (args[2].as_float(), args[3].as_float());
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let position = ctx.object(id)?.sys.position;
    let result = match ctx
        .env
        .check_step(position, [v[0], v[1], v[2]], v[3], up, down)
    {
        Some(r) => r,
        None => {
            ctx.world.events.push(Event::Unsupported("check_step"));
            0
        }
    };
    int(args, i32::from(result));
    Ok(())
}

/// 1:128 FUN_14042cc30: the physics mode's enter hook (`vtable + 0x20`) with a
/// vertical speed: leave the ground into a fall that starts at that speed.
fn sys_jump(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let up = args[1].as_float();
    let sys = &mut ctx.object(id)?.sys;
    sys.start_fall(up);
    Ok(())
}

/// 1:13 FUN_14042b3f0 -> FUN_1403c8310: queue a motion behind the playing one.
fn motion_push(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let (slot, blend) = (args[1].as_int(), args[2].as_float());
    ctx.object(id)?
        .sys
        .motion_queue
        .push((slot.max(0) as usize, blend));
    Ok(())
}

/// 1:29 FUN_14042d160 -> FUN_1403c6cc0: play a motion borrowed from another
/// object (a grab). [U] Not translated; reports nothing captured.
fn motion_capture(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    ctx.world.events.push(Event::Unsupported("motion_capture"));
    int(args, 0);
    Ok(())
}

/// 1:238 FUN_14042d3b0: the captured motion id (+0x184). [U] Always 0 until
/// captures exist.
fn motion_capture_id(_: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    int(args, 0);
    Ok(())
}

/// 1:63 FUN_14042f120 -> FUN_1403c7e70: past the loop start and not blending.
fn motion_is_in_loop(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let env = ctx.env;
    let sys = &ctx.object(id)?.sys;
    let looping = sys.motion.blend <= 0.0
        && env
            .clips(sys.model)
            .in_loop(sys.motion.slot, sys.motion.tick);
    int(args, looping as i32);
    Ok(())
}

/// 1:69 FUN_14042f320: the motion clock (+0x19c).
fn motion_time(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let tick = ctx.object(id)?.sys.motion.tick;
    args[0] = Slot::float(tick as f32);
    Ok(())
}

/// 1:74 FUN_14042d0e0: return to the ground physics mode.
fn idle(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.to_ground_mode();
    Ok(())
}

/// 1:90 FUN_14042f960 -> FUN_1403cacd0: shout an act name to every object.
/// The shout waits in a queue of 64 (a full queue drops it) until the
/// frame's shout job (FUN_1403c9d40, [`World::deliver_shouts`]) hands it on;
/// a positive delay spreads the receivers over that many ticks at random.
fn act_shout(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let name = m
        .string(args[1].value, 128)
        .ok_or(Fault::Memory(args[1].value))?;
    let shouter = ctx.sys_of_ai(m, args[0].value)?;
    let delay = args[2].as_float();
    if ctx.world.shouts.len() < crate::world::SHOUT_QUEUE {
        ctx.world.shouts.push(crate::world::Shout {
            name,
            shouter,
            delay,
        });
    }
    Ok(())
}

/// 1:94 FUN_14042b0e0: the object exists (and is not being removed).
fn is_exist(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let h = args[0].value;
    let exists = h == crate::world::PLAYER_HANDLE
        || crate::world::object_from_handle(h)
            .and_then(|id| ctx.object(id).ok())
            .is_some_and(|o| !o.sys.gone);
    int(args, exists as i32);
    Ok(())
}

/// 1:98 FUN_14042fae0: the position with w = the step height (FUN_1403b87e0).
/// [U] Taken to be the current height, its value when nothing is probed.
fn step_pos(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let p = match ctx.sys_ref_ai(m, args[0].value)? {
        SysRef::Player => ctx.env.player_position(),
        SysRef::Object(id) => ctx.object(id)?.sys.position,
    };
    shared_vector(ctx, m, [p[0], -p[1], p[2], -p[1]], args)
}

/// 1:150 FUN_14042d1e0: attack range `n` (1-based, `+0x2ac` bits).
fn motion_check_range(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let n = args[1].as_int();
    let bits = ctx.object(id)?.sys.motion.flags.ranges;
    int(
        args,
        (bits & (1u8.wrapping_shl(n.wrapping_sub(1) as u32 & 31)) != 0) as i32,
    );
    Ok(())
}

/// 1:151 FUN_14042d230: motion trigger pulse `n` (1-based, `+0x2ad` bits).
fn motion_check_trigger(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let n = args[1].as_int();
    let bits = ctx.object(id)?.sys.motion.flags.pulses;
    int(
        args,
        (bits & (1u8.wrapping_shl(n.wrapping_sub(1) as u32 & 31)) != 0) as i32,
    );
    Ok(())
}

/// 1:211 FUN_14042ba50: enable pattern `n` (+0xd10 bit).
fn pattern_enable(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.pattern_mask |= 1 << (args[1].value & 31);
    Ok(())
}

/// 1:212: disable pattern `n`.
fn pattern_disable(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.pattern_mask &= !(1 << (args[1].value & 31));
    Ok(())
}
