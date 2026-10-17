//! Contact bridge for admitted gallery brains and the existing player battle
//! context. The legacy battle step advances the shared player hit record first.
use super::{
    EffectRequest, Game, abilities, enemy_combat::WorldShape, enemy_gallery::GalleryActor,
    ensure_attack_rows, magic_room, params_of,
};
use bevy::prelude::*;
use data::battle_tables;
use player::{World, combat};
use std::collections::HashMap;

#[derive(Default)]
pub struct ContactState {
    pub incoming_ids: HashMap<(u64, usize), (u8, i32)>,
    pub previous: HashMap<(u64, usize, usize), Vec3>,
    pub outgoing_previous: HashMap<(u64, usize), Vec3>,
    serial: Option<u64>,
    rewarded: bool,
}
impl ContactState {
    fn defeat_reward(&mut self, hp: i32) -> bool {
        if hp > 0 || self.rewarded {
            false
        } else {
            self.rewarded = true;
            true
        }
    }
}

/// Something of an enemy that can hit Sora this frame: a motion attack window
/// or an attack object its script placed. Shapes are (kind, ordinal, centre,
/// radius, the previous centre to sweep from).
struct Candidate {
    window: usize,
    atkp: u16,
    sub: u8,
    script_slot: Option<usize>,
    /// Applied by `attack_strike`: no hit callback runs for it.
    direct: bool,
    shapes: Vec<(u8, usize, Vec3, f32, Option<Vec3>)>,
}

/// Script attack objects use window numbers from here up so they never collide
/// with a motion window's.
const SCRIPT_WINDOW: usize = 0x1000;

#[derive(Clone)]
struct Volume {
    key: u64,
    atkp: u16,
    sub: u8,
    centres: Vec<VolumeCentre>,
    moving: Option<Vec3>,
    magic: Option<u64>,
    guard: bool,
}
type VolumeCentre = (Vec3, f32, f32, Option<Vec3>);

fn overlap(centre: Vec3, radius: f32, height: f32, sphere: bool, target: &WorldShape) -> bool {
    if sphere && target.shape == 3 {
        combat::spheres_overlap(
            centre.to_array(),
            radius,
            target.centre.to_array(),
            target.radius,
        )
    } else {
        let delta = target.centre - centre;
        let r = radius + target.radius;
        let h = height + target.height;
        (delta.x / r).powi(2) + (delta.y / h).powi(2) + (delta.z / r).powi(2) < 1.
    }
}

fn allocate(ids: &mut combat::AttackIds, next_group: &mut i32, combo: u8) -> (u8, i32) {
    let group = if combo == 0 {
        combat::allocate_group(next_group)
    } else {
        combo as i32
    };
    (ids.allocate(), group)
}

fn event(log: &mut Option<std::fs::File>, value: serde_json::Value) {
    if let Some(file) = log {
        let _ = serde_json::to_writer(&mut *file, &value);
        let _ = std::io::Write::write_all(file, b"\n");
    }
}

pub fn tick(game: &mut Game, actors: &mut [GalleryActor], tick: u64) {
    let Game {
        player,
        clips,
        clip_poses,
        poses,
        enemy,
        action_started,
        sora_attack,
        magic,
        drive,
        cheats,
        sound_queue,
        effect_queue,
        fx_calls,
        log,
        hero_hit_tick,
        enemy_incoming_hits,
        enemy_outgoing_hits,
        world,
        ..
    } = game;
    let Some(rt) = enemy.as_mut() else { return };
    let sora_at = Vec3::from_array(player.position);
    let sora_turn = Quat::from_rotation_y(player.yaw);
    let sora_facing = Vec3::new(player.yaw.sin(), 0., player.yaw.cos());
    let Some(&clip_index) = poses.get(&(player.action, player.armed)) else {
        return;
    };
    let cp = &clip_poses[clip_index];
    let frame = player.motion_frame(clips) as usize;
    let mut volumes = Vec::new();
    for (hand, (blade, weapon, hits)) in [
        (&cp.blade, &cp.weapon, &cp.hits),
        (&cp.blade2, &cp.weapon2, &cp.hits2),
    ]
    .into_iter()
    .enumerate()
    {
        if blade.is_empty() || weapon.is_empty() {
            continue;
        }
        let f = frame.min(blade.len() - 1);
        let to_model = weapon[f.min(weapon.len() - 1)] * blade[f];
        for (window, &(start, end, atkp, group)) in hits.iter().enumerate() {
            if frame < start as usize || frame >= end as usize || !ensure_attack_rows(rt, atkp, 0) {
                continue;
            }
            let centres = rt.blade[hand]
                .get(&group)
                .into_iter()
                .flatten()
                .map(|&(offset, radius)| {
                    (
                        sora_at + sora_turn * to_model.transform_point3(offset),
                        radius,
                        radius,
                        None,
                    )
                })
                .collect();
            volumes.push(Volume {
                key: (*action_started << 7) | (hand as u64) << 5 | window as u64,
                atkp,
                sub: 0,
                centres,
                moving: None,
                magic: None,
                guard: matches!(rt.rows[&atkp].kind, 2 | 3),
            });
        }
    }
    if let Some(bones) = cp.body.get(frame.min(cp.body.len().saturating_sub(1))) {
        for (window, &(start, end, atkp, group)) in cp.actor_hits.iter().enumerate() {
            if frame < start as usize || frame >= end as usize || !ensure_attack_rows(rt, atkp, 0) {
                continue;
            }
            let centres = sora_attack
                .get(&group)
                .into_iter()
                .flatten()
                .filter_map(|&(bone, offset, radius)| {
                    Some((
                        sora_at + sora_turn * bones.get(&bone)?.transform_point3(offset),
                        radius,
                        radius,
                        None,
                    ))
                })
                .collect();
            volumes.push(Volume {
                key: (*action_started << 7) | 2 << 5 | window as u64,
                atkp,
                sub: 0,
                centres,
                moving: None,
                magic: None,
                guard: matches!(rt.rows[&atkp].kind, 2 | 3),
            });
        }
    }
    if let Some(m) = magic.as_ref() {
        for v in &m.volumes {
            if !ensure_attack_rows(rt, v.atkp, v.sub) {
                continue;
            }
            volumes.push(Volume {
                key: magic_room::KEY_BASE | v.key,
                atkp: v.atkp,
                sub: v.sub,
                centres: vec![(
                    Vec3::from_array(v.centre),
                    v.radius,
                    v.height,
                    v.previous.map(Vec3::from_array),
                )],
                moving: v.moving.map(Vec3::from_array),
                magic: Some(v.key),
                guard: v.role == player::magic::Role::Guard,
            });
        }
    }
    for volume in &volumes {
        rt.instance_seen.insert(volume.key, tick);
        if let std::collections::hash_map::Entry::Vacant(entry) = rt.instance_ids.entry(volume.key)
        {
            let combo = rt.steps[&(volume.atkp, volume.sub)][0].combo_group;
            let id = allocate(&mut rt.ids, &mut rt.next_group, combo);
            entry.insert(id);
        }
    }
    let ab = &player.abilities;
    rt.sora.mods = combat::AttackerMods {
        player_type: true,
        mp_charging: player.mp_charging,
        berserk_strength: ab.berserk_strength as i32,
        element_percent: ab.element.map(i32::from),
        combo_step: player.combo.step as i32,
        ground_limit: ab.ground_combo as i32,
        air_limit: ab.air_combo as i32,
        combo_boost: ab.combo_boost as i32,
        air_combo_boost: ab.air_combo_boost as i32,
        reaction_boost: ab.reaction_boost as i32,
        combo_boost_enabled: ab.has(abilities::bit::COMBO_BOOST),
        air_combo_boost_enabled: ab.has(abilities::bit::AIR_COMBO_BOOST),
        reaction_boost_enabled: ab.has(abilities::bit::REACTION_BOOST),
    };
    let sora_body: Vec<_> = cp
        .body
        .get(frame.min(cp.body.len().saturating_sub(1)))
        .into_iter()
        .flat_map(|bones| {
            rt.sora_body
                .iter()
                .filter_map(move |&(bone, offset, radius, high)| {
                    Some((
                        sora_at + sora_turn * bones.get(&bone)?.transform_point3(offset),
                        radius,
                        high,
                    ))
                })
        })
        .collect();
    let damage_motion = (10..=39).contains(&player.motion_id(clips));
    for actor in actors {
        let Some(battle) = actor.battle.as_mut() else {
            continue;
        };
        if battle.error.is_some() {
            continue;
        }
        if battle.contacts.defeat_reward(battle.brain.hp()) {
            let low = rt.sora_hp * 2 <= rt.sora_target.max_hp;
            let exp = player::enemy_prizes::kill_exp(
                battle.assets.exp.0,
                battle.assets.exp.1,
                rt.difficulty == combat::Difficulty::Critical,
                low.then_some(player.abilities.experience_boost as u32),
            );
            // FUN_1403ed110: No Experience (bit 116) takes the kill's EXP.
            if !player.abilities.has(player::abilities::bit::NO_EXPERIENCE) {
                super::prize_room::grant_exp(rt, exp, log, tick);
            }
            if let Some(d) = drive.as_mut()
                && d.active
            {
                d.pending_exp += player::forms::kill_exp(d.form, battle.assets.character);
            }
            if let Some(prize) = battle.assets.prize {
                let at = battle.brain.position();
                let floor = world
                    .floor(at[0], at[2], at[1] + 30.)
                    .unwrap_or(at[1].max(0.));
                for (kind, count) in player::enemy_prizes::drops(
                    &prize.counts,
                    1. + player.abilities.jackpot,
                    player.abilities.drive_converter,
                ) {
                    for _ in 0..count {
                        let random = [
                            rt.rng.range(0., 1.),
                            rt.rng.range(0., 1.),
                            rt.rng.range(0., 1.),
                        ];
                        rt.orbs.push(player::enemy_prizes::Orb::spawn(
                            kind,
                            [at[0], floor.max(at[1]) + 20., at[2]],
                            random,
                        ));
                    }
                }
                for (item, chance) in prize.items {
                    if item != 0
                        && rt.rng.range(0., 100.)
                            < (chance as f32 * (1. + player.abilities.lucky_lucky))
                    {
                        event(
                            log,
                            serde_json::json!({"event":"gallery_item_drop","tick":tick,"actor":actor.id,"item":item}),
                        );
                    }
                }
            }
            event(
                log,
                serde_json::json!({"event":"gallery_defeat","tick":tick,"actor":actor.id,"exp":exp,"character":battle.assets.character}),
            );
        }
        battle.brain.record_mut().tick(1., false);
        if battle.contacts.serial != Some(battle.serial) {
            battle.contacts.previous.clear();
            battle.contacts.incoming_ids.clear();
            battle.contacts.serial = Some(battle.serial);
        }
        let at = Vec3::from_array(battle.brain.position());
        let mut outgoing = None;
        for v in &volumes {
            let (aid, _) = rt.instance_ids[&v.key];
            for (sphere, (centre, radius, height, prior)) in v.centres.iter().enumerate() {
                let previous = if v.magic.is_some() {
                    *prior
                } else {
                    battle
                        .contacts
                        .outgoing_previous
                        .insert((v.key, sphere), *centre)
                };
                if v.guard
                    || !battle.brain.targetable()
                    || battle.brain.record_mut().has(aid)
                    || outgoing.is_some()
                {
                    continue;
                }
                let swept = if v.magic.is_some() {
                    combat::swept_volume(
                        centre.to_array(),
                        previous.map(|p| p.to_array()),
                        *radius,
                        *height,
                    )
                } else {
                    combat::swept(centre.to_array(), previous.map(|p| p.to_array()), *radius)
                };
                for c in swept {
                    if let Some(shape) = battle.pose.body.iter().find(|s| {
                        overlap(Vec3::from_array(c), *radius, *height, *radius == *height, s)
                    }) {
                        outgoing = Some((v.clone(), shape.clone(), Vec3::from_array(c)));
                        break;
                    }
                }
            }
        }
        let live_keys: Vec<_> = volumes.iter().map(|v| v.key).collect();
        battle
            .contacts
            .outgoing_previous
            .retain(|(key, _), _| live_keys.contains(key));
        if let Some((v, target, centre)) = outgoing {
            let rows = &rt.steps[&(v.atkp, v.sub)];
            let (aid, group) = rt.instance_ids[&v.key];
            let (counted, row) =
                battle
                    .brain
                    .record_mut()
                    .hit(aid, group, matches!(rows[0].kind, 5 | 6), |count| {
                        let index = battle_tables::step_index(rows, count);
                        (index, rows[index].interval)
                    });
            let attack = params_of(&rows[row]);
            let direction = (v.moving.unwrap_or(if v.magic.is_some() {
                at - centre
            } else {
                at - sora_at
            }) * Vec3::new(1., 0., 1.))
            .normalize_or(sora_facing);
            if counted
                && let Some(damage) = battle.brain.hit(
                    direction.to_array(),
                    target.high,
                    &attack,
                    &rt.sora,
                    rt.difficulty,
                )
            {
                if damage > 0 {
                    *enemy_outgoing_hits += 1;
                }
                player.connected = true;
                player.drive.hit(
                    rows[row].drive_drain,
                    true,
                    player.mp_charging,
                    player.abilities.drive_boost,
                );
                if let Some(d) = drive.as_mut()
                    && d.active
                    && d.form == player::forms::VALOR
                {
                    d.pending_exp += 1;
                }
                if rows[row].hit_sound >= 0 {
                    sound_queue.push((rows[row].hit_sound as u32, "hit"));
                }
                if rows[row].hit_effect != 0 {
                    let delta = centre - target.centre;
                    let y = delta.y.clamp(-target.height, target.height);
                    let across = Vec3::new(delta.x, 0., delta.z).normalize_or_zero()
                        * target.radius
                        * (1. - (y / target.height).powi(2)).max(0.).sqrt();
                    let contact = target.centre + across + Vec3::Y * y;
                    // A spell's spark comes from the spell's own PAX.
                    let spell_spark = v.magic.and_then(|key| {
                        magic.as_mut().and_then(|m| {
                            magic_room::spark(
                                m,
                                key,
                                rows[row].hit_effect,
                                contact.to_array(),
                                direction.to_array(),
                            )
                        })
                    });
                    match spell_spark {
                        Some(calls) => fx_calls.extend(calls),
                        None => effect_queue.push(EffectRequest::Hit {
                            effect: rows[row].hit_effect,
                            random: rows[row].hit_effect_random,
                            contact,
                            direction,
                        }),
                    }
                }
                if let Some(key) = v.magic
                    && let Some(m) = magic.as_mut()
                {
                    m.feedback.hit.push(key);
                    if combat::reaction_class(attack.reaction) == 5 {
                        let radius = rt
                            .rng
                            .range(rows[row].knockback[0] as f32, rows[row].knockback[1] as f32);
                        if let Some(volume) = m.volumes.iter().find(|volume| volume.key == key)
                            && !battle.brain.capture(
                                key,
                                volume.centre,
                                radius,
                                rows[row].spin as f32,
                            )
                        {
                            let error =
                                "this source brain's magic capture is unqualified".to_string();
                            battle.error = Some(error.clone());
                            actor.error = Some(error);
                        }
                    }
                }
                event(
                    log,
                    serde_json::json!({"event":"gallery_hit","tick":tick,"actor":actor.id,"atkp":v.atkp,"sub":v.sub,"damage":damage,"hp":battle.brain.hp(),"step":row,"parts":target.parts}),
                );
            }
        }
        if battle.paused || !battle.brain.alive() || battle.brain.serial() != battle.serial {
            continue;
        }
        let facing = Vec3::new(battle.brain.yaw().sin(), 0., battle.brain.yaw().cos());
        let toward = ((sora_at - at) * Vec3::new(1., 0., 1.)).normalize_or_zero();
        let mut incoming = None;
        // The motion's attack windows, then the attack objects the script placed.
        let mut candidates: Vec<Candidate> = battle
            .windows
            .iter()
            .enumerate()
            .filter(|(_, w)| w.active(battle.frame))
            .map(|(window, w)| Candidate {
                window,
                atkp: w.id,
                sub: 0,
                script_slot: None,
                direct: false,
                shapes: battle
                    .pose
                    .attacks
                    .get(&w.group)
                    .into_iter()
                    .flatten()
                    .map(|s| (s.shape, s.ordinal, s.centre, s.radius, None))
                    .collect(),
            })
            .collect();
        for a in battle.brain.script_attacks() {
            let (Ok(id), Ok(sub)) = (u16::try_from(a.id), u8::try_from(a.extra.max(0))) else {
                continue;
            };
            // The attack's team mask has to include Sora's team.
            if !a.hits_sora {
                continue;
            }
            candidates.push(Candidate {
                window: SCRIPT_WINDOW + a.serial as usize,
                atkp: id,
                sub,
                script_slot: Some(a.slot),
                direct: false,
                shapes: vec![(
                    3,
                    0,
                    Vec3::from_array(a.position),
                    a.radius,
                    a.previous.map(Vec3::from_array),
                )],
            });
        }
        // An attack a script applied to Sora directly (`attack_strike`) lands
        // on her first body sphere without a sweep.
        for (slot, id, extra) in battle.brain.take_script_strikes() {
            let (Ok(id), Ok(sub), Some(&(body, ..))) = (
                u16::try_from(id),
                u8::try_from(extra.max(0)),
                sora_body.first(),
            ) else {
                continue;
            };
            candidates.push(Candidate {
                window: SCRIPT_WINDOW + 0x8000 + slot,
                atkp: id,
                sub,
                script_slot: Some(slot),
                direct: true,
                shapes: vec![(3, 0, body, 0., None)],
            });
        }
        for cand in &candidates {
            let (window, w_id) = (cand.window, cand.atkp);
            if battle.brain.serial() != battle.serial || battle.error.is_some() {
                break;
            }
            let rows = match battle.assets.attack_steps(cand.atkp, cand.sub) {
                Ok(rows) => rows,
                Err(_) if cand.script_slot.is_some() => continue,
                Err(error) => {
                    battle.error = Some(error.clone());
                    actor.error = Some(error);
                    break;
                }
            };
            let ids = *battle
                .contacts
                .incoming_ids
                .entry((battle.serial, window))
                .or_insert_with(|| allocate(&mut rt.ids, &mut rt.next_group, rows[0].combo_group));
            if rt.sora_record.has(ids.0) {
                continue;
            }
            let mut blocked = false;
            for &(shape_kind, ordinal, centre, radius, forced_prior) in &cand.shapes {
                if blocked {
                    break;
                }
                if shape_kind != 3 {
                    let error = "native non-sphere melee attack sweep is unqualified".to_string();
                    battle.error = Some(error.clone());
                    actor.error = Some(error);
                    continue;
                }
                let prior = battle
                    .contacts
                    .previous
                    .insert((battle.serial, window, ordinal), centre);
                let prior = forced_prior.or(prior);
                for c in combat::swept(centre.to_array(), prior.map(|p| p.to_array()), radius) {
                    let centre = Vec3::from_array(c);
                    let mut reflected = false;
                    for guard in volumes.iter().filter(|v| v.guard) {
                        let guard_row = &rt.steps[&(guard.atkp, guard.sub)][0];
                        if !clashes(guard_row, &rows[0], sora_facing, facing, toward) {
                            continue;
                        }
                        for &(guard_at, guard_radius, _, _) in &guard.centres {
                            if !combat::spheres_overlap(
                                c,
                                radius,
                                guard_at.to_array(),
                                guard_radius,
                            ) {
                                continue;
                            }
                            rt.sora_record.mark(ids.0);
                            player.connected = true;
                            reflected = true;
                            if let Some(slot) = cand.script_slot {
                                battle.brain.mark_script_attack_reflected(slot);
                            }
                            if let Some(key) = guard.magic
                                && let Some(m) = magic.as_mut()
                            {
                                *m.reflected.entry(key).or_insert(0) += 1;
                            }
                            let contact =
                                guard_at + (centre - guard_at).normalize_or_zero() * guard_radius;
                            if guard_row.refact_self == 2 || rows[0].refact_other == 2 {
                                effect_queue.push(EffectRequest::System {
                                    id: 13,
                                    at: contact,
                                });
                            } else if !player.action.airborne()
                                && (guard_row.refact_self != 0 || rows[0].refact_other != 0)
                            {
                                effect_queue.push(EffectRequest::System {
                                    id: 11,
                                    at: contact,
                                });
                                if player.guard_recoil([toward.x, toward.z], clips).is_some() {
                                    *action_started = tick;
                                }
                                rt.sora_invulnerable = 80.;
                            } else {
                                effect_queue.push(EffectRequest::System {
                                    id: 13,
                                    at: contact,
                                });
                            }
                            if rows[0].refact_self != 2
                                && rows[0].reflect_motion != 0
                                && !battle.brain.reflect(
                                    rows[0].reflect_motion as usize,
                                    rows[0].reflect_hit_back as f32,
                                )
                            {
                                let error = "this source brain's reflect reaction is unqualified"
                                    .to_string();
                                battle.error = Some(error.clone());
                                actor.error = Some(error);
                            }
                            event(
                                log,
                                serde_json::json!({"event":"gallery_guard","tick":tick,"actor":actor.id,"atkp":w_id,"guard_atkp":guard.atkp}),
                            );
                            break;
                        }
                        if reflected {
                            break;
                        }
                    }
                    if reflected {
                        blocked = true;
                        break;
                    }
                    if incoming.is_none()
                        && *hero_hit_tick != Some(tick)
                        && let Some(&(body, body_radius, high)) =
                            sora_body.iter().find(|(body, body_r, _)| {
                                combat::spheres_overlap(c, radius, body.to_array(), *body_r)
                            })
                    {
                        incoming = Some((
                            ids,
                            rows.clone(),
                            high,
                            w_id,
                            (centre, body, body_radius),
                            cand.script_slot.filter(|_| !cand.direct),
                        ));
                    }
                }
            }
        }
        if let Some((
            (aid, group),
            rows,
            high,
            atkp,
            (claw_at, body_at, body_radius),
            script_slot,
        )) = incoming
        {
            *hero_hit_tick = Some(tick);
            if let Some(slot) = script_slot {
                battle.brain.mark_script_attack(slot);
            }
            if rt.sora_invulnerable > 0. || player.range_invulnerable(clips) || cheats.invincible {
                rt.sora_record.hit(aid, group, false, |_| (0, 0));
                event(
                    log,
                    serde_json::json!({"event":"gallery_sora_hit_invulnerable","tick":tick,"actor":actor.id,"atkp":atkp}),
                );
                continue;
            }
            if rt.sora_hp <= 0 {
                continue;
            }
            let (counted, row) =
                rt.sora_record
                    .hit(aid, group, matches!(rows[0].kind, 5 | 6), |count| {
                        let index = battle_tables::step_index(&rows, count);
                        (index, rows[index].interval)
                    });
            let attack = params_of(&rows[row]);
            let ab = &player.abilities;
            let state = combat::TargetState {
                hp: rt.sora_hp,
                player_type: true,
                defender: ab.defender as i32,
                damage_control: ab.damage_control,
                damage_control_ratio: 50,
                difficulty: rt.difficulty,
            };
            let damage = if counted {
                combat::damage_with(&battle.assets.attacker, &attack, &rt.sora_target, &state)
            } else {
                0
            };
            let veto = combat::sora_veto(ab.has(127), ab.has(128), rt.sora_hp, 0, damage_motion);
            let hp_before = rt.sora_hp;
            rt.sora_hp = combat::apply_damage(
                rt.sora_hp,
                0,
                rt.sora_target.max_hp,
                damage,
                veto && counted,
            );
            if rt.sora_hp < hp_before {
                *enemy_incoming_hits += 1;
            }
            let direction = ((sora_at - at) * Vec3::new(1., 0., 1.)).normalize_or(Vec3::Z);
            let hit = player::Hit {
                reaction: attack.reaction,
                original: attack.reaction,
                direction: [direction.x, direction.z],
                high,
                knockback: attack.knockback,
            };
            if counted && player.react(&hit, &battle.assets.sstm, clips).is_some() {
                *action_started = tick;
            }
            player
                .drive
                .damaged(damage, rt.sora_target.max_hp, player.abilities.damage_drive);
            if damage > 0 && !player.mp.charging() && player.abilities.mp_rage > 0. {
                let gain = (damage as f32 / rt.sora_target.max_hp as f32 * player.abilities.mp_rage)
                    as i32;
                if gain > 0 {
                    player.mp.add(gain);
                }
            }
            if counted && rows[row].hit_sound >= 0 {
                sound_queue.push((rows[row].hit_sound as u32, "sora_hit"));
            }
            // The attacker's spark on Sora's hit shape (FUN_1403d1b40; a
            // sphere, so height = radius), from the attacker's own effects.
            if counted && rows[row].hit_effect != 0 {
                let v = claw_at - body_at;
                let y = v.y.clamp(-body_radius, body_radius);
                let across = Vec3::new(v.x, 0., v.z).normalize_or_zero()
                    * body_radius
                    * (1. - (y / body_radius).powi(2)).max(0.).sqrt();
                effect_queue.push(EffectRequest::EnemyHit {
                    object: battle.assets.object_id,
                    effect: rows[row].hit_effect,
                    random: rows[row].hit_effect_random,
                    contact: body_at + across + Vec3::Y * y,
                    direction,
                });
            }
            if rt.sora_hp == 0 && player.die(clips).is_some() {
                *action_started = tick;
            }
            event(
                log,
                serde_json::json!({"event":"gallery_sora_hit","tick":tick,"actor":actor.id,"atkp":atkp,"damage":damage,"counted":counted,"hp":rt.sora_hp,"step":row}),
            );
        }
    }
}

/// Destroy expired owned attacks at the end of this field update. Native
/// destruction clears their seen bit on every actor before freeing the ID.
pub fn retire(game: &mut Game, actors: &mut [GalleryActor], tick: u64) {
    let Some(rt) = game.enemy.as_mut() else {
        return;
    };
    let mut expired = Vec::new();
    rt.instance_ids.retain(|key, (id, _)| {
        let keep = rt.instance_seen.get(key) == Some(&tick);
        if !keep {
            expired.push(*id)
        }
        keep
    });
    rt.instance_seen.retain(|_, seen| *seen == tick);
    let live_instances = &rt.instance_ids;
    rt.previous
        .retain(|(key, _), _| live_instances.contains_key(key));
    rt.claw_previous
        .retain(|(key, _), _| live_instances.contains_key(key));
    let mut live = rt
        .instance_ids
        .values()
        .map(|(id, _)| *id)
        .collect::<Vec<_>>();
    for actor in actors.iter_mut() {
        let Some(battle) = actor.battle.as_mut() else {
            continue;
        };
        let serial = battle.brain.serial();
        let valid = battle.brain.alive()
            && serial == battle.serial
            && battle.error.is_none()
            && actor.error.is_none();
        battle
            .contacts
            .incoming_ids
            .retain(|(started, window), (id, _)| {
                let keep = valid
                    && *started == serial
                    && battle
                        .windows
                        .get(*window)
                        .is_some_and(|w| w.active(battle.frame));
                if !keep {
                    expired.push(*id)
                }
                keep
            });
        live.extend(battle.contacts.incoming_ids.values().map(|(id, _)| *id));
        let ids = &battle.contacts.incoming_ids;
        battle
            .contacts
            .previous
            .retain(|(serial, window, _), _| ids.contains_key(&(*serial, *window)));
    }
    // This also retires IDs owned by an actor removed through the debug menu.
    expired.extend(rt.ids.live_ids().filter(|id| !live.contains(id)));
    expired.sort_unstable();
    expired.dedup();
    for id in expired {
        rt.enemy.record.clear_seen(id);
        rt.sora_record.clear_seen(id);
        for actor in actors.iter_mut() {
            if let Some(battle) = actor.battle.as_mut() {
                battle.brain.record_mut().clear_seen(id)
            }
        }
        rt.ids.release(id);
    }
}

fn clashes(
    guard: &battle_tables::Attack,
    attack: &battle_tables::Attack,
    sora_facing: Vec3,
    enemy_facing: Vec3,
    toward: Vec3,
) -> bool {
    let pair = (guard.kind.min(attack.kind), guard.kind.max(attack.kind));
    let cone = |row: &battle_tables::Attack, facing: Vec3, delta: Vec3| {
        row.reflect_range == 0
            || delta
                .dot(Quat::from_rotation_y((row.reflect_angle as f32 * 2.).to_radians()) * facing)
                < -(row.reflect_range as f32).to_radians().cos()
    };
    matches!(pair, (0, 0) | (0, 2) | (0, 3) | (1, 3))
        && cone(guard, sora_facing, toward)
        && cone(attack, enemy_facing, -toward)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ellipse_body_uses_its_vertical_extent_and_strict_boundary() {
        let target = WorldShape {
            ordinal: 0,
            group: 0,
            centre: Vec3::ZERO,
            radius: 20.,
            height: 60.,
            shape: 0,
            parts: 0,
            high: true,
        };
        assert!(overlap(Vec3::new(0., 55., 0.), 5., 5., true, &target));
        assert!(!overlap(Vec3::new(0., 65., 0.), 5., 5., true, &target));
        assert!(!overlap(Vec3::new(25., 0., 0.), 5., 5., true, &target));
    }
    #[test]
    fn defeat_rewards_once_and_respawn_requires_new_contact_state() {
        let mut state = ContactState::default();
        assert!(!state.defeat_reward(1));
        assert!(state.defeat_reward(0));
        assert!(!state.defeat_reward(0));
        assert!(!state.defeat_reward(-1));
        state = ContactState::default();
        assert!(state.defeat_reward(0));
    }
    #[test]
    fn collision_clash_pairs_do_not_turn_every_contact_into_guard() {
        let row = |kind: u8| {
            let mut bytes = vec![0; 56];
            bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
            bytes[10..12].copy_from_slice(&1u16.to_le_bytes());
            bytes[12] = kind;
            battle_tables::attack(&bytes, 1).unwrap()
        };
        assert!(clashes(&row(0), &row(0), Vec3::Z, Vec3::NEG_Z, Vec3::NEG_Z));
        assert!(clashes(&row(3), &row(1), Vec3::Z, Vec3::NEG_Z, Vec3::NEG_Z));
        assert!(!clashes(
            &row(2),
            &row(1),
            Vec3::Z,
            Vec3::NEG_Z,
            Vec3::NEG_Z
        ));
        let mut cone = row(2);
        cone.reflect_range = 90;
        assert!(clashes(&cone, &row(0), Vec3::Z, Vec3::NEG_Z, Vec3::NEG_Z));
        assert!(!clashes(&cone, &row(0), Vec3::Z, Vec3::NEG_Z, Vec3::Z));
    }
}
