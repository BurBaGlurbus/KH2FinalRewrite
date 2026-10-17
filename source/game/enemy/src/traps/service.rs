//! Traps that hand a request to a subsystem this crate does not own (the
//! camera, the screen, the mission HUD, sound, the party, the prize tables)
//! and the world state a few of them change (lights, the global status bits,
//! enemy units). Translated from the executable (Steam build 15194255); each
//! doc line names the wrapper and the routine it calls.
//!
//! A request becomes an [`Event::Service`] carrying the trap's name and its
//! operands decoded as far as the wrapper does before the hand-over (objects
//! resolved, vectors read in the engine's +Y-up frame). What the host does
//! with it is the host's; this crate only guarantees that the script's call
//! was made with these operands, in this order. [U] Where a trap returns a
//! value from the other subsystem the value comes from the host's
//! [`Env`](crate::env::Env) (0 when it has none).

use super::math::{read_native, read_sanitized};
use crate::script::{Fault, Memory, Slot};
use crate::sysobj::class_flag;
use crate::world::{
    Ctx, Event, LIGHT_HANDLES, Light, ObjId, PLAYER_OBJECT, SysRef, TrapFn, VACUUM_HANDLES, Vacuum,
    status,
};

type Args = [Slot; 16];

pub fn lookup(table: u16, index: u16) -> Option<TrapFn> {
    Some(match (table, index) {
        // Lights.
        (1, 181) => light_create,
        (1, 182) => light_set_flag,
        (1, 183) => light_set_color,
        (1, 184) => light_fadeout,
        (1, 251) => light_fadein,
        (1, 257) => light_fog_near,
        (1, 258) => light_fog_far,
        (1, 259) => light_fog_min,
        (1, 260) => light_fog_max,
        // The global status bits and the enemy units.
        (1, 187) => prize_drain_start,
        (1, 188) => prize_drain_end,
        (1, 204) => force_leave_start,
        (1, 205) => force_leave_end,
        (1, 202) => unit_disable,
        (1, 203) => unit_enable,
        (1, 320) => unit_enemy_count,
        (4, 13) => mission_pause_timer,
        (4, 24) => mission_cancel_pause_timer,
        // Host state read by scripts.
        (0, 80) => pad_is_button,
        (0, 81) => pad_is_trigger,
        (1, 197) => area_cost_rest,
        (4, 21) => mission_is_activate2d,
        (4, 26) => mission_get_timer,
        (4, 35) => score_type,
        (4, 36) => score_score,
        (1, 355) => player_can_capture_form,
        (1, 341) => prize_num_ca,
        (1, 339) => prize_vacuum_ca,
        (7, 33) => enemy_summon_entry,
        (1, 316) | (8, 8) => host_vector,
        (0, 54) => effect_dir,
        // Requests handed on.
        (0, 31) => screen_whiteout,
        (0, 88) => screen_fadein,
        (0, 104) => sound_set_bgse_volume,
        (1, 164) => camera_start_global,
        (1, 193) => camera_warp,
        (1, 207) => camera_watch,
        (1, 223) => camera_cancel,
        (1, 237) => command_override_top,
        (1, 281) => camera_begin_scope,
        (1, 283) => camera_end_scope,
        (1, 312) => camera_scope_zoom,
        (1, 313) => camera_scope_closeup,
        (1, 314) => camera_scope_target,
        (6, 0) => camera_shake,
        (1, 326) => bg_animation_speed,
        (1, 44) => history_clear_enemy,
        (1, 354) => event_layer_off,
        (1, 356) => event_layer_on,
        (1, 91) => obj_star,
        (1, 126) => motion_sync,
        (1, 236) => motion_unsync,
        (1, 185) => set_parts_color,
        (1, 186) => reset_parts_color,
        (1, 194) => set_stealth,
        (1, 195) => reset_stealth,
        (1, 255) => disable_occ,
        (1, 256) => enable_occ,
        (1, 335) => disable_occ_bone,
        (1, 336) => enable_occ_bone,
        (1, 270) => disable_anmatr_effect,
        (1, 271) => enable_anmatr_effect,
        (1, 268) => action_lightcycle,
        (6, 62) => carpet_idle,
        (1, 267) => scatter_medal,
        (1, 278) => scatter_prize,
        (1, 279) | (1, 363) | (6, 1) => prize_appear,
        (1, 327) => prize_get_all,
        (1, 340) => prize_vacuum_range,
        (1, 342) => prize_appear_num,
        (1, 362) => treasure_get,
        (1, 359) => hook_command_image,
        (1, 360) => reset_command_image,
        (1, 365) => command_disable_group,
        (1, 366) => command_enable_group,
        (1, 285) => show_picture,
        (1, 290) => picture_change,
        (1, 315) => picture_set_pos,
        (6, 27) => player_dice,
        (6, 28) => dice_set_spec,
        (6, 29) => player_card,
        (6, 30) => card_set_spec,
        (6, 70) => play_se_loop,
        (6, 71) => fadeout_se,
        (4, 8) => mission_set_gauge,
        (4, 10) => mission_set_gauge_ratio,
        (4, 14) => mission_activate2d,
        (4, 17) => mission_set_timer_param,
        (4, 18) => mission_set_count_param,
        (4, 56) => event_continue_control_off,
        (4, 38) => score_get,
        (8, 2) => set_xyzrot,
        (8, 3) => laser_start,
        (8, 4) => laser_attack,
        (8, 5) => laser_end,
        (8, 6) => laser_optimize,
        (8, 7) => laser_optimize_end,
        (10, 6) => friend_set_special_command,
        (10, 7) => friend_reset_special_command,
        (10, 14) => friend_set_target,
        (10, 48) => friend_disable_follow,
        (10, 56) => friend_invalidate_warp,
        (10, 57) => friend_add_warp,
        (6, 13) => friend_force_warp,
        (6, 14) => friend_get,
        (6, 59) => xemnas_get_obj,
        (1, 234) => party_get_weapon,
        (6, 58) => act_child_push,
        (2, 57) => limit_effect_start,
        (1, 169) => effect_unbind,
        (5, 8) => create_active_path,
        (10, 58) => friend_link_magic,
        (10, 35) => friend_get_action_param,
        (1, 242) => player_capture_form,
        (1, 243) => player_capture_form_end,
        // Vacuums.
        (2, 77) => vacuum_create,
        (2, 78) => vacuum_destroy,
        (2, 79) => vacuum_set_flag,
        (2, 80) => vacuum_set_position,
        (2, 81) => vacuum_set_18,
        (2, 82) => vacuum_set_24,
        (2, 83) => vacuum_set_28,
        (2, 84) => vacuum_set_2c,
        (7, 32) => vacuum_set_effective_range,
        // The object list and the map.
        (1, 331) => obj_each_all,
        (2, 4) => enemy_each,
        (1, 135) => bg_cross_pos,
        _ => return None,
    })
}

// --- Decoding -----------------------------------------------------------

fn object_of(ctx: &Ctx<'_>, m: &Memory<'_>, ai: u32) -> Option<ObjId> {
    match ctx.sys_ref_ai(m, ai).ok()? {
        SysRef::Player => Some(PLAYER_OBJECT),
        SysRef::Object(id) => Some(id),
    }
}

/// Hand a request on. `object` and `other` name the arguments that are AI
/// objects, `vector` the one that is a vector.
fn forward(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &Args,
    name: &'static str,
    object: Option<usize>,
    other: Option<usize>,
    vector: Option<usize>,
) -> Result<(), Fault> {
    let object = object.and_then(|k| object_of(ctx, m, args[k].value));
    let other = other.and_then(|k| object_of(ctx, m, args[k].value));
    let vector = match vector {
        Some(k) => Some(read_native(ctx, m, args[k].value)?),
        None => None,
    };
    let mut words = [0u32; 8];
    for (k, w) in words.iter_mut().enumerate() {
        *w = args[k].value;
    }
    ctx.world.events.push(Event::Service {
        name,
        object,
        other,
        words,
        vector,
    });
    Ok(())
}

macro_rules! forward_traps {
    ($($name:ident = $label:literal, $object:expr, $other:expr, $vector:expr;)*) => {$(
        fn $name(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
            forward(ctx, m, args, $label, $object, $other, $vector)
        }
    )*};
}

forward_traps! {
    screen_whiteout = "func_screen_whiteout", None, None, None;
    screen_fadein = "screen_fadein", None, None, None;
    sound_set_bgse_volume = "sound_set_bgse_volume", None, None, None;
    camera_start_global = "obj_camera_start_global", Some(0), Some(1), None;
    camera_warp = "camera_warp", None, None, None;
    camera_watch = "camera_watch", Some(0), None, None;
    camera_cancel = "camera_cancel", None, None, None;
    command_override_top = "command_override_top", Some(4), None, None;
    camera_begin_scope = "camera_begin_scope", Some(0), None, None;
    camera_end_scope = "camera_end_scope", None, None, None;
    camera_scope_zoom = "camera_set_scope_zoom", None, None, None;
    camera_scope_closeup = "camera_set_scope_closeup_distance", None, None, None;
    camera_scope_target = "camera_set_scope_target_pos", None, None, Some(0);
    camera_shake = "camera_shake", None, None, None;
    bg_animation_speed = "bg_set_animation_speed", None, None, None;
    history_clear_enemy = "func_history_clear_enemy", None, None, None;
    event_layer_off = "event_layer_off", None, None, None;
    event_layer_on = "event_layer_on", None, None, None;
    obj_star = "obj_star", Some(0), None, None;
    motion_sync = "obj_motion_sync", Some(0), Some(1), None;
    motion_unsync = "obj_motion_unsync", Some(0), Some(1), None;
    set_parts_color = "obj_set_parts_color", Some(0), None, None;
    reset_parts_color = "obj_reset_parts_color", Some(0), None, None;
    set_stealth = "obj_set_stealth", Some(0), None, None;
    reset_stealth = "obj_reset_stealth", Some(0), None, None;
    disable_occ = "obj_disable_occ", Some(0), None, None;
    enable_occ = "obj_enable_occ", Some(0), None, None;
    disable_occ_bone = "obj_disable_occ_bone", Some(0), None, None;
    enable_occ_bone = "obj_enable_occ_bone", Some(0), None, None;
    disable_anmatr_effect = "obj_motion_disable_anmatr_effect", Some(0), None, None;
    enable_anmatr_effect = "obj_motion_enable_anmatr_effect", Some(0), None, None;
    action_lightcycle = "obj_action_lightcycle", Some(0), None, None;
    carpet_idle = "obj_carpet_obj_idle", Some(0), None, None;
    scatter_medal = "obj_scatter_medal", Some(0), None, None;
    scatter_prize = "obj_scatter_prize_tr", Some(0), None, None;
    prize_appear = "prize_appear", None, None, Some(0);
    prize_get_all = "prize_get_all_tr", Some(0), None, None;
    prize_vacuum_range = "prize_vacuum_range_ca", Some(0), None, None;
    prize_appear_num = "prize_appear_num", None, None, Some(0);
    treasure_get = "treasure_get", None, None, None;
    hook_command_image = "obj_hook_command_image", Some(0), None, None;
    reset_command_image = "obj_reset_command_image", Some(0), None, None;
    command_disable_group = "command_disable_group", None, None, None;
    command_enable_group = "command_enable_group", None, None, None;
    picture_change = "picture_change", Some(0), None, None;
    picture_set_pos = "picture_set_pos", Some(0), None, None;
    player_dice = "player_dice", None, None, None;
    dice_set_spec = "dice_set_spec", None, None, None;
    player_card = "player_card", None, None, None;
    card_set_spec = "card_set_spec", None, None, None;
    fadeout_se = "obj_fadeout_se", None, None, None;
    mission_set_gauge = "mission_set_gauge", None, None, None;
    mission_set_gauge_ratio = "mission_set_gauge_ratio", None, None, None;
    mission_activate2d = "mission_activate2d", None, None, None;
    mission_set_timer_param = "mission_set_timer_param", None, None, None;
    mission_set_count_param = "mission_set_count_param", None, None, None;
    event_continue_control_off = "event_continue_control_off", None, None, None;
    set_xyzrot = "obj_set_xyzrot", Some(0), None, Some(1);
    laser_start = "special_last_xemnus_laser_start", Some(0), None, None;
    laser_attack = "special_last_xemnus_laser_attack", None, None, None;
    laser_end = "special_last_xemnus_laser_end", None, None, None;
    laser_optimize = "special_last_xemnus_laser_optimize", None, None, None;
    laser_optimize_end = "special_last_xemnus_laser_optimize_end", None, None, None;
    friend_set_special_command = "obj_set_special_command", Some(0), None, None;
    friend_reset_special_command = "obj_reset_special_command", Some(0), None, None;
    friend_set_target = "friend_set_target", Some(0), Some(2), None;
    friend_disable_follow = "friend_disable_follow_player", Some(0), None, None;
    friend_invalidate_warp = "friend_invalidate_warp_point", None, None, Some(0);
    friend_add_warp = "friend_add_warp_point", None, None, Some(0);
    friend_force_warp = "friend_force_warp", Some(0), None, None;
    friend_link_magic = "friend_link_magic", Some(0), Some(1), None;
    player_capture_form = "player_capture_form", None, None, None;
    player_capture_form_end = "player_capture_form_end", None, None, None;
    act_child_push = "obj_act_child_push", Some(0), Some(1), None;
    effect_unbind = "obj_effect_unbind", Some(0), None, None;
    create_active_path = "create_active_path", Some(0), Some(1), None;
}

/// A request that returns an AI object the host made: forward it and hand the
/// script a null object (the executable returns 0 when the other subsystem
/// has nothing).
macro_rules! forward_object_traps {
    ($($name:ident = $label:literal, $object:expr;)*) => {$(
        fn $name(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
            forward(ctx, m, args, $label, $object, None, None)?;
            args[0] = Slot::address(0);
            Ok(())
        }
    )*};
}

forward_object_traps! {
    show_picture = "obj_show_picture", Some(0);
    friend_get = "friend_get", None;
    xemnas_get_obj = "xemnas_get_obj", None;
    party_get_weapon = "party_get_weapon", Some(0);
    score_get = "score_get", None;
    friend_get_action_param = "friend_get_action_param", Some(0);
}

/// 6:70 FUN_14053f210: start a looping sound effect on the object; the
/// script gets a sound handle (a counter here).
fn play_se_loop(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    forward(ctx, m, args, "obj_play_se_loop", Some(0), None, None)?;
    let handle = ctx.world.next_effect_handle() as i32;
    args[0] = Slot::int(handle);
    Ok(())
}

/// 2:57 FUN_140432fb0 -> FUN_1403d9b60: start an effect of the object's
/// container with priority 30000 (FUN_1402c6b30); the script gets the
/// effect handle.
fn limit_effect_start(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let id = ctx.sys_of_ai(m, args[0].value)?;
    let (effect, argument) = (args[1].as_int(), args[2].as_int());
    let handle = ctx.world.next_effect_handle();
    ctx.world.events.push(Event::EffectStart {
        object: id,
        effect,
        arguments: [argument, 0],
        handle,
        position: None,
        bound: true,
        other: None,
    });
    args[0] = Slot::address(handle);
    Ok(())
}

// --- Lights ---------------------------------------------------------------

fn light<'a>(ctx: &'a mut Ctx<'_>, handle: u32) -> Result<&'a mut Light, Fault> {
    let slot = ctx.world.light_slot(handle).ok_or(Fault::Memory(handle))?;
    ctx.world.lights[slot].as_mut().ok_or(Fault::Memory(handle))
}

/// 1:181 FUN_140430950 -> FUN_1403f6f80: a light of the given kind, fully
/// faded in, grey (0x80808080), no flags. A kind 1 light takes the map's
/// fog.
fn light_create(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let kind = args[0].as_int();
    let fog = if kind == 1 {
        ctx.env.default_fog()
    } else {
        [0.; 4]
    };
    let slot = ctx.world.lights.iter().position(Option::is_none);
    let slot = slot.unwrap_or_else(|| {
        ctx.world.lights.push(None);
        ctx.world.lights.len() - 1
    });
    ctx.world.lights[slot] = Some(Light {
        kind,
        fade: crate::sysobj::Fade {
            value: 1.0,
            rate: 0.0,
        },
        flags: vec![0],
        color: [0x80; 4],
        fog,
    });
    args[0] = Slot::address(LIGHT_HANDLES + slot as u32 * 16);
    Ok(())
}

/// 1:182 FUN_140430980: set flag bit `n`.
fn light_set_flag(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let bit = args[1].value;
    let light = light(ctx, args[0].value)?;
    let word = (bit >> 5) as usize;
    if light.flags.len() <= word {
        light.flags.resize(word + 1, 0);
    }
    light.flags[word] |= 1 << (bit & 31);
    Ok(())
}

/// 1:183 FUN_1404309c0 -> FUN_1403f76e0: the colour (r, g, b, alpha 0x80) and
/// a fade in from nothing over `ticks`.
fn light_set_color(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let color = [1, 2, 3].map(|k| args[k].value as u8);
    let ticks = args[4].as_float();
    let light = light(ctx, args[0].value)?;
    light.color = [color[0], color[1], color[2], 0x80];
    light.fade.start(0.0, 1.0, ticks);
    Ok(())
}

/// 1:184 FUN_140430a20 -> FUN_1403f76a0: fade out from the current level.
fn light_fadeout(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let ticks = args[1].as_float();
    let light = light(ctx, args[0].value)?;
    let from = light.fade.value;
    light.fade.start(from, 0.0, ticks);
    Ok(())
}

/// 1:251 FUN_140430b10 -> FUN_1403f7680: fade in from nothing.
fn light_fadein(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let ticks = args[1].as_float();
    light(ctx, args[0].value)?.fade.start(0.0, 1.0, ticks);
    Ok(())
}

macro_rules! fog_traps {
    ($($name:ident = $slot:literal;)*) => {$(
        fn $name(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
            let v = args[1].as_float();
            light(ctx, args[0].value)?.fog[$slot] = v;
            Ok(())
        }
    )*};
}

fog_traps! {
    light_fog_near = 0;
    light_fog_far = 1;
    light_fog_min = 2;
    light_fog_max = 3;
}

// --- Vacuums -------------------------------------------------------------------

fn vacuum<'a>(ctx: &'a mut Ctx<'_>, handle: u32) -> Result<&'a mut Vacuum, Fault> {
    let slot = ctx.world.vacuum_slot(handle).ok_or(Fault::Memory(handle))?;
    ctx.world.vacuums[slot]
        .as_mut()
        .ok_or(Fault::Memory(handle))
}

/// 2:77 FUN_140433200 -> FUN_1403f8f20: a vacuum at the point, with the
/// argument in its three strength values, range 0, +0x28 = 100, +0x2c = 0.01.
fn vacuum_create(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let at = read_sanitized(ctx, m, args[0].value)?;
    let strength = args[1].as_float();
    let slot = ctx.world.vacuums.iter().position(Option::is_none);
    let slot = slot.unwrap_or_else(|| {
        ctx.world.vacuums.push(None);
        ctx.world.vacuums.len() - 1
    });
    ctx.world.vacuums[slot] = Some(Vacuum {
        position: at,
        flags: 0,
        effective_range: 0.,
        at_18: [strength; 3],
        at_24: 0.,
        at_28: 100.,
        at_2c: 0.01,
    });
    args[0] = Slot::address(VACUUM_HANDLES + slot as u32 * 16);
    Ok(())
}

/// 2:78 FUN_1404332c0: unlink and free the vacuum.
fn vacuum_destroy(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    if let Some(slot) = ctx.world.vacuum_slot(args[0].value) {
        ctx.world.vacuums[slot] = None;
    }
    Ok(())
}

/// 2:79 FUN_1404332f0: flag bit `n` (+0x10).
fn vacuum_set_flag(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let bit = args[1].value & 31;
    vacuum(ctx, args[0].value)?.flags |= 1 << bit;
    Ok(())
}

/// 2:80 FUN_140433320: the point it pulls toward (+0).
fn vacuum_set_position(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let at = read_sanitized(ctx, m, args[1].value)?;
    vacuum(ctx, args[0].value)?.position = at;
    Ok(())
}

/// 2:81 FUN_1404333a0: the three values at +0x18.
fn vacuum_set_18(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = [1, 2, 3].map(|k| args[k].as_float());
    vacuum(ctx, args[0].value)?.at_18 = v;
    Ok(())
}

macro_rules! vacuum_float_traps {
    ($($name:ident = $field:ident;)*) => {$(
        fn $name(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
            let v = args[1].as_float();
            vacuum(ctx, args[0].value)?.$field = v;
            Ok(())
        }
    )*};
}

vacuum_float_traps! {
    vacuum_set_24 = at_24;
    vacuum_set_28 = at_28;
    vacuum_set_2c = at_2c;
    vacuum_set_effective_range = effective_range;
}

// --- The object list and the map ----------------------------------------------------

/// 1:331 FUN_14042e040 -> FUN_1403bf4f0: the object after the one handed in
/// (the first for 0), or 0.
fn obj_each_all(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let after = crate::world::object_from_handle(args[0].value);
    let next = ctx.next_object(after, |_| true);
    args[0] = Slot::address(next.map_or(0, crate::world::object_handle));
    Ok(())
}

/// 2:4 FUN_140431ce0 -> FUN_1403db9d0: the next object of the enemy class
/// (+0x6c8 bit 4) after the one handed in, or 0.
fn enemy_each(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let after = crate::world::object_from_handle(args[0].value);
    let next = ctx.next_object(after, |o| o.sys.class_flags & class_flag::ENEMY != 0);
    args[0] = Slot::address(next.map_or(0, crate::world::object_handle));
    Ok(())
}

/// 1:135 FUN_14042e6b0 -> FUN_1403f3c60: does the map cross the segment
/// from `a` to `b`; the first crossing goes into the vector `out` when it
/// does.
fn bg_cross_pos(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let a = read_native(ctx, m, args[0].value)?;
    let b = read_native(ctx, m, args[1].value)?;
    let out = args[2].value;
    let hit = ctx.env.line_hit([a[0], a[1], a[2]], [b[0], b[1], b[2]]);
    ctx.world.last_line_cross = hit;
    if let Some(at) = hit {
        for (k, lane) in [at[0], -at[1], at[2]].iter().enumerate() {
            ctx.write_f32(m, out + 4 * k as u32, *lane);
        }
    }
    args[0] = Slot::int(hit.is_some() as i32);
    Ok(())
}

// --- Global state -----------------------------------------------------------

fn set_status(ctx: &mut Ctx<'_>, bit: u32, on: bool) {
    if on {
        ctx.world.status |= bit;
    } else {
        ctx.world.status &= !bit;
    }
}

/// 1:187 and 1:188 FUN_140430840 / FUN_140430850: DAT_142a11400 bit 3.
fn prize_drain_start(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, _: &mut Args) -> Result<(), Fault> {
    set_status(ctx, status::PRIZE_DRAIN, true);
    Ok(())
}

fn prize_drain_end(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, _: &mut Args) -> Result<(), Fault> {
    set_status(ctx, status::PRIZE_DRAIN, false);
    Ok(())
}

/// 1:204 and 1:205 FUN_140430860 / FUN_140430870: DAT_142a11400 bit 7.
fn force_leave_start(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, _: &mut Args) -> Result<(), Fault> {
    set_status(ctx, status::FORCE_LEAVE, true);
    Ok(())
}

fn force_leave_end(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, _: &mut Args) -> Result<(), Fault> {
    set_status(ctx, status::FORCE_LEAVE, false);
    Ok(())
}

/// 1:202 and 1:203 FUN_140430ff0 / FUN_140431000: bit 0 of the unit's status
/// word in the area's enemy unit table (DAT_142ae8134..).
fn unit_disable(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    ctx.world.units_disabled.insert(args[0].value);
    Ok(())
}

fn unit_enable(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    ctx.world.units_disabled.remove(&args[0].value);
    Ok(())
}

/// 4:13 and 4:24 FUN_1404335d0 / FUN_1404335e0: the mission timer's pause bit.
fn mission_pause_timer(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, _: &mut Args) -> Result<(), Fault> {
    ctx.world.mission_timer_paused = true;
    Ok(())
}

fn mission_cancel_pause_timer(
    ctx: &mut Ctx<'_>,
    _: &mut Memory<'_>,
    _: &mut Args,
) -> Result<(), Fault> {
    ctx.world.mission_timer_paused = false;
    Ok(())
}

// --- Host state -----------------------------------------------------------------

/// 1:320 FUN_140431010 -> FUN_1403fdfc0: how many enemies the area's units
/// still have (the sum of the table's per-unit counts).
fn unit_enemy_count(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let n = ctx.env.service_value("unit_enemy_count", 0);
    args[0] = Slot::int(n);
    Ok(())
}

/// 0:80 FUN_14041daa0: bit `n` of the held pad buttons.
fn pad_is_button(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let bit = args[0].value & 63;
    args[0] = Slot::int((ctx.env.pad_held() >> bit & 1) as i32);
    Ok(())
}

/// 0:81 FUN_14041dae0: bit `n` of the buttons pressed this frame.
fn pad_is_trigger(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let bit = args[0].value & 63;
    args[0] = Slot::int((ctx.env.pad_pressed() >> bit & 1) as i32);
    Ok(())
}

/// A trap that returns a number the host knows (`argument` is the index of
/// the argument handed over with the query, if any).
macro_rules! host_value_traps {
    ($($name:ident = $label:literal, $argument:expr;)*) => {$(
        fn $name(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
            let argument: Option<usize> = $argument;
            let argument = argument.map_or(0, |k| args[k].as_int());
            let v = ctx.env.service_value($label, argument);
            args[0] = Slot::int(v);
            Ok(())
        }
    )*};
}

host_value_traps! {
    area_cost_rest = "area_cost_rest", None;
    mission_is_activate2d = "mission_is_activate2d", None;
    mission_get_timer = "mission_get_timer", Some(0);
    score_type = "score_type", Some(0);
    score_score = "score_score", Some(0);
    player_can_capture_form = "player_can_capture_form", None;
    prize_num_ca = "prize_num_ca", None;
    prize_vacuum_ca = "prize_vacuum_ca", Some(2);
    enemy_summon_entry = "enemy_summon_entry", Some(0);
}

/// A vector the camera or the effect system computes (the projection of a
/// point, the inverse of a camera transform). The host answers; with no
/// answer the script gets the zero vector (w 1).
fn host_vector(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let input = read_native(ctx, m, args[0].value).unwrap_or([0.; 4]);
    let v = ctx.env.camera_vector(input).unwrap_or([0., 0., 0., 1.]);
    let cell = ctx.world.static_cell(1, 316, 0);
    for (k, lane) in [v[0], -v[1], v[2], v[3]].iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    args[0] = Slot::address(cell);
    Ok(())
}

/// 0:54 FUN_14041e380: the direction an effect faces (its matrix applied to
/// the default forward vector).
fn effect_dir(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let v = ctx
        .env
        .effect_direction(args[0].value)
        .map_or([0., 0., 0., 1.], |d| [d[0], d[1], d[2], 1.]);
    let cell = ctx.world.static_cell(0, 54, 0);
    for (k, lane) in [v[0], -v[1], v[2], v[3]].iter().enumerate() {
        ctx.write_f32(m, cell + 4 * k as u32, *lane);
    }
    args[0] = Slot::address(cell);
    Ok(())
}

#[cfg(test)]
mod tests;
