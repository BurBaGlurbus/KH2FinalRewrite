//! Donald and Goofy in the test room: their models, staff and shield, and
//! motions found by name in the installed archives, the behaviour of
//! `player::party_members`, the hits their weapon windows land on the enemy,
//! and the mesh presentation.
//!
//! Built: standing, walking, running, the two ground attacks and Donald's
//! four spells (`player::party_spell`: Fire, Blizzard, Thunder and Cure
//! with their effect elements, volumes, MP and heal), and the member being
//! hit (`player::party_hit`): the enemy's claws land on its HIT spheres,
//! Goofy's shield guards, the reaction motions play, a hit during a cast
//! interrupts it, and HP 0 lays the member down until the timer or a Cure
//! revives it. Air attacks, items, Limits and Goofy's Bash follow the specs
//! under `party-01`.
use super::*;
use data::path_lookup::Library;
use player::party_hit;
use player::party_members::{self, Follower, Target};
use player::party_spell;
use std::sync::Arc;

/// The motion ids loaded for a member: idle, walk, run and the two ground
/// attacks.
const MOTIONS: [u16; 5] = [0, 1, 2, 151, 152];
/// The reaction motions, loaded when the set holds them: the flinches and
/// air flinches, the blows and their land motions, the guard bounce, the
/// DEAD motion and the get-up.
const REACTIONS: [u16; 15] = [10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 41, 54, 55, 43, 5];

/// One motion, sampled per tick (60 Hz).
pub struct MotionData {
    /// Model vertices with the root removed.
    pub frames: Vec<Vec<Vec3>>,
    /// The weapon's vertices in model space (empty when it has no motion).
    pub weapon: Vec<Vec<Vec3>>,
    /// The weapon bone 2 matrix in model space (attack spheres).
    pub blade: Vec<Mat4>,
    /// Root translation per tick (model space).
    pub root: Vec<Vec3>,
    /// Weapon range 10: (start, end, ATKP id, collision group).
    pub hits: Vec<(u32, u32, u16, u8)>,
    pub loop_start: u32,
    /// Frame trigger 9 (the spell release, `ctl+0x155` bit 0).
    pub release: Option<u32>,
    /// Range 16 (`range_bit(obj, 1)`): when Goofy's Tornado and Turbo move.
    pub range16: Option<(u32, u32)>,
    /// The staff's top as a bone matrix (weapon slot 0 top,
    /// FUN_1403b5450): bone 2's rotation at the weapon vertex farthest from
    /// its origin [I: the Keyblade rule of `spell_fx`].
    pub staff: Vec<Mat4>,
    /// The member's HIT sphere centres per tick (model space, root
    /// removed), in the order of `PartyKind::hit_shapes`.
    pub hit: Vec<Vec<Vec3>>,
    /// Range 2's start: the tick the member can act again after a
    /// reaction (`obj+0x18c` bits 2-4).
    pub actionable: Option<u32>,
    /// Frame trigger 14: (tick, voice n) (FUN_14040dd70 to FUN_1403d69b0).
    pub voices: Vec<(u32, i16)>,
}

impl MotionData {
    pub fn end(&self) -> u32 {
        self.frames.len().saturating_sub(1) as u32
    }

    pub fn frame_of(&self, tick: u32) -> usize {
        let end = self.end();
        let t = if tick <= end || self.loop_start >= end {
            tick.min(end)
        } else {
            self.loop_start + (tick - self.loop_start) % (end - self.loop_start)
        };
        t as usize
    }
}

/// A member's installed model, weapon and motions.
pub struct PartyKind {
    pub name: &'static str,
    pub model: model_inspection::InspectionModel,
    pub weapon: Option<model_inspection::InspectionModel>,
    pub motions: HashMap<u16, MotionData>,
    /// The weapon's attack spheres by group: offset and radius (bone 2).
    pub spheres: HashMap<u8, Vec<(Vec3, f32)>>,
    /// Strength and Magic from the character's LVUP row at level 1.
    pub strength: i32,
    pub magic: i32,
    /// Starting HP and MP (00battle `plrp`, the new-game row).
    pub hp: i32,
    pub mp: i32,
    pub records: &'static [party_members::Record],
    /// The member's own effects (`obj/<name>.a.us`) and the heights of its
    /// type-1 and type-9 collision centres (bone codes 0x4001, 0x4009).
    pub pax: Option<Arc<data::object_effect::Pax>>,
    pub body_lift: f32,
    pub heal_lift: f32,
    /// The HIT spheres (type 2, group 0) of the model: radius and whether
    /// a hit on it flinches high (argument 0, 2-5).
    pub hit_shapes: Vec<(f32, bool)>,
    /// The member as a target: maximum HP and Defence (`plrp` row plus the
    /// level-1 `lvup` row), the player-type clamps and no weaknesses.
    pub target: combat::TargetStats,
    /// The revive timer of a KO (PRTY word 6: Donald 3600, Goofy 2700).
    pub ko_time: f32,
    /// SSTM (the knock-back and blow constants) and the KO rates.
    pub sstm: actor_physics::Sstm,
    pub rates: party_hit::Rates,
    /// The member character id (2 Donald, 3 Goofy) and its battle voices.
    pub character: u8,
    pub voices: Option<drive_room::Voices>,
}

/// A member in the room.
pub struct MemberRt {
    pub kind: Arc<PartyKind>,
    pub follower: Follower,
    /// The game tick the current attack started, for hit instance keys.
    pub started: u64,
    /// Absorbed by a Drive form (FUN_140400b20): out of the field until
    /// the revert brings it back (drive_room).
    pub stored: bool,
    /// Current and maximum HP, and MP with its charge (FUN_1403d3cf0).
    pub hp: i32,
    pub max_hp: i32,
    pub mp: player::magic::Mp,
    /// The spell act running (the act manager, obj+0x390) and the number
    /// of casts so far (keys of its effects and volumes).
    pub cast: Option<party_spell::Cast>,
    pub casts: u64,
    /// A spell attack began this tick and has no act yet.
    pub armed: bool,
    /// A cast the tour orders: record and heal slot (taken by the next
    /// tick).
    pub forced: Option<(party_members::Record, u8)>,
    /// The attacks that hit this member (obj+0xc98) and the KO marker
    /// effect (00effect element 14) while it lies down.
    pub record: combat::HitRecord,
    pub ko_fx: bool,
    /// The motion the running act started (`sysobj_motion_id == m` in its
    /// waits) and whether its shot was clashed by a guard last tick.
    pub cast_motion: u16,
    pub reflected: bool,
    /// The member voices and the motion and tick its frame triggers were
    /// last scanned at.
    pub voices: Option<drive_room::Voices>,
    pub voice_at: (u16, u32),
}

fn matrix(m: &[u32; 16], root: Vec3) -> Mat4 {
    let mut m = Mat4::from_cols_array(&m.map(f32::from_bits));
    m.w_axis -= root.extend(0.);
    m
}

/// Samples one motion (and the weapon's) at every tick.
fn build_motion(
    model: &model_inspection::InspectionModel,
    weapon: Option<&model_inspection::InspectionModel>,
    hand_bone: usize,
    hit_shapes: &[(usize, Vec3, f32, bool)],
    selected: &motion::SelectedMotion,
    own: Option<&motion::SelectedMotion>,
) -> Result<MotionData, Box<dyn std::error::Error>> {
    let (frame_start, frame_end, fps, frame_return) = selected.motion.frame_window()?;
    // Goofy's A201 starts at -1 (before the first frame): sampled from 0.
    let frame_start = frame_start.max(0.);
    if !(1. ..=60.).contains(&fps) || !(0. ..frame_end).contains(&frame_start) {
        return Err(format!(
            "party motion frame window outside profile ({frame_start}..{frame_end} at {fps} fps)"
        )
        .into());
    }
    let scale = 60. / fps;
    let start = (frame_start * scale).round() as u32;
    let end = (frame_end * scale).round() as u32 - start;
    let loop_start = ((frame_return * scale).round() as u32).saturating_sub(start);
    let mut data = MotionData {
        frames: Vec::new(),
        weapon: Vec::new(),
        blade: Vec::new(),
        root: Vec::new(),
        hits: Vec::new(),
        loop_start,
        release: selected
            .triggers
            .iter()
            .flat_map(|t| &t.records.frames)
            .filter(|f| f.trigger == 9)
            .map(|f| f.frame.max(0) as u32)
            .min(),
        range16: selected
            .triggers
            .iter()
            .flat_map(|t| &t.records.ranges)
            .find(|r| r.trigger == 16)
            .map(|r| {
                (
                    r.start_frame.max(0) as u32,
                    if r.end_frame < 0 {
                        end
                    } else {
                        r.end_frame as u32
                    },
                )
            }),
        staff: Vec::new(),
        hit: Vec::new(),
        voices: selected
            .triggers
            .iter()
            .flat_map(|t| &t.records.frames)
            .filter(|f| f.trigger == 14)
            .filter_map(|f| Some((f.frame.max(0) as u32, *f.parameters.first()?)))
            .collect(),
        actionable: selected
            .triggers
            .iter()
            .flat_map(|t| &t.records.ranges)
            .filter(|r| r.trigger == 2)
            .map(|r| r.start_frame.max(0) as u32)
            .min(),
    };
    if let Some(own) = own {
        data.hits = own
            .triggers
            .iter()
            .flat_map(|t| &t.records.ranges)
            .filter(|r| r.trigger == 10 && r.parameters.len() >= 2)
            .map(|r| {
                (
                    r.start_frame.max(0) as u32,
                    if r.end_frame < 0 {
                        end
                    } else {
                        r.end_frame as u32
                    },
                    r.parameters[0] as u16,
                    r.parameters[1] as u8,
                )
            })
            .collect();
    }
    for offset in 0..=end {
        let key = frame_start + offset as f32 / scale;
        let (pose, evaluated) = model.skinning.sample_evaluated(&selected.motion, key)?;
        let r = Vec3::from_array(evaluated.root_translation);
        data.root.push(r);
        data.frames.push(
            pose.position_bits
                .iter()
                .map(|p| Vec3::from_array(p.map(f32::from_bits)) - r)
                .collect(),
        );
        data.hit.push(
            hit_shapes
                .iter()
                .map(|&(bone, offset, _, _)| {
                    pose.matrix_bits
                        .get(bone)
                        .map_or(Vec3::ZERO, |m| matrix(m, r).transform_point3(offset))
                })
                .collect(),
        );
        let hand = pose
            .matrix_bits
            .get(hand_bone)
            .map_or(Mat4::IDENTITY, |m| matrix(m, r));
        if let (Some(w), Some(own)) = (weapon, own) {
            let (wp, _) = w.skinning.sample_evaluated(&own.motion, key)?;
            data.weapon.push(
                wp.position_bits
                    .iter()
                    .map(|p| hand.transform_point3(Vec3::from_array(p.map(f32::from_bits))))
                    .collect(),
            );
            let blade = wp.matrix_bits.get(2).map_or(hand, |m| {
                hand * Mat4::from_cols_array(&m.map(f32::from_bits))
            });
            data.blade.push(blade);
            let weapon_now = data.weapon.last().expect("pushed above");
            let top = weapon_now
                .iter()
                .copied()
                .max_by(|a, b| {
                    a.distance_squared(blade.w_axis.truncate())
                        .total_cmp(&b.distance_squared(blade.w_axis.truncate()))
                })
                .unwrap_or(blade.w_axis.truncate());
            data.staff.push(Mat4::from_cols(
                blade.x_axis,
                blade.y_axis,
                blade.z_axis,
                top.extend(1.),
            ));
        }
    }
    Ok(data)
}

/// Loads `obj/<name>.mdlx`, `<name>.mset` and the weapon `W_<rest>` by name.
#[allow(clippy::too_many_arguments)]
pub fn load(
    source: &ProbeSource,
    library: &mut Library,
    system: &[u8],
    lvup: &[u8],
    plrp: &[u8],
    objects: &[objentry::ObjectEntry],
    name: &'static str,
    character: usize,
    records: &'static [party_members::Record],
) -> Result<PartyKind, Box<dyn std::error::Error>> {
    let found = library
        .find(&format!("obj/{name}.mdlx"))
        .ok_or_else(|| format!("obj/{name}.mdlx is not installed"))?;
    let mut models =
        model_inspection::read_model_at(source, found.index, found.ordinal, found.name_hash)?;
    if models.is_empty() {
        return Err("party model has no mesh".into());
    }
    let model = models.remove(0);
    let selection = |n: &str| -> Result<motion::MotionSetSelection, Box<dyn std::error::Error>> {
        let set = library
            .find(&format!("obj/{n}.mset"))
            .ok_or_else(|| format!("obj/{n}.mset is not installed"))?;
        Ok(motion::MotionSetSelection {
            index: set.index.to_string(),
            ordinal: set.ordinal,
            name_hash: set.name_hash,
            battle: true,
            has_weapon: true,
        })
    };
    // The base motions plus the cast motions of the member's spells.
    let mut wanted: Vec<u16> = MOTIONS.to_vec();
    for r in records.iter().filter(|r| r.spell.is_some()) {
        if !wanted.contains(&r.motion) {
            wanted.push(r.motion);
        }
    }
    wanted.extend(REACTIONS);
    let actor_motions = motion::read_motion_ids_optional(source, &selection(name)?, &wanted)?;
    // The staff or shield: W_EX020 for P_EX020.
    let weapon_name = format!("W_{}", &name[2..]);
    let weapon_selection = selection(&weapon_name).ok();
    let mut weapon = None;
    let mut spheres: HashMap<u8, Vec<(Vec3, f32)>> = HashMap::new();
    let mut weapon_motions = Vec::new();
    if let Some(w) = library.find(&format!("obj/{weapon_name}.mdlx")) {
        let archive = library.read_found(&w)?;
        let mut models = model_inspection::read_model_at(source, w.index, w.ordinal, w.name_hash)?;
        if !models.is_empty() {
            weapon = Some(models.remove(0));
        }
        if let Ok(entry) = bar_entry_typed(&archive, 23)
            && let Ok(shapes) = object_collision::read(entry)
        {
            for e in shapes
                .iter()
                .filter(|e| e.kind == object_collision::ATTACK && e.bone == 2)
            {
                spheres.entry(e.group).or_default().push((
                    Vec3::new(
                        e.position[0] as f32,
                        e.position[1] as f32,
                        e.position[2] as f32,
                    ),
                    e.radius as f32,
                ));
            }
        }
        if let Some(sel) = &weapon_selection {
            weapon_motions = motion::read_motion_ids_optional(source, sel, &wanted)?;
        }
    }
    // The model's HIT spheres (type 2, group 0), where enemy attacks land.
    let mut hit_shapes: Vec<(usize, Vec3, f32, bool)> = Vec::new();
    if let Some(found) = library.find(&format!("obj/{name}.mdlx"))
        && let Ok(archive) = library.read_found(&found)
        && let Ok(entry) = bar_entry_typed(&archive, 23)
        && let Ok(shapes) = object_collision::read(entry)
    {
        hit_shapes = shapes
            .iter()
            .filter(|c| c.kind == object_collision::HIT && c.group == 0 && c.bone >= 0)
            .map(|c| {
                (
                    c.bone as usize,
                    Vec3::new(
                        c.position[0] as f32,
                        c.position[1] as f32,
                        c.position[2] as f32,
                    ),
                    c.radius as f32,
                    matches!(c.argument, 0 | 2 | 3 | 4 | 5),
                )
            })
            .collect();
    }
    // The hand joint: the character's SKLT primary bone.
    let sk = sklt::read(bar_entry(system, b"sklt")?)?;
    let hand_bone = sk
        .iter()
        .find(|e| e.character_id as usize == character)
        .and_then(|e| usize::try_from(e.primary_bone).ok())
        .ok_or("SKLT entry for the party member absent")?;
    let mut motions = HashMap::new();
    for (i, selected) in actor_motions.iter().enumerate() {
        let Some(selected) = selected else {
            if MOTIONS.contains(&wanted[i]) {
                return Err(format!("{name}: motion {} is not in its set", wanted[i]).into());
            }
            continue;
        };
        // A reaction motion the build cannot sample is left out: the
        // member then plays the partner or none.
        let built = build_motion(
            &model,
            weapon.as_ref(),
            hand_bone,
            &hit_shapes,
            selected,
            weapon_motions.get(i).and_then(|m| m.as_ref()),
        );
        match built {
            Ok(data) => {
                motions.insert(wanted[i], data);
            }
            Err(e) if REACTIONS.contains(&wanted[i]) => {
                eprintln!("{name}: motion {} left out: {e}", wanted[i]);
            }
            Err(e) => return Err(e),
        }
    }
    // The member's own effects and collision heights.
    let pax = library
        .read(&format!("obj/{name}.a.us"))
        .ok()
        .and_then(|bytes| {
            let table = data::bar::read_bar(&bytes).ok()?;
            let e = table
                .entries
                .iter()
                .find(|e| e.entry_type == 18 && e.size > 0)?;
            data::object_effect::read_pax(
                bytes.get(e.offset as usize..e.offset as usize + e.size as usize)?,
            )
            .ok()
        })
        .map(Arc::new);
    let (mut body_lift, mut heal_lift) = (85f32, 105f32);
    if let Some(found) = library.find(&format!("obj/{name}.mdlx"))
        && let Ok(archive) = library.read_found(&found)
        && let Ok(entry) = bar_entry_typed(&archive, 23)
        && let Ok(shapes) = object_collision::read(entry)
    {
        if let Some(c) = shapes.iter().find(|c| c.kind == 1 && c.group == 0) {
            body_lift = c.position_height as f32;
        }
        if let Some(c) = shapes.iter().find(|c| c.kind == 9 && c.group == 0) {
            heal_lift = c.position_height as f32;
        }
    }
    let row = data::battle_tables::player_level(lvup, character, 1)?;
    let (hp, mp) = visual::hud::starting_stats_of(plrp, character as u8)?;
    // The new-game plrp row's Defence (+8) joins the level row's.
    let plrp_def = (0..plrp
        .get(4..8)
        .map_or(0, |c| u32::from_le_bytes(c.try_into().unwrap())) as usize)
        .map(|i| 8 + i * 128)
        .find(|&at| {
            plrp.get(at..at + 3)
                .is_some_and(|e| u16::from_le_bytes([e[0], e[1]]) == 0 && e[2] == character as u8)
        })
        .and_then(|at| plrp.get(at + 8).copied())
        .unwrap_or(0) as i32;
    // PRTY word 6 (the KO timer) of the object's record, and the SSTM rates.
    let pref = bar_entry(system, b"pref")?;
    let prty_index = objects
        .iter()
        .find(|o| o.model_name == name.as_bytes())
        .map(|o| o.neo_moveset as i16 as i32)
        .ok_or("party object entry absent")?;
    let prty = party_records::read_prty(bar_entry(pref, b"prty")?)?;
    let offset =
        actor_physics::prty_offset(&prty.offsets, prty_index).ok_or("PRTY record absent")?;
    let ko_time = prty
        .records
        .iter()
        .find(|r| r.offset == offset && !r.overlaps_table)
        .and_then(|r| r.words.get(6))
        .map(|w| f32::from_bits(*w))
        .ok_or("PRTY record absent or overlaps table")?;
    let sstm_bytes = bar_entry(pref, b"sstm")?;
    let sstm_offset = u32::from_le_bytes(sstm_bytes.get(4..8).ok_or("SSTM pointer")?.try_into()?);
    let record = sstm_bytes
        .get(sstm_offset as usize..)
        .ok_or("SSTM record")?;
    let sstm = actor_physics::Sstm::read(record).ok_or("SSTM record truncated")?;
    let rates = party_hit::Rates::read(record).ok_or("SSTM rates truncated")?;
    Ok(PartyKind {
        name,
        model,
        weapon,
        motions,
        spheres,
        strength: row.strength as i32,
        magic: row.magic as i32,
        hp,
        mp,
        records,
        pax,
        body_lift,
        heal_lift,
        hit_shapes: hit_shapes.iter().map(|h| (h.2, h.3)).collect(),
        target: combat::TargetStats {
            max_hp: hp,
            defense: row.defense as i32 + plrp_def,
            cap: 99999,
            min: 1,
            weakness: [100; 7],
        },
        ko_time,
        sstm,
        rates,
        character: character as u8,
        voices: None,
    })
}

/// Marks a member's root and its mesh parts.
#[derive(Component)]
pub struct PartyRoot(pub usize);
#[derive(Component)]
pub struct PartyPart {
    pub member: usize,
    pub ordinal: usize,
    pub weapon: bool,
}

/// Instance keys of party attack windows live above Sora's and the spells'.
const KEY_BASE: u64 = 1 << 61;
/// Effect handles of the casts (`player::party_spell` keys).
const FX_BASE: u64 = 1 << 60;
/// Donald's MP Charge length (`prty[7]`, 1200).
const CHARGE: f32 = 1200.;

/// The enemy's HIT spheres at its current motion tick: centre, radius and
/// whether the shape is a high one.
fn enemy_bodies(rt: &EnemyRuntime) -> Vec<(Vec3, f32, bool)> {
    let enemy_at = Vec3::from_array(rt.enemy.position);
    let enemy_turn = Quat::from_rotation_y(rt.enemy.yaw);
    let ec = &rt.clips.0[&rt.enemy.motion];
    let bones = &ec.bones[(rt.enemy.motion_tick as usize).min(ec.bones.len() - 1)];
    rt.body
        .iter()
        .filter_map(|&(b, offset, r, high)| {
            let mat = bones.get(&b)?;
            Some((
                enemy_at + enemy_turn * mat.transform_point3(offset),
                r,
                high,
            ))
        })
        .collect()
}

/// `target_pos(btlobj_target)`: the centre of the enemy's first TARGET
/// shape, else above its origin.
fn enemy_centre(rt: &EnemyRuntime) -> Vec3 {
    let origin = Vec3::from_array(rt.enemy.position);
    let turn = Quat::from_rotation_y(rt.enemy.yaw);
    let shape = rt.targets.first().and_then(|&(bone, offset, _, _)| {
        let ec = rt.clips.0.get(&rt.enemy.motion)?;
        let bones = ec
            .bones
            .get((rt.enemy.motion_tick as usize).min(ec.bones.len().saturating_sub(1)))?;
        Some(origin + turn * bones.get(&bone)?.transform_point3(offset))
    });
    shape.unwrap_or(origin + Vec3::Y * 70.)
}

/// A member's attacker terms: Strength and Magic of its level row, a
/// player-type owner.
fn attacker(kind: &PartyKind) -> combat::AttackerStats {
    combat::AttackerStats {
        strength: kind.strength,
        magic: kind.magic,
        max: 99999,
        min: 1,
        mods: combat::AttackerMods {
            player_type: true,
            ..combat::AttackerMods::default()
        },
    }
}

/// Starts a cast the tour orders: the spell's act against the enemy, on the
/// next tick. Returns false when the member cannot start it.
pub fn order_cast(game: &mut Game, member: usize, spell: party_members::Spell, heal: u8) -> bool {
    let Some(m) = game.party.get_mut(member) else {
        return false;
    };
    let Some(record) = m
        .kind
        .records
        .iter()
        .find(|r| r.spell == Some(spell))
        .copied()
    else {
        return false;
    };
    if m.stored {
        return false;
    }
    // It starts as soon as the member is free; no new decision is made
    // meanwhile.
    m.forced = Some((record, heal));
    true
}

/// One volume of a cast against the enemy's body: the hit record, the
/// ATKP step row and the damage with the caster's stats.
#[allow(clippy::too_many_arguments)]
fn land_volume(
    rt: &mut EnemyRuntime,
    index: usize,
    name: &str,
    stats: &combat::AttackerStats,
    v: &player::magic::Volume,
    key: u64,
    facing: Vec3,
    tick: u64,
    sound_queue: &mut Vec<(u32, &'static str)>,
    log: &mut Option<File>,
) {
    if !rt.enabled || !rt.enemy.targetable() || !ensure_attack_rows(rt, v.atkp, v.sub) {
        return;
    }
    let bodies = enemy_bodies(rt);
    rt.instance_seen.insert(key, tick);
    let (aid, grp) = *rt.instance_ids.entry(key).or_insert_with(|| {
        let combo = rt
            .steps
            .get(&(v.atkp, v.sub))
            .map_or(0, |r| r[0].combo_group as i32);
        (
            rt.ids.allocate(),
            if combo != 0 {
                combo
            } else {
                combat::allocate_group(&mut rt.next_group)
            },
        )
    });
    if rt.enemy.record.has(aid) {
        return;
    }
    let mut hit = None;
    for c in combat::swept_volume(v.centre, v.previous, v.radius, v.height) {
        if hit.is_none()
            && let Some(&(_, _, high)) = bodies.iter().find(|(b, r, _)| {
                combat::volume_hits_sphere(c, v.radius, v.height, b.to_array(), *r)
            })
        {
            hit = Some((Vec3::from_array(c), high));
        }
    }
    let Some((at, high)) = hit else { return };
    let rows = rt.steps[&(v.atkp, v.sub)].clone();
    let cure = matches!(rows[0].kind, 5 | 6);
    let (counted, row) = rt.enemy.record.hit(aid, grp, cure, |count| {
        let r = data::battle_tables::step_index(&rows, count);
        (r, rows[r].interval)
    });
    if !counted {
        return;
    }
    let attack = params_of(&rows[row]);
    let enemy_at = Vec3::from_array(rt.enemy.position);
    let dir = match v.moving {
        Some(m) => Vec3::from_array(m),
        None => enemy_at - at,
    } * Vec3::new(1., 0., 1.);
    let dir = dir.normalize_or(facing);
    if let Some(damage) =
        rt.enemy
            .hit_by(dir.to_array(), high, &attack, stats, rt.difficulty, false)
    {
        rt.last_attacker = Attacker::Member(index);
        if let Some(&id) = rt.hit_sounds.get(&v.atkp)
            && id >= 0
        {
            sound_queue.push((id as u32, "party_hit"));
        }
        log_event(
            log,
            serde_json::json!({"event":"party_hit","tick":tick,"member":name,
            "atkp":v.atkp,"step":row,"damage":damage,"enemy_hp":rt.enemy.hp}),
        );
    }
}

/// The Shadow's last player-side attacker (obj+0xd40, FUN_140411300).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attacker {
    Sora,
    Member(usize),
}

/// One sphere of an enemy attack this tick: its centre, the points it was
/// swept through since the last tick, and the attack's identity.
#[derive(Clone, Debug)]
pub struct ClawSample {
    pub instance: u64,
    pub atkp: u16,
    pub aid: u8,
    pub group: i32,
    pub radius: f32,
    pub centre: Vec3,
    pub points: Vec<[f32; 3]>,
    /// The attack's ATKP step rows.
    pub rows: Vec<data::battle_tables::Attack>,
    /// The tour's synthetic claws name the one member they are for.
    pub only: Option<usize>,
}

/// The nearest enemy to a member: the Shadow, or a gallery enemy within the
/// search radius (personality +0x68, 800) when it is nearer.
fn pick_target(
    shadow: Option<Target>,
    shadow_centre: Option<[f32; 3]>,
    gallery: &[GalleryTarget],
    from: [f32; 3],
) -> (Option<Target>, Option<[f32; 3]>) {
    let dist =
        |p: Vec3| Vec3::from_array(from).distance(p * Vec3::new(1., 0., 1.) + Vec3::Y * from[1]);
    let mut best = shadow.map(|t| (dist(Vec3::from_array(t.position)), t, shadow_centre));
    for g in gallery {
        let d = dist(g.position);
        if d > party_members::SEARCH_RADIUS || best.as_ref().is_some_and(|b| b.0 <= d) {
            continue;
        }
        best = Some((
            d,
            Target {
                position: g.position.to_array(),
                radius: g.radius,
            },
            Some(g.centre.to_array()),
        ));
    }
    best.map_or((None, None), |b| (Some(b.1), b.2))
}

/// One attack volume of a member this tick, for the gallery enemies: the
/// same hit the Shadow takes, offered to the battle bridge.
#[derive(Clone)]
pub struct OutVolume {
    pub member: usize,
    pub key: u64,
    pub atkp: u16,
    pub sub: u8,
    /// Centre, radius, height and the previous centre of each sphere.
    pub spheres: Vec<(Vec3, f32, f32, Option<Vec3>)>,
    pub moving: Option<Vec3>,
    pub stats: combat::AttackerStats,
    pub origin: Vec3,
    /// A spell's volume sweeps as an ellipsoid from its previous centre.
    pub spell: bool,
}

/// A gallery enemy a member may fight: where it stands and its TARGET point.
#[derive(Clone, Copy, Debug)]
pub struct GalleryTarget {
    pub position: Vec3,
    pub centre: Vec3,
    pub radius: f32,
}

/// What one enemy offers a member this tick: its attack spheres and what
/// turns them into damage.
pub struct Source<'a> {
    pub samples: &'a [ClawSample],
    pub attacker: &'a combat::AttackerStats,
    pub difficulty: combat::Difficulty,
    /// The attacker's origin: a hit travels away from it.
    pub from: Vec3,
    /// The attacker's objentry (0: the room's Shadow), whose effect
    /// container holds the hit spark.
    pub object: u32,
}

/// The attacker's reaction to a guarded claw (`FUN_140411060`): the ATKP's
/// ReflectMotion and hit-back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflect {
    pub motion: u8,
    pub hit_back: i8,
}

/// The battle flag (`DAT_142a11404`, FUN_1403ab4a0): on while an enemy is
/// within 1200 of Sora or a member's attack is alive. It follows that
/// state after a delay: 45 ticks to switch on, 120 to switch off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BattleMode {
    pub on: bool,
    pub timer: f32,
}

/// `DAT_1405b67f0` and `DAT_1405b5330`.
const BATTLE_ENTER: f32 = 45.;
const BATTLE_LEAVE: f32 = 120.;
/// `DAT_1405b557c`: how near an enemy must be to Sora.
const BATTLE_RANGE: f32 = 1200.;

impl Default for BattleMode {
    fn default() -> Self {
        BattleMode {
            on: false,
            timer: BATTLE_ENTER,
        }
    }
}

impl BattleMode {
    pub fn update(&mut self, wanted: bool) {
        if wanted != self.on {
            self.timer -= 1.;
            if self.timer <= 0. {
                self.on = wanted;
            }
        }
        if wanted == self.on {
            self.timer = if self.on { BATTLE_LEAVE } else { BATTLE_ENTER };
        }
    }
}

/// A key family for the KO marker effects (00effect element 14).
const KO_FX: u64 = FX_BASE | 1 << 39;

impl MemberRt {
    /// FUN_1403d69b0: a battle voice, picked from the vtbl rows of the
    /// character.
    pub fn say(&mut self, n: i16, sound_queue: &mut Vec<(u32, &'static str)>) {
        let character = self.kind.character;
        if let Some(id) = self.voices.as_mut().and_then(|v| v.pick(character, n)) {
            sound_queue.push((id, "party_voice"));
        }
    }

    /// Frame trigger 14 of the motion playing: the voices whose tick the
    /// motion has reached since the last scan.
    fn motion_voices(&mut self, sound_queue: &mut Vec<(u32, &'static str)>) {
        let motion = self.follower.motion();
        let tick = self.follower.tick;
        let from = match self.voice_at {
            (m, t) if m == motion && t <= tick => Some(t),
            _ => None,
        };
        self.voice_at = (motion, tick);
        let due: Vec<i16> = self
            .kind
            .motions
            .get(&motion)
            .map(|d| {
                d.voices
                    .iter()
                    .filter(|&&(at, _)| from.is_none_or(|f| at > f) && at <= tick)
                    .map(|&(_, n)| n)
                    .collect()
            })
            .unwrap_or_default();
        for n in due {
            self.say(n, sound_queue);
        }
    }

    pub fn down(&self) -> bool {
        self.follower.down.is_some()
    }

    /// The model's HIT spheres at the member's pose: centre, radius and
    /// whether a hit on it flinches high.
    fn bodies(&self) -> Vec<(Vec3, f32, bool)> {
        let kind = &self.kind;
        let Some(data) = kind
            .motions
            .get(&self.follower.motion())
            .or_else(|| kind.motions.get(&0))
        else {
            return Vec::new();
        };
        let Some(centres) = data.hit.get(data.frame_of(self.follower.tick)) else {
            return Vec::new();
        };
        let at = Vec3::from_array(self.follower.position);
        let turn = Quat::from_rotation_y(self.follower.yaw);
        centres
            .iter()
            .zip(&kind.hit_shapes)
            .map(|(c, &(r, high))| (at + turn * *c, r, high))
            .collect()
    }

    /// The live guard spheres of the member's weapon (an ATKP Type 2 or 3
    /// window): Goofy's shield (ATKP 139) in the idle, walk and run motions.
    fn guards(
        &self,
        row: &dyn Fn(u16) -> Option<data::battle_tables::Attack>,
    ) -> Vec<(Vec3, f32, u16)> {
        let kind = &self.kind;
        let Some(data) = kind.motions.get(&self.follower.motion()) else {
            return Vec::new();
        };
        if data.blade.is_empty() {
            return Vec::new();
        }
        let f = data.frame_of(self.follower.tick);
        let at = Vec3::from_array(self.follower.position);
        let turn = Quat::from_rotation_y(self.follower.yaw);
        let mut out = Vec::new();
        for &(start, end, atkp, group) in &data.hits {
            if (f as u32) < start || (f as u32) > end {
                continue;
            }
            if !row(atkp).is_some_and(|r| matches!(r.kind, 2 | 3)) {
                continue;
            }
            for &(offset, radius) in kind.spheres.get(&group).into_iter().flatten() {
                out.push((
                    at + turn * data.blade[f].transform_point3(offset),
                    radius,
                    atkp,
                ));
            }
        }
        out
    }
}

impl MemberRt {
    /// The ATKP ids of the weapon windows of the motion playing.
    pub fn window_atkps(&self) -> Vec<u16> {
        self.kind
            .motions
            .get(&self.follower.motion())
            .map(|d| d.hits.iter().map(|h| h.2).collect())
            .unwrap_or_default()
    }
}

/// What a hit did to a member, for the log.
struct Taken {
    damage: i32,
    counted: bool,
    reaction: Option<u16>,
    ko: bool,
    /// Voices the hit calls for: 9 when HP first falls to a quarter
    /// (FUN_1403c34d0), 2 at the KO (FUN_1403d5dc0).
    voices: Vec<i16>,
}

/// The enemy side of a hit on a member, from the attack's own tables.
struct Blow<'a> {
    rows: &'a [data::battle_tables::Attack],
    aid: u8,
    group: i32,
    atkp: u16,
    /// The sphere struck: centre, radius and its flinch height.
    body: (Vec3, f32, bool),
    /// The attack sphere's centre.
    claw: Vec3,
    /// The attacker's origin (the hit travels away from it).
    from: Vec3,
}

/// `FUN_1401b0290` → `FUN_1403d60c0` for a member: the record, the HP
/// change (`FUN_1403d3ba0`), the KO (`FUN_1403c33d0`) and the reaction
/// (`FUN_1403d2fe0`).
fn take_hit<W: player::World>(
    src: &Source,
    m: &mut MemberRt,
    blow: &Blow,
    world: &W,
) -> (Taken, usize) {
    let rows = blow.rows;
    let cure = matches!(rows[0].kind, 5 | 6);
    let (counted, row) = m.record.hit(blow.aid, blow.group, cure, |count| {
        let r = data::battle_tables::step_index(rows, count);
        (r, rows[r].interval)
    });
    if !counted {
        return (
            Taken {
                damage: 0,
                counted,
                reaction: None,
                ko: false,
                voices: Vec::new(),
            },
            row,
        );
    }
    let attack = params_of(&rows[row]);
    let state = combat::TargetState {
        hp: m.hp,
        player_type: true,
        defender: 0,
        damage_control: 0.,
        damage_control_ratio: 50,
        difficulty: src.difficulty,
    };
    let damage = combat::damage_with(src.attacker, &attack, &m.kind.target, &state);
    let before = m.hp;
    m.hp = combat::apply_damage(m.hp, 0, m.max_hp, damage, false);
    // FUN_1403d2eb0: HP 0 runs vt+0xb0 (FUN_1403c33d0): the dead bit and
    // the revive timer, PRTY word 6.
    let ko = before > 0 && m.hp == 0;
    if ko {
        m.follower.ko(m.kind.ko_time);
    }
    // FUN_1403c1d30: HP at or under a quarter of the maximum.
    let quarter = |hp: i32| hp <= m.max_hp / 4;
    let mut voices = Vec::new();
    if !quarter(before) && quarter(m.hp) {
        voices.push(9);
    }
    if ko {
        voices.push(2);
    }
    let away = ((blow.body.0 - blow.from) * Vec3::new(1., 0., 1.)).normalize_or(Vec3::Z);
    let hit = player::Hit {
        reaction: attack.reaction,
        original: attack.reaction,
        direction: [away.x, away.z],
        high: blow.body.2,
        knockback: attack.knockback,
    };
    let kind = m.kind.clone();
    let reaction = m.follower.react(&hit, &kind.sstm, world, &|mo| {
        kind.motions.contains_key(&mo)
    });
    (
        Taken {
            damage,
            counted,
            reaction,
            ko,
            voices,
        },
        row,
    )
}

/// One enemy's attack spheres against one member: Goofy's shield first
/// (the attack-versus-attack clash runs before the body pass), then the HIT
/// spheres. One hit per member per tick. `guard_row` finds the ATKP row of
/// the member's own guard windows. Returns the attacker's reflect when a
/// claw was guarded.
#[allow(clippy::too_many_arguments)]
pub fn receive<W: player::World>(
    src: &Source,
    guard_row: &dyn Fn(u16) -> Option<data::battle_tables::Attack>,
    m: &mut MemberRt,
    index: usize,
    world: &W,
    tick: u64,
    sound_queue: &mut Vec<(u32, &'static str)>,
    effect_queue: &mut Vec<EffectRequest>,
    log: &mut Option<File>,
) -> Option<Reflect> {
    if m.stored {
        return None;
    }
    let bodies = m.bodies();
    let guards = m.guards(guard_row);
    let mut retired: Vec<u64> = Vec::new();
    // FUN_1403d1f40: a guard with a ReflectRange R only clashes with an
    // attacker within R degrees of the owner's facing (ReflectAngle turns the
    // facing by twice its value). ATKP 139 has R = 45. The claws' own cone
    // (0 for the Shadow's) needs the attacker's facing and is taken as met.
    let owner = Vec3::from_array(m.follower.position);
    let toward = ((owner - src.from) * Vec3::new(1., 0., 1.)).normalize_or_zero();
    let facing = Vec3::new(m.follower.yaw.sin(), 0., m.follower.yaw.cos());
    let cone = |row: &data::battle_tables::Attack| {
        row.reflect_range == 0
            || toward
                .dot(Quat::from_rotation_y((row.reflect_angle as f32 * 2.).to_radians()) * facing)
                < -(row.reflect_range as f32).to_radians().cos()
    };
    for claw in src.samples {
        if m.record.has(claw.aid) || claw.rows.is_empty() || claw.only.is_some_and(|i| i != index) {
            continue;
        }
        let rows = &claw.rows;
        // FUN_1403d08e0: the clash table pairs 0-0, 0-2, 0-3 and 1-3 against
        // a guard of the other team; ATKP 139 and the claws have no cone.
        let clashed = guards.iter().find(|(g, gr, gatkp)| {
            guard_row(*gatkp).is_some_and(|g_row| {
                let pair = (g_row.kind.min(rows[0].kind), g_row.kind.max(rows[0].kind));
                matches!(pair, (0, 0) | (0, 2) | (0, 3) | (1, 3)) && cone(&g_row)
            }) && claw
                .points
                .iter()
                .any(|c| combat::spheres_overlap(*c, claw.radius, g.to_array(), *gr))
        });
        if let Some(&(g, gr, gatkp)) = clashed {
            // clash(guard, claw): the claw joins the member's record, so it
            // can no longer hurt it; clash(claw, guard): the attacker's
            // FUN_140411060 plays its ReflectMotion and starts mode_revenge.
            m.record.mark(claw.aid);
            retired.push(claw.instance);
            let guarding = guard_row(gatkp)?;
            let contact = g + (claw.centre - g).normalize_or_zero() * gr;
            // FUN_1403d61d0: effect 13 at the guard's surface, and unless a
            // refact field is 2 the bounce (motion 41, slide 50).
            effect_queue.push(EffectRequest::System {
                id: 13,
                at: contact,
            });
            if guarding.refact_self != 2 && rows[0].refact_other != 2 {
                let kind = m.kind.clone();
                let sstm = kind.sstm;
                m.follower
                    .bounce(&sstm, world, &|mo| kind.motions.contains_key(&mo));
            }
            log_event(
                log,
                serde_json::json!({"event":"party_guard","tick":tick,"member":m.kind.name,
                "atkp":claw.atkp,"guard_atkp":gatkp,"reaction":m.follower.reaction.map(|r| r.motion)}),
            );
            return (rows[0].refact_self != 2 && rows[0].reflect_motion != 0).then_some(Reflect {
                motion: rows[0].reflect_motion,
                hit_back: rows[0].reflect_hit_back,
            });
        }
    }
    if m.down() {
        // FUN_1403d60c0: a dead member only takes the cure types.
        return None;
    }
    for claw in src.samples.iter().filter(|c| c.only == Some(index)) {
        if !m.record.has(claw.aid) && !retired.contains(&claw.instance) && bodies.is_empty() {
            log_event(
                log,
                serde_json::json!({"event":"party_probe_unplaced","tick":tick,"member":m.kind.name,
                "guards":guards.len(),"bodies":bodies.len(),"motion":m.follower.motion()}),
            );
        }
    }
    for claw in src.samples {
        if retired.contains(&claw.instance)
            || m.record.has(claw.aid)
            || claw.rows.is_empty()
            || claw.only.is_some_and(|i| i != index)
        {
            continue;
        }
        let Some(&body) = bodies.iter().find(|(b, r, _)| {
            claw.points
                .iter()
                .any(|c| combat::spheres_overlap(*c, claw.radius, b.to_array(), *r))
        }) else {
            continue;
        };
        let blow = Blow {
            rows: &claw.rows,
            aid: claw.aid,
            group: claw.group,
            atkp: claw.atkp,
            body,
            claw: claw.centre,
            from: src.from,
        };
        let was_casting = m.cast.is_some();
        let (taken, row) = take_hit(src, m, &blow, world);
        if !taken.counted {
            return None;
        }
        for n in &taken.voices {
            m.say(*n, sound_queue);
        }
        let rows = &claw.rows;
        if rows[row].hit_sound >= 0 {
            sound_queue.push((rows[row].hit_sound as u32, "party_hit_taken"));
        }
        // The attacker's spark on the member's hit shape (FUN_1403d1b40; a
        // sphere, so height = radius).
        if rows[row].hit_effect != 0 {
            let v = blow.claw - body.0;
            let y = v.y.clamp(-body.1, body.1);
            let across = Vec3::new(v.x, 0., v.z).normalize_or_zero()
                * body.1
                * (1. - (y / body.1).powi(2)).max(0.).sqrt();
            effect_queue.push(EffectRequest::EnemyHit {
                object: src.object,
                effect: rows[row].hit_effect,
                random: rows[row].hit_effect_random,
                contact: body.0 + across + Vec3::Y * y,
                direction: ((body.0 - src.from) * Vec3::new(1., 0., 1.)).normalize_or(Vec3::Z),
            });
        }
        log_event(
            log,
            serde_json::json!({"event":"party_hit_taken","tick":tick,"member":m.kind.name,
            "index":index,"atkp":blow.atkp,"step":row,"damage":taken.damage,"hp":m.hp,
            "reaction":taken.reaction,"ko":taken.ko,"high":body.2,"interrupted_cast":was_casting}),
        );
        if taken.ko {
            log_event(
                log,
                serde_json::json!({"event":"party_ko","tick":tick,"member":m.kind.name,
                "timer":m.follower.down}),
            );
        }
        return None;
    }
    None
}

/// A heal on a member (`FUN_1403d2eb0` with a positive amount): a dead
/// member is revived first (`FUN_1403c3c30`) and keeps SSTM+0x54 of the
/// amount, then HP is clamped to the maximum. Returns whether it revived.
pub fn heal_member(m: &mut MemberRt, amount: i32) -> bool {
    if amount <= 0 {
        return false;
    }
    let mut amount = amount;
    let was_down = m.down();
    if was_down {
        // No member owns Hyper Healing in the room.
        amount = m.kind.rates.revive_heal(amount, false);
        m.follower.revive(&m.kind.sstm);
    }
    m.hp = (m.hp + amount).min(m.max_hp);
    was_down
}

/// The party-hit tour's synthetic enemy attack: one claw sphere of ATKP
/// `atkp` put on the member's first HIT sphere (or, with `shield`, on its
/// first guard sphere), taken by the next tick like the Shadow's own.
pub fn probe_claw(game: &mut Game, member: usize, atkp: u16, shield: bool) -> bool {
    let Game { party, enemy, .. } = game;
    let (Some(m), Some(rt)) = (party.get(member), enemy.as_mut()) else {
        return false;
    };
    if !ensure_attack_rows(rt, atkp, 0) {
        return false;
    }
    for id in m.window_atkps() {
        ensure_attack_rows(rt, id, 0);
    }
    let row = |id: u16| rt.steps.get(&(id, 0)).map(|v| v[0]);
    let spheres: Vec<Vec3> = if shield {
        m.guards(&row).iter().map(|g| g.0).collect()
    } else {
        m.bodies().iter().map(|b| b.0).collect()
    };
    let Some(&centre) = spheres.first() else {
        return false;
    };
    let rows = rt.steps[&(atkp, 0)].clone();
    let aid = rt.ids.allocate();
    let group = combat::allocate_group(&mut rt.next_group);
    let instance = (1u64 << 62) | aid as u64;
    rt.claws_now.push(ClawSample {
        instance,
        atkp,
        aid,
        group,
        radius: 20.,
        centre,
        points: vec![centre.to_array()],
        rows,
        only: Some(member),
    });
    true
}

/// Puts a member at a point facing a heading (the tour stages its cases).
pub fn place_member(game: &mut Game, member: usize, at: Vec3, yaw: f32) {
    if let Some(m) = game.party.get_mut(member) {
        m.follower.position = at.to_array();
        m.follower.yaw = yaw;
    }
}

/// Sets a member's HP (the tour puts one at the edge of a KO).
pub fn set_hp(game: &mut Game, member: usize, hp: i32) {
    if let Some(m) = game.party.get_mut(member) {
        m.hp = hp.clamp(0, m.max_hp);
    }
}

fn log_event(log: &mut Option<File>, record: serde_json::Value) {
    if let Some(file) = log.as_mut() {
        let _ = serde_json::to_writer(&mut *file, &record);
        let _ = std::io::Write::write_all(file, b"\n");
    }
}

/// One tick of every member: behaviour, the spell acts, then the hits their
/// weapons and spells land.
pub fn tick(game: &mut Game, tick: u64) {
    let Game {
        player,
        params,
        world,
        party,
        enemy,
        log,
        sound_queue,
        effect_queue,
        fx_calls,
        fx_live,
        magic_tour,
        magic_shift,
        party_spells,
        party_battle,
        party_hit_tour,
        drive,
        enemy_guards,
        party_volumes,
        gallery_targets,
        ..
    } = game;
    party_volumes.clear();
    // The battle flag follows an enemy near Sora or a live member attack
    // (FUN_1403ab4a0) after its delay; the KO timer reads it.
    let wanted = enemy.as_ref().is_some_and(|rt| {
        rt.enabled
            && rt.enemy.targetable()
            && Vec3::from_array(rt.enemy.position).distance(Vec3::from_array(player.position))
                < BATTLE_RANGE
    }) || party
        .iter()
        .any(|m| !m.stored && m.follower.attack.is_some());
    party_battle.update(wanted);
    let battle = party_battle.on;
    let system_pax = drive.as_ref().and_then(|d| d.src.system_pax.clone());
    let body = player::Body {
        radius: params.radius,
        height: params.height,
        step: params.step_height,
    };
    let shadow_target = enemy
        .as_ref()
        .filter(|rt| rt.enabled && rt.enemy.targetable())
        .map(|rt| Target {
            position: rt.enemy.position,
            radius: 40.,
        });
    let shadow_centre = enemy
        .as_ref()
        .filter(|rt| rt.enabled && rt.enemy.targetable())
        .map(|rt| enemy_centre(rt).to_array());
    let hp: [Option<(i32, i32)>; 3] = {
        let sora = enemy.as_ref().map(|rt| (rt.sora_hp, rt.sora_target.max_hp));
        let mut hp = [sora, None, None];
        for (i, m) in party.iter().enumerate().take(2) {
            hp[i + 1] = Some((m.hp, m.max_hp));
        }
        hp
    };
    // The tour orders its casts; the AI casts on its own elsewhere.
    let spells = *party_spells
        && (!*magic_tour || *party_hit_tour || super::magic_tour::free_party(tick, *magic_shift));
    // Heals the acts land this tick: party slot, ATKP id, healer index.
    let mut heals: Vec<(usize, u16, usize)> = Vec::new();
    for (index, m) in party.iter_mut().enumerate() {
        if m.stored {
            continue;
        }
        // FUN_1403d68b0: the MP Charge runs only while the member is up.
        if !m.down() {
            m.mp.tick(1., 1.);
        }
        // FUN_1403cf4b0: the member's hit record advances once a tick, its
        // group lock held while a damage motion plays.
        m.record
            .tick(1., party_hit::motion::is_damage(m.follower.motion()));
        let kind = m.kind.clone();
        // The nearest enemy: the Shadow or a gallery enemy within reach.
        let (target, centre) = pick_target(
            shadow_target,
            shadow_centre,
            gallery_targets,
            m.follower.position,
        );
        let length = |id: u16| kind.motions.get(&id).map(|d| d.end() + 1);
        let root = |id: u16, t: u32| {
            kind.motions
                .get(&id)
                .and_then(|d| d.root.get(t as usize))
                .map_or([0.; 3], |r| r.to_array())
        };
        let range = |id: u16| kind.motions.get(&id).and_then(|d| d.range16);
        let actionable = |id: u16| kind.motions.get(&id).and_then(|d| d.actionable);
        let scene = party_members::Scene {
            leader: player.position,
            leader_yaw: player.yaw,
            target,
            world: &*world,
            body,
            length: &length,
            root: &root,
            centre,
            leader_y: player.position[1],
            range: &range,
            busy: m.cast.is_some() || m.forced.is_some(),
            mp: Some((m.mp.cur, m.mp.max, m.mp.charging())),
            spells,
            hp,
            actionable: &actionable,
            sstm: kind.sstm,
            rates: kind.rates,
            battle,
        };
        let was = m.follower.attack.is_some();
        m.follower.tick(kind.records, &scene);
        m.motion_voices(sound_queue);
        // FUN_1403c3660: the KO timer ran out; FUN_1403c3bd0 heals through
        // FUN_1403d2eb0, which revives the member.
        if m.follower.revive_due() {
            let amount = kind.rates.timer_heal(m.max_hp);
            heal_member(m, amount);
            log_event(
                log,
                serde_json::json!({"event":"party_revive","tick":tick,"member":kind.name,
                "hp":m.hp,"by":"timer"}),
            );
        }
        // The DEAD motion's marker (00effect element 14, FUN_1403c3660):
        // started with the motion, killed when another one plays.
        let laid = m.follower.motion() == party_hit::motion::DEAD;
        if laid && !m.ko_fx {
            m.ko_fx = true;
            fx_calls.push(spell_fx::Call {
                pax: system_pax.clone(),
                fx: player::magic::Fx::Start {
                    key: KO_FX | index as u64,
                    id: 14,
                    bound: Some(player::magic::Anchor::Party(index + 1)),
                    at: None,
                },
            });
        } else if !laid && m.ko_fx {
            m.ko_fx = false;
            fx_calls.push(spell_fx::Call {
                pax: None,
                fx: player::magic::Fx::Kill(KO_FX | index as u64),
            });
        }
        // The tour's order starts the act as the AI's pick would.
        if m.follower.attack.is_none()
            && m.follower.reaction.is_none()
            && !m.down()
            && m.cast.is_none()
            && let Some((record, heal)) = m.forced.take()
        {
            // With no enemy to face the act goes straight ahead.
            let ahead = Target {
                position: [
                    m.follower.position[0] + m.follower.yaw.sin() * 300.,
                    m.follower.position[1],
                    m.follower.position[2] + m.follower.yaw.cos() * 300.,
                ],
                radius: 40.,
            };
            m.follower.begin(&record, &target.unwrap_or(ahead), heal);
            m.started = tick;
            m.armed = true;
        }
        let now = m.follower.attack;
        if !was
            && !m.armed
            && let Some(a) = now
        {
            m.started = tick;
            m.armed = a.spell.is_some();
        }
        // The act manager starts the spell's act (FUN_1403caa90).
        if m.armed
            && let Some(a) = now
            && let Some(spell) = a.spell
        {
            m.armed = false;
            m.casts += 1;
            let release = kind
                .motions
                .get(&a.motion)
                .and_then(|d| d.release)
                .unwrap_or(26);
            let key = FX_BASE | (index as u64) << 40 | m.casts << 8;
            m.cast = Some(
                party_spell::Cast::new(spell, index + 1, a.heal as usize, key, release)
                    .with_range(kind.motions.get(&a.motion).and_then(|d| d.range16)),
            );
            m.cast_motion = a.motion;
            m.reflected = false;
            // The act first call, trap (1,262): 6 for Donald attack spells,
            // 21 when Cure heals himself and 3 for another member.
            match spell {
                party_members::Spell::Cure => m.say(
                    if a.heal as usize == index + 1 { 21 } else { 3 },
                    sound_queue,
                ),
                party_members::Spell::Fire
                | party_members::Spell::Blizzard
                | party_members::Spell::Thunder => m.say(6, sound_queue),
                _ => {}
            }
            log_event(
                log,
                serde_json::json!({"event":"party_cast","tick":tick,"member":kind.name,
                "spell":format!("{spell:?}"),"motion":a.motion,"release":release,
                "motion_end":kind.motions.get(&a.motion).map(|d| d.end()),
                "heal":a.heal,"mp":m.mp.cur,"charging":m.mp.charging()}),
            );
        }
        if let Some(a) = m.follower.attack
            && matches!(
                a.spell,
                Some(party_members::Spell::Tornado | party_members::Spell::Turbo)
            )
            && a.tick % 30 == 0
        {
            log_event(
                log,
                serde_json::json!({"event":"party_special","tick":tick,"member":kind.name,
                "motion_tick":a.tick,"position":m.follower.position,"yaw":m.follower.yaw,
                "enemy":target.map(|t| t.position)}),
            );
        }
        let over = m.follower.attack.is_none();
        // The waits of the acts end when the member's motion changes, and
        // the act thread pauses while a damage motion plays (FUN_1403ca7c0).
        let same = m.follower.attack.is_some_and(|a| a.motion == m.cast_motion);
        let suspended = m
            .follower
            .reaction
            .is_some_and(|r| party_hit::motion::suspends_acts(r.motion));
        let reflected = std::mem::take(&mut m.reflected);
        if let Some(cast) = m.cast.as_mut() {
            let pos = m.follower.position;
            let facing = [m.follower.yaw.sin(), m.follower.yaw.cos()];
            let ground = |from: [f32; 3], max: f32| -> Option<[f32; 3]> {
                world
                    .floor(from[0], from[2], from[1])
                    .filter(|h| from[1] - h <= max)
                    .map(|h| [from[0], h, from[2]])
            };
            let bg = |p: [f32; 3], _r: f32| -> bool {
                world.floor(p[0], p[2], p[1]).is_none() || {
                    let w = world.resolve_walls([p[0], p[2]], 1., p[1] - 1., p[1] + 1.);
                    (w[0] - p[0]).abs() > 1e-3 || (w[1] - p[2]).abs() > 1e-3
                }
            };
            let effect = |k: u64| fx_live.get(&k).copied();
            let ctx = party_spell::Context {
                caster: pos,
                facing,
                centre: [pos[0], pos[1] + kind.body_lift, pos[2]],
                target: centre,
                effect: &effect,
                ground: &ground,
                bg_hit: &bg,
                over,
                same,
                suspended,
                reflected,
            };
            let out = cast.tick(&ctx);
            let spell = cast.spell;
            if cast.finished() {
                m.cast = None;
            }
            for fx in out.fx {
                let pax = match &fx {
                    player::magic::Fx::Start { .. } => kind.pax.clone(),
                    _ => None,
                };
                fx_calls.push(spell_fx::Call { pax, fx });
            }
            if let Some(cost) = out.spend {
                m.mp.change(-(cost as i32), CHARGE);
                log_event(
                    log,
                    serde_json::json!({"event":"party_release","tick":tick,"member":kind.name,
                    "spell":format!("{spell:?}"),"mp":m.mp.cur,"charging":m.mp.charging()}),
                );
            }
            for strike in out.strikes {
                heals.push((strike.party, strike.atkp, index));
            }
            // The same volumes, offered to the gallery enemies.
            for v in out
                .volumes
                .iter()
                .filter(|v| v.role == player::magic::Role::Damage)
            {
                party_volumes.push(OutVolume {
                    member: index,
                    key: KEY_BASE | 1 << 47 | (v.key & 0x7fff_ffff_ffff),
                    atkp: v.atkp,
                    sub: v.sub,
                    spheres: vec![(
                        Vec3::from_array(v.centre),
                        v.radius,
                        v.height,
                        v.previous.map(Vec3::from_array),
                    )],
                    moving: v.moving.map(Vec3::from_array),
                    stats: attacker(&kind),
                    origin: Vec3::from_array(pos),
                    spell: true,
                });
            }
            // FUN_1403d08e0: the shot (Type 1) clashes with another team's
            // super guard (Type 3); attack_is_reflect ends it next tick.
            for v in out
                .volumes
                .iter()
                .filter(|v| v.atkp == party_spell::BLIZZARD_ATKP)
            {
                let hit = combat::swept_volume(v.centre, v.previous, v.radius, v.height)
                    .into_iter()
                    .any(|c| {
                        enemy_guards
                            .iter()
                            .any(|(g, gr)| combat::spheres_overlap(c, v.radius, g.to_array(), *gr))
                    });
                if hit {
                    m.reflected = true;
                    log_event(
                        log,
                        serde_json::json!({"event":"party_reflect","tick":tick,"member":kind.name,
                        "atkp":v.atkp}),
                    );
                }
            }
            if let Some(rt) = enemy.as_mut() {
                let stats = attacker(&kind);
                for v in &out.volumes {
                    let key = KEY_BASE | 1 << 47 | (v.key & 0x7fff_ffff_ffff);
                    land_volume(
                        rt,
                        index,
                        kind.name,
                        &stats,
                        v,
                        key,
                        Vec3::new(facing[0], 0., facing[1]),
                        tick,
                        sound_queue,
                        log,
                    );
                }
            }
        }
        // The enemy's claws against this member (FUN_1401b0290).
        if let Some(rt) = enemy.as_mut() {
            for id in m.window_atkps() {
                ensure_attack_rows(rt, id, 0);
            }
            let reflect = {
                let src = Source {
                    samples: &rt.claws_now,
                    attacker: &rt.shadow_attacker,
                    difficulty: rt.difficulty,
                    from: Vec3::from_array(rt.enemy.position),
                    object: 0,
                };
                let row = |id: u16| rt.steps.get(&(id, 0)).map(|v| v[0]);
                receive(
                    &src,
                    &row,
                    m,
                    index,
                    &*world,
                    tick,
                    sound_queue,
                    effect_queue,
                    log,
                )
            };
            if let Some(r) = reflect {
                rt.enemy.reflect(r.motion as usize, r.hit_back as f32);
            }
        }
        // The weapon's attack windows, offered to the gallery enemies.
        if let Some(a) = m.follower.attack
            && let Some(data) = kind.motions.get(&a.motion)
            && !data.blade.is_empty()
        {
            let f = (a.tick as usize).min(data.blade.len() - 1);
            let turn = Quat::from_rotation_y(m.follower.yaw);
            let at = Vec3::from_array(m.follower.position);
            for (w, &(start, end, atkp, group)) in data.hits.iter().enumerate() {
                let Some(list) = kind.spheres.get(&group) else {
                    continue;
                };
                if a.tick < start || a.tick > end {
                    continue;
                }
                party_volumes.push(OutVolume {
                    member: index,
                    key: KEY_BASE | (index as u64) << 48 | m.started << 4 | w as u64,
                    atkp,
                    sub: 0,
                    spheres: list
                        .iter()
                        .map(|&(offset, radius)| {
                            (
                                at + turn * data.blade[f].transform_point3(offset),
                                radius,
                                radius,
                                None,
                            )
                        })
                        .collect(),
                    moving: None,
                    stats: attacker(&kind),
                    origin: at,
                    spell: false,
                });
            }
        }
        // The weapon's attack windows against the enemy's body.
        let (Some(a), Some(rt)) = (m.follower.attack, enemy.as_mut()) else {
            continue;
        };
        let Some(data) = kind.motions.get(&a.motion) else {
            continue;
        };
        if !rt.enabled || !rt.enemy.targetable() || data.blade.is_empty() {
            continue;
        }
        let f = (a.tick as usize).min(data.blade.len() - 1);
        let turn = Quat::from_rotation_y(m.follower.yaw);
        let at = Vec3::from_array(m.follower.position);
        let enemy_at = Vec3::from_array(rt.enemy.position);
        let bodies = enemy_bodies(rt);
        for (w, &(start, end, atkp, group)) in data.hits.iter().enumerate() {
            if a.tick < start || a.tick > end {
                continue;
            }
            let Some(list) = kind.spheres.get(&group) else {
                continue;
            };
            let key = KEY_BASE | (index as u64) << 48 | m.started << 4 | w as u64;
            rt.instance_seen.insert(key, tick);
            let (aid, grp) = *rt.instance_ids.entry(key).or_insert_with(|| {
                (
                    rt.ids.allocate(),
                    combat::allocate_group(&mut rt.next_group),
                )
            });
            if rt.enemy.record.has(aid) || !ensure_attack_rows(rt, atkp, 0) {
                continue;
            }
            let mut hit = None;
            for &(offset, radius) in list {
                let c = at + turn * data.blade[f].transform_point3(offset);
                if let Some(&(_, _, high)) = bodies.iter().find(|(b, r, _)| {
                    combat::spheres_overlap(c.to_array(), radius, b.to_array(), *r)
                }) {
                    hit = Some(high);
                    break;
                }
            }
            let Some(high) = hit else {
                continue;
            };
            let rows = rt.steps[&(atkp, 0)].clone();
            let cure = matches!(rows[0].kind, 5 | 6);
            let (counted, row) = rt.enemy.record.hit(aid, grp, cure, |count| {
                let r = data::battle_tables::step_index(&rows, count);
                (r, rows[r].interval)
            });
            if !counted {
                continue;
            }
            let attack = params_of(&rows[row]);
            let dir = ((enemy_at - at) * Vec3::new(1., 0., 1.)).normalize_or(Vec3::new(
                m.follower.yaw.sin(),
                0.,
                m.follower.yaw.cos(),
            ));
            let stats = attacker(&kind);
            if let Some(damage) =
                rt.enemy
                    .hit_by(dir.to_array(), high, &attack, &stats, rt.difficulty, false)
            {
                rt.last_attacker = Attacker::Member(index);
                if let Some(&id) = rt.hit_sounds.get(&atkp)
                    && id >= 0
                {
                    sound_queue.push((id as u32, "party_hit"));
                }
                log_event(
                    log,
                    serde_json::json!({"event":"party_hit","tick":tick,
                    "member":kind.name,"atkp":atkp,"damage":damage,"enemy_hp":rt.enemy.hp}),
                );
            }
        }
    }
    // The guard spheres are read once, by the tick that follows their
    // making.
    enemy_guards.clear();
    // attack_strike on the chosen member: Cure's heal is an ATKP Type 6 row,
    // (Power x Magic + 99) / 100 with the healer's Magic, clamped to the
    // maximum HP.
    for (slot, atkp, healer) in heals {
        let Some((stats, healer_name)) =
            party.get(healer).map(|m| (attacker(&m.kind), m.kind.name))
        else {
            continue;
        };
        let Some(rt) = enemy.as_mut() else { continue };
        if !ensure_attack_rows(rt, atkp, 0) {
            continue;
        }
        let attack = params_of(&rt.steps[&(atkp, 0)][0]);
        let full = combat::TargetState::full(rt.sora_target.max_hp);
        let amount = combat::damage_with(&stats, &attack, &rt.sora_target, &full);
        let record = if slot == 0 {
            rt.sora_hp = (rt.sora_hp + amount).min(rt.sora_target.max_hp);
            serde_json::json!({"event":"party_heal","tick":tick,"healer":healer_name,
                "atkp":atkp,"amount":amount,"target":"Sora","hp":rt.sora_hp})
        } else if let Some(member) = party.get_mut(slot - 1) {
            let was_down = heal_member(member, amount);
            serde_json::json!({"event":"party_heal","tick":tick,"healer":healer_name,
                "atkp":atkp,"amount":amount,"target":member.kind.name,"hp":member.hp,
                "was_down":was_down})
        } else {
            continue;
        };
        log_event(log, record);
    }
}

/// Places and poses the members.
pub fn present(
    game: Res<Game>,
    mut roots: Query<(&PartyRoot, &mut Transform)>,
    parts: Query<(&PartyPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (root, mut t) in &mut roots {
        if let Some(m) = game.party.get(root.0) {
            t.translation = Vec3::from_array(m.follower.position);
            t.rotation = Quat::from_rotation_y(m.follower.yaw);
        }
    }
    for (part, handle) in &parts {
        let Some(m) = game.party.get(part.member) else {
            continue;
        };
        let Some(data) = m.kind.motions.get(&m.follower.motion()) else {
            continue;
        };
        let f = data.frame_of(m.follower.tick);
        let (frame, model) = if part.weapon {
            // A motion without its own weapon pose keeps the idle one.
            let (source, at) = if data.weapon.is_empty() {
                (m.kind.motions.get(&0), 0)
            } else {
                (Some(data), f)
            };
            let (Some(source), Some(weapon)) = (source, &m.kind.weapon) else {
                continue;
            };
            let Some(frame) = source.weapon.get(at) else {
                continue;
            };
            (frame, weapon)
        } else {
            (&data.frames[f], &m.kind.model)
        };
        let positions: Vec<[f32; 3]> = model.parts[part.ordinal]
            .indices
            .iter()
            .map(|&i| frame[i as usize].to_array())
            .collect();
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_battle_flag_switches_on_after_45_ticks_and_off_after_120() {
        let mut b = BattleMode::default();
        assert!(!b.on);
        for t in 1..=44 {
            b.update(true);
            assert!(!b.on, "tick {t}");
        }
        b.update(true);
        assert!(b.on);
        // Leaving takes 120 ticks of no enemy, and a brief return restarts
        // the wait.
        for _ in 0..119 {
            b.update(false);
            assert!(b.on);
        }
        b.update(true);
        for _ in 0..119 {
            b.update(false);
            assert!(b.on);
        }
        b.update(false);
        assert!(!b.on);
    }
}
