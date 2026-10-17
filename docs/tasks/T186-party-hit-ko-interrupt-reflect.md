# T186 — Donald and Goofy being hit: reactions, KO and revive, interrupted casts, the Blizzard shot's reflect

Owner Claude, 2026-10-17, requested by burba in the project chat (GitHub issue
#76, which holds the old #11). Gate: development of the test room only; no
final gameplay acceptance is inferred. Prerequisites: T108 (Drive forms keep
`MemberRt.stored`), T136 (Donald's and Goofy's acts, `party-01` specs), T140
(Sora's death), T163. Build identity: `origin/main` at `554d3bf`, package 0.1.2,
Rust 1.99.0 Windows MSVC. Source: the selected Steam KH2FM build, validated at
each run; the install was only read.

Private evidence root: a private folder outside the checkout (Ghidra runner, the
decompiles under `out/`, tour runs `run*/`), and the earlier `party-01` specs
(`donald-spec.md` §8, `goofy-spec.md` §8) and `combo-01/g`, `damage-02/g`,
`guard-01` under `%LOCALAPPDATA%/KH2FinalRewrite/research/playable-room-20261008/`.
Nothing from the game is in Git: the tour reads the motions, models and tables
from the installation at run time.

## What the executable does

- A hit on a member is the shared BTLOBJ pipeline reached through
  FRIEND vt+0xc0 (`FUN_1401b0290`: the personality's hit callback, which is an
  empty function for Donald and Goofy, then `FUN_1403c34d0` → `FUN_1403d60c0`).
  A dead member (`obj+0x9b8` bit 2) or one that cannot be damaged
  (`FUN_1403d65a0`) only takes the cure types (ATKP Type 5/6). Otherwise the
  record check, the HP change `FUN_1403d3ba0` (clamped by `FUN_1403c0860`), the
  KO handler and the reaction `FUN_1403d2fe0` follow.
- Reaction classes (`FUN_1403d2fe0`, friend case): 1 a knock-slide only; 2 a
  flinch without a slide; 3 a flinch with a slide on the ground (an air flinch
  and hop in the air); 4 a blow (physics mode 4). Motions: flinch 10-13 (front
  high, front low, back high, back low, `DAT_14074c390`; the partner when one
  is missing), air 14/15, blow 16 (front) and 18 (back) landing on blow + 1
  (`FUN_140422700`). The member can act again at the motion's range 2.
- HP 0 (`FUN_1403d2eb0` → vt+0xb0 `FUN_1403c33d0`): the dead bit and the revive
  timer `obj+0xdc0` = PRTY word 6 (Donald 3600, Goofy 2700). The DEAD motion 54
  starts only when the member can act and is on the ground
  (`FUN_1403c3660`, `FUN_1403bfb30`, `FUN_1403ba9a0`) with the 00effect marker
  element 14. The timer falls by SSTM+0x5c (8.0) a tick, by 1.0 while the
  battle flag `DAT_142a11404` is set. At 0, `FUN_1403c3bd0` heals
  `maxHP × SSTM+0x58 (0.5)` through `FUN_1403d2eb0`, which for a dead party
  member calls `FUN_1403c3c30`: the dead bit clears, motion 55 plays from 54,
  and the heal is scaled by SSTM+0x54 (0.5) unless the Hyper Healing flag
  (`FUN_1403d7470(obj)+0x10` bit 3) is set. A Cure or any heal revives the same
  way. (Disassembly of `FUN_1403d2eb0` confirms `FUN_1403c3c30` receives the
  heal amount in `edx`.)
- The battle flag (`FUN_1403ab4a0`) is on while an enemy (team 2) lies within
  1200 of Sora or a party member's attack object lives (`FUN_1403c5bd0`), and
  follows that state after 45 ticks to switch on and 120 to switch off.
- A cast is interrupted by the member's motion changing. Donald's acts wait on
  `sysobj_motion_id == m`: the wait for the release event (`L2202`) and the
  delays after it (`L2284`: four ticks before Fire's ring, ten before Thunder's
  strike) end when the flinch replaces the cast motion, and the act then skips
  its effects, MP and attack objects and ends. The act manager also pauses
  every act below priority 300 while the owner plays a damage motion
  (`FUN_1403ca7c0`: motions 10-39 and 0x8c-0x96 for a STDOBJ owner, bit 1 of
  `obj+0x6c8`, which the STDOBJ constructor sets): its timers stand still, its
  attack objects keep their last position, and it resumes afterwards. Blizzard's
  shot is a thread of its own (`magic_start_thread`) and is not paused.
- The shot's end and the reflect: the shot thread leaves its loop when
  `attack_is_hit_bg || attack_is_reflect || (pierce == 0 && attack_is_hit)`;
  its cleanup (`L4712`) kills the effect and frees the attack object.
  `attack_is_reflect` is the attack object's flag bit 9, set by
  `FUN_1403d2140` when it clashes with another team's attack whose Type pairs
  in `DAT_1405c5e98` (0-0, 0-2, 0-3, 1-3). Donald's shots are ATKP Type 1, so
  only a Type 3 super guard reflects them. The branch after
  `attack_is_reflect` that builds a new shot from the reflected one (`L3287`)
  tests `-1 >= 0` and never runs in the shipped script: a reflected shot only
  ends. ATTACK_MAGIC objects carry flag 0x100 (`FUN_1404371b0`), so the
  caster has no clash reaction either.
- Goofy's shield: his idle, walk and jump motions carry a weapon window with
  ATKP 139 (Type 2 guard, refact fields 0). A Type 0 claw touching its sphere
  clashes (0-2): the claw joins Goofy's record, the Shadow plays its
  ReflectMotion and `mode_revenge` (`FUN_140411060`), and Goofy takes the bounce
  branch of `FUN_1403d61d0`: effect 13, motion 41 and a knock-slide of 50
  backwards along his own facing (`FUN_1403d4650` with motion -1).
- Who the Shadow fights: `OBJ_DAMAGE` starts `mode_revenge` for a hit by the
  player character (`0x6c8` bit 7) and `mode_battle` for any other attacker;
  `mode_revenge` searches `target_search(obj, 1)`, the last team-1 attacker
  (`FUN_140411300` stores it in `obj+0xd40`) when it is alive, and `mode_battle`
  searches Sora again.

## Rewrite

- `player::party_hit` (new): the reaction classes, the motion tables, the
  per-tick reaction (flinch slide, air flinch, blow flight and landing, bounce,
  get-up), the SSTM KO rates, `suspends_acts`. `player::party_members::Follower`
  gains `react`, `bounce`, `ko`, `revive`, `revive_due` and the locked/down
  motion states; `Scene` carries the actionable ticks, SSTM, the rates and the
  battle flag.
- `player::party_spell::Cast` ends at a changed motion in the waits, pauses (and
  holds its attack objects) while the owner is in a damage motion, and ends the
  shot when `Context::reflected`.
- `player::shadow`: `hit_by` (a hit by a party member starts `mode_battle`),
  `attacker_pos` and the revenge target.
- `test_room::party_room`: the HIT spheres per motion tick, the reaction
  motions, PRTY word 6 and the SSTM rates at load; `receive` (shared by the
  Shadow and the gallery enemies) lands attack spheres on the members, runs the
  shield clash, the damage, the KO and the reaction; the KO marker; `heal_member`
  (a dead member is revived); the battle flag; the revenge target bookkeeping.
  `enemy_battle_hits` feeds the gallery enemies' attack spheres and their
  Type 3 guards.
- `--party-hit-tour <dir>`: synthetic claw spheres of the Shadow's own ATKP
  rows land on a member or its shield when each case says; the final nine
  hundred ticks are the Shadow's own fight.

## Second pass (same task)

- Voices: the members' hit voices come from the motions' frame trigger 14 (flinches, attacks, the
  get-up), the low-HP voice 9 (`FUN_1403c34d0`), the KO voice 2 (`FUN_1403d5dc0`) and the casts'
  first call, trap (1,262): 6, and 21 or 3 for Cure. They use the same vtbl rows and
  `FUN_1403d69b0` path as Sora's, with `voice/us/battle/<world>0_donald|goofy` (the exe's
  `%s%d_%s` and its name table sora, donald, goofy). Goofy's script has no voice trap.
- The air flinch lands on motion 5 (the mode-1 land motion, `obj+0x11c`, `FUN_1403cbec0`).
- Reflect in the room: `Game::enemy_guards` carries the Type 3 attack spheres of gallery
  enemies; a Blizzard shot clashing with one ends (`party_reflect`), and the tour's reflect case puts
  such a sphere in the shot's path.
- Gallery enemies: a member aims at the nearest of the Shadow and the gallery enemies within 800
  and its melee windows and spell volumes hit them through the bridge Sora's attacks use (the
  member's own stats; Sora's combo and Drive are not fed). `--enemy-combat-tour` keeps the party out
  (it is Sora's exercise) and still passes.

## Checks

- PASS `cargo fmt --all --check`, PASS `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  PASS `cargo test --workspace --locked`, PASS `cargo build --workspace --locked`, PASS the
  `--frames 180` smoke run. New tests: 10 in `player::party_hit` (flinch front/back/high/low, partner
  fallback, classes 1-3, blow flight and landing on blow + 1, the actionable tick, the bounce, KO rates,
  revive amounts, damage-motion suspension), 4 in `party_members` (a hit locks the member until the
  reaction lets it act, KO then DEAD then timer then get-up, timer 1 a tick in battle, bounce), 6 in
  `party_spell` (a hit before the release or in the delays ends the act with nothing spent or started, a hit
  after the ring's start pauses it with its attack in place, the shot flies on while the act is paused,
  a reflected shot ends with no second attack object), 2 in `shadow` (party hit starts `mode_battle`;
  revenge targets the last attacker), 1 in `party_room` (battle flag 45/120).
- PASS release `--party-hit-tour` against the installed build (`run6`, 45 captures, log
  `party_hit_*` events): a claw on Donald 12 ticks into Fire interrupts it (`interrupted_cast`, no
  `party_release`, flinch 10); a hit 30 ticks into Thunder (release at 26, strike at 36) also ends it
  with nothing spent; a Blizzard-shot blow on Goofy plays motion 18 (back blow) and a claw plays 12;
  Donald at 1 HP struck: `party_ko` with timer 3600, `party_revive` by the timer 450 ticks later (8 a
  tick outside battle) with 4 HP (a quarter of 18); Goofy KO'd (timer 2700) and raised by Donald's Cure
  (`party_heal` `was_down`, 10 halved to 5 HP); a claw on Goofy's shield from the front gave
  `party_guard` with motion 41 (the same claws from behind, outside the 45 degree cone, hit his body); a
  hit on Donald 36 ticks into a Fire cast (ring up) interrupted the act; the Shadow's own fight at the
  end hit Donald by its claws (`party_hit_taken` at tick 2077 in run 4 and again in run 6).
- Compared with the executable (static): `FUN_1401b0290`, `FUN_1403d60c0`, `FUN_1403d3ba0`,
  `FUN_1403d2eb0` (with disassembly), `FUN_1403d2fe0`, `FUN_1403d43b0`, `FUN_1403d3840`,
  `FUN_1403d4650`, `FUN_1403d61d0`, `FUN_1403c3660`, `FUN_1403c3bd0`, `FUN_1403c3c30`,
  `FUN_1403ca550/7c0/250`, `FUN_1403d2140`, `FUN_1403d1f40`, `FUN_1403ab4a0`, `FUN_1404371b0`,
  the Friend vtable slots, SSTM +0x54/+0x58/+0x5c (0.5, 0.5, 8.0), PRTY word 6, and Donald's BDX acts
  (`fire` L2003, `blizzard` L2571 and its shot L3166-L4712, `thunder` L5052, `cure` L5453).
- PASS release `--party-hit-tour` (`run7`): 31 `party_voice` / `party_hit_taken` sound events with
  Donald's and Goofy's banks (for example 9 then 2 at the KO tick), and the reflect case: Donald's
  Blizzard cast at tick 1783 ended with `party_reflect` at 1808 when the shot met the guard sphere.
- PASS release `--enemy-combat-tour` (`run10`, verdict pass with positive damage both ways), and a
  run with the party in (`run8`): Donald's Fire hit a gallery actor (`gallery_hit` with `member` 0) and
  gallery actors hit Donald and Goofy (`party_hit_taken` from ATKP 405 and 7, a blow with motion 18).
- NOT RUN: owner playtest; comparison with the installed game's frames or timings.

## Not reproduced / uncertain

- A voice's priority (row byte 2) is not used, as for Sora's: two voices on one tick both play.
- No HUD gauge for Donald and Goofy: their HP is in the log and the Cure menu's
  colour only. The original's `dk::GAUGE_FRIEND` (face, ring masks, state groups 0x40-0x4a,
  `FUN_140182730` per frame) is a separate port.
- The `patn` override (`FUN_14040a8e0`, `FUN_1403ee9b0`) is taken as absent for
  the party; reaction class 5 (Magnet's capture, `FUN_14040fbf0`) and 14 are not
  built for members.
- Air hits on a member (Goofy's Tornado and Turbo hover) use Sora's air-flinch data: the hop
  and the horizontal speed are `Arc::launch(50)` and `strength × HitBackToJump` (the same
  `FUN_1403d4800` call), landing on motion 5; the friend object's own mode-1 physics was not traced
  separately.
- Mode 4 (blow) is assumed for the friend's blow (`FUN_140409d30` can choose
  mode 8 or 0x21 for other flag states).
- `FUN_140410100`'s counters (`DAT_142aea190`) belong to a mini-game manager and
  are not modelled.
- Hyper Healing, Auto Change, Second Chance and Once More do not exist for the
  party in the room (no equipment).
- Goofy's A201 (motion 11) starts at frame -1; it is sampled from 0.
- ATKP 139 (Goofy's shield) has ReflectRange 45, not 0 as `goofy-spec.md` §5.4 says: the
  shield only clashes with attackers within 45 degrees of his facing. The claws' own cone
  (0 for the Shadow's) is taken as met for other attackers.
- A member's hit on a gallery enemy shows the default spark, not the member's own spell effect.
- The reflect needs an enemy Type 3 attack sphere in reach; the three supported gallery enemies
  were not found to make one in the runs, so the reflect in the room was checked with a synthetic
  guard sphere in the path of the real shot.
- No comparison against the installed game's frames or timings.

## Verdict

PARTIAL.
