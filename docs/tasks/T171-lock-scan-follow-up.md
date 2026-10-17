# T171 — Lock-on camera, Scan gauge and reticle: the follow-up items

Owner Claude, 2026-10-17, requested by burba in the project chat (GitHub issue
#53, which holds the old #54 to #59). Gate: development test room only; no
final gameplay gate inferred. Prerequisites: T170 (the mode 2 camera, the
gauge, the reticle and the HUD slot table). Branch `claude/project-thread-lockscan`
from `main` at `554d3bf` (package 0.1.2). Selected KH2FM Steam app 2552430
build 15194255 English remains a quarantined candidate; the executable used
for every decompile below is the one T170 bound by manifest hash.

Private evidence root: `%LOCALAPPDATA%/KH2FinalRewrite/research/lock-scan-02/`
(`g/` decompiles, disassembly, data dumps, cross-reference lists) beside the
T170 folder `lock-scan-01/`. The installed source stayed read-only.

## What was done, item by item

1. Dynamic camera-collision objects (was #55). The eye clip FUN_140164940
   first cuts its segment at a list of spheres collected from every actor's
   kind-18 collision shapes (FUN_140167c20 -> FUN_1403b3e90 -> FUN_1403c9c00),
   then casts the static polygons. `follow_camera::sphere_hit` / `sphere_entry`
   / `clip_with` translate FUN_140167a50, FUN_140179200, FUN_140179670 and the
   new head of FUN_140164940, with two behaviours of the executable kept on
   purpose: a sphere that contains the segment start stops the scan (the loop
   does not step to the next entry after a skip, disassembly 140167bd0 /
   140167bd4), and an accepted entry point becomes the end of the segment the
   later spheres are tested against (140167bb4..bc8) while the reach it is
   compared with stays the original length. A sphere cut with no polygon
   behind it ends exactly on the sphere (no push-off). Unit tests cover
   head-on, aside, beyond-the-end, ending-inside, skip-and-stop, nearest entry
   in either list order and the 0x40 cap. The cut applies in the field camera's
   eye-target clip (FUN_140165110 ends in FUN_140164940 with the sphere flag
   set), the post-spring clip and mode 2. The producer
   (`EnemyCombat::spheres`, `ShapePose::spheres`,
   `enemy_target::camera_spheres`) reads kind-18 entries attached to the actor
   origin (bone field 0x4000), which the executable keeps live without script
   help. The installed data has kind-18 shapes only on boss objects and a few
   field props (92 entries over 15 objects), none of which a supported actor
   uses, so no scene shows it; the push-off variant 30 used by mode 6 and the
   activation masks (FUN_1403ba550, actor +0x5a0 / +0x5a8) are not modelled.
2. Zoom gate (part of #58). FUN_1403babf0 (bit 3 of the object named by the
   follow actor's +0xc handle) gates the right-stick zoom in the field camera
   (FUN_140165110 / FUN_140165570 -> FUN_1401654d0) and in mode 2
   (FUN_140176c70 -> FUN_140176bc0); pad bit 32 (raw R2, controller table entry
   (0x200 -> 0x1_0000_0000)) overrides it. `CameraInput::zoom_locked` /
   `zoom_override` implement it with tests for both modes; the test room feeds
   the override from the pad. What sets the object's bit 3 is not established
   ([U]), and no supported actor sets it, so `zoom_locked` stays false.
3. Reticle side panel and hide flag (was #56). FUN_1403dd770 reads the target
   actor's flag +0x6c8 bit 1 with its counters +0xd1c / +0xd20 and passes the
   result to FUN_14017bf20, which shows and unfreezes the second Sprite3D
   (group 20, same point and scale, created hidden and frozen by FUN_14017bc60)
   or hides and freezes it; the manager's byte +0x40 bit 0 destroys the
   reticle instead of keeping it. `visual::lock_on::Held::flagged` /
   `hidden` do both, with a test that checks the panel's frozen clock and the
   restart after a hide. The counters are written from byte +0x31 of a spawn
   record (FUN_1403b3080 / FUN_1403b4bd0, game mode 0x10) and then count down
   as pickups pop out (FUN_14040b5a0); no supported actor is spawned from such
   a record, and none of the lock manager's own methods writes 1 to the hide
   byte (they only clear it; an outside writer was not found), so both stay
   false in the test room. Meaning of the panel ("carries pickups") is inferred ([I]).
4. HUD pop-up heights (was #59). The registrants of FUN_140182b10 were
   enumerated: the radar takes slot 18 / sub-slot 1 with height 0; the
   information window (slot 1), NEXT_EXP (14) and GET_MUNNY (17) use sub-slot
   0, which the gauge (sub-slot 1) does not stack on; the counter, mission
   gauge and information pop-ups take slot and sub-slot from event-script
   arguments. Nothing the rewrite plays registers a height on sub-slot 1, so
   the gauge offset is zero in every supported scene and the existing spring
   and table (T170) are already the whole behaviour here. Documented in
   `hud_slots.rs`.
5. Controller input (was #54). A synthetic test now pins the pad tables the
   camera reads: right stick up / down / left / right -> logical bits
   18 / 19 / 20 / 21, R3 -> 17, R2 held -> 32, next to the existing R1 lock and
   cycle-chord tests. This is a table check; no physical controller was
   attached (NOT RUN).

6. Read-only audit by two independent agents against the decompiles (their
   reports are model output, each finding re-checked against the decompile
   before use). Fixed from them: the sphere scan did not shorten the segment
   (first version of item 1); the field camera's eye-target clip ignored the
   spheres; the automatic swing behind the player also needs the right stick
   idle (pad +0x2c, FUN_140165110) and the recenter press is tested before the
   manual turn there; releasing the lock does not delete the reticle but
   restarts it on the group before its own (FUN_14017bfe0 -> FUN_14016b270;
   the `fiel` groups 0, 4, 0x16 and 0x1a are ten-frame single-play animations,
   read from the installed sheet) and it deletes itself when that ends
   (FUN_14016b3c0), so `Reticle` now keeps an outro per release or class
   change (a retarget in the same class still replaces the sprite at once,
   FUN_14017bb80); the Scan gauge is re-created when the same target moves
   between the automatic and the manual lock (FUN_1403dcd70 releases and
   re-creates it through FUN_1403dd440).
   Left open from them: the gauge conditions are evaluated only on lock
   transitions in the executable but every frame here (only visible when Scan
   is toggled or the target stops being targetable mid-lock); two hits in one
   frame make one damage event here and two there; zero-damage events; the
   `cam+0x42` follow gate (pad bit 32 xor a config bit, FUN_140178ae0) whose
   default setting is not established; whether the near/far screen test sees
   linear or clip-space depth (about one unit at the near gate).

## Not done, and why

* Secondary HP pools (#57): the combat state holds one pool per actor and the
  loader rejects shapes that point at another pool (`collision_shapes`:
  "unimplemented HP pool"). The gauge reads `sheet pool[index * 12]`
  (FUN_1403dd440); multi-pool damage belongs to the enemy combat work, not to
  the gauge.
* Camera modes 6 and 8 (#58): FUN_140177600 and FUN_140177c90 are
  follow-camera variants reached through area scripts (they also use the
  sphere cut with the 30-unit push-off, `param_5 == 1`). Nothing in the
  rewrite selects them, so they would be unreachable code. Mode 1 is in
  PR 104 and was not touched. The vtable +0x78 "controllable" gate on the
  follow actor, tested by modes 1 and 2, is not ported either.
* Installed/rewrite side-by-side capture (the first #53 item): the owner chose
  to continue without it; nothing here changes that.

## Checks and verdict

PASS `cargo fmt --all --check`, warnings-denied locked workspace/all-targets
Clippy, locked workspace tests, locked workspace build and the 180-frame smoke
(existing default controller-mapping warning); the numbers are in the PR.
New unit tests: 8 in `follow_camera` (sphere entry, skip-and-stop, nearest
entry, clip with and without a polygon, field-camera cut, zoom gate in both
modes, follow needs the right stick idle, recenter beats turn), 3 in
`lock_on` (panel and hide flag, outro on release and same-class retarget), 1
in `controller` (camera chords), 1 in `enemy_combat` (sphere pose), 1 in
`enemy_target` (gauge re-created on auto <-> manual).
PASS the three-actor combat tour on the installed source (private
`kh2-lockscan-out/tour1`): verdict pass, 14 damaging contacts to enemies and
3 to Sora, Scan hidden then equipped, manual lock held and released; the
captures were inspected and match the T170 run (blue manual reticle on the
held Shadow, the small Scan label at the top right). The tour has no actor
with kind-18 spheres, a side-panel flag or a zoom lock, so the new camera and
reticle branches are covered by unit tests only.
NOT RUN: a physical controller, the installed/rewrite side-by-side capture,
any scene that shows a dynamic sphere, the side panel or the zoom lock.
The decompiles are private, single-pass and audited only by two read-only
agent passes (model output, re-checked against the decompile where acted on),
so coverage counts do not change and no complete function or 1:1 unit is
marked PASS.

Verdict: PARTIAL. Issue #53's open items: the dynamic spheres, the zoom gate,
the reticle panel/hide flag and release outro, the HUD pop-up question and the
controller tables are now translated or explained; secondary HP pools, camera
modes 6 and 8, the installed comparison and a physical controller remain.
Next: when multi-pool enemy combat or event cameras exist, feed them into the
gauge and the camera modes; when a reference capture is allowed, compare the
installed lock-on camera, gauge and reticle numerically.
