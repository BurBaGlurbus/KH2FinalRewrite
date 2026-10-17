//! `--party-hit-tour <new-capture-dir>`: an unattended run of Donald and
//! Goofy being hit. Sora is invincible and the Shadow stands where the run
//! puts it. Synthetic claw spheres of the Shadow's own ATKP rows land on a
//! member's HIT sphere (or Goofy's shield) when the case says, so each one
//! is the same on every run:
//!
//! - a hit on Donald before the release of his Fire, and one in the delay
//!   before Thunder's strike (the cast ends with nothing spent);
//! - a blow and a flinch on Goofy;
//! - Donald at 1 HP struck: KO, the DEAD motion, the revive timer running
//!   at SSTM+0x5c outside battle, the get-up;
//! - Goofy KO'd and revived by Donald's Cure;
//! - a claw on Goofy's shield: the guard bounce and the Shadow's reflect;
//! - a hit on Donald while his Fire ring is up (the act pauses, the ring
//!   stays);
//! - then the Shadow's own AI fights the party for the rest of the run.
//!
//! Each case waits for its member to be free, so the order of the cases is
//! all that is fixed. The run's log carries `party_hit_tour` (each case's
//! steps), `party_cast`, `party_release`, `party_hit_taken`, `party_guard`,
//! `party_ko`, `party_revive` and `party_heal` events.
use super::*;
use player::party_members::Spell;

const DONALD: usize = 0;
const GOOFY: usize = 1;
/// ATKP 9, the Shadow's claw (reaction 3), and ATKP 1164, Donald's Blizzard
/// shot (reaction 4: a blow with KB1 20, KB2 2).
const CLAW: u16 = 9;
const BLOW: u16 = 1164;
/// The natural fight at the end lasts this long.
const FIGHT: u64 = 900;
/// A case that has not finished by then is given up.
const PATIENCE: u64 = 2000;

#[derive(Clone, Copy)]
enum Case {
    /// The Shadow on or off; on, it stands 110 ahead of Sora.
    Enemy(bool),
    Wait(u64),
    /// Orders `spell` once the member is free and lands a claw `delay`
    /// ticks after the cast starts.
    CastHit {
        member: usize,
        spell: Spell,
        delay: u64,
    },
    /// A claw of this ATKP on a free member.
    Hit {
        member: usize,
        atkp: u16,
    },
    /// The member at 1 HP, then struck: KO, and the case ends when it is
    /// up again.
    KoHit {
        member: usize,
    },
    /// A claw on the member's shield.
    Shield {
        member: usize,
    },
    /// Donald's Blizzard flies into a Type 3 guard sphere (an enemy's super
    /// guard) 300 ahead of him: the shot ends.
    Reflect {
        member: usize,
    },
    /// The Shadow fights on its own, the party's AI casts.
    Fight,
}

const CASES: &[Case] = &[
    Case::Enemy(false),
    Case::CastHit {
        member: DONALD,
        spell: Spell::Fire,
        delay: 12,
    },
    Case::Wait(60),
    Case::CastHit {
        member: DONALD,
        spell: Spell::Thunder,
        delay: 30,
    },
    Case::Wait(60),
    Case::Hit {
        member: GOOFY,
        atkp: BLOW,
    },
    Case::Wait(60),
    Case::Hit {
        member: GOOFY,
        atkp: CLAW,
    },
    Case::Wait(60),
    Case::KoHit { member: DONALD },
    Case::Wait(60),
    Case::KoHit { member: GOOFY },
    Case::Wait(60),
    Case::Shield { member: GOOFY },
    Case::Wait(60),
    Case::Reflect { member: DONALD },
    Case::Wait(60),
    Case::Enemy(true),
    Case::CastHit {
        member: DONALD,
        spell: Spell::Fire,
        delay: 36,
    },
    Case::Wait(120),
    Case::Fight,
];

/// The tour's progress.
#[derive(Default)]
pub struct State {
    case: usize,
    stage: u8,
    since: u64,
    mark: u64,
    cure_done: bool,
}

/// The tick the run ends at (the schedule decides; this bounds it).
pub const END: u64 = 12000;

/// Stages a member 220 ahead of Sora and to one side, facing him, where
/// the camera sees it whichever way it looks.
fn stage(game: &mut Game, member: usize, enemy: Option<bool>) {
    let sora = Vec3::from_array(game.player.position);
    let (s, c) = game.player.yaw.sin_cos();
    let side = if member == DONALD { -1. } else { 1. };
    let at = sora + Vec3::new(s, 0., c) * 220. + Vec3::new(c, 0., -s) * 110. * side;
    let yaw = (sora.x - at.x).atan2(sora.z - at.z);
    party_room::place_member(game, member, at, yaw);
    // The attacker stands 300 ahead of the member (inside Goofy's shield
    // cone) or 300 behind it (outside).
    if let Some(ahead) = enemy
        && let Some(rt) = game.enemy.as_mut()
    {
        let f = Vec3::new(yaw.sin(), 0., yaw.cos());
        let from = at + f * if ahead { 300. } else { -300. };
        rt.enemy.position = from.to_array();
        rt.enemy.spawn = from.to_array();
    }
}

fn free(game: &Game, member: usize) -> bool {
    game.party.get(member).is_some_and(|m| {
        m.follower.attack.is_none()
            && m.follower.reaction.is_none()
            && !m.down()
            && m.cast.is_none()
            && m.forced.is_none()
            && !m.stored
    })
}

fn capture(game: &mut Game, tick: u64, after: &[u64]) {
    for a in after {
        let at = tick + a;
        if game.captures.last().is_none_or(|l| *l < at) {
            game.captures.push(at);
        }
    }
}

fn log(game: &mut Game, tick: u64, case: usize, what: &str, extra: serde_json::Value) {
    let mut e = serde_json::json!({"event":"party_hit_tour","tick":tick,"case":case,"step":what});
    if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
        o.extend(x.clone());
    }
    game.event(e);
}

/// Runs the current case; called before the party's tick.
pub fn drive(game: &mut Game, tick: u64) {
    if tick == 1 {
        for m in &mut game.party {
            m.hp = m.max_hp;
        }
    }
    if tick.is_multiple_of(300) {
        let members: Vec<_> = game
            .party
            .iter()
            .map(|m| {
                serde_json::json!({"name":m.kind.name,"hp":m.hp,"attack":m.follower.attack.is_some(),
                "reaction":m.follower.reaction.map(|r| r.motion),"down":m.follower.down,
                "cast":m.cast.is_some(),"forced":m.forced.is_some(),"motion":m.follower.motion()})
            })
            .collect();
        let at = (game.party_hit_state.case, game.party_hit_state.stage);
        game.event(
            serde_json::json!({"event":"party_hit_tour_progress","tick":tick,
            "case":at.0,"stage":at.1,"members":members}),
        );
    }
    let mut state = std::mem::take(&mut game.party_hit_state);
    let Some(&case) = CASES.get(state.case) else {
        game.party_hit_state = state;
        return;
    };
    let index = state.case;
    let sora = Vec3::from_array(game.player.position);
    let forward = Vec3::new(game.player.yaw.sin(), 0., game.player.yaw.cos());
    let mut done = false;
    match case {
        Case::Enemy(on) => {
            if let Some(rt) = game.enemy.as_mut() {
                rt.enabled = on;
                let at = sora + forward * 110.;
                rt.enemy.position = at.to_array();
                rt.enemy.spawn = at.to_array();
            }
            log(game, tick, index, "enemy", serde_json::json!({"on":on}));
            done = true;
        }
        Case::Wait(n) => {
            if state.stage == 0 {
                state.since = tick;
                state.stage = 1;
            } else {
                done = tick >= state.since + n;
            }
        }
        Case::CastHit {
            member,
            spell,
            delay,
        } => match state.stage {
            0 if free(game, member) => {
                stage(game, member, None);
                let ordered = party_room::order_cast(game, member, spell, 0);
                log(
                    game,
                    tick,
                    index,
                    "cast",
                    serde_json::json!({"member":member,"spell":format!("{spell:?}"),"ordered":ordered}),
                );
                state.stage = 1;
                state.mark = tick;
            }
            1 => {
                let started = game
                    .party
                    .get(member)
                    .filter(|m| m.cast.is_some())
                    .map(|m| m.started);
                if let Some(started) = started
                    && tick == started + delay
                {
                    let placed = party_room::probe_claw(game, member, CLAW, false);
                    log(
                        game,
                        tick,
                        index,
                        "hit",
                        serde_json::json!({"member":member,"atkp":CLAW,"placed":placed,
                        "cast_age":delay}),
                    );
                    capture(game, tick, &[2, 10, 25, 50, 90]);
                    state.stage = 2;
                    state.mark = tick;
                } else if started.is_none() && tick > state.mark + 400 {
                    done = true;
                }
            }
            2 => done = free(game, member) && tick > state.mark + 20,
            _ => {}
        },
        Case::Hit { member, atkp } => match state.stage {
            0 if free(game, member) => {
                stage(game, member, Some(false));
                let placed = party_room::probe_claw(game, member, atkp, false);
                log(
                    game,
                    tick,
                    index,
                    "hit",
                    serde_json::json!({"member":member,"atkp":atkp,"placed":placed}),
                );
                capture(game, tick, &[2, 10, 25, 50, 80]);
                state.stage = 1;
                state.mark = tick;
            }
            1 => done = free(game, member) && tick > state.mark + 20,
            _ => {}
        },
        Case::KoHit { member } => match state.stage {
            0 if free(game, member) => {
                stage(game, member, Some(false));
                party_room::set_hp(game, member, 1);
                let placed = party_room::probe_claw(game, member, CLAW, false);
                log(
                    game,
                    tick,
                    index,
                    "ko_hit",
                    serde_json::json!({"member":member,"placed":placed}),
                );
                capture(game, tick, &[2, 30, 120, 300]);
                state.stage = 1;
                state.mark = tick;
                state.cure_done = false;
            }
            1 => {
                let down = game.party.get(member).is_some_and(|m| m.down());
                // Goofy is raised by Donald's Cure, Donald by his timer.
                if member == GOOFY && down && !state.cure_done && tick > state.mark + 150 {
                    let ordered = party_room::order_cast(game, DONALD, Spell::Cure, 2);
                    log(
                        game,
                        tick,
                        index,
                        "cure",
                        serde_json::json!({"healer":DONALD,"slot":2,"ordered":ordered}),
                    );
                    capture(game, tick, &[15, 30, 45, 70]);
                    state.cure_done = true;
                }
                if !down && tick > state.mark + 2 {
                    capture(game, tick, &[3, 20, 70]);
                    state.stage = 2;
                    state.mark = tick;
                } else if tick > state.mark + PATIENCE {
                    done = true;
                }
            }
            2 => done = free(game, member) && tick > state.mark + 20,
            _ => {}
        },
        Case::Shield { member } => match state.stage {
            0 if free(game, member) => {
                stage(game, member, Some(true));
                let placed = party_room::probe_claw(game, member, CLAW, true);
                log(
                    game,
                    tick,
                    index,
                    "shield",
                    serde_json::json!({"member":member,"atkp":CLAW,"placed":placed}),
                );
                capture(game, tick, &[2, 12, 30, 60]);
                state.stage = 1;
                state.mark = tick;
            }
            1 => done = free(game, member) && tick > state.mark + 20,
            _ => {}
        },
        Case::Reflect { member } => match state.stage {
            0 if free(game, member) => {
                stage(game, member, None);
                let ordered = party_room::order_cast(game, member, Spell::Blizzard, 0);
                log(
                    game,
                    tick,
                    index,
                    "reflect_cast",
                    serde_json::json!({"member":member,"ordered":ordered}),
                );
                capture(game, tick, &[40, 52, 60]);
                state.stage = 1;
                state.mark = tick;
            }
            1 => {
                // The guard sphere stands in the shot's path every tick.
                if let Some(m) = game.party.get(member) {
                    let (s, c) = m.follower.yaw.sin_cos();
                    let p = m.follower.position;
                    let at = Vec3::new(p[0] + s * 300., p[1] + 90., p[2] + c * 300.);
                    game.enemy_guards.push((at, 80.));
                }
                if (tick > state.mark + 8 && free(game, member)) || tick > state.mark + PATIENCE {
                    done = true;
                }
            }
            _ => {}
        },
        Case::Fight => {
            if state.stage == 0 {
                game.cheats.enemy_frozen = false;
                game.party_spells = true;
                log(game, tick, index, "fight", serde_json::json!({}));
                state.stage = 1;
                state.mark = tick;
                capture(game, tick, &[60, 200, 400, 600, 850]);
            }
            if tick >= state.mark + FIGHT {
                done = true;
            }
        }
    }
    if done {
        state.case += 1;
        state.stage = 0;
        if state.case >= CASES.len() {
            game.autoplay_end = tick + 2;
        }
    }
    game.party_hit_state = state;
}
