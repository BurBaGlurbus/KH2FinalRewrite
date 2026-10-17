# Current state of the project

Written 2026-10-10 and updated 2026-10-11, after every work branch was merged into `main`. It says what each workstream has built and what is unfinished. The per-task detail, evidence and checks stay in [`docs/tasks/`](tasks/README.md); this page only summarises them.

Everything below runs from the user's own selected Steam install (KH2 Final Mix, build 15194255), which stays read-only and is re-validated at every launch. No game data is in Git. Nothing has been compared pixel for pixel against captures of the original game, and no workstream claims a 1:1 pass yet: every record listed here is PROGRESS or PARTIAL.

## Where the work lives

All work is on `main`. The five work branches (movement, combat, Twilight Town, test room and pause menu, cutscene) were merged on 2026-10-11 (T100), so the separate clones are no longer needed for development.

Task records that clashed between branches were renumbered during the merge: Twilight Town is now T097, item pictures T098, and the cutscene T099.

## Workstreams

### Title sequence and launcher

Built:
- The boot sequence plays on every launch: opening fade, Disney, Square Enix, copyright, then the intro and the title screen with its music (T086). The studio splash was removed.
- The owner's edits: a shorter copyright hold and the "Final Re:WRITE" banner (T087).
- `KH2RW.exe` with no arguments opens a plain launcher: find or browse to the game folder, quick-check or full-scan it, refuse any other build, then pick a test (Title Screen, Test Room, Twilight Town, the cutscene, and model/image viewers) (T089). New Game on the title continues straight into the test room.

Unfinished:
- The owner has not signed off on the banner's look; its font is the nearest system face, not the original's.
- The controller and storage dialogs before the title are not built.
- No comparison against a capture of the installed game.

### Test room, pause menu and Jiminy's Journal

Built:
- The test room with Sora, the Keyblade, the follow camera, HUD gauges and command menu, a Shadow that fights back, Guard, hit effects, sounds, music and field talk with Axel as a test NPC (T056–T076).
- The pause (camp) menu: Items, Abilities, Customize, Party, Status, Config, Equip, and the Journal's top pages (Notes, Select World, Collection, Roxas's notes) (T073–T088).

- Item pictures on the Items, Use and Equip screens (T098).
- Journal Characters, Links and Album pages with faces and photos; Character Files, Heartless and Nobodies lists and detail pages; Treasures and Maps grids; Ansem Reports list and reader; first-time tutorial notices; keyboard key-cap prompts (T093).

Unfinished:
- Journal: the Pieces and Puzzles pages; the 3D model pages of Character Files; the "Slain" counts; scroll-bar thumbs. The Story, Missions, Minigames, Limits and Synthesis Notes pages are built (T164) but show only test values in the room.
- Equip: form sub-weapons and the weapon-change list; status totals are not recomputed after equipping.
- Customize and Party: party members' settings, locked members and world guests.
- NEW! badges show on the Items row and the Stock list for items the room adds (T162); the other rows and lists, Load/Save and the item shop are not built.
- Tutorial notices and Journal "seen" marks are kept in memory only, never in a save.

### Gravity and movement feel

First pass: Sora's movement block is loaded the way the original does; jump and fall follow the original's curve instead of an invented launch speed; air momentum, moving along the stick, 8-frame blends, knock-slides and blow flight.

Later passes:
- Physics runs on the original's two-frame step.
- Jump take-off after the crouch, the landing lock, and aerial hops toward the target.
- Landing during an aerial attack cuts it into the landing motion, as the original does.
- One collision call per tick through the shared world collision.

Unfinished:
- No interactive playtest sign-off from the owner on the latest pass.
- `docs/research/movement-feel-1to1-plan.md` is on `main` and contains decompiled addresses and constants. The owner has not decided whether it should stay in the repository.

### Combos, magic, items, EXP and the Drive gauge

Built (all in the test room, against the one Shadow):
- Attacks are chosen the original's way, with combo counters, combo abilities and auto-targeting (T090).
- Damage follows the original's hit records, level rows and ability rules (T091).
- Magic: the Magic menu, MP and MP Charge, and Fire, Blizzard, Thunder, Cure, Magnet and Reflect (T092).
- Sora's hit reactions: air flinch, blow, Aerial Recovery, death (T094).
- Battle items from the Items submenu and shortcuts (T095).
- Kills give EXP and prizes, level-ups change stats, and hits fill the Drive gauge (T096).
- The jump is variable: letting go of Jump cuts the rise short. Square starts Upper Slash, Finishing Leap and Horizontal Slash mid-combo, and Retaliating Slash out of a blow (T105).

Unfinished:
- Spells show the effects of their own effect files since T136 (the old stand-in shapes appear only with F1); no voices play. Cure asks which party member to heal; Donald casts Fire, Blizzard, Thunder and Cure on his own (Magic stat, MP and MP Charge, his own effects) and Goofy his Tornado and Turbo; Magic Lock-On picks a target for a cast made without one.
- Prize sprites, the heal effect and item treasure boxes are drawn and picked up (T161); their physics, the box flight and pickup message are not.
- Level-up panels, the level-up effect and bonus-level HP/MP/Drive growth are built (T160); the EXP popup and encounter bonus ids are not.
- Game over: Sora's death event, the Continue / Load Game list over his lying pose and Continue's reload are built (T140); Load Game, the other game-over modes and the Drive-form KO are not.
- Only one enemy exists, so targeting between several enemies is untested.

### Twilight Town (T097)

Built:
- All 42 Twilight Town areas load by name, with exits, the iris transition and loading of the next area at its entrance; a launcher entry "Twilight Town".
- Sora collides with the map.
- Map drawing: draw groups, texture regions, alpha test, map objects with their motions (the train), scrolling sky, fog and clear colour.
- The place-name banner, exit destination names, and per-area music.
- Townspeople load, walk their paths, react to Sora, and talk through the field talk window.
- Every area is unlocked; there are no story blocks.

- NPCs turn their heads toward Sora, and map textures animate.

Unfinished:
- Townspeople fade in and out and push with Sora (T163); the other NPC programs (sitting and others) are not ported.
- Map effects (sun, lens flare, dust), area enemies and battles, and story events.
- Glow post-effect, per-object level of detail and fade distances, alpha-pass ordering, subtractive blends, texture frame swaps.
- Story progression is fixed at a chosen "first visit" point.
- Exits into other worlds.

### Axel and Roxas cutscene (T099)

Built:
- The scene before the Axel fight plays from the original event data: cameras, actors and motions, subtitles, voices, sound effects and music, and a pause menu with Continue and Skip Scene. Launcher entry "Cutscene: Roxas and Axel".
- Effects from the event's effect file, including Axel's summon effects.
- Both Keyblades are held in Roxas's hands; Axel faces the right way.
- No letterbox bars, at the owner's request.

Unfinished (verdict PARTIAL):
- Some effect modes and layer ordering, HD effect textures.
- The motion-blur pass, mouth/eye texture animation and mesh-group visibility were added but not checked frame by frame.
- The pause menu uses a plain list, not the original's art; no cursor sounds.
- What happens after the scene ends (music, fade) is unknown.

### Donald and Goofy in the party — PARTIAL (T136, T186)

Donald and Goofy follow Sora and fight in the test room (ground attacks, Donald's four spells, Goofy's Tornado and Turbo, the Cure target menu). The enemy's claws now hit them: flinch and blow reactions, Goofy's shield, KO with the revive timer, a revive by Cure, and a hit during a cast ends or pauses it as in the executable (T186). Not built: their voices, a HUD gauge, items, Limits, Goofy's Bash, air attacks.

### Drive forms — not started

The Drive gauge fills, but no form can be entered. Form commands, the Anti/Final form roll, form EXP, and the form models, motions and attack sets are not implemented, and forms also need the party above.

## Open decisions

- Whether `docs/research/movement-feel-1to1-plan.md` stays in the repository: it contains decompiled addresses and constants.
- Whether the old clone folders can be deleted. They hold the owner's release builds.
