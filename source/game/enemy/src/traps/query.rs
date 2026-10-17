//! Table 1, 2, 6 and 10 traps that read or set one field of the engine object
//! (state queries, small setters), translated from the executable (Steam
//! build 15194255). Each names the trap wrapper and the routine it calls.

use super::math::read_native;
use crate::script::{Fault, Memory, Slot};
use crate::sysobj::{JumpMotions, Target, class_flag, state_flag};
use crate::world::{Ctx, ObjId, PLAYER_OBJECT, TrapFn};
use player::actor_physics;

type Args = [Slot; 16];

pub fn lookup(table: u16, index: u16) -> Option<TrapFn> {
    Some(match (table, index) {
        (0, 0) | (1, 122) | (2, 36) => nothing,
        (1, 2) => sysobj_moveto,
        (1, 17) => leave_force,
        (1, 18) => obj_attach,
        (1, 85) => texanm_start,
        (1, 86) => texanm_stop,
        (1, 102) => obj_detach,
        (1, 108) => show_part,
        (1, 109) => hide_part,
        (1, 125) => tex_fade_start,
        (1, 161) => is_attach,
        (1, 113) => player_get_movement,
        (1, 137) => bg_get_normal,
        (1, 220) => move_to_space,
        (1, 304) => search_by_glance,
        (1, 347) => can_see,
        (1, 367) => get_move_to_space_pos,
        (6, 46) => rc_stop_all,
        (6, 52) => bghit_get_normal,
        (6, 69) => larxene_dead,
        (1, 40) => is_pirate_shade,
        (1, 95) => fly_to_jump,
        (1, 115) => set_jump_motion,
        (1, 146) => obj_hitback,
        (1, 192) => is_motion_cancel,
        (1, 224) => is_action_air,
        (1, 254) => is_ride,
        (1, 265) => set_medal,
        (1, 266) => get_medal,
        (1, 272) => is_dead,
        (1, 319) => is_sora,
        (1, 323) => count_entry,
        (1, 324) => pattern_reset,
        (2, 35) => damage_attack_type,
        (2, 70) => damage_parts,
        (2, 95) => dup_sheet,
        (2, 97) => set_attacker,
        (6, 45) => hide_shadow,
        (6, 49) => is_stop,
        (6, 56) => target_set_group,
        (6, 57) => target_get_group,
        (6, 60) => set_stealth_color,
        (6, 63) => is_damage_motion,
        (6, 65) => set_scissoring,
        (6, 66) => clear_hitback,
        (6, 47) | (7, 1) => stop_end_all,
        (7, 0) => stop_all_start,
        (10, 17) => is_zako,
        (10, 19) => is_limit,
        (10, 22) => is_blow,
        (10, 31) => is_summon,
        (10, 52) => is_reflect_motion,
        _ => return None,
    })
}

fn int(args: &mut Args, v: i32) {
    args[0] = Slot::int(v);
}

fn flag(args: &mut Args, v: bool) {
    int(args, v as i32);
}

/// 0:0 FUN_14041c9f0, 1:122 FUN_140430540 and 2:36 FUN_140431d90: empty in
/// this build (the script debug print, the object dump, the add-damage hook).
fn nothing(_: &mut Ctx<'_>, _: &mut Memory<'_>, _: &mut Args) -> Result<(), Fault> {
    Ok(())
}

/// Store a vector (given in the engine's +Y-up frame) in the trap's static
/// cell and hand the script its address.
fn return_vec(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
    key: (u16, u16),
    v: [f32; 4],
) {
    let cell = ctx.world.static_cell(key.0, key.1, 0);
    for (k, lane) in [v[0], -v[1], v[2], v[3]].iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    args[0] = Slot::address(cell);
}

/// 1:2 FUN_14042b1d0 -> FUN_1403b9c60: point the object's move request at a
/// position (the horizontal heading to it) with a speed; the result is the
/// horizontal distance to it. The arguments are the engine object handle,
/// the position vector and the speed.
fn sysobj_moveto(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of(args[0].value)?;
    let target = read_native(ctx, m, args[1].value)?;
    let speed = args[2].as_float();
    let sys = &mut ctx.object(id)?.sys;
    let d = [target[0] - sys.position[0], 0., target[2] - sys.position[2]];
    let len = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let unit = if len != 0. {
        [d[0] / len, 0., d[2] / len]
    } else {
        d
    };
    sys.move_direction = unit;
    sys.move_speed = speed;
    args[0] = Slot::float(len);
    Ok(())
}

/// 1:113 FUN_14042fe30 -> FUN_1403aa4c0: the movement Sora's controller asks
/// for (+0xb50 of her controller block).
fn player_get_movement(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let v = ctx.env.player_move_input();
    return_vec(ctx, m, args, (1, 113), [v[0], v[1], v[2], 1.0]);
    Ok(())
}

/// The map polygon's normal; the host answers, straight up without it.
fn map_normal(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args, key: (u16, u16), poly: i32) {
    let n = ctx.env.map_normal(poly).unwrap_or([0., 1., 0.]);
    return_vec(ctx, m, args, key, [n[0], n[1], n[2], 1.0]);
}

/// 1:137 FUN_14042e7f0 -> FUN_140170ac0: the normal of map polygon `n`.
fn bg_get_normal(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let poly = (args[0].value & 0xffff) as i32;
    map_normal(ctx, m, args, (1, 137), poly);
    Ok(())
}

/// 6:52 FUN_14053ec80 -> FUN_14040c3c0: the normal of the polygon a map hit
/// record names (the last hit here).
fn bghit_get_normal(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    map_normal(ctx, m, args, (6, 52), -1);
    Ok(())
}

/// The root displacement of the playing motion over its first `time` ticks,
/// turned into the object's heading (the executable samples a copy of the
/// motion player to that time, FUN_14016fec0).
fn root_over(ctx: &mut Ctx<'_>, id: ObjId, time: f32) -> Result<[f32; 3], Fault> {
    let env = ctx.env;
    let sys = &ctx.object(id)?.sys;
    let clips = env.clips(sys.model);
    let slot = sys.motion.slot;
    let n = (time.max(0.) as u32).min(clips.length(slot));
    let mut sum = [0.; 3];
    for t in 0..n {
        let d = clips.root_delta(slot, t);
        for k in 0..3 {
            sum[k] += d[k];
        }
    }
    let (sin, cos) = sys.yaw.sin_cos();
    Ok([
        sum[0] * cos + sum[2] * sin,
        sum[1],
        -sum[0] * sin + sum[2] * cos,
    ])
}

/// 1:220 FUN_14042bbb0 -> FUN_1403b9ac0: put the object where the playing
/// motion's root would be after `time` ticks (FUN_1403b6510 teleports to it).
fn move_to_space(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let d = root_over(ctx, id, args[1].as_float())?;
    let sys = &mut ctx.object(id)?.sys;
    sys.position = [0, 1, 2].map(|k| sys.position[k] + d[k]);
    sys.knock = None;
    Ok(())
}

/// 1:367 FUN_14042bb40 -> FUN_1403b8510: the same position as a vector,
/// without moving.
fn get_move_to_space_pos(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let d = root_over(ctx, id, args[1].as_float())?;
    let p = ctx.object(id)?.sys.position;
    return_vec(
        ctx,
        m,
        args,
        (1, 367),
        [p[0] + d[0], p[1] + d[1], p[2] + d[2], 1.0],
    );
    Ok(())
}

/// 1:347 FUN_14042c330 -> FUN_1403f3c20: nothing in the map blocks the line
/// between the two objects.
fn can_see(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let from = ctx.sys_of_ai(m, args[0].value)?;
    let to = ctx.sys_of_ai(m, args[1].value)?;
    let (a, b) = (ctx.object(from)?.sys.position, ctx.object(to)?.sys.position);
    let clear = !ctx.env.line_blocked(a, b);
    flag(args, clear);
    Ok(())
}

/// 1:304 FUN_14042dee0 -> FUN_1403b3f50: the object of this entry id the
/// viewer is looking at, bound to the AI object. Glanced objects get
/// +0x124 bit 12, the ones passed over bit 13 once they had it. [U] The
/// executable measures from the point the camera looks at; here from Sora.
fn search_by_glance(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let entry = args[1].value;
    let view = ctx.env.player_position();
    let mut candidates: Vec<(ObjId, f32)> = Vec::new();
    let mut ids = vec![ctx.me.id];
    ids.extend(ctx.world.objects.iter().flatten().map(|o| o.id));
    ids.sort_unstable();
    for id in ids {
        let o = ctx.object(id)?;
        if o.sys.entry_id == entry
            && o.sys.state_flags & state_flag::GLANCE_LOST == 0
            && o.sys.flags & crate::world::SEARCH_SKIP_FLAGS == 0
        {
            let p = o.sys.position;
            let d = [p[0] - view[0], p[1] - view[1], p[2] - view[2]];
            candidates.push((id, (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()));
        }
    }
    let mut best: Option<(ObjId, f32)> = None;
    for (id, d) in candidates.iter().copied() {
        match best {
            Some((_, bd)) if bd <= d => {
                // The one passed over loses its glance.
                let sys = &mut ctx.object(id)?.sys;
                if sys.state_flags & state_flag::GLANCED != 0 {
                    sys.state_flags |= state_flag::GLANCE_LOST;
                }
            }
            Some((old, _)) => {
                let sys = &mut ctx.object(old)?.sys;
                if sys.state_flags & state_flag::GLANCED != 0 {
                    sys.state_flags |= state_flag::GLANCE_LOST;
                }
                best = Some((id, d));
            }
            None => best = Some((id, d)),
        }
    }
    if let Some((id, _)) = best {
        ctx.object(id)?.sys.state_flags |= state_flag::GLANCED;
    }
    let found = best.map(|(id, _)| id);
    ctx.bind_ai(m, args[0].value, found)
}

/// 6:46 FUN_14053ea60: every object but the two given starts a stop request
/// (`key`), the party-class ones only while their own target is stopped.
fn rc_stop_all(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let first = ctx.sys_of_ai(m, args[0].value)?;
    let key = args[1].value;
    let second = ctx.sys_of_ai(m, args[3].value)?;
    let mut ids: Vec<ObjId> = ctx.world.objects.iter().flatten().map(|o| o.id).collect();
    ids.push(ctx.me.id);
    for id in ids {
        if id == first || id == second {
            continue;
        }
        let sys = &mut ctx.object(id)?.sys;
        if !sys.stop_requests.contains(&key) && sys.stop_requests.len() < 4 {
            sys.stop_requests.push(key);
        }
    }
    Ok(())
}

/// 6:69 FUN_14053f180: every other object that shares this object's part id
/// is finished (the class's virtual +0x80, the same as `obj_dead`).
fn larxene_dead(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let part = ctx.object(id)?.sys.entry_part;
    let mut ids: Vec<ObjId> = ctx.world.objects.iter().flatten().map(|o| o.id).collect();
    ids.push(ctx.me.id);
    for other in ids {
        if other != id && ctx.object(other)?.sys.entry_part == part {
            ctx.object(other)?.sys.gone = true;
            ctx.world
                .events
                .push(crate::world::Event::ObjectDead { object: other });
        }
    }
    Ok(())
}

/// 1:17 FUN_14042c870: the object's class leave routine (virtual +0x38),
/// which for scripted objects is `obj_leave` (1:28) without its checks.
fn leave_force(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let obj = ctx.object(id)?;
    if !obj.sys.left {
        obj.sys.fade.start(1.0, 0.0, 0.0);
    }
    obj.sys.left = true;
    Ok(())
}

/// 1:18 FUN_14042c550 -> FUN_1403d9fe0: attach the first object to the
/// second (a grab): it is carried at bone `a` with flags `b` (physics mode 3,
/// which clears the others) and the world places it on its holder. A
/// nonzero fifth argument is the script entry and AI object the attach keeps.
fn obj_attach(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let held = ctx.sys_of_ai(m, args[0].value)?;
    let holder = ctx.sys_of_ai(m, args[1].value)?;
    let (bone, flags) = (args[2].as_int(), args[3].value as u16);
    let callback = (args[4].as_int() != 0).then(|| (args[4].as_int(), args[5].value));
    let sys = &mut ctx.object(held)?.sys;
    sys.to_ground_mode();
    sys.attached = Some(crate::sysobj::Attach {
        to: holder,
        bone,
        flags,
        callback,
    });
    // Without the "keep facing" bit (2) the carried object's offset and
    // turn reset (+0x680..+0x68f) and it is marked (+0x9b8 bit 3).
    Ok(())
}

/// 1:102 FUN_14042c5f0 -> FUN_1403da250: let go; the object is back on the
/// ground mode.
fn obj_detach(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &mut ctx.object(id)?.sys;
    sys.attached = None;
    sys.to_ground_mode();
    Ok(())
}

/// 1:161 FUN_14042b980: the object is attached to another (+0x6a0).
fn is_attach(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.attached.is_some();
    flag(args, v);
    Ok(())
}

/// 1:109 FUN_14042fd40 -> FUN_1403cc9b0 -> FUN_1401a7cf0: hide the model (the
/// part argument is ignored by the routines).
fn hide_part(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.model_hidden = true;
    Ok(())
}

/// 1:108 FUN_14042fd10 -> FUN_1403cd810 -> FUN_1401a8a90: show the model.
fn show_part(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.model_hidden = false;
    Ok(())
}

/// 1:85 FUN_14042f820 -> FUN_1403cd8a0 -> FUN_1401a87a0: start the model's
/// texture animation.
fn texanm_start(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.texture_animation = true;
    Ok(())
}

/// 1:86 FUN_14042f850 -> FUN_1403cd8b0 -> FUN_1401a8570: stop it.
fn texanm_stop(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.texture_animation = false;
    Ok(())
}

/// 1:125 FUN_1404305d0 -> FUN_1403b7810: start the texture fade (+0xaa4)
/// from a value to another over a number of ticks.
fn tex_fade_start(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let (from, to, ticks) = (args[1].as_float(), args[2].as_float(), args[3].as_float());
    ctx.object(id)?.sys.tex_fade.start(from, to, ticks);
    Ok(())
}

/// 1:40 FUN_14042bd60 -> FUN_1403baa90: the entry's pirate bit (entry +0x48
/// bit 5) with the shade state (+0xac0) at 1.
fn is_pirate_shade(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &ctx.object(id)?.sys;
    let v = sys.pirate_entry && sys.shade_state == 1;
    flag(args, v);
    Ok(())
}

/// 1:95 FUN_14042fa40 -> FUN_1403b8150: leave the fly mode into a fall that
/// continues the last tick's vertical movement (+0xa3c / g seeds the arc).
fn fly_to_jump(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &mut ctx.object(id)?.sys;
    let up = sys.moved[1];
    sys.start_fall(up);
    Ok(())
}

/// 1:115 FUN_14042cdd0: the fall and land motions of the next jump are the
/// two motions after `motion` (+0x118 = motion + 1, +0x11c = motion + 2).
fn set_jump_motion(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let motion = args[1].as_int();
    ctx.object(id)?.sys.jump_motions = JumpMotions {
        fall: motion + 1,
        land: motion + 2,
    };
    Ok(())
}

/// 1:146 FUN_14042b720 -> FUN_1403d4960: ask for a knock-back along a
/// direction with a strength. A negative strength flips the direction; the
/// slide is (strength + the sheet's hit-back addition) * 0.1 and decays.
/// [U] The routine's first gate (FUN_1403ba190: the +0x9b8 bit 0 / +0x6c8 bit
/// 27 conditions) is reduced to its last test, the "no knock" flag (+0x120
/// bit 7).
fn obj_hitback(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let direction = read_native(ctx, m, args[1].value)?;
    let strength = args[2].as_float();
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sys = &mut ctx.object(id)?.sys;
    if sys.flags >> 7 & 1 != 0 {
        return Ok(());
    }
    let (mut d, mut s) = ([direction[0], direction[1], direction[2]], strength);
    if s < 0. {
        s = -s;
        d = d.map(|c| -c);
    }
    sys.hitback = [d[0], d[1], d[2], s];
    sys.knock = actor_physics::Knock::new(d, s, sys.hitback_addition as f32);
    Ok(())
}

/// 1:192 FUN_14042d280: +0x18c bit 4 (the motion was cancelled).
fn is_motion_cancel(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let bit = ctx.object(id)?.sys.motion.flags.word >> 4 & 1;
    int(args, bit as i32);
    Ok(())
}

/// 1:224 FUN_14042d120: the current physics mode has the jump or fly bits
/// (mode flags +0xc & 3).
fn is_action_air(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let air = ctx.object(id)?.sys.mode_flags() & 3 != 0;
    flag(args, air);
    Ok(())
}

/// 1:254 FUN_14042c400: something rides the object (+0x9d8).
fn is_ride(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let riding = ctx.object(id)?.sys.rider != 0;
    flag(args, riding);
    Ok(())
}

/// 1:265 FUN_14042e110: +0xd1c.
fn set_medal(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.medal = args[1].as_int();
    Ok(())
}

/// 1:266 FUN_14042e140: +0xd1c.
fn get_medal(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.medal;
    int(args, v);
    Ok(())
}

/// 1:272 FUN_14042dd10: +0x9b8 bit 2 (finished by `obj_dead`).
fn is_dead(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let dead = ctx.object(id)?.sys.gone;
    flag(args, dead);
    Ok(())
}

/// 1:319 FUN_14042b2a0: the class bit of Sora (+0x6cb bit 0). The player proxy
/// is Sora.
fn is_sora(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let sora = id == PLAYER_OBJECT || ctx.object(id)?.sys.class_flags & class_flag::SORA != 0;
    flag(args, sora);
    Ok(())
}

/// 1:323 FUN_14042e020 -> FUN_1403b3e20: how many objects in the list have
/// this entry id.
fn count_entry(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let entry = args[0].value;
    let mut count = 0;
    ctx.search(|o| {
        count += i32::from(o.sys.entry_id == entry);
        false
    });
    int(args, count);
    Ok(())
}

/// 1:324 FUN_14042bab0: forbid-bit `n` of the pattern mask is cleared.
fn pattern_reset(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let bit = args[1].value & 31;
    ctx.object(id)?.sys.pattern_mask &= !(1 << bit);
    Ok(())
}

/// 2:35 FUN_140432020: the attack type byte (ATKP +4) of the hit.
fn damage_attack_type(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = crate::world::object_field(args[0].value).map_or(0, |(id, _)| id);
    let v = ctx.object(id)?.sys.damage_attack_type;
    int(args, i32::from(v));
    Ok(())
}

/// 2:70 FUN_140432100: the damage record's part byte (+0x25).
fn damage_parts(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = crate::world::object_field(args[0].value).map_or(0, |(id, _)| id);
    let v = ctx.object(id)?.sys.damage_parts;
    int(args, i32::from(v));
    Ok(())
}

/// 2:95 FUN_140431aa0 -> FUN_1404112b0: give the object its own copy of
/// battle sheet `n` (the sheet is per object here already).
fn dup_sheet(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.sheet = args[1].as_int();
    Ok(())
}

/// 2:97 FUN_140431ec0 -> FUN_140411300: remember the attacker; one on Sora's
/// team (+0x4dc == 1) is also kept for friendly fire (+0xd40).
fn set_attacker(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let attacker = ctx.sys_of_ai(m, args[1].value)?;
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let (target, team) = if attacker == PLAYER_OBJECT {
        (super::target::PLAYER_TARGET, 1)
    } else {
        (attacker, ctx.object(attacker)?.sys.team)
    };
    let sys = &mut ctx.object(id)?.sys;
    sys.last_attacker = Some(target);
    if team == 1 {
        sys.friend_attacker = Some(target);
    }
    Ok(())
}

/// 6:45 FUN_14053e960: +0x124 bit 9.
fn hide_shadow(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.state_flags |= state_flag::HIDE_SHADOW;
    Ok(())
}

/// 6:49 FUN_14053de60 -> FUN_1403baad0: the object is stopped. [U] Only the
/// stop requests (+0x64c bit 0) are modelled; the other two conditions (a
/// global stop for class bits 2 and 6, DAT_142a24edc and FUN_14038ee40) read
/// the engine's global stop state.
fn is_stop(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let stopped = !ctx.object(id)?.sys.stop_requests.is_empty();
    flag(args, stopped);
    Ok(())
}

/// 6:56 FUN_14053e560 -> FUN_1403bee60: rewrite the record to the same target
/// in group `n` (no second handle, kind -1).
fn target_set_group(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let handle = args[0].value;
    let group = args[1].as_int();
    let t = super::target::record_of(ctx, handle)?;
    super::target::set_record(
        ctx,
        handle,
        Target {
            id: t.id,
            kind: -1,
            group,
        },
    )
}

/// 6:57 FUN_14053e5b0: the record's group word (+4).
fn target_get_group(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let t = super::target::record_of(ctx, args[0].value)?;
    int(args, t.group);
    Ok(())
}

/// 6:60 FUN_14053eda0: the three colour bytes at +0x7e9.
fn set_stealth_color(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.stealth_color = [1, 2, 3].map(|k| args[k].value as u8);
    Ok(())
}

/// 6:63 FUN_14053ee50 -> FUN_1403d4880: the playing motion is a damage
/// reaction (slots 10 to 39).
fn is_damage_motion(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let slot = ctx.object(id)?.sys.motion.slot;
    flag(args, (10..40).contains(&slot));
    Ok(())
}

/// 10:52 FUN_1405370d0 -> FUN_1403d48d0: the playing motion is a reflect
/// reaction (slots 140 to 150).
fn is_reflect_motion(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let slot = ctx.object(id)?.sys.motion.slot;
    flag(args, (140..151).contains(&slot));
    Ok(())
}

/// 6:65 FUN_14053eeb0: +0x8c8 bit 5.
fn set_scissoring(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.scissoring = args[1].value != 0;
    Ok(())
}

/// 6:66 FUN_14053eef0: zero the knock-back request (+0xd24..+0xd33).
fn clear_hitback(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    ctx.object(id)?.sys.hitback = [0.; 4];
    Ok(())
}

/// 7:0 FUN_14053f2a0 -> FUN_1403dbaa0(key): every object of class bit 4
/// (the enemies) starts a stop request `key` (FUN_1403b6730); 6:46
/// `obj_rc_stop_all` does the same for the other objects. A request is
/// kept once per key (the table holds four).
fn stop_all_start(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let key = args[0].value;
    let me = ctx.me.id;
    let ids: Vec<_> = ctx
        .world
        .objects
        .iter()
        .flatten()
        .map(|o| o.id)
        .chain(std::iter::once(me))
        .collect();
    for id in ids {
        let sys = &mut ctx.object(id)?.sys;
        if sys.class_flags & class_flag::ENEMY == 0 {
            continue;
        }
        if !sys.stop_requests.contains(&key) && sys.stop_requests.len() < 4 {
            sys.stop_requests.push(key);
        }
    }
    Ok(())
}

/// 6:47 FUN_14053eb10 and 7:1 FUN_14053f2b0 -> FUN_14040a770: every object
/// drops stop request `key`.
fn stop_end_all(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let key = args[0].value;
    let me = ctx.me.id;
    let ids: Vec<_> = ctx
        .world
        .objects
        .iter()
        .flatten()
        .map(|o| o.id)
        .chain(std::iter::once(me))
        .collect();
    for id in ids {
        ctx.object(id)?.sys.stop_requests.retain(|k| *k != key);
    }
    Ok(())
}

/// 10:17 FUN_140536810: +0x6c8 bit 11.
fn is_zako(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.class_flags & class_flag::ZAKO != 0;
    flag(args, v);
    Ok(())
}

/// 10:19 FUN_140536890: +0x126 bit 0 (+0x124 bit 16).
fn is_limit(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.state_flags & state_flag::LIMIT != 0;
    flag(args, v);
    Ok(())
}

/// 10:22 FUN_140536970: the physics mode has the blow bit (+0xc bit 6).
fn is_blow(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.mode_flags() & 0x40 != 0;
    flag(args, v);
    Ok(())
}

/// 10:31 FUN_140536c30: +0x6c8 bit 22.
fn is_summon(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let v = ctx.object(id)?.sys.class_flags & class_flag::SUMMON != 0;
    flag(args, v);
    Ok(())
}
