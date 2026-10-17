//! Drive Forms in the test room: the Drive / Revert command, the Antiform
//! and forced-Final roll, the form-change task (drive-start motion, wait,
//! object swap, drive-in motion), the form gauge's drain and the automatic
//! revert, form EXP and level-ups (`player::forms` / `drive`).
//!
//! The original destroys the player object and builds a new one with the
//! form's object entry (FUN_14040d000 → FUN_140405030). Here the same
//! `Player` keeps its state and the kit (model, motions, PTYA set,
//! movement record) is exchanged; the presentation respawns the model.
use super::*;
use data::{command_table, path_lookup::Library, sklt};
use player::forms::{self, Forms};
use std::sync::Mutex;

/// Test-room choices: Sora owns Valor, Wisdom, Limit, Master and Final
/// Form, all at level 1, and has nine Drive bars, so every form can be
/// tried in the room.
pub const ROOM_FORMS: [u8; 5] = [1, 2, 3, 4, 5];
pub const ROOM_DRIVE_MAX: u8 = 9;

/// FUN_14040d000: the wait before the swap (30 frames), the grey overlay's
/// fade in (12) and out (15) (FUN_1403f76e0 / FUN_1403f76a0).
const CHANGE_WAIT: u32 = 30;
pub const OVERLAY_IN: f32 = 12.;
pub const OVERLAY_OUT: f32 = 15.;
/// FUN_1403f76e0(fx, 0x30, 0x30, 0x30, 12.0, 0x80) → FUN_1403f7040: the
/// scene light colour moves from 0x80 toward 0x30 (the alpha byte is not
/// read).
const LIGHT_TARGET: f32 = 0x30 as f32;

/// System-bank sounds the drive effects play (drivefx-01 §4): the drive
/// start aura (P_EX100 effect 1), the revert (effect 15), Antiform's
/// arrival (HTLF effect 100) and the form level-up (00effect 16).
const SE_DRIVE: u32 = 114;
const SE_REVERT: u32 = 115;
const SE_ANTI: u32 = 132;
const SE_FORM_LEVEL: u32 = 26;

/// Menu messages: "Revert" (0x81e4, command-01 §1).
pub const REVERT_LABEL: u32 = 0x81e4;
/// The Drive and Revert commands (03system `cmd`), and Wisdom's Shoot.
const CMD_REVERT: u16 = 5;
const SHOOT: u16 = 0x16d;
/// The Limit row, the four limits and their follow-ups, for their names.
const LIMIT_NAMES: [u16; 13] = [
    0x2c3, 0x2ba, 0x2bb, 0x2bc, 0x2bd, 0x2be, 0x2bf, 0x2c0, 0x2c1, 0x2c2, 0x2ab, 0x2ac, 0x2ae,
];

/// What builds a Drive form's kit: the object table, the forms' object
/// ids (03system `memt` row 0), Sora's weapon joints (SKLT), the PTYA,
/// PRTY and command tables, and base Sora's motion sets.
pub struct FormSources {
    objects: Vec<objentry::ObjectEntry>,
    /// Base Sora's object name (form 0), whose kit a Keyblade change
    /// rebuilds.
    base_object: String,
    /// `memt` row 0 (FUN_140405190): the object id of each form.
    pub form_objects: [u16; 7],
    weapon_joint: usize,
    second_joint: Option<usize>,
    ptya: Vec<u8>,
    pref: Vec<u8>,
    went: Vec<u8>,
    /// 03system `wmst`: 0x20-byte weapon motion-set names, row set × 2 +
    /// slot (FUN_1403ef750).
    wmst: Vec<u8>,
    items: Vec<u8>,
    sstm: actor_physics::Sstm,
    base_set: motion::MotionSetSelection,
    spell_ids: Vec<u16>,
    hurt: bool,
    basic_fk: bool,
    /// 00battle `fmlv`.
    pub fmlv: Vec<forms::FormLevel>,
    /// plrp key-0 form records: starting abilities by form.
    pub starting: Vec<(u8, Vec<u16>)>,
    /// The command table, and each form's Drive command (its item's
    /// +0x10, FUN_1403b6920).
    pub commands: HashMap<u16, command_table::Command>,
    pub form_command: [u16; 7],
    /// Command names (system-font strings) by command id, and "Revert".
    pub names: HashMap<u16, Vec<u8>>,
    pub attack_label: Vec<u8>,
    pub magic_label: Vec<u8>,
    /// The level-up panels' text: item names (forms and abilities), sys
    /// 0x43f2 "Form Gauge Powered Up!" and 0x51f "Sora".
    pub item_names: HashMap<u16, Vec<u8>>,
    pub gauge_up: Vec<u8>,
    pub sora_title: Vec<u8>,
    /// Sora's level-up and bonus-level panel texts and the `bons` table.
    pub growth: super::level_growth::Texts,
    pub drive_label: Vec<u8>,
    pub revert_label: Vec<u8>,
    /// 00effect's PAX (the form level-up's element 16) and the resident
    /// `effect/texcommon.dpd`.
    pub system_pax: Option<Arc<data::object_effect::Pax>>,
    pub texcommon: Option<data::object_effect::Dpd>,
    /// `obj/PRIZE.mdlx`'s effect: the prizes' sprites.
    pub prize_pax: Option<Arc<data::object_effect::Pax>>,
}

/// The Keyblade items a kit holds: slot 0's (the member's equipped weapon)
/// and slot 1's (the form's own, 0 = none chosen).
#[derive(Clone, Copy, Debug)]
pub struct Weapons {
    pub main: u16,
    pub sub: u16,
}

/// An object's own effects: the PAX (entry type 18) of `obj/<name>.a.us`.
pub fn object_pax(library: &mut Library, name: &str) -> Option<data::object_effect::Pax> {
    file_pax(library, &format!("obj/{name}.a.us"))
}

/// The first PAX of an installed BAR file.
pub fn file_pax(library: &mut Library, path: &str) -> Option<data::object_effect::Pax> {
    let bytes = library.read(path).ok()?;
    let table = data::bar::read_bar(&bytes).ok()?;
    let e = table
        .entries
        .iter()
        .find(|e| e.entry_type == 18 && e.size > 0)?;
    let pax = bytes.get(e.offset as usize..e.offset as usize + e.size as usize)?;
    data::object_effect::read_pax(pax).ok()
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|w| u16::from_le_bytes([w[0], w[1]]))
}

impl FormSources {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        system: &[u8],
        battle: &[u8],
        objects: &[objentry::ObjectEntry],
        weapon_joint: usize,
        base_object: &str,
        base_set: motion::MotionSetSelection,
        spell_ids: Vec<u16>,
        hurt: bool,
        basic_fk: bool,
        sstm: actor_physics::Sstm,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // memt: u32 version, u32 count, 0x34-byte rows; row 0's u16 at
        // +0x16 + form × 2 is the form's object (FUN_140405190).
        let memt = bar_entry(system, b"memt")?;
        let mut form_objects = [0u16; 7];
        for (f, o) in form_objects.iter_mut().enumerate().skip(1) {
            *o = u16_at(memt, 8 + 0x16 + f * 2).ok_or("memt truncated")?;
        }
        let sklt = sklt::read(bar_entry(system, b"sklt")?)?;
        let second_joint = sklt
            .iter()
            .find(|e| e.character_id == 1)
            .and_then(|e| usize::try_from(e.secondary_bone).ok());
        let items = bar_entry(system, b"item")?;
        let item_rows = data::item_table::read(items)?;
        let mut form_command = [0u16; 7];
        for (f, c) in form_command.iter_mut().enumerate().skip(1) {
            *c = item_rows
                .iter()
                .find(|r| r.id == forms::FORM_ITEMS[f])
                .map_or(0, |r| r.command);
        }
        let commands = command_table::read(bar_entry(system, b"cmd\0")?)?
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        let fmlv = data::battle_tables::form_levels(bar_entry(battle, b"fmlv")?)?
            .into_iter()
            .map(|r| forms::FormLevel {
                id: r.id,
                growth: r.growth,
                ability: r.ability,
                exp: r.exp,
            })
            .collect();
        let starting =
            data::battle_tables::form_starting_abilities(bar_entry(battle, b"plrp")?, 0)?;
        Ok(FormSources {
            objects: objects.to_vec(),
            base_object: base_object.to_owned(),
            form_objects,
            weapon_joint,
            second_joint,
            ptya: bar_entry(battle, b"ptya")?.to_vec(),
            pref: bar_entry(system, b"pref")?.to_vec(),
            went: bar_entry(system, b"went")?.to_vec(),
            wmst: bar_entry(system, b"wmst")?.to_vec(),
            items: items.to_vec(),
            sstm,
            base_set,
            spell_ids,
            hurt,
            basic_fk,
            fmlv,
            starting,
            commands,
            form_command,
            names: HashMap::new(),
            attack_label: Vec::new(),
            magic_label: Vec::new(),
            item_names: HashMap::new(),
            gauge_up: Vec::new(),
            sora_title: Vec::new(),
            growth: super::level_growth::Texts {
                bons: bar_entry(battle, b"bons")?.to_vec(),
                ..Default::default()
            },
            drive_label: Vec::new(),
            system_pax: None,
            texcommon: None,
            prize_pax: None,
            revert_label: Vec::new(),
        })
    }

    /// The command ids whose names the menu shows.
    pub fn named_commands(&self) -> Vec<(u16, u32)> {
        self.form_command
            .iter()
            .filter(|&&c| c != 0)
            .chain([&CMD_REVERT, &SHOOT])
            .chain(LIMIT_NAMES.iter())
            .filter_map(|c| self.commands.get(c).map(|r| (*c, r.name)))
            .collect()
    }

    pub fn command(&self, form: u8) -> Option<&command_table::Command> {
        self.commands
            .get(&self.form_command.get(form as usize).copied().unwrap_or(0))
    }

    /// The form's object entry (`memt`).
    fn object(&self, form: u8) -> Result<&objentry::ObjectEntry, Box<dyn std::error::Error>> {
        if form == 0 {
            return self
                .objects
                .iter()
                .find(|o| o.model_name == self.base_object.as_bytes())
                .ok_or_else(|| "base Sora's object entry is absent".into());
        }
        let id = *self
            .form_objects
            .get(form as usize)
            .ok_or("form out of range")? as u32;
        self.objects
            .iter()
            .find(|o| o.object_id == id)
            .ok_or_else(|| format!("form {form} object {id:#x} absent").into())
    }

    /// Builds form `form`'s kit from the installed files (run off the
    /// main thread; the change task waits for it like FUN_14039e340).
    pub fn build(
        &self,
        source: &ProbeSource,
        form: u8,
        weapons: Weapons,
    ) -> Result<character_kit::Kit, Box<dyn std::error::Error>> {
        let object = self.object(form)?;
        let name = String::from_utf8_lossy(&object.model_name).into_owned();
        let library = Library::open(source)?;
        let find = |path: &str| library.find(path).ok_or(format!("{path} is not installed"));
        let model = find(&format!("obj/{name}.mdlx"))?;
        let mut actors =
            model_inspection::read_model_at(source, model.index, model.ordinal, model.name_hash)?;
        if actors.len() != 1 {
            return Err("form model selection is not one model".into());
        }
        let actor = actors.remove(0);
        let set_name = String::from_utf8_lossy(&object.animation_name).into_owned();
        let set = find(&format!("obj/{set_name}"))?;
        let motion_set = motion::MotionSetSelection {
            index: set.index.to_owned(),
            ordinal: set.ordinal,
            name_hash: set.name_hash,
            ..self.base_set.clone()
        };
        // Weapon slot 0: the equipped Keyblade through the object's WENT
        // set (FUN_1403e0cc0 / FUN_1403d7c70); slot 1 (the objects whose
        // wmst row names a second motion set) takes the form's own
        // Keyblade (FUN_1402fc630), the main one while its record is empty
        // [the original's handling of a Limit Form record was not traced].
        let keyblades =
            crate::keyblade_board::Keyblades::read(&self.went, &self.items, &self.objects, &name)?;
        let load_weapon = |item: u16| -> Result<
            (model_inspection::InspectionModel, character_kit::Blade),
            Box<dyn std::error::Error>,
        > {
            let weapon_name = keyblades
                .model(item)
                .ok_or_else(|| format!("Keyblade {item:#x} has no model in the object's WENT set"))?
                .to_owned();
            let w = find(&format!("obj/{weapon_name}.mdlx"))?;
            let mut library = Library::open(source)?;
            let blade = character_kit::blade_spheres(&library.read_found(&w)?);
            let mut models =
                model_inspection::read_model_at(source, w.index, w.ordinal, w.name_hash)?;
            if models.is_empty() {
                return Err(format!("{weapon_name} has no model").into());
            }
            Ok((models.remove(0), blade))
        };
        let (weapon, blade0) = load_weapon(weapons.main)?;
        // The Keyblade's own effects (swing trail, hit sparks).
        let weapon_pax = keyblades
            .model(weapons.main)
            .and_then(|m| object_pax(&mut Library::open(source).ok()?, m));
        // FUN_1403ef750: each weapon slot's motion set is `obj/%s.mset` of
        // wmst row set × 2 + slot; an empty name (Antiform) leaves the
        // weapon without motions and unattached.
        let set_index = object.neo_moveset as usize;
        let wmst_name = |slot: usize| -> Option<String> {
            let at = (set_index * 2 + slot) * 0x20;
            let raw = self.wmst.get(at..at + 0x20)?;
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            (end > 0).then(|| String::from_utf8_lossy(&raw[..end]).into_owned())
        };
        let weapon_set = |slot: usize| -> Result<Option<motion::MotionSetSelection>, String> {
            let Some(n) = wmst_name(slot) else {
                return Ok(None);
            };
            let f = find(&format!("obj/{n}.mset"))?;
            Ok(Some(motion::MotionSetSelection {
                index: f.index.to_owned(),
                ordinal: f.ordinal,
                name_hash: f.name_hash,
                ..self.base_set.clone()
            }))
        };
        let weapon_motion_set = weapon_set(0)?;
        // Base Sora (objentry +0x57 = 0) has no slot 1.
        let second_weapon_motion_set = if form == 0 { None } else { weapon_set(1)? };
        let second = if second_weapon_motion_set.is_some() {
            Some(load_weapon(if weapons.sub != 0 {
                weapons.sub
            } else {
                weapons.main
            })?)
        } else {
            None
        };
        let (weapon2, blade1) = match second {
            Some((model, blade)) => (Some(model), blade),
            None => (None, character_kit::Blade::new()),
        };
        let collision_archive = if self.hurt {
            let mut lib = Library::open(source)?;
            Some(lib.read_found(&model)?)
        } else {
            None
        };
        let (params, _, _) = player_params(&self.objects, &name, &self.pref, &self.sstm)?;
        let pax = object_pax(&mut Library::open(source)?, &name);
        // Slot 1 (the form Keyblade) hangs on SKLT's second bone.
        let second_joint = second_weapon_motion_set
            .is_some()
            .then_some(self.second_joint)
            .flatten();
        character_kit::load_kit(
            source,
            character_kit::KitInputs {
                form,
                actor,
                weapon,
                weapon2,
                blades: [blade0, blade1],
                weapon_pax,
                weapon_joint: self.weapon_joint,
                second_joint,
                motion_set,
                fallback: (form != 0).then(|| self.base_set.clone()),
                weapon_motion_set,
                second_weapon_motion_set,
                ptya: &self.ptya,
                ptya_pointer: object.neo_moveset as usize,
                spell_ids: self.spell_ids.clone(),
                hurt: self.hurt,
                collision_archive,
                basic_fk: self.basic_fk,
                params,
                pax,
                // Limit Form's limits (limitform-01 §3.4).
                scripted: if form == forms::LIMIT {
                    [211, 212, 213]
                        .into_iter()
                        .chain(221..=227)
                        .chain(231..=234)
                        .chain(241..=248)
                        .collect()
                } else {
                    Vec::new()
                },
            },
        )
    }
}

/// The battle state DAT_142a11404 as FUN_1403ab6f0 keeps it: an enemy in
/// range sets it after 45 frames, out of range clears it after 120.
#[derive(Default)]
pub struct BattleState {
    pub on: bool,
    counter: u32,
    since_reset: u32,
}

impl BattleState {
    pub fn update(&mut self, target: bool) {
        self.since_reset = self.since_reset.saturating_add(1);
        if target != self.on {
            self.counter += 1;
            let need = if target { 45 } else { 120 };
            if self.since_reset <= 30 || self.counter >= need {
                self.on = target;
                self.counter = 0;
            }
        } else {
            self.counter = 0;
        }
    }
}

/// A form kit being built off the main thread.
type KitSlot = Arc<Mutex<Option<Result<character_kit::Kit, String>>>>;
/// Sora's model and Keyblade parts.
type ModelParts = Or<(With<SoraPart>, With<WeaponPart>)>;

/// The form-change task (FUN_14040caa0's record).
pub struct Change {
    pub target: u8,
    /// Frames since the task started.
    pub frames: u32,
    /// FUN_1403ba9a0 at the start (flags bit 0): the new object falls.
    pub airborne: bool,
}

/// One kit's pieces as the room keeps them while another is active.
struct Stash {
    target_shape: Option<object_collision::Entry>,
    params: Params,
    clips: Clips,
    poses: HashMap<(Action, bool), usize>,
    clip_poses: Vec<ClipPoses>,
    body: Vec<(usize, Vec3, f32, bool)>,
    attack: HashMap<u8, Vec<(usize, Vec3, f32)>>,
    /// The kit's two Keyblades' attack spheres.
    blade: [character_kit::Blade; 2],
    /// Its slot 0 Keyblade's effects (None: the room's default).
    weapon_pax: Option<Arc<data::object_effect::Pax>>,
}

/// Drive state of the session.
pub struct DriveRt {
    pub src: Arc<FormSources>,
    source: Arc<ProbeSource>,
    pub forms: Forms,
    /// DAT_1409acdd4: the form Sora is in or changing to.
    pub form: u8,
    /// The kit the room plays now.
    pub shown: u8,
    /// DAT_142a10508 bit 0: a form is active.
    pub active: bool,
    pub change: Option<Change>,
    stash: HashMap<u8, Stash>,
    /// Models of every kit built so far (actor, Keyblade, slot-1 joint).
    pub models: HashMap<
        u8,
        (
            model_inspection::InspectionModel,
            model_inspection::InspectionModel,
            Option<model_inspection::InspectionModel>,
        ),
    >,
    /// The member's equipped Keyblade (slot 0 of every kit) and the kits'
    /// rebuild after it changes.
    pub main_weapon: u16,
    reweapon: Option<u8>,
    /// Keyblades each kit shows: 0 (Antiform), 1 or 2.
    pub weapons: HashMap<u8, u8>,
    loading: Option<(u8, KitSlot)>,
    ready: HashMap<u8, character_kit::Kit>,
    /// The shared battle RNG DAT_140783ca0 (image value 1).
    pub seed: u32,
    pub battle: BattleState,
    /// Test-room choice (F12 toggles it): with no party members in the
    /// room, Donald and Goofy count as present and usable for the party
    /// requirement, so Valor, Wisdom, Master and Final can be tried
    /// anywhere. A room with the party uses the real members.
    pub party: bool,
    /// The room has party members (party_room): the slots come from them.
    real_party: bool,
    slots: [Option<forms::Member>; 2],
    /// Members absorbed by the form and the frames since they left
    /// (FUN_140400990: they fade out over 15).
    stored: Vec<(usize, u32)>,
    /// FUN_1403f3fa0's rotating base sector (DAT_142ae5d78).
    sector: u8,
    /// A menu request: drive into a form (its command's form), or 0 to
    /// revert.
    pub request: Option<u8>,
    /// The presentation rebuilds the model for this kit.
    pub respawn: Option<u8>,
    /// Grey overlay alpha 0..=1 of 0x80 and its per-frame step.
    pub overlay: f32,
    overlay_step: f32,
    /// Level-ups to announce ("Form Gauge Powered Up").
    pub level_ups: Vec<forms::LevelUp>,
    /// Form EXP earned this frame (applied with Sora's ability slots).
    pub pending_exp: i32,
    /// The defeated-enemy character id for Wisdom / Final EXP.
    pub enemy_character: u16,
    /// The forms behind the open Drive submenu's rows.
    pub menu_forms: Vec<u8>,
    /// Each kit's object PAX, the effect starts this tick, the kit whose
    /// object was just replaced, and the motion trigger scan's position.
    pub paxes: HashMap<u8, Arc<data::object_effect::Pax>>,
    pub fx_requests: Vec<drive_fx::FxRequest>,
    pub fx_owner_left: Option<u8>,
    fx_scan: Option<(u64, Action, u32)>,
    previous_shown: u8,
    /// Limit Form: the running limit, a requested one, the reaction
    /// command offered this tick, and the limit's colour filter (colour
    /// and ramp 0..=1).
    pub limit: Option<player::limit_form::Limit>,
    pub limit_request: Option<u16>,
    pub offer: Option<u16>,
    pub tint: [u8; 3],
    pub tint_k: f32,
    tint_step: f32,
    /// The change's afterimage: ramp k (alpha = trunc(0x70·k)), its speed
    /// and the ticks left once the swap starts its fade.
    after: Option<(f32, f32, u32)>,
    /// Wisdom Form's bullets in flight.
    pub bullets: Vec<player::wisdom_form::Bullet>,
    bullet_serial: u64,
}

impl DriveRt {
    pub fn new(src: FormSources, source: Arc<ProbeSource>, enemy_character: u16) -> Self {
        let mut forms = Forms::default();
        for &(f, ref list) in &src.starting {
            if let Some(r) = forms.record_mut(f) {
                for &id in list {
                    r.add_ability(id);
                }
            }
        }
        for f in ROOM_FORMS {
            forms.obtain(f, KINGDOM_KEY);
        }
        DriveRt {
            src: Arc::new(src),
            source,
            forms,
            form: 0,
            shown: 0,
            active: false,
            change: None,
            stash: HashMap::new(),
            models: HashMap::new(),
            main_weapon: KINGDOM_KEY,
            reweapon: None,
            weapons: HashMap::new(),
            loading: None,
            ready: HashMap::new(),
            seed: 1,
            battle: BattleState::default(),
            party: true,
            real_party: false,
            slots: [None, None],
            stored: Vec::new(),
            sector: 0,
            request: None,
            respawn: None,
            overlay: 0.,
            overlay_step: 0.,
            level_ups: Vec::new(),
            pending_exp: 0,
            enemy_character,
            menu_forms: Vec::new(),
            bullets: Vec::new(),
            limit: None,
            limit_request: None,
            offer: None,
            tint: [128; 3],
            tint_k: 0.,
            tint_step: 0.,
            after: None,
            paxes: HashMap::new(),
            fx_requests: Vec::new(),
            fx_owner_left: None,
            fx_scan: None,
            previous_shown: 0,
            bullet_serial: 0,
        }
    }

    /// The afterimage's previous-frame weight, while it is on.
    pub fn afterimage(&self) -> Option<f32> {
        self.after.map(|(k, _, _)| (0x70 as f32 * k).trunc() / 128.)
    }

    /// FUN_1403f7040 / FUN_1401626f0: the scene light factor,
    /// (128 + trunc((0x30 − 0x80) × k)) / 128 for the overlay ramp k.
    pub fn scene_light(&self) -> f32 {
        (128. + ((LIGHT_TARGET - 128.) * self.overlay).trunc()) / 128.
    }

    /// The party slots as FUN_140400740 sees them.
    pub fn members(&self) -> [Option<forms::Member>; 2] {
        if self.real_party {
            self.slots
        } else if self.party {
            [
                Some(forms::Member {
                    character: 2,
                    usable: true,
                }),
                Some(forms::Member {
                    character: 3,
                    usable: true,
                }),
            ]
        } else {
            [None, None]
        }
    }

    /// FUN_140401d60 case 4 for form `form`'s Drive command.
    pub fn selectable(&self, form: u8, gauge: &player::drive::Gauge) -> bool {
        let Some(cmd) = self.src.command(form) else {
            return false;
        };
        forms::DriveCheck {
            form,
            cost: cmd.cost,
            level: gauge.level,
            busy: self.active || self.change.is_some(),
            player_form: self.form,
            party: forms::party_requirement(form, &self.members(), 1.),
            limit_barred: false,
            locked: false,
        }
        .selectable()
            && self.forms.owned[form as usize]
    }

    /// The Drive submenu: the owned forms in form order, each with its
    /// command's colour (0 usable, 5 greyed).
    pub fn menu(&self, gauge: &player::drive::Gauge) -> Vec<(u8, u16, usize)> {
        (1..6u8)
            .filter(|&f| self.forms.owned[f as usize])
            .map(|f| {
                let colour = if self.selectable(f, gauge) { 0 } else { 5 };
                (f, self.src.form_command[f as usize], colour)
            })
            .collect()
    }

    /// FUN_1403b6cb0: command `id` (followed up its +2 chain to its root)
    /// is disabled in the current form (cmd +0x2c bit form).
    pub fn masked(&self, id: u16) -> bool {
        if !self.active {
            return false;
        }
        let mut c = self.src.commands.get(&id);
        for _ in 0..8 {
            match c {
                Some(cmd) if cmd.execute != cmd.id => match self.src.commands.get(&cmd.execute) {
                    Some(next) => c = Some(next),
                    None => break,
                },
                _ => break,
            }
        }
        c.is_some_and(|cmd| cmd.form_mask >> self.form & 1 != 0)
    }

    /// In Limit Form (its Magic slot is "Limit").
    pub fn limit_form(&self) -> bool {
        self.active && self.form == forms::LIMIT && self.change.is_none()
    }

    /// Submenu 12's rows and colours (FUN_1403d88e0).
    pub fn limit_rows(&self, mp: i32, charging: bool) -> Vec<(u16, usize)> {
        player::limit_form::LIMITS
            .iter()
            .map(|&c| {
                let cost = self.src.commands.get(&c).map_or(0, |r| r.cost);
                (
                    c,
                    player::limit_form::colour(cost, self.limit.is_some(), mp, charging),
                )
            })
            .collect()
    }

    /// The scene colour of the limit's filter (0x80 = 1.0).
    pub fn scene_tint(&self) -> [f32; 3] {
        self.tint
            .map(|c| (128. + ((c as f32 - 128.) * self.tint_k).trunc()) / 128.)
    }

    /// The Attack row's label: Wisdom's "Shoot" (0x16d) in that form,
    /// else Attack (None when the names were not loaded).
    pub fn attack_label(&self) -> Option<Vec<u8>> {
        let label = if self.active && self.form == forms::WISDOM {
            self.src.names.get(&SHOOT)?.clone()
        } else {
            self.src.attack_label.clone()
        };
        (!label.is_empty()).then_some(label)
    }

    /// FUN_1403f36c0: in Antiform the Revert command is greyed while the
    /// battle flag is set.
    pub fn revert_usable(&self) -> bool {
        self.active && self.change.is_none() && !(self.form == forms::ANTI && self.battle.on)
    }

    fn start_load(&mut self, form: u8) {
        if self.ready.contains_key(&form)
            || self.stash.contains_key(&form)
            || self.loading.as_ref().is_some_and(|l| l.0 == form)
        {
            return;
        }
        let slot = Arc::new(Mutex::new(None));
        let (src, source, out) = (
            Arc::clone(&self.src),
            Arc::clone(&self.source),
            Arc::clone(&slot),
        );
        let weapons = self.weapons_for(form);
        std::thread::spawn(move || {
            let built = src.build(&source, form, weapons).map_err(|e| e.to_string());
            if let Ok(mut o) = out.lock() {
                *o = Some(built);
            }
        });
        self.loading = Some((form, slot));
    }

    /// The Keyblades form `form`'s kit holds: the equipped one, and the
    /// form's own (FUN_1402fc630).
    fn weapons_for(&self, form: u8) -> Weapons {
        Weapons {
            main: self.main_weapon,
            sub: self.forms.record(form).map_or(0, |r| r.keyblade),
        }
    }

    /// The equipped Keyblade or a form's Keyblade changed: every kit built
    /// with the old ones is dropped, and the one on screen is rebuilt in the
    /// background and swapped in when ready (the weapon-change list,
    /// FUN_14036afa0 / FUN_14036b210, replaces the weapon objects).
    pub fn keyblades_changed(&mut self, main: u16) {
        self.main_weapon = main;
        self.ready.clear();
        self.stash.clear();
        self.loading = None;
        self.reweapon = Some(self.shown);
        self.start_load(self.shown);
    }

    /// Whether form `form`'s kit can be swapped in now.
    fn loaded(&mut self, form: u8, log: &mut Vec<serde_json::Value>) -> Result<bool, String> {
        if self.ready.contains_key(&form) || self.stash.contains_key(&form) {
            return Ok(true);
        }
        let Some((f, slot)) = &self.loading else {
            self.start_load(form);
            return Ok(false);
        };
        if *f != form {
            return Ok(false);
        }
        let built = slot
            .lock()
            .map_err(|_| "form loader stopped".to_owned())?
            .take();
        match built {
            Some(Ok(kit)) => {
                log.push(serde_json::json!({"event":"form_kit","form":form,"summary":kit.summary}));
                self.ready.insert(form, kit);
                self.loading = None;
                Ok(true)
            }
            Some(Err(e)) => {
                self.loading = None;
                Err(e)
            }
            None => Ok(false),
        }
    }
}

/// FUN_14040ce60: start a change to `form` (0 = revert). A form plays the
/// drive-start motion; the task swaps the object.
pub(super) fn start_change(game: &mut Game, form: u8) {
    let Some(d) = game.drive.as_mut() else { return };
    if form == d.form {
        return;
    }
    let airborne = game.player.in_air();
    if form != 0 {
        if game.clips.drive.is_some() {
            game.player.start_drive();
            game.action_started = game.tick;
        }
        d.start_load(form);
        // FUN_1403f6f80 / FUN_1403f76e0: the grey overlay, 12 frames in.
        d.overlay_step = 1. / OVERLAY_IN;
        // FUN_140400b20: the members the form takes leave the field.
        let mut leaving = Vec::new();
        for (i, m) in game.party.iter_mut().enumerate() {
            if !m.stored && forms::absorbs(form, character_of(m.kind.name)) {
                m.stored = true;
                m.follower.attack = None;
                leaving.push((i, 0));
            }
        }
        d.stored.extend(leaving);
    }
    d.form = form;
    game.sound_queue.push(if form != 0 {
        (SE_DRIVE, "drive_start")
    } else {
        (SE_REVERT, "revert")
    });
    let Some(d) = game.drive.as_mut() else { return };
    d.change = Some(Change {
        target: form,
        frames: 0,
        airborne,
    });
}

/// FUN_140405440 case 4: roll, pay the result's cost, change.
fn execute(game: &mut Game, requested: u8) {
    let Some(d) = game.drive.as_mut() else { return };
    let members = d.members();
    let src = Arc::clone(&d.src);
    let final_cost = src.command(forms::FINAL).map_or(5, |c| c.cost);
    let ctx = forms::RollContext {
        disabled: false,
        abilities: &game.player.abilities,
        area_anti: 1,
        battle: d.battle.on,
        anti_party: forms::party_requirement(forms::ANTI, &members, 1.),
        final_party: forms::party_requirement(forms::FINAL, &members, 1.),
        final_owned: d.forms.owned[forms::FINAL as usize],
        // The room has no story progress (flag 0x4842 clear).
        final_story: false,
        final_cost,
        level: game.player.drive.level,
    };
    let rolled = d.forms.roll(requested, &ctx, &src.fmlv, &mut d.seed);
    let cost = src.command(rolled.form).map_or(0, |c| c.cost);
    game.player.drive.pay(cost);
    let tick = game.tick;
    game.event(
        serde_json::json!({"event":"drive","tick":tick,"requested":requested,
        "form":rolled.form,"final_obtained":rolled.final_obtained,"cost":cost,
        "anti_counter":game.drive.as_ref().map(|d| d.forms.records[5].exp)}),
    );
    start_change(game, rolled.form);
}

/// Exchanges the room's active kit for `form`'s.
fn swap_kit(game: &mut Game, form: u8) -> Result<(), String> {
    let d = game.drive.as_mut().ok_or("no drive state")?;
    let incoming = match d.stash.remove(&form) {
        Some(s) => s,
        None => {
            let kit = d.ready.remove(&form).ok_or("form kit not loaded")?;
            let n = if kit.weapon_motions == 0 {
                0
            } else if kit.clip_poses.iter().any(|c| !c.weapon2.is_empty()) {
                2
            } else {
                1
            };
            d.weapons.insert(form, n);
            d.models.insert(form, (kit.actor, kit.weapon, kit.weapon2));
            if let Some(p) = kit.pax {
                d.paxes.insert(form, Arc::new(p));
            }
            Stash {
                blade: kit.blades,
                weapon_pax: kit.weapon_pax.map(Arc::new),
                target_shape: kit.target_shape,
                params: kit.params,
                clips: kit.clips,
                poses: kit.actions.into_iter().collect(),
                clip_poses: kit.clip_poses,
                body: kit.body,
                attack: kit.attack,
            }
        }
    };
    let shown = d.shown;
    d.previous_shown = shown;
    let blade = game.enemy.as_mut().map_or_else(
        || [Default::default(), Default::default()],
        |rt| std::mem::replace(&mut rt.blade, incoming.blade.clone()),
    );
    let body = game
        .enemy
        .as_mut()
        .map(|rt| std::mem::replace(&mut rt.sora_body, incoming.body.clone()))
        .unwrap_or_default();
    let outgoing = Stash {
        target_shape: std::mem::replace(&mut game.player_target_shape, incoming.target_shape),
        params: std::mem::replace(&mut game.params, incoming.params),
        clips: std::mem::replace(&mut game.clips, incoming.clips),
        poses: std::mem::replace(&mut game.poses, incoming.poses),
        clip_poses: std::mem::replace(&mut game.clip_poses, incoming.clip_poses),
        body,
        attack: std::mem::replace(&mut game.sora_attack, incoming.attack),
        blade,
        weapon_pax: std::mem::replace(&mut game.weapon_pax, incoming.weapon_pax),
    };
    game.weapon_pax_changed = true;
    // The trail's length follows the new Keyblade's size.
    game.trail_length = None;
    // The new kit's attack rows (ATKP by id, sub-table 0) for the hits.
    if let Some(rt) = game.enemy.as_mut() {
        let ids: Vec<u16> = game
            .clip_poses
            .iter()
            .flat_map(|c| c.hits.iter().chain(&c.hits2))
            .map(|h| h.2)
            .collect();
        for id in ids {
            if ensure_attack_rows(rt, id, 0)
                && let Some(row) = rt.rows.get(&id).copied()
            {
                rt.hit_effects
                    .entry(id)
                    .or_insert((row.hit_effect, row.hit_effect_random));
            }
        }
    }
    let d = game.drive.as_mut().ok_or("no drive state")?;
    d.stash.insert(shown, outgoing);
    d.shown = form;
    d.respawn = Some(form);
    Ok(())
}

/// The swap and the new object's first frame (FUN_14040d000's body,
/// FUN_1403c17c0, FUN_140405630).
fn finish_change(game: &mut Game, abilities: Option<&[u16]>) -> Result<(), String> {
    let change = game
        .drive
        .as_mut()
        .and_then(|d| d.change.take())
        .ok_or("no change")?;
    let f = change.target;
    swap_kit(game, f)?;
    let airborne = change.airborne && game.player.in_air();
    let action = game.player.arrive(f, airborne, &game.clips);
    game.action_started = game.tick;
    let _ = action;
    let d = game.drive.as_mut().ok_or("no drive state")?;
    // The old object's effects fade with it; a revert's new object starts
    // elements 15..27 of its PAX (FUN_140405630).
    d.fx_owner_left = Some(d.previous_shown);
    if f == 0
        && let Some(p) = d.paxes.get(&0).cloned()
    {
        for id in 15..=27 {
            d.fx_requests.push(drive_fx::FxRequest {
                pax: p.clone(),
                id,
                owner: 0,
            });
        }
    }
    if f != 0 {
        d.active = true;
        if f == forms::ANTI {
            game.sound_queue.push((SE_ANTI, "antiform"));
        }
        let Some(d) = game.drive.as_mut() else {
            return Err("no drive state".into());
        };
        let level = d.forms.record(f).map_or(1, |r| r.level);
        // FUN_1403c17c0: HP and MP to full, an MP charge cancelled, the
        // form's bars.
        game.player.mp.cur = game.player.mp.max;
        game.player.mp.remaining = 0.;
        game.player.mp.total = 0.;
        game.player.mp_charging = false;
        game.player.drive.enter_form(f, level);
        if let Some(rt) = game.enemy.as_mut() {
            rt.sora_hp = rt.sora_target.max_hp;
        }
    } else {
        d.active = false;
        game.player.drive.leave_form();
        // FUN_140400440: the absorbed members come back around Sora.
        let stored = std::mem::take(&mut d.stored);
        let (mut seed, mut sector) = (d.seed, d.sector);
        for (i, _) in stored {
            let spot =
                forms::return_spot(&game.world, game.player.position, &mut seed, &mut sector);
            if let Some(m) = game.party.get_mut(i) {
                m.stored = false;
                m.follower.position = spot.0;
                m.follower.yaw = spot.1;
                m.follower.attack = None;
            }
        }
        if let Some(d) = game.drive.as_mut() {
            (d.seed, d.sector) = (seed, sector);
        }
    }
    game.player.shoot = f == forms::WISDOM;
    rebuild_abilities(game, abilities);
    let tick = game.tick;
    let gauge = game.player.drive;
    game.event(
        serde_json::json!({"event":"form_change","tick":tick,"form":f,
        "bars":gauge.bars,"time":gauge.time,"drive_level":gauge.level}),
    );
    Ok(())
}

/// FUN_1404014a0 with the active form's record linked: Sora's support
/// abilities, then the form's 24 slots.
pub fn rebuild_abilities(game: &mut Game, sora: Option<&[u16]>) {
    if let Some(s) = sora {
        game.ability_slots = s.iter().copied().take(80).collect();
        game.ability_slots.resize(80, 0);
    }
    let form = game
        .drive
        .as_ref()
        .filter(|d| d.active)
        .and_then(|d| d.forms.record(d.form))
        .copied();
    game.player.abilities =
        game.ability_source
            .build_with_form(&game.ability_slots, &game.equipment, form.as_ref());
    // FUN_1403c1d90 rebuilds the stat totals and the abilities together.
    game.refresh_gear();
}

/// Form EXP for Sora (FUN_1403e8380), with level-ups' ability changes
/// applied to Sora's slots.
pub fn add_form_exp(game: &mut Game, amount: i32, sora: Option<&mut Vec<u16>>) {
    let Some(d) = game.drive.as_mut() else { return };
    if !d.active || amount <= 0 {
        return;
    }
    let f = d.form;
    let slots: &mut Vec<u16> = match sora {
        Some(s) => s,
        None => &mut game.ability_slots,
    };
    let src = Arc::clone(&d.src);
    let ups = d.forms.add_exp(f, amount, &src.fmlv, slots, false);
    for up in &ups {
        if let Some(a) = up.ability {
            // FUN_1403c3f00(ability, 1): into Sora's first empty slot.
            if let Some(s) = slots.iter_mut().find(|s| **s == 0) {
                *s = a;
            }
        }
    }
    if ups.is_empty() {
        return;
    }
    let snapshot = slots.clone();
    d.level_ups.extend(&ups);
    // FUN_1403ed390: system effect 16 on Sora.
    if let Some(p) = d.src.system_pax.clone() {
        d.fx_requests.push(drive_fx::FxRequest {
            pax: p,
            id: 16,
            owner: 0xff,
        });
    }
    game.sound_queue.push((SE_FORM_LEVEL, "form_level"));
    let tick = game.tick;
    game.event(
        serde_json::json!({"event":"form_level","tick":tick,"form":f,
        "levels":ups.iter().map(|u| serde_json::json!({"level":u.level,"form_growth":u.form_growth,
            "sora_growth":u.sora_growth,"ability":u.ability})).collect::<Vec<_>>()}),
    );
    rebuild_abilities(game, Some(&snapshot));
}

/// One frame of the Drive system after the player's tick: menu requests,
/// the change task, the form gauge's drain and the automatic revert.
pub fn tick(game: &mut Game, abilities: Option<&[u16]>) {
    let Some(d) = game.drive.as_mut() else { return };
    // A rebuilt kit with the new Keyblades replaces the one on screen.
    if let Some(form) = d.reweapon
        && d.change.is_none()
    {
        let mut log = Vec::new();
        let ready = d.loaded(form, &mut log);
        for entry in log {
            game.event(entry);
        }
        match ready {
            Ok(true) => {
                if let Some(d) = game.drive.as_mut() {
                    d.reweapon = None;
                }
                match swap_kit(game, form) {
                    Ok(()) => {
                        // The old kit swapped out is stale: not kept.
                        if let Some(d) = game.drive.as_mut() {
                            d.stash.remove(&form);
                        }
                        let tick = game.tick;
                        game.event(
                            serde_json::json!({"event":"keyblade_kit","tick":tick,"form":form}),
                        );
                    }
                    Err(e) => {
                        game.event(serde_json::json!({"event":"keyblade_kit_error","error":e}))
                    }
                }
            }
            Ok(false) => {}
            Err(e) => {
                if let Some(d) = game.drive.as_mut() {
                    d.reweapon = None;
                }
                game.event(serde_json::json!({"event":"keyblade_kit_error","error":e}));
            }
        }
    }
    let Some(d) = game.drive.as_mut() else { return };
    // FUN_1403ab6f0's battle state.
    let p = game.player.position;
    let target = game.enemy.as_ref().is_some_and(|rt| {
        let e = rt.enemy.position;
        !matches!(
            rt.enemy.current_act(),
            Some(shadow::ActKind::Leave | shadow::ActKind::Dead)
        ) && ((e[0] - p[0]).powi(2) + (e[2] - p[2]).powi(2)).sqrt() < BATTLE_DISTANCE
    });
    d.battle.update(target);
    // The party slots (FUN_1403c3270): Donald and Goofy when in the room;
    // a member the form took is unavailable.
    d.real_party = !game.party.is_empty();
    d.slots = [0, 1].map(|k| {
        game.party.get(k).map(|m| forms::Member {
            character: character_of(m.kind.name),
            usable: !m.stored,
        })
    });
    for s in &mut d.stored {
        s.1 += 1;
    }
    // Unattended runs (between the camp visits): Limit Form at 7490, then
    // Strike Raid and Infinity with MP refilled, each follow-up pressed as
    // soon as it is offered.
    // The magic tour casts in a form (`--magic-form`).
    if game.magic_tour && game.magic_form != 0 {
        match game.tick {
            5 => game.player.drive.refill(),
            10 => d.request = Some(game.magic_form),
            _ => {}
        }
    }
    if game.autoplay.is_some() && game.ability_tour.is_none() && !game.magic_tour {
        use player::limit_form::{INFINITY, STRIKE_RAID};
        match game.tick {
            7480 => game.player.drive.refill(),
            7490 => d.request = Some(forms::LIMIT),
            7695 | 8055 => game.player.mp.cur = game.player.mp.max,
            7700 => d.limit_request = Some(INFINITY),
            8060 => d.limit_request = Some(STRIKE_RAID),
            _ => {}
        }
    }
    d.overlay = (d.overlay + d.overlay_step).clamp(0., 1.);
    // The afterimage (drivefx-02 §3.4): the 42000 driver steps k; after
    // the swap the task lives one loop tick and yield(2) more [I: 4].
    if let Some((k, speed, left)) = d.after.as_mut() {
        *k = (*k + *speed).clamp(0., 1.);
        if *speed < 0. {
            *left = left.saturating_sub(1);
            if *left == 0 {
                d.after = None;
            }
        }
    }
    if let Some(f) = d.request.take() {
        if f == 0 {
            if d.revert_usable() {
                start_change(game, 0);
            }
        } else if d.selectable(f, &game.player.drive) {
            execute(game, f);
        }
    }
    let Some(d) = game.drive.as_mut() else { return };
    if let Some(c) = d.change.as_mut() {
        c.frames += 1;
        let target = c.target;
        let frames = c.frames;
        let mut log = Vec::new();
        // T0+30: the afterimage on at full strength (drives only).
        if target != 0 && frames == CHANGE_WAIT {
            d.after = Some((1., 0., 0));
        }
        let ready = if target == 0 {
            Ok(true)
        } else if frames < CHANGE_WAIT {
            Ok(false)
        } else {
            d.loaded(target, &mut log)
        };
        for v in log {
            game.event(v);
        }
        // FUN_14040ce00: HP and MP +6 a frame until the swap.
        if target != 0 {
            game.player.mp.cur = (game.player.mp.cur + 6).min(game.player.mp.max);
            if let Some(rt) = game.enemy.as_mut() {
                rt.sora_hp = (rt.sora_hp + 6).min(rt.sora_target.max_hp);
            }
        }
        // FUN_14040d000 waits for motion 0x34 only while a party member is
        // still in the field (FUN_140400840): the stand-in party's
        // remaining member for Valor, Wisdom and Limit [I: members that
        // left count as gone].
        let party_present = if game.party.is_empty() {
            game.drive.as_ref().is_some_and(|d| {
                d.party && matches!(target, forms::VALOR | forms::WISDOM | forms::LIMIT)
            })
        } else {
            game.party.iter().any(|m| !m.stored)
        };
        match ready {
            Ok(true) if target == 0 || !party_present || game.player.drive_done(&game.clips) => {
                if target != 0
                    && let Some(d) = game.drive.as_mut()
                {
                    d.overlay_step = -1. / OVERLAY_OUT;
                }
                if let Err(e) = finish_change(game, abilities) {
                    eprintln!("Drive form change failed: {e}");
                }
                // The swap: lerp k 1 → 0 over 20 (speed −0.05).
                if let Some(d) = game.drive.as_mut()
                    && let Some(a) = d.after.as_mut()
                {
                    *a = (a.0, -0.05, 4);
                }
            }
            Ok(_) => {}
            Err(e) => {
                // The form could not be built: stay as base Sora.
                eprintln!("Drive form {target} not loaded: {e}");
                let tick = game.tick;
                game.event(serde_json::json!({"event":"form_load_failed","tick":tick,"form":target,"error":e}));
                if let Some(d) = game.drive.as_mut() {
                    d.change = None;
                    d.after = None;
                    d.form = d.shown;
                    d.overlay_step = -1. / OVERLAY_OUT;
                    // The members the form took stay with Sora.
                    for (i, _) in std::mem::take(&mut d.stored) {
                        if let Some(m) = game.party.get_mut(i) {
                            m.stored = false;
                        }
                    }
                }
                game.player.arrive(0, game.player.in_air(), &game.clips);
            }
        }
        return;
    }
    if !d.active {
        return;
    }
    // FUN_1403a92c0: the form gauge drains every frame.
    let (battle, boost, summon) = (
        d.battle.on,
        game.player.abilities.form_boost,
        game.player.abilities.summon_boost,
    );
    game.player.drive.drain_frame(battle, boost, summon);
    // FUN_140404a40: at time 0, revert once the Revert command could run.
    if game.player.drive.time <= 0. && !game.player.dead() && game.player.can_command(&game.clips) {
        start_change(game, 0);
    }
}

/// The 4th command row: "Drive" (colour 5 in a form or with no usable
/// form) or "Revert" in a form.
pub fn menu_row(game: &Game) -> Option<(Vec<u8>, usize, bool)> {
    let d = game.drive.as_ref()?;
    if d.active || d.change.is_some() {
        let colour = if d.revert_usable() { 0 } else { 5 };
        Some((d.src.revert_label.clone(), colour, false))
    } else {
        let any = d.menu(&game.player.drive).iter().any(|m| m.2 < 4);
        let colour = if d.forms.owned[1..6].iter().any(|&o| o) {
            if any { 0 } else { 1 }
        } else {
            5
        };
        Some((d.src.drive_label.clone(), colour, true))
    }
}

/// Slot 1's hand (Valor, Master and Final's second Keyblade) and its
/// model parts.
#[derive(Component)]
pub struct SecondHand;
#[derive(Component)]
pub struct SecondPart(usize);

/// Rebuilds Sora's model after a kit swap, and poses the second Keyblade.
#[allow(clippy::too_many_arguments)]
pub fn present(
    mut commands: Commands,
    mut game: ResMut<Game>,
    roots: Query<Entity, With<SoraRoot>>,
    hands: Query<Entity, (With<WeaponRoot>, Without<SoraRoot>)>,
    parts: Query<Entity, ModelParts>,
    mut seconds: Query<(Entity, &mut Transform, &mut Visibility), With<SecondHand>>,
    second_parts: Query<(&SecondPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(form) = game.drive.as_mut().and_then(|d| d.respawn.take()) else {
        // The second Keyblade follows slot 1's joint.
        let clip = game.poses[&(game.player.action, game.player.armed)];
        let frame = game.player.motion_frame(&game.clips) as usize;
        let cp = &game.clip_poses[clip];
        let hands = &cp.weapon2;
        if let Some(pose) = cp
            .weapon2_frames
            .get(frame.min(cp.weapon2_frames.len().saturating_sub(1)))
        {
            for (part, handle) in &second_parts {
                let positions: Vec<[f32; 3]> = game.weapon2_indices[part.0]
                    .iter()
                    .map(|&i| pose[i as usize].to_array())
                    .collect();
                if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                }
            }
        }
        for (_, mut t, mut v) in &mut seconds {
            if let Some(m) = hands.get(frame.min(hands.len().saturating_sub(1))) {
                *t = Transform::from_matrix(*m);
            }
            *v = if game.player.armed && !hands.is_empty() {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
        return;
    };
    let (Ok(root), Ok(hand)) = (roots.single(), hands.single()) else {
        return;
    };
    for e in &parts {
        commands.entity(e).despawn();
    }
    for (e, ..) in &seconds {
        commands.entity(e).despawn();
    }
    let g = &mut *game;
    let Some(d) = g.drive.as_ref() else { return };
    let Some((actor, weapon, weapon2)) = d.models.get(&form) else {
        return;
    };
    let count = d.weapons.get(&form).copied().unwrap_or(1);
    let pose = g.clip_poses[g.poses[&(Action::Idle, false)]].frames[0].clone();
    let mut part_indices = Vec::new();
    for (ordinal, entity) in spawn_model(
        &mut commands,
        actor,
        &pose,
        root,
        &mut meshes,
        &mut materials,
        &mut images,
    ) {
        part_indices.push(actor.parts[ordinal].indices.clone());
        if let Some(entity) = entity {
            commands.entity(entity).insert(SoraPart(ordinal));
        }
    }
    // As in setup: the weapon's own motion poses it in the hand joint's
    // space; without one its bind pose is moved into its root joint's.
    let battle_idle = &g.clip_poses[g.poses[&(Action::Idle, true)]];
    let weapon_pose: Vec<Vec3> = if battle_idle.weapon_frames.is_empty() {
        let weapon_root =
            Mat4::from_cols_array(&weapon.poses[0].matrix_bits[0].map(f32::from_bits));
        weapon.poses[0]
            .position_bits
            .iter()
            .map(|p| {
                weapon_root
                    .inverse()
                    .transform_point3(Vec3::from_array(p.map(f32::from_bits)))
            })
            .collect()
    } else {
        battle_idle.weapon_frames[0].clone()
    };
    // Slot 1's Keyblade is posed by its own motion at the same key time.
    let weapon2_pose = battle_idle
        .weapon2_frames
        .first()
        .cloned()
        .unwrap_or_else(|| weapon_pose.clone());
    let mut hands_to_fill = Vec::new();
    if count >= 1 {
        hands_to_fill.push(hand);
    }
    if count >= 2 {
        let e = commands
            .spawn((
                SecondHand,
                ChildOf(root),
                Transform::default(),
                Visibility::Hidden,
            ))
            .id();
        hands_to_fill.push(e);
    }
    let mut weapon_indices = Vec::new();
    let mut weapon2_indices = Vec::new();
    for (k, &h) in hands_to_fill.iter().enumerate() {
        let (model, pose) = if k == 0 {
            (weapon, &weapon_pose)
        } else {
            (weapon2.as_ref().unwrap_or(weapon), &weapon2_pose)
        };
        for (ordinal, entity) in spawn_model(
            &mut commands,
            model,
            pose,
            h,
            &mut meshes,
            &mut materials,
            &mut images,
        ) {
            if k == 0 {
                weapon_indices.push(model.parts[ordinal].indices.clone());
            } else {
                weapon2_indices.push(model.parts[ordinal].indices.clone());
            }
            if let Some(entity) = entity {
                if k == 0 {
                    commands.entity(entity).insert(WeaponPart(ordinal));
                } else {
                    commands.entity(entity).insert(SecondPart(ordinal));
                }
            }
        }
    }
    g.part_indices = part_indices;
    g.weapon_indices = weapon_indices;
    g.weapon2_indices = weapon2_indices;
    g.display = pose;
    g.weapon_display = weapon_pose;
    g.blend_from = None;
    g.display_joints.clear();
    g.blend_from_joints.clear();
    g.shown_action = None;
}

/// Wisdom Form's Shoot: frame trigger 1 of the attack motion fires a
/// bullet (`player::wisdom_form`); live bullets fly and join the spell
/// volumes, and a bullet that hit is removed.
pub fn shots(game: &mut Game) {
    let Game {
        drive,
        player,
        clips,
        magic,
        enemy,
        ..
    } = game;
    let (Some(d), Some(m)) = (drive.as_mut(), magic.as_mut()) else {
        return;
    };
    let hits = &m.last_hits;
    d.bullets.retain(|b| !hits.contains(&b.key));
    if let Some(l) = d.limit.as_mut() {
        for &k in hits {
            l.hit(k);
        }
        for (key, atkp, centre, radius, previous, dir) in l.volumes() {
            m.volumes.push(player::magic::Volume {
                key,
                atkp,
                sub: 0,
                centre,
                radius,
                height: radius,
                previous,
                moving: Some(dir),
                role: player::magic::Role::Damage,
            });
        }
    }
    if d.active && d.form == forms::WISDOM && d.change.is_none() {
        let clip = player.clip(clips);
        if matches!(player.action, Action::Attack(_) | Action::AirAttack(_)) {
            for &(f, n) in &clip.effects {
                if f != player.frame {
                    continue;
                }
                let Some(sub) = player::wisdom_form::shot_sub(n) else {
                    continue;
                };
                d.bullet_serial += 1;
                let p = player.position;
                // The muzzle is effect n's position [U: the effect's
                // geometry is not decoded; Sora's TARGET height here].
                let muzzle = [p[0], p[1] + m.src.target_lift, p[2]];
                let dir = [player.yaw.sin(), 0., player.yaw.cos()];
                d.bullets.push(player::wisdom_form::Bullet::new(
                    BULLET_KEYS | d.bullet_serial,
                    sub,
                    muzzle,
                    dir,
                ));
            }
        }
    }
    let target = enemy
        .as_ref()
        .filter(|rt| rt.enemy.targetable())
        .and_then(|rt| enemy_targets(rt).first().map(|t| t.centre));
    d.bullets.retain_mut(|b| b.step(target));
    for b in &d.bullets {
        m.volumes.push(player::magic::Volume {
            key: b.key,
            atkp: player::wisdom_form::ATKP,
            sub: b.sub,
            centre: b.pos,
            radius: player::wisdom_form::RADIUS,
            height: player::wisdom_form::RADIUS,
            previous: b.previous(),
            moving: Some(b.dir),
            role: player::magic::Role::Damage,
        });
    }
}

/// Wisdom bullets' volume keys (below the spells' serial keys' range).
const BULLET_KEYS: u64 = 1 << 60;

/// A party member's character id (objentry +0x4c) by its model.
fn character_of(name: &str) -> u16 {
    match name {
        "P_EX020" => 2,
        "P_EX030" => 3,
        _ => 0,
    }
}

/// FUN_140400990: an absorbed member fades out over 15 frames and is
/// then gone from view [the fade itself is not drawn: the member shows
/// until the 15th frame].
pub fn party_visibility(
    game: Res<Game>,
    mut roots: Query<(&party_room::PartyRoot, &mut Visibility)>,
) {
    let Some(d) = game.drive.as_ref() else { return };
    for (root, mut v) in &mut roots {
        let hidden = d
            .stored
            .iter()
            .any(|&(i, frames)| i == root.0 && frames >= PARTY_FADE);
        let want = if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *v != want {
            *v = want;
        }
    }
}

/// FUN_140400990's fade, in frames.
const PARTY_FADE: u32 = 16;

/// Frame trigger 1 of Sora's current motion (FUN_14040dd70 case 1): each
/// trigger frame passed this tick starts that element of the shown kit's
/// PAX, bound to him.
pub fn motion_effects(game: &mut Game) {
    let key = (game.action_started, game.player.action);
    let frame = game.player.frame;
    let clip = game.player.clip(&game.clips).clone();
    let Some(d) = game.drive.as_mut() else { return };
    let from = match d.fx_scan {
        Some((s, a, f)) if (s, a) == key => Some(f),
        _ => None,
    };
    d.fx_scan = Some((key.0, key.1, frame));
    let Some(pax) = d.paxes.get(&d.shown).cloned() else {
        return;
    };
    for &(f, n) in &clip.effects {
        let due = match from {
            Some(prev) => f > prev && f <= frame,
            None => f <= frame,
        };
        if due && n > 0 {
            d.fx_requests.push(drive_fx::FxRequest {
                pax: pax.clone(),
                id: n as u16,
                owner: d.shown,
            });
        }
    }
}

/// One tick of Limit Form's limits, after the player's: a requested limit
/// starts (FUN_1403d8b40), a pressed follow-up runs, the limit's threads
/// step, its effects start, and the filter ramps.
pub fn limit_tick(game: &mut Game, rc: bool, abilities: Option<&mut Vec<u16>>) {
    let Some(d) = game.drive.as_mut() else { return };
    d.tint_k = (d.tint_k + d.tint_step).clamp(0., 1.);
    let request = d.limit_request.take();
    let offered = d.offer.take();
    if !d.limit_form() {
        d.limit = None;
        return;
    }
    if let Some(cmd) = request
        && d.limit.is_none()
    {
        let cost = d.src.commands.get(&cmd).map_or(0, |r| r.cost);
        let colour =
            player::limit_form::colour(cost, false, game.player.mp.cur, game.player.mp_charging);
        if colour < 4 && game.player.can_command(&game.clips) {
            let l = player::limit_form::Limit::start(cmd, cost, &mut game.player, &game.clips);
            game.action_started = game.tick;
            let Some(d) = game.drive.as_mut() else { return };
            d.tint = l.tint;
            d.tint_step = 1. / player::limit_form::TINT_IN;
            d.limit = Some(l);
            let tick = game.tick;
            game.event(
                serde_json::json!({"event":"limit_start","tick":tick,"cmd":cmd,"cost":cost}),
            );
        }
    }
    let Some(d) = game.drive.as_mut() else { return };
    let Some(mut l) = d.limit.take() else { return };
    let mut exp = false;
    if rc && let Some(cmd) = offered {
        exp = l.follow_up(cmd, &mut game.player, &game.clips);
        game.action_started = game.tick;
        let tick = game.tick;
        game.event(serde_json::json!({"event":"limit_follow_up","tick":tick,"cmd":cmd}));
    }
    let targets: Vec<Vec3> = game
        .enemy
        .as_ref()
        .filter(|rt| rt.enemy.targetable())
        .map(|rt| {
            enemy_targets(rt)
                .iter()
                .map(|t| Vec3::from_array(t.centre))
                .collect()
        })
        .unwrap_or_default();
    let p = game.player.position;
    let hand = game
        .clip_poses
        .get(game.poses[&(game.player.action, game.player.armed)])
        .and_then(|c| {
            c.weapon.get(
                (game.player.motion_frame(&game.clips) as usize)
                    .min(c.weapon.len().saturating_sub(1)),
            )
        })
        .copied()
        .unwrap_or(Mat4::IDENTITY);
    let root = Mat4::from_rotation_translation(
        Quat::from_rotation_y(game.player.yaw),
        Vec3::from_array(p),
    );
    let top = (root * hand).transform_point3(Vec3::new(-78., 0., 0.));
    let scene = player::limit_form::Scene {
        targets: &targets.iter().map(|v| v.to_array()).collect::<Vec<_>>(),
        body: [p[0], p[1] + 70., p[2]],
        keyblade_top: top.to_array(),
    };
    let input = player::Input::default();
    let Some(d) = game.drive.as_mut() else { return };
    let out = l.tick(
        &mut game.player,
        &game.clips,
        &input,
        &scene,
        &game.world,
        &mut d.seed,
    );
    let pax = d.paxes.get(&forms::LIMIT).cloned();
    if let Some(pax) = pax {
        for id in out.effects {
            d.fx_requests.push(drive_fx::FxRequest {
                pax: pax.clone(),
                id,
                owner: forms::LIMIT,
            });
        }
        let _ = out.effects_at;
    }
    d.offer = out.offer;
    if out.ended {
        d.tint_step = -1. / player::limit_form::TINT_OUT;
        let tick = game.tick;
        game.event(serde_json::json!({"event":"limit_end","tick":tick}));
    } else {
        d.limit = Some(l);
    }
    if exp {
        let Some(d) = game.drive.as_mut() else { return };
        d.pending_exp += 1;
    }
    let _ = abilities;
}

/// Strike Raid's thrown Keyblade (limitform `fly`): drawn where the
/// thread holds it (the path point plus Judgment's wobble), blade along
/// its direction [I: motion 235's spin is not applied].
type HandQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Transform, &'static mut Visibility),
    (With<WeaponRoot>, Without<SoraRoot>),
>;

pub fn thrown_keyblade(
    game: Res<Game>,
    roots: Query<&Transform, (With<SoraRoot>, Without<WeaponRoot>)>,
    mut hands: HandQuery,
) {
    let Some(k) = game
        .drive
        .as_ref()
        .and_then(|d| d.limit.as_ref())
        .and_then(|l| l.keyblade)
    else {
        return;
    };
    let (Ok(root), Ok((mut t, mut v))) = (roots.single(), hands.single_mut()) else {
        return;
    };
    let at = Vec3::from_array(k.pos) + Vec3::from_array(k.offset);
    let dir = Vec3::from_array(k.dir).normalize_or(Vec3::Z);
    let world = Transform {
        translation: at,
        rotation: Quat::from_rotation_arc(Vec3::NEG_X, dir),
        scale: Vec3::ONE,
    };
    *t = Transform::from_matrix(root.to_matrix().inverse() * world.to_matrix());
    *v = Visibility::Inherited;
}

/// DAT_1405c5d08: the form items whose names title the panels.
pub const FORM_ITEMS: [u16; 6] = [0x1a, 0x1b, 0x233, 0x1f, 0x1d, 0x1e];

/// FUN_1403e8380's panels for one level: the form's (title = its item
/// name, "Form Gauge Powered Up!", the new form ability) and Sora's (title
/// "Sora", his growth ability and the bonus ability), skipped when empty.
/// An ability line is `{icon}<name>!` (msg 0xae8); the icon glyph is not
/// drawn [I: the system font here has no inline icons].
pub fn level_panels(src: &FormSources, up: &forms::LevelUp) -> Vec<visual::level_up::Entry> {
    let line = |item: u16| {
        src.item_names.get(&item).map(|n| {
            let mut v = n.clone();
            v.push(0x48);
            v
        })
    };
    let title = FORM_ITEMS
        .get(up.form as usize - 1)
        .and_then(|i| src.item_names.get(i))
        .cloned()
        .unwrap_or_default();
    let mut out = vec![visual::level_up::Entry {
        kind: visual::level_up::Kind::Form,
        form: up.form,
        level: up.level,
        title,
        line1: src.gauge_up.clone(),
        line2: up.form_growth.and_then(line),
    }];
    let sora: Vec<Vec<u8>> = [up.sora_growth, up.ability]
        .into_iter()
        .flatten()
        .filter_map(line)
        .collect();
    if let Some(first) = sora.first() {
        out.push(visual::level_up::Entry {
            kind: visual::level_up::Kind::Form,
            form: 0,
            level: up.level,
            title: src.sora_title.clone(),
            line1: first.clone(),
            line2: sora.get(1).cloned(),
        });
    }
    out
}

/// Battle voices (FUN_1403d69b0 → FUN_1403cdb60): 00battle `vtbl` rows
/// {u8 char, u8 n, u8 prio, u8 0, 5 × (i8 sound, i8 weight)} and the
/// character's voice bank.
#[derive(Clone)]
pub struct Voices {
    rows: Vec<[u8; 14]>,
    bank: u16,
    /// The voice pick's draws (the shared LCG DAT_140783ca0 [I: kept
    /// apart here]).
    seed: u32,
    pub scan: Option<(u64, Action, u32)>,
}

impl Voices {
    pub fn read(vtbl: &[u8], bank: u16) -> Option<Self> {
        let count = u32::from_le_bytes(vtbl.get(4..8)?.try_into().ok()?) as usize;
        let rows = (0..count.min(4096))
            .map(|k| {
                vtbl.get(8 + k * 14..22 + k * 14)
                    .map(|r| r.try_into().unwrap())
            })
            .collect::<Option<Vec<[u8; 14]>>>()?;
        Some(Voices {
            rows,
            bank,
            seed: 1,
            scan: None,
        })
    }

    /// voice_pick(char, n): a weighted roll over the row's sounds (0..99),
    /// else a uniform pick among the unweighted ones. Returns the sound
    /// request id.
    pub fn pick(&mut self, character: u8, n: i16) -> Option<u32> {
        let row = *self
            .rows
            .iter()
            .find(|r| r[0] == character && r[1] as i16 == n)?;
        self.seed = self.seed.wrapping_mul(0x10dcd);
        let mut r = (self.seed / 0x28f_5c29) as i32;
        let mut cand = Vec::new();
        for k in 0..5 {
            let (idx, weight) = (row[4 + k * 2] as i8, row[5 + k * 2] as i8);
            if idx < 0 {
                continue;
            }
            if weight < 1 {
                cand.push(idx);
            } else {
                r -= weight as i32;
                if r < 0 {
                    return Some(self.bank as u32 * 1000 + idx as u32);
                }
            }
        }
        if cand.is_empty() {
            return None;
        }
        self.seed = self.seed.wrapping_mul(0x10dcd);
        let len = cand.len() as u64;
        let i = (self.seed as u64 / (1u64 << 32).div_ceil(len)) as usize;
        Some(self.bank as u32 * 1000 + cand[i.min(cand.len() - 1)] as u32)
    }
}
