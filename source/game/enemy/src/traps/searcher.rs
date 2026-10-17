//! Table 6 target searcher traps (`trap_target_searcher_*`): a searcher is a
//! struct in the script's memory that remembers recent picks and holds the
//! range limits; a search walks the object list for the TARGET shapes of the
//! objects of one kind and writes the best one into a target record.
//! Translated from the executable (Steam build 15194255): FUN_14053fb60
//! (init), FUN_140540080 (add), FUN_14053fb80 (search), FUN_14053ff10 (search
//! one object) and FUN_140540300 (reset).
//!
//! Searcher words: 0 capacity, 1 (byte 4 write cursor, byte 5 shapes seen,
//! byte 6 flags), 2 history count, 3 kind (0 enemies, 1 and 2 Sora), 4
//! nearest allowed, 5 farthest allowed, 6 half angle (pi = all around), then
//! (object handle, shape id) pairs from word 7.
//!
//! [U] The TARGET shapes themselves (collision records with bone offsets and
//! radii) are not modelled: an object has one shape, id 0, at its position
//! unless the host's [`Env::target_shapes`] says otherwise; the candidate
//! test is the kind's class bit plus "alive" in place of FUN_1403ba260.

use super::math::{read_native, wrap_angle};
use crate::script::{Fault, Memory, Slot};
use crate::sysobj::{Target, Vec3, class_flag};
use crate::world::{
    Ctx, ObjId, PLAYER_HANDLE, PLAYER_OBJECT, SEARCH_SKIP_FLAGS, TrapFn, object_handle,
};

type Args = [Slot; 16];

pub fn lookup(table: u16, index: u16) -> Option<TrapFn> {
    if table != 6 {
        return None;
    }
    Some(match index {
        3 => searcher_init,
        4 => searcher_reset,
        5 => searcher_search,
        8 => searcher_add,
        11 => searcher_search_object,
        16 => target_clear,
        _ => return None,
    })
}

const CAPACITY: u32 = 0;
/// Byte 0 of word 1 is the write cursor, byte 1 the shapes seen by the last
/// search.
const CURSOR: u32 = 4;
const COUNT: u32 = 8;
const KIND: u32 = 12;
const NEAREST: u32 = 16;
const FARTHEST: u32 = 20;
const ANGLE: u32 = 24;
const ENTRIES: u32 = 28;

fn word(ctx: &Ctx<'_>, m: &Memory<'_>, at: u32) -> Result<u32, Fault> {
    ctx.read_u32(m, at).ok_or(Fault::Memory(at))
}

fn put(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, at: u32, v: u32) -> Result<(), Fault> {
    ctx.write_u32(m, at, v)
        .then_some(())
        .ok_or(Fault::Memory(at))
}

/// 6:3 FUN_14053dce0 -> FUN_14053fb60: capacity `n`, empty history, range 0
/// to 1000, all around.
fn searcher_init(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let s = args[0].value;
    put(ctx, m, s + CAPACITY, args[1].value)?;
    put(ctx, m, s + COUNT, 0)?;
    put(ctx, m, s + KIND, 0)?;
    put(ctx, m, s + NEAREST, 0)?;
    put(ctx, m, s + CURSOR, 0)?;
    put(ctx, m, s + FARTHEST, 1000f32.to_bits())?;
    put(ctx, m, s + ANGLE, std::f32::consts::PI.to_bits())
}

/// 6:4 FUN_14053dd00 -> FUN_140540300: forget the history (count 0, cursor 0).
fn searcher_reset(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let s = args[0].value;
    put(ctx, m, s + COUNT, 0)?;
    let w = word(ctx, m, s + CURSOR)? & !0xff;
    put(ctx, m, s + CURSOR, w)
}

/// 6:16 FUN_14053e540 -> FUN_1403bda50: an empty target record.
fn target_clear(ctx: &mut Ctx<'_>, _: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    super::target::set_record(
        ctx,
        args[0].value,
        Target {
            id: 0,
            kind: -1,
            group: 0,
        },
    )
}

/// The TARGET shapes of an object: (shape id, centre).
fn shapes(ctx: &mut Ctx<'_>, id: ObjId) -> Vec<(u32, Vec3)> {
    if let Some(list) = ctx.env.target_shapes(id) {
        return list;
    }
    let position = if id == PLAYER_OBJECT {
        ctx.env.player_position()
    } else {
        ctx.object(id).map_or([0.; 3], |o| o.sys.position)
    };
    vec![(0, position)]
}

/// Is `id` already in the history (FUN_140540210)?
fn remembered(ctx: &Ctx<'_>, m: &Memory<'_>, s: u32, id: ObjId, shape: u32) -> Result<bool, Fault> {
    let count = word(ctx, m, s + COUNT)?;
    let handle = if id == PLAYER_OBJECT {
        PLAYER_HANDLE
    } else {
        object_handle(id)
    };
    for k in 0..count {
        let at = s + ENTRIES + 8 * k;
        if word(ctx, m, at)? == handle && word(ctx, m, at + 4)? == shape {
            return Ok(true);
        }
    }
    Ok(false)
}

/// FUN_140540080: remember (object, shape); a shape of -1 remembers all of
/// the object's shapes. The ring holds `capacity` entries.
fn remember(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    s: u32,
    id: ObjId,
    shape: i32,
) -> Result<(), Fault> {
    let wanted: Vec<u32> = if shape < 0 {
        shapes(ctx, id).iter().map(|(k, _)| *k).collect()
    } else {
        vec![shape as u32]
    };
    for shape in wanted {
        if remembered(ctx, m, s, id, shape)? {
            continue;
        }
        let capacity = word(ctx, m, s + CAPACITY)?;
        let cursor = word(ctx, m, s + CURSOR)? & 0xff;
        let handle = if id == PLAYER_OBJECT {
            PLAYER_HANDLE
        } else {
            object_handle(id)
        };
        put(ctx, m, s + ENTRIES + 8 * cursor, handle)?;
        put(ctx, m, s + ENTRIES + 8 * cursor + 4, shape)?;
        let count = word(ctx, m, s + COUNT)?;
        if count < capacity {
            put(ctx, m, s + COUNT, count + 1)?;
        }
        let mut next = cursor + 1;
        while capacity != 0 && next >= capacity {
            next -= capacity;
        }
        let w = word(ctx, m, s + CURSOR)? & !0xff | next;
        put(ctx, m, s + CURSOR, w)?;
    }
    Ok(())
}

/// 6:8 FUN_14053e240: add every shape of the object to the history.
fn searcher_add(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let s = args[0].value;
    let id = ctx.sys_of_ai(m, args[1].value)?;
    remember(ctx, m, s, id, -1)
}

fn set_pick(ctx: &mut Ctx<'_>, record: u32, id: ObjId, shape: u32) -> Result<(), Fault> {
    super::target::set_record(
        ctx,
        record,
        Target {
            id: super::target::target_id_of(id),
            kind: -1,
            group: shape as i32,
        },
    )
}

/// 6:5 FUN_14053dd20 -> FUN_14053fb80: the best TARGET shape of the objects
/// of the searcher's kind, inside its range and angle and not remembered
/// when `track` is set; the nearest, or the farthest when `nearest` is 0.
/// `flat` ignores the height difference. With `track` the pick joins the
/// history.
fn searcher_search(ctx: &mut Ctx<'_>, m: &mut Memory<'_>, args: &mut Args) -> Result<(), Fault> {
    let s = args[0].value;
    let record = args[1].value;
    let origin = read_native(ctx, m, args[2].value)?;
    let facing = read_native(ctx, m, args[3].value)?;
    let (nearest, flat, track) = (args[4].value != 0, args[5].value != 0, args[6].value != 0);
    let kind = word(ctx, m, s + KIND)?;
    let near = f32::from_bits(word(ctx, m, s + NEAREST)?);
    let far = f32::from_bits(word(ctx, m, s + FARTHEST)?);
    let angle = f32::from_bits(word(ctx, m, s + ANGLE)?);
    let w = word(ctx, m, s + CURSOR)?;
    put(ctx, m, s + CURSOR, w & !0xff00)?;
    super::target::set_record(
        ctx,
        record,
        Target {
            id: 0,
            kind: -1,
            group: 0,
        },
    )?;
    // The objects of the kind, in list order, with Sora last.
    let mut ids: Vec<ObjId> = Vec::new();
    let me = ctx.me.id;
    let mut pool: Vec<ObjId> = ctx.world.objects.iter().flatten().map(|o| o.id).collect();
    pool.push(me);
    pool.sort_unstable();
    for id in pool {
        let o = ctx.object(id)?;
        let class = o.sys.class_flags;
        let wanted = if kind == 0 {
            class & class_flag::ENEMY != 0
        } else {
            class & class_flag::SORA != 0
        };
        if wanted && o.sys.flags & SEARCH_SKIP_FLAGS == 0 && !o.sys.gone && !o.sys.left {
            ids.push(id);
        }
    }
    if kind != 0 {
        ids.push(PLAYER_OBJECT);
    }
    let mut best: Option<(ObjId, u32, f32)> = None;
    let mut seen: u32 = 0;
    for id in ids {
        for (shape, at) in shapes(ctx, id) {
            seen += 1;
            if track && remembered(ctx, m, s, id, shape)? {
                continue;
            }
            let mut d = [at[0] - origin[0], at[1] - origin[1], at[2] - origin[2]];
            if flat {
                d[1] = 0.;
            }
            let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if distance < near || distance > far {
                continue;
            }
            if angle < std::f32::consts::PI {
                let turn = wrap_angle(facing[0].atan2(facing[2]) - d[0].atan2(d[2]));
                if turn.abs() >= angle {
                    continue;
                }
            }
            let key = if nearest { distance } else { -distance };
            if best.is_none_or(|(_, _, k)| key < k) {
                best = Some((id, shape, key));
            }
        }
    }
    let w = word(ctx, m, s + CURSOR)?;
    put(ctx, m, s + CURSOR, w & !0xff00 | (seen.min(255) << 8))?;
    if let Some((id, shape, _)) = best {
        set_pick(ctx, record, id, shape)?;
        if track {
            remember(ctx, m, s, id, shape as i32)?;
        }
    }
    Ok(())
}

/// 6:11 FUN_14053e460 -> FUN_14053ff10: the shape of one object nearest to a
/// point (its distance less the shape's radius), written into the record.
fn searcher_search_object(
    ctx: &mut Ctx<'_>,
    m: &mut Memory<'_>,
    args: &mut Args,
) -> Result<(), Fault> {
    let record = args[0].value;
    let id = ctx.sys_of_ai(m, args[1].value)?;
    let origin = read_native(ctx, m, args[2].value)?;
    super::target::set_record(
        ctx,
        record,
        Target {
            id: 0,
            kind: -1,
            group: 0,
        },
    )?;
    let mut best: Option<(u32, f32)> = None;
    for (shape, at) in shapes(ctx, id) {
        let d = [at[0] - origin[0], at[1] - origin[1], at[2] - origin[2]];
        let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if best.is_none_or(|(_, k)| distance < k) {
            best = Some((shape, distance));
        }
    }
    if let Some((shape, _)) = best {
        set_pick(ctx, record, id, shape)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
