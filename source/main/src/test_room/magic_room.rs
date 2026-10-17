//! Sora's magic in the test room: the owned spells, the cast command
//! (`player::magic`), the running spells' volumes and what they do to
//! the Shadow and to Sora (heals, recoil, Leaf Bracer).
//!
//! Spell attacks go through the same hit pipeline as the Keyblade (attack
//! ids, hit records, ATKP level rows by sub-table) in `combat_tick`; this
//! module only owns the casts and the volumes they emit.
use super::spell_fx::Call;
use super::*;
use player::magic::{self, Feedback, Spell};

/// The room's world id for `magc` record selection (a record applies in
/// world 0 or its own; Pride Lands' 0x0a variants are not used here).
const WORLD: u8 = 2;
/// Attack-instance keys of spell volumes live above the weapon windows'
/// `(action_started << 7) | hand << 5 | window` ids.
pub const KEY_BASE: u64 = 1 << 62;
/// The key of the party list's first name in `MagicSource::names` (Sora,
/// Donald, Goofy follow it).
pub const MEMBER_NAMES: u16 = 0xf000;
/// Sound id of the "cannot" cue (system bank 5, command-01 §6.3).
const SE_ERROR: u32 = 5;

/// 03system and 00battle data of the six spells.
pub struct MagicSource {
    pub records: Vec<magic::Record>,
    pub rows: Vec<magic::Row>,
    pub commands: HashMap<u16, data::command_table::Command>,
    /// The spell commands' names (system-font strings) by command id.
    pub names: HashMap<u16, Vec<u8>>,
    /// Sora's TARGET shape lift above his origin (Blizzard starts there).
    pub target_lift: f32,
    /// The body centre (bone 0x4001) and heal point (0x4009) above Sora's
    /// origin.
    pub body_lift: f32,
    pub heal_lift: f32,
    /// The effect container (PAX, BAR entry type 18) of each spell file
    /// by the `magc` record's file name, and the resident common textures.
    pub paxes: HashMap<String, Arc<object_effect::Pax>>,
    pub texcommon: Option<Arc<object_effect::Dpd>>,
}

/// `magc` (00battle), `pref/magi` and the `cmd` table.
pub fn load(
    battle: &[u8],
    pref: &[u8],
    system: &[u8],
    target_lift: f32,
) -> Result<MagicSource, Box<dyn std::error::Error>> {
    let magc = bar_entry(battle, b"magc")?;
    let count = magc
        .get(4..8)
        .map(|w| u32::from_le_bytes(w.try_into().unwrap()) as usize)
        .ok_or("magc truncated")?;
    let records = (0..count)
        .map(|k| {
            magc.get(8 + k * magic::Record::SIZE..)
                .and_then(magic::Record::read)
                .ok_or("magc record truncated")
        })
        .collect::<Result<Vec<_>, _>>()?;
    // `pref/magi`: i32 count, then one offset per row; an empty offset or
    // an index past the count reads row 0 (FUN_1403e9560).
    let magi = bar_entry(pref, b"magi")?;
    let n = magi
        .get(0..4)
        .map(|w| u32::from_le_bytes(w.try_into().unwrap()) as usize)
        .ok_or("magi truncated")?;
    let mut rows = Vec::new();
    for i in 0..n.min(64) {
        let off = magi
            .get(4 + i * 4..8 + i * 4)
            .map(|w| u32::from_le_bytes(w.try_into().unwrap()) as usize)
            .ok_or("magi truncated")?;
        let row = if off == 0 {
            None
        } else {
            magi.get(off..).and_then(magic::Row::read)
        };
        rows.push(row);
    }
    let first = rows.first().copied().flatten().unwrap_or_default();
    let rows = rows.into_iter().map(|r| r.unwrap_or(first)).collect();
    let commands = data::command_table::read(bar_entry(system, b"cmd\0")?)?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();
    Ok(MagicSource {
        records,
        rows,
        commands,
        names: HashMap::new(),
        target_lift,
        body_lift: 85.,
        heal_lift: 105.,
        paxes: HashMap::new(),
        texcommon: None,
    })
}

/// Reads the effect container of every spell file the records name
/// (`magic/FIRE_1.mag` ...) and `effect/texcommon.dpd`. A file that cannot
/// be read leaves its spell without effects.
pub fn load_effects(source: &ProbeSource, src: &mut MagicSource) {
    let Ok(mut library) = data::path_lookup::Library::open(source) else {
        eprintln!("Spell effects unavailable: the installation's file library did not open");
        return;
    };
    src.texcommon = library
        .read("effect/texcommon.dpd")
        .ok()
        .and_then(|b| object_effect::read_dpd(&b).ok())
        .map(Arc::new);
    let files: Vec<String> = src
        .records
        .iter()
        .filter(|r| r.world == 0)
        .map(|r| r.file.clone())
        .collect();
    for file in files {
        if src.paxes.contains_key(&file) {
            continue;
        }
        let pax = library
            .read(&file)
            .ok()
            .and_then(|bytes| {
                let table = bar::read_bar(&bytes).ok()?;
                let e = table
                    .entries
                    .iter()
                    .find(|e| e.entry_type == 18 && e.size > 0)?;
                let pax = bytes.get(e.offset as usize..e.offset as usize + e.size as usize)?;
                object_effect::read_pax(pax).ok()
            })
            .map(Arc::new);
        match pax {
            Some(p) => {
                src.paxes.insert(file, p);
            }
            None => eprintln!("Spell effects: {file} could not be read"),
        }
    }
}

/// The cast motion ids the records use (world 0 only), without
/// repeats.
pub fn motion_ids(records: &[magic::Record]) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for r in records.iter().filter(|r| r.world == 0) {
        for m in [r.ground.0, r.finisher.0, r.air.0] {
            if m >= 0 && !out.contains(&(m as u16)) {
                out.push(m as u16);
            }
        }
    }
    out
}

/// Live magic state for the session.
pub struct MagicRt {
    pub src: MagicSource,
    /// Owned element items by magic type (the level).
    pub levels: [u8; 6],
    /// Camp Customize's "Don't MP Charge" bits by magic type.
    pub dont_charge: u8,
    pub spells: Vec<Spell>,
    pub serial: u64,
    /// What the last tick's collisions told the spells.
    pub feedback: Feedback,
    /// Clash counts of guard volumes by key, kept across ticks.
    pub reflected: HashMap<u64, u32>,
    /// This tick's volumes, for `combat_tick`.
    pub volumes: Vec<magic::Volume>,
    /// The volume keys that hit last tick (Wisdom's shots read them).
    pub last_hits: Vec<u64>,
    /// The open Magic submenu's rows, by magic type.
    pub menu_types: Vec<u8>,
    /// The cast in progress (script trigger 20's thread) and Reflect's
    /// shared block.
    pub cast: Option<CastFx>,
    pub barrier: Option<u64>,
    pub barrier_count: u32,
    /// The effect container of each spell's serial (for `Fx::Start`).
    pax_of: HashMap<u64, Arc<object_effect::Pax>>,
    /// Hit sparks started (their handles).
    sparks: u64,
}

/// A cast between its start and its release or interruption.
pub struct CastFx {
    pub serial: u64,
    pub released: bool,
    pub magic_type: u8,
    pub level: u8,
    pub pax: Option<Arc<object_effect::Pax>>,
    /// Cure's chosen party slot.
    pub target: usize,
}

impl MagicRt {
    pub fn new(src: MagicSource, levels: [u8; 6]) -> Self {
        MagicRt {
            src,
            levels,
            dont_charge: 0,
            spells: Vec::new(),
            serial: 0,
            feedback: Feedback::default(),
            reflected: HashMap::new(),
            volumes: Vec::new(),
            last_hits: Vec::new(),
            menu_types: Vec::new(),
            cast: None,
            barrier: None,
            barrier_count: 0,
            pax_of: HashMap::new(),
            sparks: 0,
        }
    }

    /// FUN_1403c59d0: the record in force for a magic type.
    pub fn current(&self, magic_type: u8) -> Option<&magic::Record> {
        magic::Record::current(
            &self.src.records,
            magic_type,
            self.levels[magic_type as usize],
            WORLD,
        )
    }

    /// FUN_1403f2910: the Magic submenu's entries in magic-type order, each
    /// with its menu colour.
    pub fn menu_list(&self, mp: &magic::Mp) -> Vec<(u8, usize)> {
        (0..6u8)
            .filter_map(|t| self.colour(t, mp).map(|c| (t, c)))
            .collect()
    }

    /// The menu colour of the spell of `magic_type` (FUN_140401d60).
    pub fn colour(&self, magic_type: u8, mp: &magic::Mp) -> Option<usize> {
        let rec = self.current(magic_type)?;
        let cmd = self.src.commands.get(&rec.command)?;
        Some(magic::colour(
            cmd.cost,
            mp,
            self.dont_charge >> magic_type & 1 != 0,
            false,
        ))
    }
}

/// The hit spark of a spell's volume (`attack_set_pax`: the element of the
/// spell's own PAX chosen by the ATKP's hit-effect category, FUN_1402c6730),
/// started at the contact point facing the hit direction. None when the
/// volume is not a spell's or its PAX has no such element.
pub fn spark(
    m: &mut MagicRt,
    volume: u64,
    effect: u8,
    contact: [f32; 3],
    direction: [f32; 3],
) -> Option<Vec<Call>> {
    // Volume keys are `serial << 8 | slot`; the Wisdom shots' and others
    // are not.
    let pax = m.pax_of.get(&(volume >> 8))?.clone();
    // mask = (1 << (32 + hitEffect − 1)) | 1 (a normal target).
    let mask = (1u64 << (32 + effect as u64 - 1)) | 1;
    let element = pax.by_category(mask)?;
    m.sparks += 1;
    let key = 1 << 63 | m.sparks;
    Some(vec![
        Call {
            pax: Some(pax.clone()),
            fx: magic::Fx::Start {
                key,
                id: element.id,
                bound: None,
                at: Some(contact),
            },
        },
        Call {
            pax: None,
            fx: magic::Fx::Dir(key, direction),
        },
    ])
}

/// The cast command for a magic type (menu or shortcut). Returns whether a
/// cast started.
pub fn request(game: &mut Game, magic_type: u8, tick: u64) -> bool {
    let Game {
        magic,
        player,
        clips,
        sound_queue,
        drive,
        fx_calls,
        magic_tour,
        voices,
        cure_target,
        ..
    } = game;
    let Some(m) = magic.as_mut() else {
        return false;
    };
    // The unattended tour casts with a full bar each time.
    if *magic_tour {
        player.mp = magic::Mp::new(player.mp.max);
        player.mp_charging = false;
    }
    let Some(rec) = m.current(magic_type).cloned() else {
        return false;
    };
    let Some(cmd) = m.src.commands.get(&rec.command).copied() else {
        return false;
    };
    let colour = magic::colour(
        cmd.cost,
        &player.mp,
        m.dont_charge >> magic_type & 1 != 0,
        false,
    );
    // The command's form mask (cmd +0x2c) is checked against the form 0.
    if colour >= 4 {
        sound_queue.push((SE_ERROR, "magic_denied"));
        return false;
    }
    // Cure on a shortcut targets Sora himself; from the menu, the member
    // picked in the target list.
    let cure_target = std::mem::take(cure_target);
    let target_is_self = magic_type == magic::CURE && cure_target == 0;
    // obj+0xde0: the Drive form picks the variant row (Wisdom +10, Master
    // +20, Final +30).
    let form = drive.as_ref().map_or(0, |d| d.form);
    let Some((action, start)) = player.cast(&rec, cmd.cost, form, target_is_self, clips) else {
        let (combo_window, finisher_window, ready, combo) = player.command_gate(clips);
        let record = serde_json::json!({"event":"cast_refused","tick":tick,"magic":magic_type,
            "action":format!("{:?}", player.action),"frame":player.motion_frame(clips),
            "combo_window":combo_window,"finisher_window":finisher_window,"ready":ready,
            "combo_step":combo.step,"combo_finish":combo.finish});
        game.event(record);
        return false;
    };
    // The cast's battle voice (FUN_1403c5170: vt+0xa0 = FUN_1403d69b0, the
    // frame trigger 14 voice; none in Antiform).
    let anti = drive
        .as_ref()
        .is_some_and(|d| d.active && d.form == player::forms::ANTI);
    if start.voice >= 0
        && !anti
        && let Some(id) = voices.as_mut().and_then(|v| v.pick(1, start.voice as i16))
    {
        sound_queue.push((id, "spell_voice"));
    }
    // Script trigger 20: the cast effect starts with the motion; an earlier
    // cast that never released leaves its effect.
    if let Some(old) = m.cast.take()
        && !old.released
    {
        fx_calls.push(Call {
            pax: None,
            fx: magic::Fx::Kill(magic::cast_key(old.serial)),
        });
    }
    m.serial += 1;
    let pax = m.src.paxes.get(&rec.file).cloned();
    if let Some(p) = &pax {
        m.pax_of.insert(m.serial, p.clone());
    }
    for fx in magic::cast_effects(magic_type, start.row, magic::cast_key(m.serial)) {
        fx_calls.push(Call {
            pax: pax.clone(),
            fx,
        });
    }
    m.cast = Some(CastFx {
        serial: m.serial,
        released: false,
        magic_type,
        level: rec.level,
        pax,
        target: cure_target,
    });
    // Leaf Bracer (ability 114): Cure on oneself gives 60 ticks of
    // invulnerability and a super guard from the cast's start.
    let leaf = magic_type == magic::CURE && player.abilities.has(114);
    if leaf {
        m.serial += 1;
        let ctx_party = [player.position];
        let ground = |_: [f32; 3], _: f32| None;
        let bg = |_: [f32; 3], _: f32| false;
        let ctx = magic::Context {
            dt: 1.,
            caster: player.position,
            facing: [player.yaw.sin(), player.yaw.cos()],
            caster_centre: [
                player.position[0],
                player.position[1] + m.src.target_lift,
                player.position[2],
            ],
            body_centre: [
                player.position[0],
                player.position[1] + m.src.body_lift,
                player.position[2],
            ],
            airborne: false,
            casting: true,
            target: None,
            party: &ctx_party,
            ground: &ground,
            bg_hit: &bg,
            barrier: None,
            barrier_count: 0,
        };
        let serial = m.serial;
        m.spells.push(Spell::leaf_bracer(serial, &ctx));
    }
    let mp = player.mp;
    let record = serde_json::json!({"event":"cast","tick":tick,"magic":magic_type,
        "level":rec.level,"command":rec.command,"action":format!("{action:?}"),
        "motion":start.motion,"row":start.row,"finisher":start.finisher,
        "airborne":start.airborne,"mp":mp.cur,"charging":mp.charging()});
    game.action_started = tick;
    game.event(record);
    true
}

/// One tick of the casts: release, then every spell's volumes, effects,
/// heals, recoil and invulnerability.
pub fn tick(game: &mut Game, tick: u64) {
    let mut events: Vec<serde_json::Value> = Vec::new();
    {
        let Game {
            magic,
            player,
            selected_target,
            enemy,
            world,
            fx_calls,
            fx_live,
            magic_tour,
            clips,
            party: members,
            ..
        } = game;
        let Some(m) = magic.as_mut() else { return };
        // The tour records the windows the installed motions give (range
        // triggers 11, 50 and 0-2, frame trigger 17), the data the combo gate
        // reads.
        if *magic_tour && tick == 1 {
            // The tour owns Magic Lock-On so Thunder and the rest find the enemy.
            player.abilities.set(player::abilities::bit::MAGIC_LOCK_ON);
            let window = |c: &player::Clip| {
                serde_json::json!({"end":c.end,"combo":c.combo_windows,
                    "finisher":c.finisher_windows,"actionable":c.actionable})
            };
            events.push(serde_json::json!({"event":"motion_windows","tick":tick,
                "attacks":clips.attacks.iter().map(|a| window(&a.clip)).collect::<Vec<_>>(),
                "spells":clips.spells.iter().map(|s| {
                    let mut w = window(&s.clip);
                    w["motion"] = s.motion.into();
                    w["release"] = serde_json::json!(s.clip.release);
                    w
                }).collect::<Vec<_>>()}));
        }
        // The party members start hurt so Cure has something to heal.
        if *magic_tour && tick == 1 {
            for member in members.iter_mut() {
                member.hp = member.max_hp / 2;
            }
        }
        // The tour's enemy takes every hit and stays.
        if *magic_tour && let Some(rt) = enemy.as_mut() {
            rt.enemy.hp = rt.enemy.hp.max(500);
        }
        let pos = player.position;
        let facing = [player.yaw.sin(), player.yaw.cos()];
        let action = player.action;
        let centre = [pos[0], pos[1] + m.src.target_lift, pos[2]];
        // Party members' origins (a member a Drive form absorbed is out of
        // range).
        let party_pos: Vec<[f32; 3]> = std::iter::once(pos)
            .chain(members.iter().map(|m| {
                if m.stored {
                    [f32::INFINITY; 3]
                } else {
                    m.follower.position
                }
            }))
            .collect();
        // The locked target's TARGET centre: the enemy while it can be hit.
        let target = *selected_target;
        let ground = |from: [f32; 3], max: f32| -> Option<[f32; 3]> {
            world
                .floor(from[0], from[2], from[1])
                .filter(|h| from[1] - h <= max)
                .map(|h| [from[0], h, from[2]])
        };
        // FUN_1403d0bb0 stand-in: the centre left the floor (arena edge) or
        // entered a wall.
        let bg = |p: [f32; 3], _r: f32| -> bool {
            world.floor(p[0], p[2], p[1]).is_none() || {
                let w = world.resolve_walls([p[0], p[2]], 1., p[1] - 1., p[1] + 1.);
                (w[0] - p[0]).abs() > 1e-3 || (w[1] - p[2]).abs() > 1e-3
            }
        };
        let ctx =
            |airborne: bool, casting: bool, barrier: Option<u64>, count: u32| magic::Context {
                dt: 1.,
                caster: pos,
                facing,
                caster_centre: centre,
                body_centre: [pos[0], pos[1] + m.src.body_lift, pos[2]],
                airborne,
                casting,
                target,
                party: &party_pos,
                ground: &ground,
                bg_hit: &bg,
                barrier,
                barrier_count: count,
            };
        // Feedback from last tick's collisions and effect update reaches the
        // spells now.
        m.feedback.reflected = m.reflected.iter().map(|(&k, &c)| (k, c)).collect();
        m.feedback.fx = fx_live.iter().map(|(&k, &v)| (k, v)).collect();
        if let Some(r) = player.take_released() {
            let (cast_serial, pax, cure_slot) = match m.cast.as_mut() {
                Some(c) => {
                    c.released = true;
                    (c.serial, c.pax.clone(), c.target)
                }
                None => (0, None, 0),
            };
            m.serial += 1;
            if let Some(p) = &pax {
                m.pax_of.insert(m.serial, p.clone());
            }
            let row = m
                .src
                .rows
                .get(r.row)
                .or(m.src.rows.first())
                .copied()
                .unwrap_or_default();
            let c = ctx(r.airborne, true, m.barrier, m.barrier_count);
            let spell = Spell::new(m.serial, r.magic_type, r.level, r.row, row, r.form, &c)
                .with_cast(magic::cast_key(cast_serial), r.volley)
                .with_target(cure_slot);
            m.spells.push(spell);
            // The release's sounds are the effect elements' own (trigger 21
            // starts them).
            events.push(serde_json::json!({"event":"spell_release","tick":tick,
                "magic":r.magic_type,"level":r.level,"row":r.row,"finisher":r.finisher,
                "volley":r.volley,"form":r.form,
                "mp":player.mp.cur,"charging":player.mp.charging(),
                "target":target.map(|t| t.to_vec())}));
        }
        // A cast whose motion ended without releasing ends its effect.
        if let Some(c) = m.cast.as_ref()
            && !player.casting()
        {
            if !c.released {
                fx_calls.push(Call {
                    pax: None,
                    fx: magic::Fx::Kill(magic::cast_key(c.serial)),
                });
                events.push(serde_json::json!({"event":"cast_interrupted","tick":tick,
                    "magic":c.magic_type,"level":c.level}));
            }
            m.cast = None;
        }
        let mut volumes = Vec::new();
        let mut strikes = Vec::new();
        let mut recoil = None;
        let mut invulnerable: Option<f32> = None;
        let fb = std::mem::take(&mut m.feedback);
        m.last_hits = fb.hit.clone();
        let airborne = matches!(action, Action::AirCast(_) | Action::AirAttack(_));
        let casting = matches!(action, Action::Cast(_) | Action::AirCast(_));
        let (mut barrier, mut barrier_count) = (m.barrier, m.barrier_count);
        for s in m.spells.iter_mut() {
            let out = s.tick(&ctx(airborne, casting, barrier, barrier_count), &fb);
            volumes.extend(out.volumes);
            strikes.extend(out.strikes);
            for fx in out.fx {
                let pax = match &fx {
                    magic::Fx::Start { key, .. } => m.pax_of.get(&(key >> 8)).cloned(),
                    _ => None,
                };
                fx_calls.push(Call { pax, fx });
            }
            if let Some(b) = out.barrier {
                barrier = b;
            }
            if let Some(n) = out.barrier_count {
                barrier_count = n;
            }
            if out.recoil.is_some() {
                recoil = out.recoil;
            }
            if let Some(t) = out.invulnerable {
                invulnerable = Some(invulnerable.map_or(t, |v| v.max(t)));
            }
        }
        (m.barrier, m.barrier_count) = (barrier, barrier_count);
        m.spells.retain(|s| !s.finished());
        // Serials of long finished spells no longer start effects.
        let newest = m.serial;
        m.pax_of.retain(|&k, _| k + 64 > newest);
        m.volumes = volumes;
        if let Some(strength) = recoil {
            // obj_hitback(S, roty(obj_dir, pi), f7): back along the facing.
            player.recoil([-facing[0], -facing[1]], strength);
        }
        if let Some(rt) = enemy.as_mut() {
            if let Some(t) = invulnerable {
                rt.sora_invulnerable = rt.sora_invulnerable.max(t);
            }
            // attack_strike on the party: Cure's heal is an ATKP Type 6 row
            // (Power x Magic + 99) / 100, added to HP and clamped.
            for st in strikes {
                if !ensure_attack_rows(rt, st.atkp, st.sub) {
                    continue;
                }
                let attack = params_of(&rt.steps[&(st.atkp, st.sub)][0]);
                let full = combat::TargetState::full(rt.sora_target.max_hp);
                let heal = combat::damage_with(&rt.sora, &attack, &rt.sora_target, &full);
                if st.party == 0 {
                    rt.sora_hp = (rt.sora_hp + heal).min(rt.sora_target.max_hp);
                    events.push(
                        serde_json::json!({"event":"heal","tick":tick,"atkp":st.atkp,
                        "sub":st.sub,"amount":heal,"sora_hp":rt.sora_hp}),
                    );
                } else if let Some(member) = members.get_mut(st.party - 1) {
                    // A dead member is revived by the heal (FUN_1403c3c30).
                    let revived = super::party_room::heal_member(member, heal);
                    events.push(
                        serde_json::json!({"event":"heal","tick":tick,"atkp":st.atkp,
                        "sub":st.sub,"amount":heal,"member":member.kind.name,"hp":member.hp,
                        "revived":revived}),
                    );
                }
            }
        }
    }
    for e in events {
        game.event(e);
    }
}
