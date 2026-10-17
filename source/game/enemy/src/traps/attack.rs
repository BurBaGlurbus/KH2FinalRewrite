//! Table 2 traps on attack objects (`YS::ATTACK`), see [`crate::attack`]. An
//! attack handle is `ATTACK_HANDLES + 0x100 * index`.

use super::math::{native, read_native};
use crate::attack::{Attack, flag};
use crate::script::{Fault, Memory, Slot};
use crate::world::{ATTACK_HANDLES, Ctx, Event, TrapFn, object_handle};

type Args = [Slot; 16];

pub fn lookup(table: u16, index: u16) -> Option<TrapFn> {
    if table == 7 {
        return Some(match index {
            2 => hit_mark_pos,
            14 => set_reaction_offset,
            34 => set_rc_owner,
            _ => return None,
        });
    }
    if table != 2 {
        return None;
    }
    Some(match index {
        71 => set_hitmark_pos,
        10 => attack_new,
        11 => set_radius,
        12 => set_pos,
        13 => free,
        14 => is_hit,
        17 => strike,
        18 => is_strike,
        19 => set_line,
        22 => set_obj_pax,
        24 => get_owner,
        25 => get_param_id,
        29 => is_hit_bg,
        37 => set_team,
        38 => set_hit_callback,
        39 => is_reflect,
        40 => is_hit_wall,
        41 => is_hit_floor,
        42 => hit_bg_pos,
        43 => reflect_vector,
        44 => reflecter,
        51 => set_rc,
        52 => rc_receiver,
        61 => set_effect,
        62 => set_time,
        67 => new_combo_group,
        74 => set_reflect_callback,
        96 => is_valid,
        _ => return None,
    })
}

fn index_of(handle: u32) -> Option<usize> {
    let o = handle.checked_sub(ATTACK_HANDLES)?;
    (o % 0x100 == 0 && o < 0x0100_0000).then_some((o / 0x100) as usize)
}

/// The attack a handle names, while it exists (FUN_1403d1ea0).
fn attack<'a>(ctx: &'a mut Ctx<'_>, handle: u32) -> Option<&'a mut Attack> {
    ctx.world.attacks.get_mut(index_of(handle)?)?.as_mut()
}

/// 2:10 FUN_140432250 -> FUN_1404371b0: `attack_new(ai, id, group, extra)`.
fn attack_new(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let owner = ctx.sys_of_ai(m, args[0].value).ok();
    let team = match owner {
        Some(id) => ctx.object(id)?.sys.team,
        None => 0,
    };
    let mut a = Attack::new(
        owner,
        args[1].as_int(),
        args[2].as_int(),
        args[3].as_int(),
        team,
    );
    ctx.world.attack_serial += 1;
    a.serial = ctx.world.attack_serial;
    let slot = ctx.world.attacks.iter().position(Option::is_none);
    let i = match slot {
        Some(i) => i,
        None => {
            ctx.world.attacks.push(None);
            ctx.world.attacks.len() - 1
        }
    };
    ctx.world.attacks[i] = Some(a);
    args[0] = Slot::address(ATTACK_HANDLES + 0x100 * i as u32);
    Ok(())
}

/// 2:11 FUN_1404322f0 -> FUN_140437c00.
fn set_radius(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let (r0, r1) = (args[1].as_float(), args[2].as_float());
    if let Some(a) = attack(ctx, args[0].value) {
        a.set_radius(r0, r1);
    }
    Ok(())
}

/// 2:12 FUN_140432330 -> FUN_140437a80.
fn set_pos(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = read_native(ctx, m, args[1].value)?;
    let frame = ctx.world.frame_time;
    if let Some(a) = attack(ctx, args[0].value) {
        a.set_position(v, frame);
    }
    Ok(())
}

/// 2:13 FUN_1404323b0 -> FUN_1403d0eb0: the script is done with it.
fn free(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    if let Some(i) = index_of(args[0].value)
        && let Some(slot) = ctx.world.attacks.get_mut(i)
        && let Some(a) = slot.as_mut()
    {
        a.flags |= flag::FREED;
        *slot = None;
    }
    Ok(())
}

fn flag_result(
    ctx: &mut Ctx<'_>,
    args: &mut Args,
    f: impl FnOnce(&Attack) -> bool,
) -> Result<(), Fault> {
    let hit = attack(ctx, args[0].value).is_some_and(|a| f(a));
    args[0] = Slot::int(hit as i32);
    Ok(())
}

/// 2:14 FUN_1404323d0 -> FUN_1403d1f20.
fn is_hit(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    flag_result(ctx, args, Attack::is_hit)
}

/// 2:29 FUN_140432420: struck the map.
fn is_hit_bg(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    flag_result(ctx, args, |a| {
        a.flags & (flag::HIT_WALL | flag::HIT_FLOOR) != 0
    })
}

/// 2:39 FUN_1404325c0.
fn is_reflect(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    flag_result(ctx, args, |a| a.flags & flag::REFLECTED != 0)
}

/// 2:40 FUN_1404325f0.
fn is_hit_wall(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    flag_result(ctx, args, |a| a.flags & flag::HIT_WALL != 0)
}

/// 2:41 FUN_140432620.
fn is_hit_floor(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    flag_result(ctx, args, |a| a.flags & flag::HIT_FLOOR != 0)
}

/// 2:96 (validity of a handle).
fn is_valid(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    flag_result(ctx, args, |_| true)
}

/// 2:17 FUN_1404328e0: apply the attack to a target directly (FUN_1403d23c0
/// makes the damage record, then the target class's damage entry takes it).
/// The attack counts as having struck; the damage itself is the host's:
/// the event names the attack and the target.
fn strike(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let target = ctx.sys_of_ai(m, args[1].value).ok();
    let handle = args[0].value;
    let Some(slot) = index_of(handle) else {
        return Ok(());
    };
    if let Some(a) = attack(ctx, handle) {
        a.flags |= flag::STRUCK;
    }
    ctx.world.events.push(Event::AttackStrike {
        attack: handle,
        slot,
        target,
        arguments: [args[2].as_int(), args[3].as_int()],
    });
    Ok(())
}

/// 2:18 FUN_140432990 -> FUN_1403d2060: whether the attack could hit the
/// target. [U] Only the team mask and the owner test are translated.
fn is_strike(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let target = ctx.sys_of_ai(m, args[1].value).ok();
    let team = match target {
        Some(id) => ctx.object(id)?.sys.team,
        None => 0,
    };
    let ok = attack(ctx, args[0].value).is_some_and(|a| {
        a.owner != target && a.team_mask & (1u32 << (team as u32 & 0x1f)) as u8 != 0
    });
    args[0] = Slot::int(ok as i32);
    Ok(())
}

/// 2:19 FUN_1404329e0 -> FUN_140437a40.
fn set_line(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let end = read_native(ctx, m, args[2].value)?;
    let start = read_native(ctx, m, args[1].value)?;
    if let Some(a) = attack(ctx, args[0].value) {
        a.set_line(start, end);
    }
    Ok(())
}

/// 2:22 FUN_140432a90: the object whose PAX container the attack uses.
fn set_obj_pax(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let object = ctx.sys_of_ai(m, args[1].value).ok();
    if let Some(a) = attack(ctx, args[0].value) {
        a.pax_object = object;
    }
    Ok(())
}

/// 2:24 FUN_140432ad0: the owner's engine object (0 without one).
fn get_owner(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let owner = attack(ctx, args[0].value).and_then(|a| a.owner);
    args[0] = Slot::address(owner.map_or(0, object_handle));
    Ok(())
}

/// 2:25 FUN_140432b00.
fn get_param_id(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = attack(ctx, args[0].value).map_or(0, |a| a.param_id);
    args[0] = Slot::int(id);
    Ok(())
}

/// 2:37 FUN_140432540 -> FUN_1403d2390.
fn set_team(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let team = args[1].as_int();
    if let Some(a) = attack(ctx, args[0].value) {
        a.set_team(team);
    }
    Ok(())
}

/// 2:38 FUN_140432560: the script entry to call when the attack hits, with
/// the AI object it gets. The script that registers it is the current one
/// (DAT_142a25048); the callback runs there.
fn set_hit_callback(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let (entry, ai) = (args[1].as_int(), args[2].value);
    let me = ctx.me.id;
    if let Some(a) = attack(ctx, args[0].value) {
        a.hit_callback = Some((entry, ai));
        a.callback_object = Some(me);
    }
    Ok(())
}

/// 2:74 FUN_140432c40: the same for a reflect (+0xa4..+0xac).
fn set_reflect_callback(
    ctx: &mut Ctx<'_>,
    _: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let (entry, ai) = (args[1].as_int(), args[2].value);
    let me = ctx.me.id;
    if let Some(a) = attack(ctx, args[0].value) {
        a.reflect_callback = Some((entry, ai));
        a.callback_object = Some(me);
    }
    Ok(())
}

/// 2:71 FUN_140432ba0 -> FUN_1403d0750: the hit mark's two vectors, the
/// executable's globals the hit spark reads.
fn set_hitmark_pos(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let first = read_native(ctx, m, args[0].value)?;
    let second = read_native(ctx, m, args[1].value)?;
    ctx.world.hit_mark = [first, second];
    Ok(())
}

/// 7:2 FUN_14053f2c0: the hit mark's position (DAT_14074c0d8).
fn hit_mark_pos(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = native(ctx.world.hit_mark[0]);
    let cell = ctx.world.static_cell(7, 2, 0);
    for (k, lane) in v.iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    args[0] = Slot::address(cell);
    Ok(())
}

/// 7:14 FUN_14053f300: +0x128, the offset added to the reaction direction.
fn set_reaction_offset(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let v = read_native(ctx, m, args[1].value)?;
    if let Some(a) = attack(ctx, args[0].value) {
        a.reaction_offset = v;
    }
    Ok(())
}

/// 7:34 FUN_14053fac0: +0x118, the object that owns the attack's receiver.
fn set_rc_owner(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let owner = ctx.sys_of_ai(m, args[1].value).ok();
    if let Some(a) = attack(ctx, args[0].value) {
        a.rc_owner = owner;
    }
    Ok(())
}

/// 2:42 FUN_140432650: where the attack hit the map (+0xb4).
fn hit_bg_pos(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = native(attack(ctx, args[0].value).map_or([0.; 4], |a| a.hit_position));
    let cell = ctx.world.static_cell(2, 42, 0);
    for (k, lane) in v.iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    args[0] = Slot::address(cell);
    Ok(())
}

/// 2:43 FUN_140432680 -> FUN_140437530: the direction a reflected attack
/// returns along: toward its owner when the owner still exists (otherwise
/// the way the reflecter faces), turned toward the reflecter's target when
/// that exists.
fn reflect_vector(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let (at, owner, reflecter) = attack(ctx, args[0].value).map_or(([0.; 4], None, None), |a| {
        (a.position, a.owner, a.reflecter)
    });
    let unit = |v: [f32; 3]| {
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        if len != 0. { v.map(|c| c / len) } else { v }
    };
    let mut out = [0.; 3];
    let owner_at = owner.and_then(|id| ctx.object(id).ok().map(|o| o.sys.position));
    if let Some(p) = owner_at {
        out = unit([p[0] - at[0], p[1] - at[1], p[2] - at[2]]);
    } else if let Some(id) = reflecter {
        let yaw = if id == crate::world::PLAYER_OBJECT {
            ctx.env.player_yaw()
        } else {
            ctx.object(id).map_or(0., |o| o.sys.yaw)
        };
        out = [yaw.sin(), 0., yaw.cos()];
    }
    // The reflecter's own target bends it.
    let target = reflecter.and_then(|id| {
        if id == crate::world::PLAYER_OBJECT {
            ctx.env
                .lockon_target()
                .and_then(|t| ctx.env.target_position(t))
        } else {
            let t = ctx.object(id).ok()?.sys.target;
            super::target::target_point(ctx, t)
        }
    });
    if let Some(p) = target {
        let toward = unit([p[0] - at[0], p[1] - at[1], p[2] - at[2]]);
        out = unit([out[0] + toward[0], out[1] + toward[1], out[2] + toward[2]]);
    }
    let cell = ctx.world.static_cell(2, 43, 0);
    for (k, lane) in [out[0], -out[1], out[2], 1.0].iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    args[0] = Slot::address(cell);
    Ok(())
}

/// 2:44 FUN_140432750: the object that reflected it (+0x90), 0 for none.
fn reflecter(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = attack(ctx, args[0].value).and_then(|a| a.reflecter);
    args[0] = Slot::address(id.map_or(0, object_handle));
    Ok(())
}

/// 2:51 FUN_140432790 -> FUN_140437b70: the receiver and its parameters.
fn set_rc(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let receiver = args[1].value;
    let (p, q, r) = (args[2].as_int(), args[3].as_int(), args[4].as_int());
    let _ = m;
    if let Some(a) = attack(ctx, args[0].value) {
        a.receiver = receiver;
        a.rc = [p, q, if r > 0 { r } else { q }];
    }
    Ok(())
}

/// 2:52 FUN_140432800.
fn rc_receiver(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let r = attack(ctx, args[0].value).map_or(0, |a| a.receiver);
    args[0] = Slot::address(r);
    Ok(())
}

/// 2:61 FUN_140432840 -> FUN_140437a00: the effect attached to the volume.
fn set_effect(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let effect = args[1].value;
    if let Some(a) = attack(ctx, args[0].value) {
        a.effect = effect;
    }
    Ok(())
}

/// 2:62 FUN_140432890 -> FUN_140437c30.
fn set_time(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let (d, r0, r1) = (args[1].as_float(), args[2].as_float(), args[3].as_float());
    if let Some(a) = attack(ctx, args[0].value) {
        a.set_time(d, r0, r1);
    }
    Ok(())
}

/// 2:67 FUN_140432b80 -> FUN_1403cf490: the next combo group id.
fn new_combo_group(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    ctx.world.combo_group += 1;
    if ctx.world.combo_group > 0x1000_0000 {
        ctx.world.combo_group = 0x100;
    }
    args[0] = Slot::int(ctx.world.combo_group as i32);
    Ok(())
}
