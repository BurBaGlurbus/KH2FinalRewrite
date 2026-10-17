//! Playable source-bound test room: Sora walks/runs, jumps/lands and attacks
//! with motions read from the selected Steam installation.
//!
//! Every launch checks the selected source (executable and indexes rehashed,
//! each package byte read verified against the full-scan record), then decodes the
//! room, Sora's model/motion set and his PRTY/SSTM/PTYA records from it. The
//! controller (`player`) is a test-room profile: data-backed where a
//! public lead exists, explicit choices elsewhere. It is not a measured
//! reproduction of the installed game's movement or combat rules.
use crate::area::{self, AreaSource, LoadedArea, MapWorld};
use bevy::{
    app::AppExit,
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    core_pipeline::tonemapping::{DebandDither, Tonemapping},
    image::ImageSampler,
    input::mouse::AccumulatedMouseMotion,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    render::view::{
        NoIndirectDrawing,
        screenshot::{Screenshot, ScreenshotCaptured},
    },
};
use data::{
    bar, model_inspection, model_inspection_textures::TextureMode, motion, object_collision,
    object_effect, objentry, original_payload, party_records, probe_source::ProbeSource, ptya,
    sklt, sprite_sequence,
};
use player::shadow::{self, ScriptRandom, Shadow};
use player::{
    Action, Clip, Clips, Input, Params, Player, TriangleWorld, World,
    follow_camera::{self, CameraInput, FollowCamera},
};
use player::{abilities, actor_physics, combat, magic};
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use visual::area_fog::{AddFogged, FogMaterial, FogPlugin};
use visual::controller::{self, Pad};
use visual::enemy_gauge::EnemyGauge;
use visual::fade::{self, Fade};
use visual::field_command_menu::{self, CommandMenu, Decision, Item};
use visual::gauge::{Canvas, Draw, Gauge, GaugeStats, Texture};
use visual::hud::{self, Sheet, SystemFont};
use visual::input;
use visual::menu_screen::{self, Menu2ld};
use visual::overlay::{Ui2dFrame, Ui2dPlugin};
use visual::pause_menu::journal::{self, Curtain, Journal};
use visual::pause_menu::{self, Camp};
use visual::room::{CollisionTriangle, LoadedRoom, load_rooms, mesh, rgba_image};
use visual::sound::{self, Banks};

#[derive(Clone, Deserialize)]
struct Locator {
    index: String,
    ordinal: usize,
    name_hash: [u8; 16],
}
mod ability_inventory;
mod character_kit;
mod debug_menu;
mod drive_fx;
mod drive_room;
mod enemy_battle;
mod enemy_battle_hits;
mod enemy_battle_tour;
mod enemy_catalog;
mod enemy_combat;
mod enemy_gallery;
mod enemy_motion_fx;
mod enemy_script;
mod enemy_target;
mod enemy_tour;
mod game_over;
mod gear;
mod gear_tour;
mod item_drop_view;
mod item_room;
mod journal_tour;
mod level_growth;
mod magic_room;
mod magic_tour;
mod party_room;
mod prize_room;
mod settings_room;
mod spell_fx;
mod weapon_trail;

fn player_object_default() -> String {
    "P_EX100".into()
}

#[derive(Deserialize)]
struct Config {
    schema_version: u32,
    source_content_identity: String,
    /// Paths relative to this configuration file.
    /// A game room (map) selection; omitted when `arena` is used.
    #[serde(default)]
    room_selection: Option<PathBuf>,
    /// A generated test arena instead of a game room.
    #[serde(default)]
    arena: Option<ArenaConfig>,
    /// A field area loaded by name the way the game loads it.
    #[serde(default)]
    area: Option<AreaConfig>,
    actor_selection: PathBuf,
    motion_set: Locator,
    battle_mode: bool,
    has_weapon: bool,
    battle: Locator,
    ptya_pointer: usize,
    /// Superseded: the attack is chosen from the whole PTYA set
    /// (player::ptya); kept only to read older configurations.
    #[serde(default)]
    ground_combo_ids: Vec<u8>,
    #[serde(default)]
    air_combo_ids: Vec<u8>,
    system: Locator,
    /// The player's own 00objentry model name; its +0x4e PRTY index selects
    /// the movement record (FUN_1403da940 → FUN_14040a480).
    #[serde(default = "player_object_default")]
    player_object_name: String,
    /// 00objentry locator and the object model name whose WeaponJoint byte
    /// names the actor joint carrying the weapon. Weapon-type entries carry
    /// a hand-like joint while player entries carry 1; that reading is a
    /// test-room inference from installed records, not observed routing.
    objentry: Locator,
    object_model_name: String,
    weapon_selection: PathBuf,
    /// The weapon's own motion set (W_EX010.mset); its clips share Sora's
    /// slot numbers and pose the weapon in the hand joint's space.
    #[serde(default)]
    weapon_motion_set: Option<Locator>,
    /// Character id (public dictionary: 1 = Sora) whose SKLT primary bone
    /// in the system archive carries the weapon. Without it the objentry
    /// byte above is used.
    #[serde(default)]
    sklt_character_id: Option<u32>,
    /// Field 2D sequence archive holding the 'gaug' HUD sequence
    /// (field2d/<lang>/<world>0field.2dd).
    #[serde(default)]
    hud: Option<Locator>,
    /// Character archive holding the 'face' sequence (obj/P_EX100.a.<lang>).
    #[serde(default)]
    face: Option<Locator>,
    /// Command menu sources.
    #[serde(default)]
    menu: Option<MenuSources>,
    /// Target enemy (test-room choice of placement).
    #[serde(default)]
    enemy: Option<EnemyConfig>,
    /// Optional spawn (x, probe height, z) in scene coordinates.
    #[serde(default)]
    spawn: Option<[f32; 3]>,
    /// Facing at that spawn (scene yaw); areas default to the entrance's.
    #[serde(default)]
    spawn_yaw: Option<f32>,
    /// Sound banks: `se/seNNN.win32.scd` entries, or object archives with
    /// the name of their `se/obj/seNNN` replacement asset.
    #[serde(default)]
    sounds: Vec<BankSource>,
    /// Effect sources: object archives holding the Keyblade's and the
    /// enemy's PAX, and `effect/texcommon.dpd` (the common atlas).
    #[serde(default)]
    effects: Option<EffectConfig>,
    /// A field NPC to talk to (needs `menu` for the reaction label and
    /// `system` for the text colour).
    #[serde(default)]
    npc: Option<NpcConfig>,
    /// The camp (pause) menu: menu/<lang>/camp.2ld (needs `menu`).
    #[serde(default)]
    camp: Option<Locator>,
    /// Jiminy's Journal (needs `camp`).
    #[serde(default)]
    journal: Option<JournalConfig>,
    /// The room's field and battle music (03system 'arif': tt08 set 0 =
    /// bgm/music118 and music119).
    #[serde(default)]
    music: Option<RoomMusic>,
}

/// The journal's files: menu/<lang>/jmemo.2ld (the book), jiminy.bar (the
/// tables), msg/<lang>/jm.bar (message slot 0x65), menu/jm_world.2ld (the
/// world icons), menu/jm_top_<code>.2ld by code and the menu-chain curtain
/// menu/<lang>/pause.2ld.
#[derive(Clone, Deserialize)]
struct JournalConfig {
    book: Locator,
    tables: Locator,
    messages: Locator,
    worlds: Locator,
    tops: std::collections::BTreeMap<String, Locator>,
    curtain: Locator,
    /// Story flags set in the save [test-room value]; 0x4837 is the exe's
    /// "show everything" flag.
    #[serde(default)]
    flags: Vec<u16>,
    /// Opened-chest bits (save+0x23ac) and owned map items [test-room
    /// values].
    #[serde(default)]
    opened_bits: Vec<u16>,
    #[serde(default)]
    owned_items: Vec<u16>,
}

/// The area to start in and the story point that fixes its programs.
#[derive(Deserialize)]
struct AreaConfig {
    /// World code (`tt`).
    world: String,
    area: u8,
    entrance: u8,
    /// Progress flag ranges set in ascending order before loading.
    story: Vec<[u32; 2]>,
    /// Unattended runs stand still instead of walking to an exit.
    #[serde(default)]
    stay: bool,
    /// Program overrides [map, btl, evt] for the first load (−1 = story).
    #[serde(default = "no_programs")]
    programs: [i16; 3],
}

fn no_programs() -> [i16; 3] {
    [-1; 3]
}

#[derive(Deserialize)]
struct RoomMusic {
    field: Locator,
    battle: Locator,
}

/// Field-talk test NPC. In vanilla no field talk has Axel speaking, so
/// three values are composed (dialogue-01 §7B): the talk reaction (cmd 31
/// via the towns-person reaction sphere), the talk camera shape (the TARGET
/// shape of `target_model`) and the script (`script`, libretto op 1 per
/// message id).
#[derive(Deserialize)]
struct NpcConfig {
    /// Model selection (relative to this configuration): obj/B_EX110_RTN.mdlx.
    model_selection: PathBuf,
    /// Its motion set (direct slots; slot 0 is the idle 'R00A').
    motion_set: Locator,
    /// Archive whose object collision has the TARGET shape used for the
    /// camera (obj/B_EX110.mdlx shares the skeleton).
    target_model: Locator,
    /// Placement relative to Sora's spawn (scene units) and facing (rad).
    offset: [f32; 3],
    yaw: f32,
    /// Message archive with the script's lines (msg/us/hb.bar for Axel).
    messages: Locator,
    script: Vec<u32>,
    /// msg/<lang>/fontinfo.bar, fontimage.bar and the world message bar.
    font_info: Locator,
    font_image: Locator,
    world_messages: Locator,
    /// World index into the ftst colour table (tt = 2).
    world: usize,
    /// Composed talk-camera shape (the type-19 role) used instead of the
    /// TARGET fallback: the towns-person dimensions on the head bone.
    #[serde(default)]
    camera_shape: Option<ShapeConfig>,
}

#[derive(Deserialize)]
struct ShapeConfig {
    bone: usize,
    offset: [f32; 3],
    pos_h: f32,
    radius: f32,
    height: f32,
}

#[derive(Deserialize)]
struct EffectConfig {
    weapon: Locator,
    #[serde(default)]
    enemy: Option<Locator>,
    atlas: Locator,
    /// 00effect.bar (the system PAX: guard and clash sparks).
    #[serde(default)]
    system: Option<Locator>,
}

struct EffectSources {
    weapon: object_effect::Pax,
    enemy: Option<object_effect::Pax>,
    /// The other battle enemies' object effects by objentry id.
    enemy_paxes: Vec<(u32, object_effect::Pax)>,
    system: Option<object_effect::Pax>,
    atlas: visual::effects::Atlas,
}

/// Effects raised by combat this tick.
enum EffectRequest {
    /// Keyblade hit spark (FUN_1403d1b40): ATKP hit effect and random
    /// count, contact point and hit direction in scene coordinates.
    Hit {
        effect: u8,
        random: u8,
        contact: Vec3,
        direction: Vec3,
    },
    /// An enemy attack landing on Sora (FUN_1403d1b40 with the attacker's
    /// own container): `object` is the attacker's objentry id (0 = the
    /// room's enemy); the spark starts at the contact point.
    EnemyHit {
        object: u32,
        effect: u8,
        random: u8,
        contact: Vec3,
        direction: Vec3,
    },
    /// A battle enemy's motion frame trigger 1: element `id` of its own
    /// container, bound to the actor.
    ActorElement { object: u32, actor: u64, id: u16 },
    /// The same element placed at a point by a script (`obj_effect_start_pos`).
    ActorElementAt { object: u32, id: u16, at: Vec3 },
    /// The enemy's `dead` act: effect_start_bind(obj, 2).
    EnemyDead,
    /// A system (00effect) element at a point, unrotated
    /// (FUN_1403d0f00 clash effects).
    System { id: u16, at: Vec3 },
}

struct RunningEffect {
    instance: ::effect::Instance,
    label: &'static str,
    /// Element transform kept for effects bound to the enemy.
    bound: Option<object_effect::Element>,
}

/// Running effect instances and the quads they drew this tick.
#[derive(Resource)]
struct Effects {
    weapon: Option<Arc<object_effect::Pax>>,
    enemy: Option<Arc<object_effect::Pax>>,
    enemy_paxes: HashMap<u32, Arc<object_effect::Pax>>,
    system: Option<Arc<object_effect::Pax>>,
    running: Vec<RunningEffect>,
    rng: ::effect::Rng,
    quads: Vec<::effect::Quad>,
    /// The view matrix the quads were drawn with, and its inverse.
    view_inverse: Mat4,
    uv_scale: [f32; 2],
    reported: Vec<u32>,
}

#[derive(Component)]
struct EffectMesh {
    additive: bool,
}

#[derive(Deserialize)]
struct BankSource {
    #[serde(flatten)]
    locator: Locator,
    #[serde(default)]
    replacement: Option<String>,
}

#[derive(Deserialize)]
struct ArenaConfig {
    half_size: f32,
    wall_height: f32,
    tile: f32,
}

impl ArenaConfig {
    /// Development arena dimensions; these do not describe a KH2 room.
    /// Expand saved configurations too, without overwriting the user's file.
    fn expanded(&self) -> Result<Self, &'static str> {
        if !self.half_size.is_finite()
            || !self.wall_height.is_finite()
            || !self.tile.is_finite()
            || !(1. ..=12_000.).contains(&self.half_size)
            || !(1. ..=6_000.).contains(&self.wall_height)
            || !(16. ..=4_000.).contains(&self.tile)
        {
            return Err("arena dimensions outside bounded development profile");
        }
        Ok(Self {
            half_size: self.half_size.max(4500.),
            wall_height: self.wall_height.max(800.),
            tile: self.tile.max(300.),
        })
    }
}

fn standard_difficulty() -> u8 {
    1
}

#[derive(Deserialize)]
struct EnemyConfig {
    /// DAT_1409abd48: 0 Beginner, 1 Standard (default), 2 Proud, 3 Critical.
    #[serde(default = "standard_difficulty")]
    difficulty: u8,
    /// Model selection (relative to this configuration), e.g. M_EX020.mdlx.
    model_selection: PathBuf,
    /// The same mdlx as a locator, for its object-collision table.
    model: Locator,
    /// Its motion set (direct slot = motion id).
    motion_set: Locator,
    /// Weapon mdlx (W_EX010) for the Keyblade's attack spheres.
    weapon_model: Locator,
    /// Sora's mdlx (P_EX100) for his HIT spheres.
    actor_model: Locator,
    enmp_id: u16,
    /// World battle level used when the ENMP level is 0 (test-room choice).
    battle_level: u16,
    /// Placement relative to Sora's spawn, in scene units.
    offset: [f32; 3],
}

#[derive(Clone, Deserialize)]
struct MenuSources {
    /// field2d/<lang>/<world><n>command.2dd ('comw' sequence).
    command: Locator,
    /// msg/<lang>/fontimage.bar and msg/<lang>/fontinfo.bar.
    font_image: Locator,
    font_info: Locator,
    /// msg/<lang>/sys.bar (command labels).
    messages: Locator,
}

/// HUD sources loaded with the session.
struct HudSource {
    sheets: Vec<Sheet>,
    /// The field 2D archive's 'fiel' sheet: the lock-on reticle.
    field: Sheet,
    /// The field 2D archive's 'mapj' area-change iris (areas only).
    iris: Option<Sheet>,
    stats: (i32, i32),
    menu: Option<(Sheet, SystemFont, Vec<Vec<u8>>)>,
}

/// Top-menu labels: Attack, Magic, Items, Drive (03system 'cmd' message ids).
const MENU_MESSAGES: [u32; 4] = [0x81e0, 0x81e1, 0x81e2, 0x81e3];

/// Sora's damage motions (10..19, Aerial Recovery 40, DEAD 54), in
/// `Clips::hurt` order, then the item-use motion (9).
const SORA_HURT: [u16; 13] = {
    let mut m = [9u16; 13];
    let mut i = 0;
    while i < 12 {
        m[i] = player::HURT_MOTIONS[i];
        i += 1;
    }
    m
};
/// How many of `SORA_HURT` are damage motions.
const HURT_COUNT: usize = 12;

/// Sora's item slots in a fresh room [test-room values]: a Potion and an
/// Ether, taken out of the stock.
const ROOM_ITEM_SLOTS: [u16; 2] = [1, 3];
fn room_stock() -> std::collections::BTreeMap<u16, u8> {
    let mut s: std::collections::BTreeMap<u16, u8> = ROOM_STOCK.into_iter().collect();
    for id in ROOM_ITEM_SLOTS {
        if let Some(n) = s.get_mut(&id) {
            *n = n.saturating_sub(1);
        }
    }
    s
}

/// Guard (PTYA 0x0b) and its success recoil (guard-01).
const SORA_GUARD: [u16; 2] = [173, 42];

/// Public MotionSet names for the locomotion IDs used here.
const LOCOMOTION: [(&str, u16); 6] = [
    ("IDLE", 0),
    ("WALK", 1),
    ("RUN", 2),
    ("JUMP", 3),
    ("FALL", 4),
    ("LAND", 5),
];
/// Test-room choices, recorded in the startup log.
const STEP_HEIGHT: f32 = 24.;
/// Run at stick magnitude 0.8 (DAT_140585c7c).
const WALK_THRESHOLD: f32 = actor_physics::RUN_MAGNITUDE;
const BODY_RADIUS: f32 = 30.;
const BODY_HEIGHT: f32 = 140.;
const FLOOR_MIN_NORMAL_Y: f32 = 0.5;
/// Test-room choice: put the weapon away after 6 s without attacking.
const DISARM_TICKS: u32 = 360;
const PARTY_STAND: u32 = 1 << 0;
const PARTY_COLLIDE: u32 = 1 << 4;

/// What a joint-space blend of two shown poses needs: the actor's skinning
/// and parents, and the joint carrying the Keyblade.
struct Skin {
    skinning: data::model_pose::Skinning,
    parents: Vec<i32>,
    weapon_joint: usize,
}

/// Precomputed skinned positions for every frame of one clip.
struct ClipPoses {
    frames: Vec<Vec<Vec3>>,
    /// The evaluated joints of each frame with the root motion removed
    /// (empty for basic FK), blended joint by joint when a motion switch
    /// cross-fades.
    joints: Vec<Vec<data::motion_eval::World>>,
    skin: Option<Arc<Skin>>,
    /// Model-space matrix of the objentry weapon joint for each frame.
    weapon: Vec<Mat4>,
    /// The same for weapon slot 1's joint (forms with two Keyblades).
    weapon2: Vec<Mat4>,
    /// Weapon vertices in hand-joint space per frame, from the weapon's own
    /// motion at the same slot and key time (empty when it has none).
    weapon_frames: Vec<Vec<Vec3>>,
    /// The same for weapon slot 1's Keyblade.
    weapon2_frames: Vec<Vec<Vec3>>,
    /// Root-position curve translation for each frame (model space).
    root: Vec<Vec3>,
    /// Keyblade bone 2 matrix in hand-joint space per frame (attack spheres).
    blade: Vec<Mat4>,
    blade2: Vec<Mat4>,
    /// Attack windows from the weapon motion's range trigger 10:
    /// (start, end, ATKP id, collision group), in clip ticks; then slot 1's.
    hits: Vec<(u32, u32, u16, u8)>,
    hits2: Vec<(u32, u32, u16, u8)>,
    /// Trail windows (the weapon motion's range trigger 12), in clip ticks.
    trails: Vec<(u32, u32)>,
    /// The actor motion's own range-10 windows (the drive motions' ATKP
    /// 124 / 125), on the object's attack shapes.
    actor_hits: Vec<(u32, u32, u16, u8)>,
    /// Sora's HIT-bone matrices per frame (model space, root removed).
    body: Vec<HashMap<usize, Mat4>>,
    /// Weapon motion frame trigger 8 ("play SE"): (clip tick, sound id).
    sounds: Vec<(u32, u32)>,
}

/// Frame trigger 8 (FUN_14040dd70): id = p0·1000 + p1; with more than
/// three parameters and p3 set, only when p0 is the equipped weapon's bank.
fn sound_triggers(selected: &motion::SelectedMotion, weapon_bank: i16) -> Vec<(u32, u32)> {
    selected
        .triggers
        .iter()
        .flat_map(|t| &t.records.frames)
        .filter(|f| f.trigger == 8 && f.parameters.len() >= 2)
        .filter(|f| {
            f.parameters.len() <= 3 || f.parameters[3] == 0 || f.parameters[0] == weapon_bank
        })
        .filter(|f| f.parameters[0] >= 0 && f.parameters[1] >= 0)
        .map(|f| {
            (
                f.frame.max(0) as u32,
                f.parameters[0] as u32 * 1000 + f.parameters[1] as u32,
            )
        })
        .collect()
}

/// Sora's footstep bank in a room: the 12soundinfo row of the room's own
/// world, None for any other world or a room without a row.
fn step_room_bank(
    info: &Option<(u8, visual::footsteps::WorldInfo)>,
    world: u8,
    area: u8,
) -> Option<i16> {
    info.as_ref()
        .filter(|(w, _)| *w == world)
        .and_then(|(_, i)| i.sora_bank(area))
}

/// The room's ambient bed (a looped sound) from the same row.
fn room_ambient(
    info: &Option<(u8, visual::footsteps::WorldInfo)>,
    world: u8,
    area: u8,
) -> Option<u32> {
    info.as_ref()
        .filter(|(w, _)| *w == world)
        .and_then(|(_, i)| i.ambient(area))
}

/// Kingdom Key sound bank (its SEB stub's bank number, sound-01 §1.1).
const WEAPON_BANK: i16 = 500;
/// Largest sound bank read whole (the ambient banks exceed the 32 MiB
/// default).
const SOUND_BANK_LIMIT: u32 = 256 * 1024 * 1024;
/// Mixer key flag of the room's ambient bed, so that its stop and fade never
/// reach a sound request that has the same id.
const AMBIENT_KEY: u32 = 0x4000_0000;
/// System bank (se000, engine slot 2) sounds: cursor move, sub-menu open,
/// close, low-HP alarm (magicsfx-01 §5).
const SE_CURSOR: u32 = 1;
const SE_OPEN: u32 = 3;
const SE_CLOSE: u32 = 4;
const SE_LOW_HP: u32 = 9;
/// Manual lock engaged, and released by the toggle (FUN_1401dd170(7) / (8) in
/// the lock manager, FUN_1403dd650 / FUN_1403dcd70).
const SE_LOCK: u32 = 7;
const SE_UNLOCK: u32 = 8;

/// The room collision the controller and the enemy read: the generated
/// arena's triangles, or a game map's COCT through the exe's routines.
enum RoomWorld {
    Triangles(TriangleWorld),
    Map(MapWorld),
}

impl RoomWorld {
    /// The collision triangles in scene coordinates (for the skateboard).
    pub fn skate_triangles(&self) -> Vec<[Vec3; 3]> {
        match self {
            RoomWorld::Triangles(w) => w.triangles().map(|t| t.map(Vec3::from_array)).collect(),
            RoomWorld::Map(w) => w.triangles(),
        }
    }
}

impl World for RoomWorld {
    fn floor(&self, x: f32, z: f32, top: f32) -> Option<f32> {
        match self {
            RoomWorld::Triangles(w) => w.floor(x, z, top),
            RoomWorld::Map(w) => w.floor(x, z, top),
        }
    }
    fn resolve_walls(&self, position: [f32; 2], radius: f32, low: f32, high: f32) -> [f32; 2] {
        match self {
            RoomWorld::Triangles(w) => w.resolve_walls(position, radius, low, high),
            RoomWorld::Map(w) => w.resolve_walls(position, radius, low, high),
        }
    }
    fn collide(
        &self,
        start: player::Vec3,
        displacement: player::Vec3,
        airborne: bool,
        body: &player::Body,
    ) -> player::Collided {
        match self {
            RoomWorld::Triangles(w) => w.collide(start, displacement, airborne, body),
            RoomWorld::Map(w) => w.collide(start, displacement, airborne, body),
        }
    }
}

/// Window art, font, text colour and the Talk label for field talks.
struct AreaTalk {
    sheet: Sheet,
    font: SystemFont,
    colour: u32,
    label: Vec<u8>,
}

/// The current area and the files to load the next one.
#[derive(Resource)]
struct AreaRes {
    source: AreaSource,
    current: LoadedArea,
    /// The area before this one (unattended runs avoid going back).
    previous: Option<u8>,
    /// A jump requested by an exit: world, area, entrance.
    pending: Option<(u8, u8, u8)>,
    /// The area-change presentation in progress.
    transition: Option<Transition>,
    /// Unattended runs walk toward an exit (false = stand still).
    explore: bool,
    /// NPC slots that already have meshes.
    rendered: usize,
    /// The live background group mask (ARD scripts plus zone switches).
    bg_mask: u64,
    /// Music to switch at the next tick (taken by `area_music`).
    music: Option<area::MusicChange>,
    /// The place name banner (text, frames left) shown on entering an
    /// area (BILLBOARD_PLACE_M: 120 frames).
    banner: Option<(Vec<u8>, u32)>,
    /// The exit's destination name while facing a guide zone.
    guide: Option<Vec<u8>>,
    /// Ticks left of a story-block disable reason.
    guide_hold: u32,
    /// Areas (world, area) Sora has stood in (their names show instead
    /// of "???").
    visited: std::collections::HashSet<(u8, u8)>,
    /// The townsperson the talk resource currently points at.
    talk_npc: Option<usize>,
    was_talking: bool,
}

/// Same-world area change (FUN_140156520 mode 0 / FUN_1401561b0): the
/// 'mapj' iris closes (group 1, 71 frames), a 2-frame fade to black
/// (FUN_140156ce0(fade, 1)), the next area loads, then a fade in
/// (128, 128, 64, 0) while the iris opens (group 0) [exit-01 §4].
struct Transition {
    stage: Stage,
    iris: Option<sprite_sequence::Player>,
    fade: Fade,
    target: (u8, u8, u8),
    /// The next area, decoded on a worker thread while the iris closes
    /// (the screen holds black until it is done).
    job: Option<std::thread::JoinHandle<AreaJob>>,
}

/// The decoded area, the fork that decoded it and the decode time (ms).
type AreaJob = Result<(LoadedRoom, LoadedArea, AreaSource, f64), String>;

/// Starts decoding an area on a worker thread with a fork of the source.
fn start_area_job(
    source: &AreaSource,
    (world, area, entrance): (u8, u8, u8),
) -> Option<std::thread::JoinHandle<AreaJob>> {
    let mut fork = match source.fork() {
        Ok(fork) => fork,
        Err(e) => {
            eprintln!("Area {world}/{area}: background load unavailable ({e}); loading in place.");
            return None;
        }
    };
    std::thread::Builder::new()
        .name("area load".into())
        .spawn(move || {
            let started = Instant::now();
            let (room, loaded) = fork
                .load(world, area, entrance, [-1; 3])
                .map_err(|e| e.to_string())?;
            Ok((room, loaded, fork, started.elapsed().as_secs_f64() * 1e3))
        })
        .ok()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stage {
    Closing,
    Black,
    Loading,
    Opening,
}

/// The scene root every room part hangs under.
#[derive(Resource)]
struct SceneRoot(Entity);

#[derive(Component)]
struct RoomPart;

/// A scrolling chunk's UV animation (UVSC record, draw-01 §7).
#[derive(Component)]
struct UvScroll {
    material: Handle<FogMaterial>,
    speed: [f32; 2],
    ratio: [f32; 2],
    scale: [f32; 2],
}

/// TEXA workers of the current area (draw-02 section 1): frame scripts
/// that swap sprites into an environment's texture.
#[derive(Resource)]
struct TexaWorkers {
    list: Vec<TexaWorker>,
    last_tick: u64,
}

struct TexaWorker {
    image: Handle<Image>,
    texa: visual::room::Texa,
    /// Crop origin of the environment's cropped copy.
    origin: [u32; 2],
    player: visual::room::TexaPlayer,
    shown: Option<usize>,
}

fn texa_tick(
    game: Res<Game>,
    workers: Option<ResMut<TexaWorkers>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut workers) = workers else { return };
    let ticks = if workers.last_tick == u64::MAX {
        1
    } else {
        game.tick.saturating_sub(workers.last_tick).min(4)
    };
    workers.last_tick = game.tick;
    for w in &mut workers.list {
        let mut sprite = None;
        for _ in 0..ticks {
            sprite = w.player.tick(&w.texa);
        }
        if ticks == 0 || sprite.is_none() || sprite == w.shown {
            continue;
        }
        if let Some(mut image) = images.get_mut(&w.image) {
            let (width, height) = (image.width(), image.height());
            if let Some(data) = image.data.as_mut() {
                visual::room::texa_upload(
                    &w.texa,
                    sprite.unwrap_or(0),
                    w.origin,
                    width,
                    height,
                    data,
                );
            }
            w.shown = sprite;
        }
    }
}

/// UV scroll of the map's chunks: counts accumulate per tick as
/// `(int)speed`, the offset is `((ratio * acc) >> 16 & 0xfff) / 4096`.
fn uv_scroll_tick(
    game: Res<Game>,
    scrolls: Query<&UvScroll>,
    mut materials: ResMut<Assets<FogMaterial>>,
) {
    let tick = game.tick as i64;
    for s in &scrolls {
        let off = |speed: f32, ratio: f32| -> f32 {
            let acc = tick * (speed as i32 as i64);
            ((((ratio * acc as f32) as i64) >> 16) & 0xfff) as f32 / 4096.
        };
        let (u, v) = (off(s.speed[0], s.ratio[0]), off(s.speed[1], s.ratio[1]));
        let transform =
            bevy::math::Affine2::from_translation(Vec2::new(u * s.scale[0], v * s.scale[1]));
        // Only a changed offset touches the material (a write re-prepares it).
        if materials
            .get(&s.material)
            .is_some_and(|m| m.base.uv_transform != transform)
            && let Some(mut m) = materials.get_mut(&s.material)
        {
            m.base.uv_transform = transform;
        }
    }
}

/// A map chunk's background group bit (`NO_GROUP` = always drawn).
#[derive(Component)]
struct BgChunk(u8);

#[derive(Resource)]
struct Game {
    params: Params,
    /// Builds the player's ability record from equipped slots.
    ability_source: AbilitySource,
    /// Current equipment feeding the battle ability rebuild, separate from decoded data.
    equipment: pause_menu::items::Equipment,
    /// What that equipment adds to Sora's Strength, Magic, Defense, AP and
    /// resists (FUN_1403c1d90); the level and record numbers excluded.
    gear: player::equipment_stats::Totals,
    /// The member's 80 ability slots persist even when no camp UI is loaded.
    ability_slots: Vec<u16>,
    ability_tour: Option<ability_inventory::Tour>,
    enemy_tour: bool,
    enemy_battle_tour: bool,
    /// `--magic-tour`: the scripted spell and combo run (magic_tour.rs).
    magic_tour: bool,
    /// The Drive form the tour casts in (`--magic-form`, 0 = none) and the
    /// ticks its change takes before the first cast.
    magic_form: u8,
    magic_shift: u64,
    /// The unattended `--growth-tour` (level-up and bonus panels).
    growth_tour: bool,
    /// The unattended `--prize-tour` (prize sprites and the heal effect).
    prize_tour: bool,
    /// The unattended `--npc-tour` (Sora walks into a townsperson).
    npc_tour: bool,
    /// The unattended `--journal-tour` (the journal's Limits pages).
    journal_tour: bool,
    /// The unattended `--badge-tour` (an item pickup, then the camp's NEW! badges).
    badge_tour: bool,
    hero_hit_tick: Option<u64>,
    enemy_incoming_hits: u64,
    enemy_outgoing_hits: u64,
    lock_queued: bool,
    cycle_queued: bool,
    selected_target: Option<[f32; 3]>,
    target_locked: bool,
    player_target_shape: Option<object_collision::Entry>,
    clips: Clips,
    world: RoomWorld,
    player: Player,
    poses: HashMap<(Action, bool), usize>,
    clip_poses: Vec<ClipPoses>,
    part_indices: Vec<Vec<u32>>,
    tick: u64,
    log: Option<File>,
    limit: Option<u64>,
    autoplay: Option<PathBuf>,
    captures_taken: usize,
    capture_pending: bool,
    /// The room's pad record (logical bits, pad mode 1 in the field) and
    /// this tick's PADDATA (stick bytes).
    pad: Pad,
    pad_data: controller::PadData,
    jump_queued: bool,
    attack_queued: bool,
    guard_queued: bool,
    /// L1-held shortcut presses of this tick (bit k = slot k) and the
    /// shortcut command ids (camp Customize).
    shortcut_queued: u8,
    shortcuts: [u16; 4],
    /// The back button (Circle, logical bit 3) pressed this tick.
    back_queued: bool,
    /// Test-room aid (F4): a blow-reaction hit with no damage, to try the
    /// blow, Aerial Recovery and Retaliating Slash without a blowing enemy.
    debug_blow: bool,
    /// What the debug menu (F3) left switched on.
    cheats: debug_menu::Cheats,
    /// Sora's magic: owned spells, running casts and their volumes.
    magic: Option<magic_room::MagicRt>,
    /// The spell scripts' effect calls of this tick and the effects' state
    /// after the last effect update (`spell_fx`).
    fx_calls: Vec<spell_fx::Call>,
    /// The party's AI may cast (Donald's spells).
    party_spells: bool,
    fx_live: HashMap<u64, magic::FxLive>,
    /// The party slot the Magic menu's Cure target list picked (0 Sora)
    /// and the Magic row Cure was chosen from, while that list is open.
    cure_target: usize,
    cure_pick: Option<usize>,
    /// Donald and Goofy, following Sora.
    party: Vec<party_room::MemberRt>,
    items: Option<item_room::ItemRt>,
    /// Drive forms (None when the form tables could not be read).
    drive: Option<drive_room::DriveRt>,
    /// Sora's battle voices: 00battle `vtbl`, the voice bank and the scan.
    voices: Option<drive_room::Voices>,
    /// Footsteps (frame trigger 2): the world's 12soundinfo, the current
    /// room's Sora bank, the step counter and the clip scan.
    step_info: Option<(u8, visual::footsteps::WorldInfo)>,
    step_bank: Option<i16>,
    /// The room's ambient bed request id (12soundinfo bank and number).
    ambient: Option<u32>,
    steps: visual::footsteps::Steps,
    step_scan: Option<(u64, Action, u32)>,
    /// The map's own effects, whose category elements are the ground dust.
    map_pax: Option<Arc<object_effect::Pax>>,
    dust_scan: Option<(u64, Action, u32)>,
    /// The scan of the actor's own frame-8 sounds.
    move_se_scan: Option<(u64, Action, u32)>,
    /// The shown kit's attack shapes by collision group (bone, offset,
    /// radius).
    sora_attack: HashMap<u8, Vec<(usize, Vec3, f32)>>,
    menu_up_queued: bool,
    menu_down_queued: bool,
    enemy: Option<EnemyRuntime>,
    display: Vec<Vec3>,
    /// The clip frame the shown pose came from (research logging).
    display_frame: usize,
    /// Pose, Keyblade transform and start tick of the active crossfade.
    blend_from: Option<(Vec<Vec3>, Transform, u64)>,
    /// The joints behind `display` and behind `blend_from`'s pose (empty
    /// when the shown clip has none), for joint-space blends.
    display_joints: Vec<data::motion_eval::World>,
    blend_from_joints: Vec<data::motion_eval::World>,
    display_hand: Transform,
    weapon_indices: Vec<Vec<u32>>,
    /// The same for slot 1's Keyblade (the form's own).
    weapon2_indices: Vec<Vec<u32>>,
    /// The equipped Keyblade's own effects once a kit was built for it (None:
    /// the room's default), and whether the effect sources must take it.
    weapon_pax: Option<Arc<object_effect::Pax>>,
    weapon_pax_changed: bool,
    weapon_display: Vec<Vec3>,
    shown_action: Option<(Action, u64)>,
    action_started: u64,
    exit_code: Option<u8>,
    show_collision: bool,
    rendered_frames: u64,
    autoplay_failed: bool,
    /// Sound requests raised this tick: (id, cause).
    sound_queue: Vec<(u32, &'static str)>,
    /// Action start tick and clip tick already scanned for sound triggers.
    sound_frame: Option<(u64, u32)>,
    effect_queue: Vec<EffectRequest>,
    /// Elements to start next tick at a place other than Sora's own.
    fx_queue: Vec<drive_fx::Placed>,
    /// Battle actors' scene position and yaw this tick, and the motion
    /// serial and frame their triggers were last scanned at.
    actor_poses: HashMap<u64, (Vec3, f32)>,
    actor_scan: HashMap<u64, (u64, f32)>,
    /// Weapon trail: the action tick and clip frame last scanned, and the
    /// calibrated bone length.
    trail_scan: Option<(u64, u32)>,
    trail_length: Option<f32>,
    /// Map effect groups to end (None = all of them).
    fx_clear_map: Vec<Option<u8>>,
    /// The area's map effects still to be started (a new area).
    map_fx_pending: bool,
    /// Looping sound requests to stop (a map effect that ended).
    sound_stops: Vec<u32>,
    /// Field talk: control locked, and this tick's reaction / advance
    /// presses (Triangle; decide, back or Triangle).
    talking: bool,
    rc_queued: bool,
    advance_queued: bool,
    /// Unattended capture ticks and end for this room.
    captures: Vec<u64>,
    autoplay_end: u64,
}

/// Sound output and banks; requests resolve by id (bank·1000 + sound).
#[derive(Resource)]
struct Audio {
    output: Option<audio::Output>,
    banks: Banks,
    alarm: bool,
    /// The ambient bed that is playing (its request id).
    ambient: Option<u32>,
}

struct MenuState {
    menu: CommandMenu,
    sheet: Sheet,
    font: SystemFont,
}

/// Sora's field gauge: the translated GAUGE_PLAYER logic, drawn into a
/// software canvas shown as a full-window overlay.
#[derive(Resource)]
struct Hud {
    /// Gauge sheet, then the face sheet when configured.
    sheets: Vec<Sheet>,
    /// The area-change iris layout.
    iris: Option<Sheet>,
    menu: Option<MenuState>,
    gauge: Gauge,
    /// The Scan target gauge (GAUGE_ENEMY, layer 6) and what it shows.
    enemy_gauge: EnemyGauge,
    scan: enemy_target::ScanTracker,
    /// The HUD slot-height table that popups register in (the Scan gauge stacks
    /// on it).
    slots: visual::hud_slots::HudSlots,
    /// The `fiel` sheet and the lock-on reticle drawn from it.
    field: Sheet,
    reticle: visual::lock_on::Reticle,
    reticle_draw: Option<Draw>,
    stats: GaugeStats,
    canvas: Canvas,
    image: Handle<Image>,
    updated: bool,
    /// The form level-up panels (sprite priority 9000, after the gauge).
    panels: visual::level_up::Panels,
}

/// The original's default follow camera, ticked with the simulation in the
/// game's own world space (Y down: scene (x, -y, -z)).
#[derive(Resource)]
struct CameraRig {
    camera: FollowCamera,
    previous_position: [f32; 3],
    /// Right-drag turn not yet consumed by a tick, in manual-turn units.
    pending_turn: f32,
    recenter_queued: bool,
    /// The map's camera collision in game world space.
    collision: CameraCollision,
    collision_hits: u64,
}

/// The camera's collision: a map's COCT worlds queried like the executable's
/// camera ray (planes, surface nibbles), or plain triangles for the generated
/// arena.
struct CameraCollision {
    worlds: Vec<::collision::World>,
    triangles: Vec<[[f32; 3]; 3]>,
}

impl CameraCollision {
    fn new(room: &visual::room::LoadedRoom) -> Self {
        Self {
            // The COCT vertices already are game world coordinates.
            worlds: room
                .camera_collisions
                .iter()
                .filter_map(|c| ::collision::World::new(c, u64::MAX).ok())
                .collect(),
            triangles: room
                .camera_triangles
                .iter()
                .map(|t| t.map(|p| to_world(p.to_array())))
                .collect(),
        }
    }

    fn polygons(&self) -> usize {
        self.triangles.len()
    }

    fn cast(&self, from: [f32; 3], to: [f32; 3]) -> Option<follow_camera::RayHit> {
        if self.worlds.is_empty() {
            return follow_camera::triangle_hit(&self.triangles, from, to);
        }
        let squared = |p: [f32; 3]| (0..3).map(|i| (p[i] - from[i]).powi(2)).sum::<f32>();
        self.worlds
            .iter()
            .filter_map(|w| w.camera_ray(from, to))
            .min_by(|a, b| squared(a.point).total_cmp(&squared(b.point)))
            .map(|hit| follow_camera::RayHit {
                point: hit.point,
                normal: hit.normal,
                surface: hit.surface,
            })
    }
}

/// Scene (Y up) to game world (Y down) and back; the map is converted with
/// (x, -y, -z).
fn to_world(p: [f32; 3]) -> [f32; 3] {
    [p[0], -p[1], -p[2]]
}
fn from_world(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], -p[1], -p[2])
}

/// Starts the map elements of every background group set in `mask`, in
/// bit order and then table order (FUN_1402c7010 per set bit).
fn start_map_effects(game: &mut Game, pax: &Arc<object_effect::Pax>, mask: u64, only: Option<u8>) {
    for bit in 0..64u8 {
        if mask >> bit & 1 == 0 || only.is_some_and(|o| o != bit) {
            continue;
        }
        for (index, element) in pax.elements.iter().enumerate() {
            if element.group == bit {
                game.fx_queue.push(drive_fx::Placed {
                    req: drive_fx::FxRequest {
                        pax: pax.clone(),
                        id: element.id,
                        owner: 0xfd,
                    },
                    anchor: drive_fx::Anchor::Map { index },
                });
            }
        }
    }
}

fn camera_input(player: &Player, previous: [f32; 3], speed: f32) -> CameraInput {
    let [s, c] = [player.yaw.sin(), player.yaw.cos()];
    CameraInput {
        player_position: to_world(player.position),
        player_forward: to_world([s, 0., c]),
        moved: player.position[0] != previous[0] || player.position[2] != previous[2],
        speed,
        ..Default::default()
    }
}

#[derive(Component)]
struct SoraRoot;
#[derive(Component)]
struct SoraPart(usize);
#[derive(Component)]
struct WeaponPart(usize);
#[derive(Component)]
struct WeaponRoot;
#[derive(Component)]
struct CollisionOverlay;

impl Game {
    /// The game-over tour's scripted knock-out tick.
    fn tour_defeat_due(&self) -> bool {
        self.tick == game_over::TOUR_DEFEAT
    }

    fn event(&mut self, value: serde_json::Value) {
        if let Some(log) = &mut self.log {
            serde_json::to_writer(&mut *log, &value).expect("test room log write");
            writeln!(log).expect("test room log newline");
            log.flush().expect("test room log flush");
        }
    }
}

fn read_limited(path: &Path) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    File::open(path)?.take(1_048_577).read_to_end(&mut bytes)?;
    if bytes.len() > 1_048_576 {
        return Err("selection exceeds 1 MiB".into());
    }
    Ok(bytes)
}

fn read_archive(
    source: &ProbeSource,
    locator: &Locator,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut checked = source.open_index(&locator.index)?;
    let entry = checked
        .inventory
        .entries
        .get(locator.ordinal)
        .filter(|e| e.name_hash == locator.name_hash)
        .ok_or("archive source entry mismatch")?;
    let original = original_payload::read_original(&mut checked.reader, entry)?;
    checked.verify_unchanged()?;
    Ok(original.bytes)
}

/// A file of an index found by its path's name hash (None: not there).
fn read_named(
    source: &ProbeSource,
    index: &str,
    path: &str,
) -> Result<Option<Vec<u8>>, Box<dyn std::error::Error>> {
    let hash = data::name_hash::of_path(path);
    let mut checked = source.open_index(index)?;
    let Some(entry) = checked
        .inventory
        .entries
        .iter()
        .find(|e| e.name_hash == hash)
    else {
        return Ok(None);
    };
    let bytes = original_payload::read_original(&mut checked.reader, entry)?.bytes;
    checked.verify_unchanged()?;
    Ok(Some(bytes))
}

fn bar_entry<'a>(bytes: &'a [u8], name: &[u8; 4]) -> Result<&'a [u8], Box<dyn std::error::Error>> {
    let table = bar::read_bar(bytes)?;
    let matches: Vec<_> = table
        .entries
        .iter()
        .filter(|e| &e.name_bytes == name && e.offset > 0 && e.size > 0)
        .collect();
    let [entry] = matches.as_slice() else {
        return Err(format!("expected one {} entry", String::from_utf8_lossy(name)).into());
    };
    bytes
        .get(entry.offset as usize..entry.offset as usize + entry.size as usize)
        .ok_or_else(|| "BAR entry exceeds archive".into())
}

/// Clip timing and honoured range triggers from one decoded motion.
/// Clip timing relative to FrameStart, plus the key time of FrameStart.
/// Loads the ATKP rows of `id` in sub-table `sub` into the runtime (and the
/// id's base row and hit sound for the clash rules) on first use.
fn ensure_attack_rows(rt: &mut EnemyRuntime, id: u16, sub: u8) -> bool {
    if rt.steps.contains_key(&(id, sub)) {
        return true;
    }
    match data::battle_tables::attack_steps_sub(&rt.atkp, id, sub) {
        Ok(rows) => {
            let first = rows[0];
            rt.rows.entry(id).or_insert(first);
            rt.hit_sounds.entry(id).or_insert(first.hit_sound);
            rt.steps.insert((id, sub), rows);
            true
        }
        Err(_) => false,
    }
}

/// The damage routine's view of an ATKP row.
fn params_of(a: &data::battle_tables::Attack) -> combat::AttackParams {
    combat::AttackParams {
        power: a.power,
        stat: a.stat,
        element: a.element,
        reaction: a.reaction,
        knockback: a.knockback,
        attack_type: a.kind,
        combo_kind: a.combo_kind,
        flags: a.flags,
    }
}

/// A PTYA record for the attack chooser (the 0x44 bytes as read).
fn ptya_record(e: &ptya::PtyaEntry) -> player::ptya::Record {
    let f = |k: usize| f32::from_bits(e.float_bits[k]);
    player::ptya::Record {
        id: e.id,
        kind: e.kind,
        alternate: e.sub,
        combo_offset: e.combo_offset,
        flags: e.flag,
        motion: e.motion_id,
        blend_arg: e.next_motion_id,
        jump: f(0),
        jump_max: f(1),
        jump_min: f(2),
        speed_min: f(3),
        speed_max: f(4),
        near: f(5),
        far: f(6),
        high: f(7),
        low: f(8),
        cos_min: f(9),
        cos_max: f(10),
        blend: f(11),
        reach: f(12),
        ability: e.ability,
        w42: e.score,
    }
}

fn clip_for(selected: &motion::SelectedMotion) -> Result<(Clip, u32), Box<dyn std::error::Error>> {
    let (frame_start, frame_end, fps, frame_return) = selected.motion.frame_window()?;
    // Final Form's motions run at 120 frames a second.
    if !(1. ..=120.).contains(&fps)
        || !(0. ..frame_end).contains(&frame_start)
        || !(frame_start..=frame_end).contains(&frame_return)
    {
        return Err("motion frame window outside test-room profile".into());
    }
    let scale = 60. / fps;
    let start = (frame_start * scale).round() as u32;
    let end = (frame_end * scale).round() as u32 - start;
    let mut clip = Clip {
        end,
        loop_start: (frame_return * scale).round() as u32 - start,
        ..Clip::default()
    };
    for record in selected.triggers.iter().flat_map(|t| &t.records.ranges) {
        let start = record.start_frame.max(0) as u32;
        let stop = if record.end_frame < 0 {
            end
        } else {
            record.end_frame as u32
        };
        match record.trigger {
            11 => clip.combo_windows.push((start, stop)),
            50 => clip.finisher_windows.push((start, stop)),
            2 => {
                clip.blend_to_idle = Some(clip.blend_to_idle.map_or(start, |b: u32| b.min(start)));
                clip.actionable.push((start, stop));
            }
            0 | 1 => clip.actionable.push((start, stop)),
            // Script windows (16, 17), reaction offers (20) and 26.
            16 | 17 | 20 | 26 => {
                let p = record.parameters.first().copied().unwrap_or(0);
                clip.windows.push((start, stop, record.trigger, p));
            }
            3 => clip.no_gravity.push((start, stop)),
            27 => clip.invulnerable.push((start, stop)),
            42 => clip.inertia.push((start, stop)),
            // 28 turns toward the lock-on target, 22 toward the command
            // target; one enemy here, so they are the same.
            28 | 22 => {
                if let Some(&deg) = record.parameters.first() {
                    clip.turn_to_target.push((start, stop, deg as f32));
                }
            }
            _ => {}
        }
    }
    for record in selected.triggers.iter().flat_map(|t| &t.records.frames) {
        let frame = record.frame.max(0) as u32;
        match record.trigger {
            0 => clip.takeoff = Some(clip.takeoff.map_or(frame, |f: u32| f.min(frame))),
            21 => {
                if let Some(&deg) = record.parameters.first() {
                    clip.face_target.push((frame, deg as f32));
                }
            }
            1 => {
                if let Some(&n) = record.parameters.first() {
                    clip.effects.push((frame, n));
                }
            }
            // Ground dust (FUN_140424c60): p0 is the foot id.
            19 => {
                if let Some(&p0) = record.parameters.first() {
                    clip.dust.push((frame, p0));
                }
            }
            // A footstep (FUN_1403b4fc0): p0 picks the step within the
            // floor material's group of sixteen.
            2 => {
                if let Some(&p0) = record.parameters.first() {
                    clip.steps.push((frame, p0));
                }
            }
            // Play SE (FUN_14040dd70 case 8): id = p0 * 1000 + p1; with more
            // than three parameters and p3 set, only for the equipped
            // weapon's bank. The gate on the actor's flag bit 0 is not
            // modelled (its meaning is unresolved).
            8 => {
                if let &[p0, p1, ref rest @ ..] = record.parameters.as_slice()
                    && p0 >= 0
                    && p1 >= 0
                    && (rest.len() < 2 || rest[1] == 0 || p0 == WEAPON_BANK)
                {
                    clip.sounds.push((frame, p0 as u32 * 1000 + p1 as u32));
                }
            }
            // A script event (TR17) and a script mark.
            7 => clip.events.push((
                frame,
                record.parameters.first().copied().unwrap_or(0),
                record.parameters.get(1).copied().unwrap_or(0),
            )),
            9 => clip.marks.push(frame),
            14 => {
                if let Some(&n) = record.parameters.first() {
                    clip.voices.push((frame, n));
                }
            }
            // Item use (FUN_14040dd70 case 6 → FUN_1403f6790).
            6 => clip.item_use = Some(clip.item_use.map_or(frame, |f: u32| f.min(frame))),
            // Spell release (FUN_14040dd70 case 0x11), filtered by the
            // motion id when it has an argument.
            17 => clip
                .release
                .push((frame, record.parameters.first().map(|&m| m as u16))),
            _ => {}
        }
    }
    Ok((clip, start))
}

/// What builds the battle ability record (stats +0x1d0): the SSTM adders,
/// each ability item's bit (+4) and action/growth class (+6 non-zero), and the
/// equipment items' granted abilities.
struct AbilitySource {
    rates: abilities::Rates,
    items: HashMap<u16, (u16, bool)>,
    equipment_abilities: HashMap<u16, u16>,
    /// Each equipment item's stat row (FUN_1403e3ef0) by item id.
    rows: HashMap<u16, player::equipment_stats::Row>,
    /// The Keyblades, armor and accessories the development room owns.
    owned: Vec<u16>,
    inventory: ability_inventory::Inventory,
}

impl AbilitySource {
    /// FUN_1403c1d90's ability part: reset (FUN_1404016b0), the weapon's
    /// stat-row ability (FUN_1403c1190), then the equipped slots
    /// (FUN_1404014a0).
    fn build(
        &self,
        slots: &[u16],
        equipment: &pause_menu::items::Equipment,
    ) -> abilities::Abilities {
        self.build_with_form(slots, equipment, None)
    }

    /// The same with an active Drive form's record linked (stats+0x254):
    /// Sora's growth abilities give way to the form's slots.
    fn build_with_form(
        &self,
        slots: &[u16],
        equipment: &pause_menu::items::Equipment,
        form: Option<&player::forms::FormRecord>,
    ) -> abilities::Abilities {
        let mut a = abilities::Abilities::default();
        // FUN_1403c1d90: member weapons, linked form weapon, armor, accessories,
        // then the slot pass. Each occurrence adds independently, even duplicates.
        for item in std::iter::once(equipment.weapon)
            .chain(std::iter::once(equipment.secondary_weapon))
            .chain(form.map(|r| r.keyblade))
            .chain(equipment.armor.iter().copied())
            .chain(equipment.accessories.iter().copied())
        {
            if let Some(&ability) = self.equipment_abilities.get(&item)
                && let Some(&(bit, _)) = self.items.get(&ability)
            {
                a.add(ability, bit, &self.rates);
            }
        }
        a.equip(
            slots,
            form.map(|r| r.abilities.as_slice()),
            |id| self.items.get(&id).copied(),
            &abilities::GROWTH,
            &self.rates,
        );
        a
    }

    /// What the equipment adds to the status totals (FUN_1403c1d90 through
    /// FUN_1403c1190): the member weapons, the linked form weapon, armor and
    /// accessories, in that order.
    fn gear(
        &self,
        equipment: &pause_menu::items::Equipment,
        form_weapon: Option<u16>,
    ) -> player::equipment_stats::Totals {
        gear::totals(&self.rows, equipment, form_weapon)
    }
}

#[cfg(test)]
mod ability_source_tests {
    use super::*;

    #[test]
    fn saved_arena_expands_without_accepting_unbounded_geometry() {
        let arena = ArenaConfig {
            half_size: 1500.,
            wall_height: 400.,
            tile: 150.,
        };
        let expanded = arena.expanded().unwrap();
        assert_eq!(
            (expanded.half_size, expanded.wall_height, expanded.tile),
            (4500., 800., 300.)
        );
        assert_eq!(arena.half_size, 1500.);
        assert!(
            ArenaConfig {
                half_size: f32::NAN,
                ..arena
            }
            .expanded()
            .is_err()
        );
        assert!(
            ArenaConfig {
                half_size: 1_000_000.,
                wall_height: 800.,
                tile: 300.
            }
            .expanded()
            .is_err()
        );
        assert!(
            ArenaConfig {
                half_size: 4500.,
                wall_height: 800.,
                tile: 0.
            }
            .expanded()
            .is_err()
        );
    }

    fn source() -> AbilitySource {
        AbilitySource {
            rates: abilities::Rates::read(&vec![0; 0x17c]).unwrap(),
            items: HashMap::from([
                (abilities::item::COMBO_PLUS, (60, false)),
                (abilities::item::FINISHING_PLUS, (61, false)),
                (0x303, (2, true)),
            ]),
            equipment_abilities: HashMap::from([
                (1001, abilities::item::COMBO_PLUS),
                (1002, abilities::item::FINISHING_PLUS),
            ]),
            rows: HashMap::new(),
            owned: Vec::new(),
            inventory: ability_inventory::Inventory {
                slots: vec![0; 80],
                ap: 0,
                growth: std::array::from_fn(|_| Vec::new()),
            },
        }
    }

    #[test]
    fn changing_equipment_rebuilds_without_the_previous_weapon_ability() {
        let source = source();
        let mut equipment = pause_menu::items::Equipment {
            weapon: 1001,
            ..Default::default()
        };
        let before = source.build(&[], &equipment);
        assert_eq!((before.ground_combo, before.finishers), (4, 1));
        equipment.weapon = 1002;
        let after = source.build(&[], &equipment);
        assert_eq!((after.ground_combo, after.finishers), (3, 2));
        assert!(!after.has(60) && after.has(61));
        equipment.weapon = 0;
        assert_eq!(
            source.build(&[], &equipment),
            abilities::Abilities::default()
        );
    }

    #[test]
    fn weapon_form_armor_accessory_and_slot_modifiers_all_stack() {
        let source = source();
        let equipment = pause_menu::items::Equipment {
            weapon: 1001,
            secondary_weapon: 1001,
            armor: vec![1001],
            accessories: vec![1001],
            ..Default::default()
        };
        let form = player::forms::FormRecord {
            keyblade: 1001,
            ..Default::default()
        };
        let slots = [0x8000 | abilities::item::COMBO_PLUS];
        assert_eq!(
            source
                .build_with_form(&slots, &equipment, Some(&form))
                .ground_combo,
            9
        );
        assert_eq!(source.build(&slots, &equipment).ground_combo, 8);
    }

    #[test]
    fn form_suppresses_normal_action_growth_slots_and_restores_them_on_revert() {
        let source = source();
        let equipment = pause_menu::items::Equipment::default();
        let slots = [0x8303, 0x8000 | abilities::item::COMBO_PLUS];
        let normal = source.build(&slots, &equipment);
        assert_eq!((normal.growth[0], normal.ground_combo), (1, 4));
        let mut form = player::forms::FormRecord::default();
        let driven = source.build_with_form(&slots, &equipment, Some(&form));
        assert_eq!((driven.growth[0], driven.ground_combo), (0, 4));
        form.abilities[0] = 0x8303;
        assert_eq!(
            source
                .build_with_form(&slots, &equipment, Some(&form))
                .growth[0],
            1
        );
        assert_eq!(source.build(&slots, &equipment), normal);
    }
}

struct Loaded {
    room: LoadedRoom,
    /// Keyblade items' models (the skateboard), and the room's own weapon.
    keyblades: crate::keyblade_board::Keyblades,
    weapon_model: String,
    board_motion: Option<motion::SelectedMotion>,
    arena_mesh: Option<visual::room::ArenaMesh>,
    params: Params,
    /// SSTM battle parameters (blow, hit-back, fall max).
    sstm: actor_physics::Sstm,
    abilities: AbilitySource,
    magic: Option<magic_room::MagicSource>,
    items: Option<item_room::ItemSource>,
    /// Base Sora's kit, and what builds the Drive forms' kits.
    kit: character_kit::Kit,
    forms: Option<drive_room::FormSources>,
    enemy_character: u16,
    fmab: player::growth::Fmab,
    voices: Option<drive_room::Voices>,
    /// The world's 12soundinfo rows (Sora's footstep bank by room).
    step_info: Option<(u8, visual::footsteps::WorldInfo)>,
    /// Donald and Goofy's models and motions (empty when not installed).
    party: Vec<std::sync::Arc<party_room::PartyKind>>,
    spawn: Vec3,
    /// Sora's facing at the spawn (scene yaw).
    yaw: f32,
    world: RoomWorld,
    /// The field area, when the room is a game area.
    area: Option<(AreaSource, LoadedArea)>,
    /// Message window art, font and labels for townspeople talk.
    area_talk: Option<AreaTalk>,
    stay: bool,
    summary: serde_json::Value,
    hud: Option<HudSource>,
    enemy: Option<EnemySource>,
    banks: Banks,
    effects: Option<EffectSources>,
    npc: Option<NpcSource>,
    camp: Option<CampSource>,
    journal: Option<JournalSource>,
    /// The game-over sequence's files (absent when the install lacks any).
    game_over: Option<game_over::Source>,
    /// The menus' configured files, for a text language change.
    language: Option<settings_room::LanguageSources>,
    /// Field and battle music SCD bytes.
    music: Option<[Vec<u8>; 2]>,
}

/// The journal's art, tables and messages.
struct JournalSource {
    book: Menu2ld,
    curtain: Menu2ld,
    world_icons: Vec<menu_screen::Texture>,
    tops: Vec<(String, Vec<menu_screen::Texture>)>,
    worlds: Vec<data::journal_table::World>,
    stories: Vec<data::journal_table::Story>,
    texts: HashMap<u16, Vec<u8>>,
    flags: Vec<u16>,
    reports: Vec<data::journal_table::Report>,
    limits: Vec<data::journal_table::Limit>,
    quests: Vec<data::journal_table::Quest>,
    minigames: Vec<data::journal_table::Minigame>,
    moogle_names: Vec<Vec<u8>>,
    treasures: Vec<data::journal_table::Treasure>,
    item_names: HashMap<u16, journal::TreasureItem>,
    opened_bits: Vec<u16>,
    owned_items: Vec<u16>,
    chars: Vec<data::journal_table::Char>,
    diag: Vec<data::journal_table::Diag>,
    albums: Vec<data::journal_table::Album>,
}

fn load_journal(
    source: &ProbeSource,
    j: &JournalConfig,
    sys: &Locator,
    system: &[u8],
) -> Result<JournalSource, Box<dyn std::error::Error>> {
    use data::journal_table;
    use visual::pause_menu::journal;
    let two_ld = |l: &Locator| menu_screen::load(source, &l.index, l.ordinal, l.name_hash);
    let images = |l: &Locator| menu_screen::load_images(source, &l.index, l.ordinal, l.name_hash);
    let book = two_ld(&j.book)?;
    let curtain = two_ld(&j.curtain)?;
    let world_icons = images(&j.worlds)?;
    let mut tops = Vec::new();
    for (code, l) in &j.tops {
        tops.push((code.clone(), images(l)?));
    }
    let tables = read_archive(source, &j.tables)?;
    let worlds = journal_table::read_worlds(bar_entry(&tables, b"worl")?)?;
    let stories = journal_table::read_stories(bar_entry(&tables, b"stor")?)?;
    let chars = journal_table::read_chars(bar_entry(&tables, b"char")?)?;
    let diag = journal_table::read_diag(bar_entry(&tables, b"diag")?)?;
    let albums = journal_table::read_albums(bar_entry(&tables, b"albu")?)?;
    let reports = journal_table::read_reports(bar_entry(&tables, b"anse")?)?;
    let limits = journal_table::read_limits(bar_entry(&tables, b"limi")?)?;
    let quests = journal_table::read_quests(bar_entry(&tables, b"ques")?)?;
    let minigames = journal_table::read_minigames(bar_entry(&tables, b"mini")?)?;
    let mut ids: Vec<u32> = journal::MESSAGES.iter().map(|&i| i as u32).collect();
    ids.extend(journal::PAGE_MESSAGES.map(u32::from));
    for c in &chars {
        ids.extend([c.name, c.description, c.origin].map(u32::from));
    }
    for a in &albums {
        ids.extend([a.title, a.caption].map(u32::from));
    }
    for r in &reports {
        ids.extend([r.title, r.text].map(u32::from));
    }
    for l in &limits {
        ids.extend([l.name, l.description].map(u32::from));
    }
    for q in &quests {
        ids.extend([q.heading, q.text].map(u32::from));
    }
    for m in &minigames {
        ids.extend([m.name, m.label].map(u32::from));
    }
    for w in &worlds {
        ids.extend([w.names[0], w.alt_names[0], w.names[1], w.alt_names[1]].map(u32::from));
    }
    for s in &stories {
        ids.extend([s.summary, s.objective, s.entry].map(u32::from));
    }
    ids.retain(|&i| i != 0);
    ids.sort_unstable();
    ids.dedup();
    let found = hud::load_messages_optional(
        source,
        &j.messages.index,
        j.messages.ordinal,
        j.messages.name_hash,
        &ids,
    )?;
    // The chests' items: names from the system table.
    let treasures = journal_table::read_treasures(bar_entry(system, b"trsr")?)?;
    let items = data::item_table::read(bar_entry(system, b"item")?)?;
    let wanted: Vec<u16> = {
        let mut v: Vec<u16> = treasures.iter().map(|t| t.item).collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let rows: Vec<&data::item_table::Item> = wanted
        .iter()
        .filter_map(|id| items.iter().find(|r| r.id == *id))
        .collect();
    let name_ids: Vec<u32> = rows.iter().map(|r| r.name as u32).collect();
    let names =
        hud::load_messages_optional(source, &sys.index, sys.ordinal, sys.name_hash, &name_ids)?;
    let item_names: HashMap<u16, journal::TreasureItem> = rows
        .iter()
        .zip(names)
        .map(|(r, n)| {
            (
                r.id,
                journal::TreasureItem {
                    name: n.unwrap_or_default(),
                    icon: r.icon,
                    map: r.kind == 22,
                },
            )
        })
        .collect();
    let mut texts: HashMap<u16, Vec<u8>> = ids
        .iter()
        .zip(found)
        .filter_map(|(&id, m)| m.map(|m| (id as u16, m)))
        .collect();
    // 0x8adc is the system message 0xadc.
    let fake = hud::load_messages(
        source,
        &sys.index,
        sys.ordinal,
        sys.name_hash,
        &[journal::NO_STORY as u32],
    )?;
    texts.insert(
        journal::NO_STORY,
        fake.into_iter().next().unwrap_or_default(),
    );
    let moogle_names = hud::load_messages_optional(
        source,
        &sys.index,
        sys.ordinal,
        sys.name_hash,
        &journal::MOOGLE_NAMES,
    )?
    .into_iter()
    .map(Option::unwrap_or_default)
    .collect();
    Ok(JournalSource {
        book,
        curtain,
        world_icons,
        tops,
        worlds,
        stories,
        texts,
        flags: j.flags.clone(),
        reports,
        limits,
        quests,
        minigames,
        moogle_names,
        treasures,
        item_names,
        opened_bits: j.opened_bits.clone(),
        owned_items: j.owned_items.clone(),
        chars,
        diag,
        albums,
    })
}

/// Camp (pause) menu sources: camp.2ld, the entries' labels and help
/// lines (sys.bar), and the system font.
struct CampSource {
    menu: Menu2ld,
    entries: Vec<pause_menu::Entry>,
    font: SystemFont,
    /// "LV", "HP", "MP" and Sora's name (0xb9fc..0xb9fe, 0x851f).
    labels: [Vec<u8>; 3],
    name: Vec<u8>,
    /// Sora's Status help line and the 0x18 banner.
    help: Vec<u8>,
    banner: Vec<u8>,
    /// Status stats: bit, label, help, style.
    stats: Vec<(usize, Vec<u8>, Vec<u8>, usize)>,
    config: Vec<pause_menu::ConfigRow>,
    numbers: SoraNumbers,
    /// The item table with its texts and the Items screens' messages.
    items: pause_menu::items::ItemsData,
    /// The Abilities screens' messages and the Kingdom Key's ability.
    abilities: pause_menu::abilities::AbilityTexts,
    weapon_ability: u16,
    party_help: [Vec<u8>; 3],
    customize: pause_menu::customize::CustomizeTexts,
    equip_texts: pause_menu::equip::EquipTexts,
    equip_stats: Vec<pause_menu::equip::ItemStats>,
    /// The font image's icon plane (0x09 icons).
    icons: SystemFont,
    /// 03system.bin `ipic`: the item pictures' sequence.
    picture_sequence: Option<data::sprite_sequence::Sequence>,
    /// The tutorial notices' headings, "PAGE " and callouts.
    notice_headings: [Vec<u8>; 4],
    notice_page: Vec<u8>,
    notice_callouts: Vec<(String, Vec<pause_menu::notice::Callout>)>,
}

/// Items screen messages (camp-05 §9).
const ITEM_MESSAGES: [u32; 19] = [
    0x8455, 0x845a, 0x8456, 0x8457, 0x8448, 0xb943, 0xb944, 0xbb86, 0xbb87, 0xb90c, 0xb90f, 0xb90d,
    0xb90e, 0xb910, 0xb94f, 0xbb90, 0x9fcf, 0x9fd0, 0x9fd1,
];

/// The room's stock and Sora's equipment [test-room values: a new save's
/// inventory is not decoded]: 3 Potions, a Hi-Potion, 2 Ethers; the
/// Kingdom Key with one empty armor, accessory and two item slots.
const ROOM_STOCK: [(u16, u8); 9] = [
    (1, 3),
    (2, 1),
    (3, 2),
    // The element items (type 0x12): Fire, Blizzard, Thunder, Cure, Magnet
    // and Reflect at their top level [test-room values: a new save has
    // none].
    (0x15, 3),
    (0x16, 3),
    (0x17, 3),
    (0x18, 3),
    (0x57, 3),
    (0x58, 3),
];

/// Armor and accessory slots of the development room [test-room values: the
/// slots a new save gains are not decoded; four of each lets armor and
/// accessories stack].
const ROOM_GEAR_SLOTS: usize = 4;

fn room_equipment() -> pause_menu::items::Equipment {
    pause_menu::items::Equipment {
        weapon: KINGDOM_KEY,
        armor: vec![0; ROOM_GEAR_SLOTS],
        accessories: vec![0; ROOM_GEAR_SLOTS],
        items: ROOM_ITEM_SLOTS.to_vec(),
        originals: ROOM_ITEM_SLOTS.to_vec(),
        ..Default::default()
    }
}

/// The element items by magic type (Fire, Blizzard, Thunder, Cure,
/// Magnet, Reflect).
const ELEMENT_ITEMS: [u16; 6] = [0x15, 0x16, 0x17, 0x18, 0x57, 0x58];
/// Shortcut slots of a fresh room [test-room values]: Fire, Blizzard,
/// Thunder and Cure by their level-1 command ids; Customize changes them.
const ROOM_SHORTCUTS: [u16; 4] = [0x31, 0x33, 0x32, 0x34];

/// Equip messages (camp-09 §10): sections, None, no items, unequip help,
/// the two boxes, the info labels.
const EQUIP_MESSAGES: [u32; 22] = [
    0x845b, 0x845d, 0x8463, 0x8465, 0x8467, 0x8449, 0x844a, 0xcb8d, 0xcb8e, 0x8469, 0x846a, 0xc22e,
    0xc648, 0xc649, 0xc64a, 0xc64b, 0xc651, 0xc652, 0xc653, 0xc654, 0xc64c, 0,
];

/// Customize messages (camp-07 §1.3 / §7): help, category labels,
/// shortcut headings, remove help, MP Charge and Auto-Reload choices and
/// help, the Auto-Reload warning.
const CUSTOMIZE_MESSAGES: [u32; 19] = [
    0xbb89, 0xbb8a, 0x847d, 0x8481, 0x8482, 0x847d, 0x847e, 0x847f, 0x8480, 0x9df7, 0x9df9, 0x9df8,
    0x9dfb, 0x9dfa, 0x9dfd, 0x9dfc, 0x9dff, 0x9dfe, 0x846a,
];

/// Abilities screen messages (camp-06 §7).
const ABILITY_MESSAGES: [u32; 15] = [
    0xbb8b, 0xbb8c, 0xb2ee, 0xb2ef, 0xb2f0, 0xb2ed, 0xb372, 0xb629, 0xb627, 0xb62a, 0xb628, 0xb373,
    0xb619, 0xb61a, 0x8454,
];

/// [test-room value] AP added to Sora's level-1 AP so the room's abilities
/// fit (the Critical start record's 50 plus Aerial Recovery's 2).
const ROOM_EXTRA_AP: i32 = 50;
/// The Kingdom Key (its stat record gives the weapon ability).
const KINGDOM_KEY: u16 = 0x29;

fn load_camp_abilities(
    system: &[u8],
    msg: impl Fn(&[u32]) -> Result<Vec<Option<Vec<u8>>>, Box<dyn std::error::Error>>,
) -> Result<(pause_menu::abilities::AbilityTexts, u16), Box<dyn std::error::Error>> {
    let item = bar_entry(system, b"item")?;
    let table = data::item_table::read(item)?;
    let stats = data::item_table::read_stats(item)?;
    // FUN_1403e3ef0: weapons (types 2..13) use the stat entry at +6.
    let weapon_ability = table
        .iter()
        .find(|r| r.id == KINGDOM_KEY)
        .and_then(|r| stats.iter().find(|s| s.id == r.weapon_stats))
        .map_or(0, |s| s.ability);
    let mut t = msg(&ABILITY_MESSAGES)?
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect::<Vec<_>>()
        .into_iter();
    let mut take = || t.next().unwrap_or_default();
    let texts = pause_menu::abilities::AbilityTexts {
        help: [take(), take()],
        categories: [take(), take(), take(), take()],
        empty: take(),
        choices: [take(), take()],
        choice_help: [take(), take()],
        items_banner: take(),
        abilities_banner: take(),
        switch: take(),
        triangle: take(),
    };
    Ok((texts, weapon_ability))
}

/// 03system `item` with names and descriptions, and the screens' texts.
fn load_camp_items(
    system: &[u8],
    msg: impl Fn(&[u32]) -> Result<Vec<Option<Vec<u8>>>, Box<dyn std::error::Error>>,
) -> Result<pause_menu::items::ItemsData, Box<dyn std::error::Error>> {
    use pause_menu::items::{ItemInfo, ItemTexts, ItemsData};
    let table = data::item_table::read(bar_entry(system, b"item")?)?;
    // Consumables' commands (FUN_1403c41c0: type 0 → record +0x10) and the
    // commands' names (03system `cmd` +8).
    let commands = data::command_table::read(bar_entry(system, b"cmd\0")?)?;
    let command_name = |r: &data::item_table::Item| -> u32 {
        if r.kind != 0 {
            return 0;
        }
        commands
            .iter()
            .find(|c| c.id == r.command)
            .map_or(0, |c| c.name)
    };
    let ids: Vec<u32> = table
        .iter()
        .flat_map(|r| [r.name as u32, r.description as u32, command_name(r)])
        .collect();
    let texts = msg(&ids)?;
    let table = table
        .iter()
        .zip(texts.chunks(3))
        .map(|(r, t)| ItemInfo {
            command: if r.kind == 0 { r.command } else { 0 },
            command_name: t[2].clone().unwrap_or_default(),
            id: r.id,
            kind: r.kind,
            amount: r.amount,
            sell: r.sell,
            icon: r.icon,
            param: r.weapon_stats,
            picture: r.picture,
            name: t[0].clone().unwrap_or_default(),
            description: t[1].clone().unwrap_or_default(),
        })
        .collect();
    let fixed: Vec<u32> = ITEM_MESSAGES.to_vec();
    let mut f = msg(&fixed)?
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect::<Vec<_>>()
        .into_iter();
    let mut take = || f.next().unwrap_or_default();
    // Category glyph messages (0x1405ba790): 0xb9c7 + t for t ≤ 0x11,
    // 0xbe2b for type 0x14.
    let category_ids: Vec<u32> = (0..=0x14u32)
        .map(|t| match t {
            0..=0x11 => 0xb9c7 + t,
            0x14 => 0xbe2b,
            _ => 0,
        })
        .collect();
    let categories = msg(&category_ids)?
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect();
    let texts = ItemTexts {
        stock: take(),
        empty: take(),
        use_: take(),
        throw: take(),
        blank: take(),
        use_on: take(),
        all: take(),
        help: [take(), take()],
        tabs: [take(), take(), take(), take(), take()],
        throw_query: take(),
        no_effect: take(),
        ok: take(),
        yes: take(),
        no: take(),
        categories,
    };
    Ok(ItemsData {
        table,
        texts,
        ..ItemsData::default()
    })
}

/// Sora's Status numbers the room does not track elsewhere.
#[derive(Clone, Copy)]
struct SoraNumbers {
    level: u8,
    next_exp: u32,
    ap: i32,
    strength: i32,
    magic: i32,
    defense: i32,
}

/// Status stats Sora shows with nothing obtained (camp-04 §2.4): bit,
/// label id, help id, style.
const CAMP_STATS: [(usize, u32, u32, usize); 14] = [
    (0, 0xc559, 0xc56e, 0),
    (1, 0xc55a, 0xc56f, 0),
    (2, 0xc55b, 0xc570, 0),
    (3, 0xc55c, 0xc571, 1),
    (4, 0xc55d, 0xc572, 1),
    (5, 0xc55e, 0xc573, 1),
    (7, 0xc567, 0xc57c, 4),
    (10, 0xc560, 0xc575, 2),
    (11, 0xc561, 0xc576, 2),
    (12, 0xc562, 0xc577, 2),
    (13, 0xc563, 0xc578, 3),
    (14, 0xc564, 0xc579, 3),
    (15, 0xc565, 0xc57a, 3),
    (16, 0xc566, 0xc57b, 3),
];

/// Config rows (table 0x1405bc340): label, choice labels, help per choice.
const CAMP_CONFIG: [(u32, &[u32], &[u32]); 8] = [
    (0xb717, &[0xb71e, 0xb71f], &[0xb720, 0xb721]),
    (0xc2f5, &[0xc2f8, 0xc2f9], &[0xc2fa, 0xc2fb]),
    (0xc2f6, &[0xc2fc, 0xc2fd], &[0xc2fe, 0xc2ff]),
    (0xc2f7, &[0xc302, 0xc300, 0xc301], &[0xc305, 0xc303, 0xc304]),
    (0xb719, &[0xb726, 0xb727], &[0xb728, 0xb729]),
    (0xb71a, &[0xb72a, 0xb752], &[0xb72c, 0xb72d]),
    (0xb71c, &[0xb734, 0xb735], &[0xb736, 0xb737]),
    (
        0xb71d,
        &[0xb738, 0xb739, 0xb73a, 0xce30],
        &[0xb73b, 0xb73c, 0xb73d, 0xce31],
    ),
];

/// Camp entries (table 0x1405badb0): id, label and help ids. The test
/// room offers Items, Abilities, Customize, Party, Status, Jiminy's
/// Journal and Config [test-room choice: the menu flags come from a save].
const CAMP_ENTRIES: [(u8, u32, u32); 7] = [
    (0, 0x844b, 0xb70d),
    (1, 0x844d, 0xb70f),
    (2, 0x8451, 0xb713),
    (3, 0x844e, 0xb710),
    (4, 0x844f, 0xb711),
    (5, 0x8450, 0xb712),
    (7, 0xb617, 0xb715),
];

/// One enemy clip: skinned positions, HIT-bone matrices and root
/// translation per tick, plus its loop start.
struct EnemyClip {
    frames: Vec<Vec<Vec3>>,
    bones: Vec<HashMap<usize, Mat4>>,
    root: Vec<Vec3>,
    loop_start: u32,
    /// Attack windows (range trigger 10): start, end, ATKP id, group.
    windows: Vec<(u32, u32, u16, u8)>,
    /// Frame trigger 8 sounds: (tick, sound id).
    sounds: Vec<(u32, u32)>,
}

struct EnemyClips(HashMap<usize, EnemyClip>);
impl shadow::Clips for EnemyClips {
    fn length(&self, slot: usize) -> u32 {
        self.0.get(&slot).map_or(1, |c| c.frames.len() as u32)
    }
    fn loop_start(&self, slot: usize) -> u32 {
        self.0.get(&slot).map_or(0, |c| c.loop_start)
    }
    fn root(&self, slot: usize, tick: u32) -> [f32; 3] {
        self.0
            .get(&slot)
            .and_then(|c| c.root.get(tick as usize))
            .map_or([0.; 3], |r| r.to_array())
    }
}

struct RoomFloor<'a>(&'a RoomWorld);
impl shadow::Env for RoomFloor<'_> {
    fn floor(&self, x: f32, z: f32, top: f32) -> Option<f32> {
        self.0.floor(x, z, top)
    }
}

/// Live enemy state for the session.
struct EnemyRuntime {
    sora_target_radius: f32,
    enabled: bool,
    battle_level: u16,
    clips: EnemyClips,
    body: Vec<(usize, Vec3, f32, bool)>,
    targets: Vec<(usize, Vec3, f32, f32)>,
    /// Each hand's Keyblade attack spheres by collision group: offset,
    /// radius.
    blade: [character_kit::Blade; 2],
    /// Every ATKP step row by (id, sub-table); FUN_1403eb230 picks one per
    /// hit count. Spell rows are added when a spell first hits.
    steps: HashMap<(u16, u8), Vec<data::battle_tables::Attack>>,
    /// The ATKP table, for spell rows by sub-table.
    atkp: Vec<u8>,
    sora: combat::AttackerStats,
    enemy: Shadow,
    rng: ScriptRandom,
    claws: HashMap<u8, Vec<(usize, Vec3, f32)>>,
    shadow_attacker: combat::AttackerStats,
    sora_target: combat::TargetStats,
    sora_body: Vec<(usize, Vec3, f32, bool)>,
    /// Summed TARGET radii for aerial hops (FUN_1403d7520).
    target_reach: f32,
    /// Sora's hit record (obj+0xc98): the claws' hit-once mask and the
    /// 30-tick group lock of a player-type object.
    sora_record: combat::HitRecord,
    /// Attack ids and combo groups of live attack instances.
    ids: combat::AttackIds,
    instance_ids: HashMap<u64, (u8, i32)>,
    instance_seen: HashMap<u64, u64>,
    next_group: i32,
    difficulty: combat::Difficulty,
    claw_previous: HashMap<(u64, usize), Vec3>,
    /// Sora's HP and KO timer (test room refills after a KO).
    sora_hp: i32,
    ko_ticks: u32,
    /// The kill's EXP inputs, przt record and Sora's LVUP rows.
    exp_rule: (u16, u16),
    prize: data::battle_tables::Prize,
    sora_levels: Vec<data::battle_tables::PlayerLevel>,
    /// Sora's level, EXP, munny, the prizes in the world, and whether the
    /// enemy was dead last tick.
    level: u8,
    exp_total: u32,
    munny: u32,
    orbs: Vec<player::enemy_prizes::Orb>,
    /// Treasure boxes of dropped items waiting on the ground.
    drops: Vec<player::enemy_prizes::ItemDrop>,
    next_drop: u32,
    was_dead: bool,
    /// Ability items learned by level-ups, waiting for the camp.
    learned: Vec<u16>,
    /// Status numbers changed (level-up): the camp refreshes them.
    numbers_dirty: bool,
    /// Level-up and bonus panels waiting for the HUD, whether the level-up
    /// effect is due, and the bonus levels taken.
    panels: Vec<level_growth::Panel>,
    level_effect: bool,
    /// Ticks left of the heal effect on Sora (FUN_1403d7bd0: +0xd40).
    heal_ticks: f32,
    bonus_taken: Vec<u8>,
    /// Last frame's attack sphere centres by (window instance, sphere).
    previous: HashMap<(u64, usize), Vec3>,
    indices: Vec<Vec<u32>>,
    /// ATKP HitSound by id (Keyblade, then claws).
    hit_sounds: HashMap<u16, i32>,
    /// ATKP hit effect and its random count by id (Keyblade).
    hit_effects: HashMap<u16, (u8, u8)>,
    /// Full ATKP rows (Keyblade, guard and claws) for the clash rules.
    rows: HashMap<u16, data::battle_tables::Attack>,
    /// Sora's invulnerability after a guard (+0xd70), in ticks.
    sora_invulnerable: f32,
    /// Motion serial and tick already scanned for sound triggers.
    sound_tick: Option<(u32, u32)>,
}

/// A placeholder for a spell's hit volume: the spell effects (the `.mag`
/// PAX containers) are not drawn yet, so each live ATTACK_MAGIC volume shows
/// as a translucent shape tinted by its element.
#[derive(Component)]
struct SpellVolume(usize);

/// Tints by ATKP id: fire, blizzard, thunder, cure, magnet, reflect.
fn spell_tint(atkp: u16) -> [f32; 4] {
    match atkp {
        502 => [1., 0.45, 0.1, 0.35],
        248 => [0.5, 0.85, 1., 0.45],
        249..=251 => [1., 0.95, 0.3, 0.35],
        708 | 709 => [0.3, 1., 0.5, 0.35],
        658 | 976 | 1440 => [0.7, 0.4, 1., 0.3],
        573 | 1247 => [0.9, 0.9, 1., 0.3],
        574..=578 | 667..=669 => [0.9, 0.5, 1., 0.4],
        _ => [1., 1., 1., 0.3],
    }
}

#[derive(Component)]
struct EnemyRoot;
#[derive(Component)]
struct EnemyPart(usize);

struct EnemySource {
    sora_target_radius: f32,
    battle_level: u16,
    model: model_inspection::InspectionModel,
    clips: EnemyClips,
    /// HIT spheres: bone, offset, radius, high-flinch argument.
    body: Vec<(usize, Vec3, f32, bool)>,
    /// TARGET shapes (type 3, enabled): bone, offset, radius, height.
    targets: Vec<(usize, Vec3, f32, f32)>,
    /// Keyblade attack spheres by collision group: offset, radius.
    blade: HashMap<u8, Vec<(Vec3, f32)>>,
    steps: HashMap<(u16, u8), Vec<data::battle_tables::Attack>>,
    atkp: Vec<u8>,
    difficulty: combat::Difficulty,
    sora: combat::AttackerStats,
    stats: combat::TargetStats,
    offset: Vec3,
    claws: HashMap<u8, Vec<(usize, Vec3, f32)>>,
    shadow_attacker: combat::AttackerStats,
    sora_target: combat::TargetStats,
    sora_body: Vec<(usize, Vec3, f32, bool)>,
    /// Summed TARGET radii for aerial hops (FUN_1403d7520).
    target_reach: f32,
    hit_sounds: HashMap<u16, i32>,
    hit_effects: HashMap<u16, (u8, u8)>,
    rows: HashMap<u16, data::battle_tables::Attack>,
    /// The kill's EXP (enmp +0x56 x the level row's EXP) and the przt
    /// record its death scatters.
    exp: (u16, u16),
    prize: data::battle_tables::Prize,
    /// Sora's LVUP rows, levels 1..=99.
    sora_levels: Vec<data::battle_tables::PlayerLevel>,
}

/// Walkable collision levels (8-unit height bins) by XZ area, largest first,
/// each with the centroid of its largest triangle.
fn floor_levels(triangles: &[CollisionTriangle]) -> Vec<(f32, [f32; 3])> {
    let mut bins: HashMap<i32, (f32, f32, [f32; 3])> = HashMap::new();
    for [a, b, c] in triangles
        .iter()
        .filter(|t| t.surface_flags & PARTY_STAND != 0)
        .map(|t| &t.corners)
    {
        let normal = (*b - *a).cross(*c - *a);
        if normal.length() <= 1e-6 || normal.normalize().y.abs() < FLOOR_MIN_NORMAL_Y {
            continue;
        }
        let area = 0.5 * normal.y.abs();
        let centroid = (*a + *b + *c) / 3.;
        let bin = bins
            .entry((centroid.y / 8.).round() as i32)
            .or_insert((0., 0., [0.; 3]));
        bin.0 += area;
        if area > bin.1 {
            bin.1 = area;
            bin.2 = (centroid + Vec3::Y).to_array();
        }
    }
    let mut levels: Vec<_> = bins.into_values().map(|(a, _, p)| (a, p)).collect();
    levels.sort_by(|x, y| y.0.total_cmp(&x.0));
    levels
}

/// The camp menu's layout, texts and tables (menu/<lang>/camp.2ld and
/// msg/<lang>/sys.bar with the system font); again on a language change.
fn load_camp_source(
    source: &ProbeSource,
    l: &Locator,
    m: &MenuSources,
    battle: &[u8],
    system: &[u8],
) -> Result<CampSource, Box<dyn std::error::Error>> {
    let menu = menu_screen::load(source, &l.index, l.ordinal, l.name_hash)?;
    let ids: Vec<u32> = CAMP_ENTRIES
        .iter()
        .flat_map(|&(_, label, help)| [label, help])
        .collect();
    let texts = hud::load_messages(
        source,
        &m.messages.index,
        m.messages.ordinal,
        m.messages.name_hash,
        &ids,
    )?;
    let entries = CAMP_ENTRIES
        .iter()
        .zip(texts.chunks(2))
        .map(|(&(id, _, _), t)| pause_menu::Entry {
            id,
            label: t[0].clone(),
            help: t[1].clone(),
        })
        .collect();
    let font = hud::load_system_font(
        source,
        (
            &m.font_image.index,
            m.font_image.ordinal,
            m.font_image.name_hash,
        ),
        (
            &m.font_info.index,
            m.font_info.ordinal,
            m.font_info.name_hash,
        ),
    )?;
    let icons = hud::load_font(
        source,
        (
            &m.font_image.index,
            m.font_image.ordinal,
            m.font_image.name_hash,
        ),
        (
            &m.font_info.index,
            m.font_info.ordinal,
            m.font_info.name_hash,
        ),
        b"icon",
    )?;
    let msg = |ids: &[u32]| {
        hud::load_messages(
            source,
            &m.messages.index,
            m.messages.ordinal,
            m.messages.name_hash,
            ids,
        )
    };
    let mut status = msg(&[0xb9fc, 0xb9fd, 0xb9fe, 0x851f, 0xc5ae, 0xb61a])?;
    let banner = status.pop().unwrap_or_default();
    let status_help = status.pop().unwrap_or_default();
    let name = status.pop().unwrap_or_default();
    let labels = [status[0].clone(), status[1].clone(), status[2].clone()];
    // "<0d>´s Status.": code 0x0d takes the character's name.
    let mut help = Vec::new();
    for &b in &status_help {
        if b == 0x0d {
            help.extend_from_slice(&name);
        } else {
            help.push(b);
        }
    }
    let stat_ids: Vec<u32> = CAMP_STATS
        .iter()
        .flat_map(|&(_, label, help, _)| [label, help])
        .collect();
    let stat_texts = msg(&stat_ids)?;
    let stats = CAMP_STATS
        .iter()
        .zip(stat_texts.chunks(2))
        .map(|(&(bit, _, _, style), t)| (bit, t[0].clone(), t[1].clone(), style))
        .collect();
    let config_ids: Vec<u32> = CAMP_CONFIG
        .iter()
        .flat_map(|(label, choices, helps)| {
            std::iter::once(*label)
                .chain(choices.iter().copied())
                .chain(helps.iter().copied())
        })
        .collect();
    let mut config_texts = msg(&config_ids)?.into_iter();
    let config = CAMP_CONFIG
        .iter()
        .map(|(_, choices, helps)| {
            let mut take =
                |n: usize| -> Vec<Vec<u8>> { (0..n).filter_map(|_| config_texts.next()).collect() };
            let label = take(1).pop().unwrap_or_default();
            pause_menu::ConfigRow {
                label,
                choices: take(choices.len()),
                help: take(helps.len()),
            }
        })
        .collect();
    let lvup = bar_entry(battle, b"lvup")?;
    let plrp = bar_entry(battle, b"plrp")?;
    let lv = data::battle_tables::player_level(lvup, 1, 1)?;
    let byte = |at: usize| plrp.get(8 + at).copied().unwrap_or(0) as i32;
    // Sora at level 1 (LVUP row + PLRP id 0) [test-room values]; the
    // equipment's share is added from `Game::gear`.
    let numbers = SoraNumbers {
        level: 1,
        next_exp: lv.exp,
        ap: lv.ap as i32 + byte(5) + ROOM_EXTRA_AP,
        strength: lv.strength as i32 + byte(6),
        magic: lv.magic as i32 + byte(7),
        defense: lv.defense as i32 + byte(8),
    };
    let (abilities, weapon_ability) = load_camp_abilities(system, |ids| {
        hud::load_messages_optional(
            source,
            &m.messages.index,
            m.messages.ordinal,
            m.messages.name_hash,
            ids,
        )
    })?;
    // Customize texts (customize-spec §1.3 / §7).
    let mut cust = msg(&CUSTOMIZE_MESSAGES)?.into_iter();
    let mut take = || cust.next().unwrap_or_default();
    let customize = pause_menu::customize::CustomizeTexts {
        help: [take(), take()],
        labels: [take(), take(), take()],
        shortcut_headings: [take(), take(), take(), take()],
        remove_help: take(),
        mp_choices: [take(), take()],
        mp_help: [take(), take()],
        reload_choices: [take(), take()],
        reload_help: [take(), take()],
        reload_warning: take(),
    };
    let mut eq = msg(&EQUIP_MESSAGES[..21])?.into_iter();
    let mut take = || eq.next().unwrap_or_default();
    // 0x845f Master and 0x8461 Final label the other two form weapons.
    let mut form_labels = msg(&[0x845f, 0x8461])?.into_iter();
    let equip_texts = pause_menu::equip::EquipTexts {
        sections: [take(), take(), take(), take(), take()],
        form_labels: [
            Vec::new(),
            form_labels.next().unwrap_or_default(),
            form_labels.next().unwrap_or_default(),
        ],
        none: take(),
        no_items: take(),
        unequip_help: [take(), take(), take()],
        reload_box: take(),
        accessory_box: take(),
        stat_labels: [
            take(),
            take(),
            take(),
            take(),
            take(),
            take(),
            take(),
            take(),
            take(),
        ],
    };
    let equip_stats = data::item_table::read_stats(bar_entry(system, b"item")?)?
        .into_iter()
        .map(|s| pause_menu::equip::ItemStats {
            id: s.id,
            ability: s.ability,
            strength: s.strength,
            magic: s.magic,
            defense: s.defense,
            ap: s.ap,
            resists: s.resists,
            other_resists: [s.resist[0], s.resist[5], s.resist[6]],
        })
        .collect();
    // Party help (party-spec §11).
    let mut party = msg(&[0xbb88, 0x846b, 0x846c])?.into_iter();
    let mut next = || party.next().unwrap_or_default();
    let party_help = [next(), next(), next()];
    let items = load_camp_items(system, |ids| {
        hud::load_messages_optional(
            source,
            &m.messages.index,
            m.messages.ordinal,
            m.messages.name_hash,
            ids,
        )
    })?;
    Ok(CampSource {
        menu,
        entries,
        font,
        labels,
        name,
        help,
        banner,
        stats,
        config,
        numbers,
        items,
        icons,
        abilities,
        weapon_ability,
        party_help,
        customize,
        equip_texts,
        equip_stats,
        picture_sequence: bar_entry(system, b"ipic")
            .ok()
            .map(data::sprite_sequence::read)
            .transpose()?,
        notice_headings: {
            let mut h = msg(&pause_menu::notice::HEADING_IDS)?.into_iter();
            std::array::from_fn(|_| h.next().unwrap_or_default())
        },
        notice_page: msg(&[pause_menu::notice::PAGE_ID])?
            .pop()
            .unwrap_or_default(),
        notice_callouts: {
            let texts = msg(&[0x561e, 0x561f, 0x5620])?;
            let file = read_named(source, "Image/dt/kh2_first.hed", "THelpText.bin")?;
            file.map_or_else(Vec::new, |b| {
                pause_menu::notice::parse_callouts(&b, |id| {
                    texts
                        .get(id.wrapping_sub(0x561e) as usize)
                        .cloned()
                        .unwrap_or_default()
                })
            })
        },
    })
}

/// The movement block (obj+0x128) of the object named `name`:
/// FUN_14040a440 defaults with SSTM FallMax, then FUN_14040a480 copies the
/// PRTY record named by its objentry +0x4e (FUN_1403e9590 lookup) when it
/// is nonzero. Returns the parameters, the PRTY index and its offset.
fn player_params(
    objects: &[objentry::ObjectEntry],
    name: &str,
    pref: &[u8],
    sstm: &actor_physics::Sstm,
) -> Result<(Params, i32, Option<u32>), Box<dyn std::error::Error>> {
    let mut obj_params = actor_physics::ObjParams::defaults(sstm.fall_max);
    let player_object = objects
        .iter()
        .find(|o| o.model_name == name.as_bytes())
        .ok_or("player object entry absent")?;
    let prty_index = player_object.neo_moveset as i16 as i32;
    let mut prty_offset = None;
    if prty_index != 0 {
        let prty = party_records::read_prty(bar_entry(pref, b"prty")?)?;
        let offset =
            actor_physics::prty_offset(&prty.offsets, prty_index).ok_or("PRTY record absent")?;
        let record = prty
            .records
            .iter()
            .find(|r| r.offset == offset && !r.overlaps_table)
            .ok_or("PRTY record absent or overlaps table")?;
        obj_params.apply_prty(&record.words);
        prty_offset = Some(offset);
    }
    let params = Params {
        walk_speed: obj_params.walk,
        run_speed: obj_params.run,
        jump_height: obj_params.jump,
        turn_speed: obj_params.turn,
        fall_max: obj_params.fall_max,
        land_inertia: obj_params.word10,
        land_drag: obj_params.word11,
        step_height: STEP_HEIGHT,
        walk_threshold: WALK_THRESHOLD,
        radius: BODY_RADIUS,
        height: BODY_HEIGHT,
        disarm_ticks: DISARM_TICKS,
    };
    params.validate()?;
    Ok((params, prty_index, prty_offset))
}

fn load(
    source: &Arc<ProbeSource>,
    config_path: &Path,
    basic_fk: bool,
) -> Result<Loaded, Box<dyn std::error::Error>> {
    let config: Config = serde_json::from_slice(&read_limited(config_path)?)?;
    if config.schema_version != 1 || config.source_content_identity != source.content_identity() {
        return Err("test room configuration identity/schema invalid".into());
    }
    let base = config_path.parent().ok_or("configuration directory")?;
    // The session check (executable and indexes rehashed, every package
    // byte read verified by chunk) admits this session.
    source.verify_session()?;
    let mut actors =
        model_inspection::read_selected_models(source, &base.join(&config.actor_selection))?;
    if actors.len() != 1 {
        return Err("test room requires exactly one character model".into());
    }
    let actor = actors.remove(0);
    let mut weapons =
        model_inspection::read_selected_models(source, &base.join(&config.weapon_selection))?;
    if weapons.len() != 1 {
        return Err("test room requires exactly one weapon model".into());
    }
    let weapon = weapons.remove(0);
    let objects = objentry::read(&read_archive(source, &config.objentry)?)?;
    let object = objects
        .iter()
        .find(|o| o.model_name == config.object_model_name.as_bytes())
        .ok_or("weapon object entry absent")?;
    let objentry_joint = object.weapon_joint as usize;
    let object_candidates: Vec<_> = objects
        .iter()
        .filter(|o| o.model_name.starts_with(b"P_EX100") || o.model_name.starts_with(b"W_EX010"))
        .map(|o| serde_json::json!({"id":o.object_id,"type":o.object_type,"joint":o.weapon_joint,
            "model":String::from_utf8_lossy(&o.model_name),"motion":String::from_utf8_lossy(&o.animation_name)}))
        .collect();
    // PRTY walk/run/jump/turn and SSTM fall max (public labels).
    let system = read_archive(source, &config.system)?;
    let weapon_joint = match config.sklt_character_id {
        Some(id) => {
            let entries = sklt::read(bar_entry(&system, b"sklt")?)?;
            let entry = entries
                .iter()
                .find(|e| e.character_id == id)
                .ok_or("SKLT entry for the character absent")?;
            usize::try_from(entry.primary_bone).map_err(|_| "SKLT primary bone negative")?
        }
        None => objentry_joint,
    };
    if weapon_joint >= actor.skinning.bones.len() {
        return Err("weapon joint outside the actor skeleton".into());
    }
    let mut area_load: Option<(AreaSource, LoadedArea)> = None;
    // An area decodes on a worker thread while Sora's kit loads below.
    let mut area_job = None;
    let (room, arena_mesh) = match (&config.arena, &config.room_selection) {
        (Some(a), _) => {
            let a = a.expanded()?;
            let (room, mesh) = visual::room::arena(a.half_size, a.wall_height, a.tile);
            (Some(room), Some(mesh))
        }
        (None, Some(path)) => (Some(load_rooms(source, &base.join(path))?), None),
        (None, None) => match &config.area {
            Some(a) => {
                let world = ::area::progress::world_index(&a.world).ok_or("unknown world code")?;
                let story: Vec<(u32, u32)> = a.story.iter().map(|r| (r[0], r[1])).collect();
                let source = Arc::clone(source);
                let (number, entrance, programs) = (a.area, a.entrance, a.programs);
                area_job = Some(std::thread::Builder::new().name("area load".into()).spawn(
                    move || {
                        let mut src =
                            AreaSource::open(source, &story).map_err(|e| e.to_string())?;
                        let (room, loaded) = src
                            .load(world, number, entrance, programs)
                            .map_err(|e| e.to_string())?;
                        Ok::<_, String>((room, src, loaded))
                    },
                )?);
                (None, None)
            }
            None => {
                return Err("configuration needs a room selection, an area or an arena".into());
            }
        },
    };

    // Movement block (obj+0x128): FUN_14040a440 defaults with SSTM
    // FallMax, then FUN_14040a480 copies the PRTY record named by the
    // player's objentry +0x4e (FUN_1403e9590 lookup) when it is nonzero.
    let pref = bar_entry(&system, b"pref")?;
    let sstm_bytes = bar_entry(pref, b"sstm")?;
    let sstm_offset = u32::from_le_bytes(sstm_bytes.get(4..8).ok_or("SSTM pointer")?.try_into()?);
    let sstm = sstm_bytes
        .get(sstm_offset as usize..)
        .and_then(actor_physics::Sstm::read)
        .ok_or("SSTM record truncated")?;
    let battle = read_archive(source, &config.battle)?;
    let ability_source = {
        let rates = sstm_bytes
            .get(sstm_offset as usize..)
            .and_then(abilities::Rates::read)
            .ok_or("SSTM record truncated")?;
        let item = bar_entry(&system, b"item")?;
        let table = data::item_table::read(item)?;
        let stats = data::item_table::read_stats(item)?;
        // Ability items (types 0x12..0x14): bit at +4, action/growth class at +6.
        let items = table
            .iter()
            .filter(|r| (0x12..=0x14).contains(&r.kind))
            .map(|r| (r.id, (r.amount, r.weapon_stats & 0xff != 0)))
            .collect();
        // FUN_1403e3ef0 selects weapons' +6 or armor/accessories' +4.
        let equipment_abilities = table
            .iter()
            .filter_map(|r| {
                let stat = r.equipment_stats_id()?;
                stats
                    .iter()
                    .find(|s| s.id == stat)
                    .map(|s| (r.id, s.ability))
            })
            .collect();
        AbilitySource {
            rates,
            items,
            equipment_abilities,
            rows: gear::rows(&table, &stats),
            owned: gear::owned(&table),
            inventory: ability_inventory::load(&table, &battle)?,
        }
    };
    let (params, prty_index, prty_offset) =
        player_params(&objects, &config.player_object_name, pref, &sstm)?;
    // The growth abilities' rows (pref `fmab`, FUN_1403e92d0).
    let fmab = bar_entry(pref, b"fmab")
        .ok()
        .and_then(player::growth::Fmab::read)
        .unwrap_or_default();

    // PTYA attack entries chosen by ID for the grounded and aerial combos.
    let hud = match &config.hud {
        Some(l) => {
            let mut sheets = vec![hud::load(
                source,
                &l.index,
                l.ordinal,
                l.name_hash,
                b"gaug",
            )?];
            if let Some(f) = &config.face {
                sheets.push(hud::load(
                    source,
                    &f.index,
                    f.ordinal,
                    f.name_hash,
                    b"face",
                )?);
            }
            let menu = match &config.menu {
                Some(m) => Some((
                    hud::load(
                        source,
                        &m.command.index,
                        m.command.ordinal,
                        m.command.name_hash,
                        b"comw",
                    )?,
                    hud::load_system_font(
                        source,
                        (
                            &m.font_image.index,
                            m.font_image.ordinal,
                            m.font_image.name_hash,
                        ),
                        (
                            &m.font_info.index,
                            m.font_info.ordinal,
                            m.font_info.name_hash,
                        ),
                    )?,
                    hud::load_messages(
                        source,
                        &m.messages.index,
                        m.messages.ordinal,
                        m.messages.name_hash,
                        &MENU_MESSAGES,
                    )?,
                )),
                None => None,
            };
            // The same field2d archive (<world>0field.2dd) holds 'mapj', the
            // area-change iris (FUN_140153e50 table entry 2) [exit-01 §4.2].
            let iris = match config.area {
                Some(_) => Some(hud::load(
                    source,
                    &l.index,
                    l.ordinal,
                    l.name_hash,
                    b"mapj",
                )?),
                None => None,
            };
            Some(HudSource {
                field: hud::load(source, &l.index, l.ordinal, l.name_hash, b"fiel")?,
                sheets,
                iris,
                stats: hud::starting_stats(bar_entry(&battle, b"plrp")?)?,
                menu,
            })
        }
        None => None,
    };
    let ptya_bytes = bar_entry(&battle, b"ptya")?;
    // The six spells' tables and cast motions (magc, pref/magi, cmd).
    let mut magic_src = magic_room::load(&battle, pref, &system, 70.)
        .map_err(|e| format!("magic tables: {e}"))
        .ok();
    if let Some(m) = magic_src.as_mut() {
        magic_room::load_effects(source, m);
    }
    let spell_ids = magic_src
        .as_ref()
        .map(|m| magic_room::motion_ids(&m.records))
        .unwrap_or_default();
    let selection = motion::MotionSetSelection {
        index: config.motion_set.index.clone(),
        ordinal: config.motion_set.ordinal,
        name_hash: config.motion_set.name_hash,
        battle: config.battle_mode,
        has_weapon: config.has_weapon,
    };
    let weapon_set = config
        .weapon_motion_set
        .as_ref()
        .map(|set| motion::MotionSetSelection {
            index: set.index.clone(),
            ordinal: set.ordinal,
            name_hash: set.name_hash,
            battle: config.battle_mode,
            has_weapon: config.has_weapon,
        });
    // Sora's HIT spheres (object collision type 2), for enemy attacks.
    let collision_archive = match &config.enemy {
        Some(e) => Some(read_archive(source, &e.actor_model)?),
        None => None,
    };
    // Sora's object effects (obj/P_EX100.a.us), 00effect's and the
    // resident common effect textures.
    let (base_pax, texcommon, prize_pax) = match data::path_lookup::Library::open(source) {
        Ok(mut library) => (
            drive_room::object_pax(&mut library, &config.player_object_name),
            library
                .read("effect/texcommon.dpd")
                .ok()
                .and_then(|b| object_effect::read_dpd(&b).ok()),
            drive_room::file_pax(&mut library, "obj/PRIZE.mdlx"),
        ),
        Err(_) => (None, None, None),
    };
    let system_pax = config
        .effects
        .as_ref()
        .and_then(|e| e.system.as_ref())
        .and_then(|l| visual::effects::load_pax(source, &l.index, l.ordinal, l.name_hash).ok());
    let kit = character_kit::load_kit(
        source,
        character_kit::KitInputs {
            form: 0,
            actor,
            weapon,
            weapon2: None,
            blades: Default::default(),
            weapon_pax: None,
            weapon_joint,
            second_joint: None,
            motion_set: selection.clone(),
            fallback: None,
            weapon_motion_set: weapon_set.clone(),
            second_weapon_motion_set: None,
            ptya: ptya_bytes,
            ptya_pointer: config.ptya_pointer,
            spell_ids: spell_ids.clone(),
            hurt: config.enemy.is_some(),
            collision_archive,
            basic_fk,
            params,
            pax: base_pax,
            scripted: Vec::new(),
        },
    )?;
    let sora_body = kit.body.clone();
    let (sora_target_radius, sora_target_lift) = (kit.target_radius, kit.target_lift);
    let (sora_body_lift, sora_heal_lift) = (kit.body_lift, kit.heal_lift);
    let forms = drive_room::FormSources::new(
        &system,
        &battle,
        &objects,
        weapon_joint,
        &config.player_object_name,
        selection,
        spell_ids,
        config.enemy.is_some(),
        basic_fk,
        sstm,
    )
    .inspect_err(|e| eprintln!("Drive forms unavailable: {e}"))
    .ok()
    .map(|mut f| {
        f.system_pax = system_pax.map(Arc::new);
        f.texcommon = texcommon;
        f.prize_pax = prize_pax.map(Arc::new);
        f
    });
    // The forms' and Revert's names for the command menu.
    let forms = match (forms, &config.menu) {
        (Some(mut f), Some(menu)) => {
            let mut wanted = f.named_commands();
            wanted.push((0, MENU_MESSAGES[0]));
            wanted.push((0, MENU_MESSAGES[1]));
            wanted.push((0, MENU_MESSAGES[3]));
            wanted.push((0, drive_room::REVERT_LABEL));
            let ids: Vec<u32> = wanted.iter().map(|w| w.1).collect();
            let texts = hud::load_messages_optional(
                source,
                &menu.messages.index,
                menu.messages.ordinal,
                menu.messages.name_hash,
                &ids,
            )?;
            for ((cmd, id), text) in wanted.into_iter().zip(texts) {
                let text = text.unwrap_or_default();
                match id {
                    drive_room::REVERT_LABEL => f.revert_label = text,
                    i if i == MENU_MESSAGES[3] && cmd == 0 => f.drive_label = text,
                    i if i == MENU_MESSAGES[0] && cmd == 0 => f.attack_label = text,
                    i if i == MENU_MESSAGES[1] && cmd == 0 => f.magic_label = text,
                    _ => {
                        f.names.insert(cmd, text);
                    }
                }
            }
            // The level-up panels' names (item table +8) and sys texts.
            let table = data::item_table::read(bar_entry(&system, b"item")?)?;
            let mut items: Vec<u16> = drive_room::FORM_ITEMS.to_vec();
            items.extend(player::forms::GROWTH_ITEMS.iter().flatten());
            items.extend(f.fmlv.iter().map(|r| r.ability).filter(|&a| a != 0));
            // Sora's level rows and bonus levels teach abilities (types
            // 0x12..=0x14): their names are panel lines.
            items.extend(
                table
                    .iter()
                    .filter(|r| (0x12..=0x14).contains(&r.kind))
                    .map(|r| r.id),
            );
            items.sort_unstable();
            items.dedup();
            let rows: Vec<_> = items
                .iter()
                .filter_map(|id| table.iter().find(|r| r.id == *id))
                .collect();
            let mut ids: Vec<u32> = rows.iter().map(|r| r.name as u32).collect();
            ids.extend([0x43f2, 0x51f]);
            ids.extend(level_growth::STAT_MESSAGES);
            ids.extend(level_growth::BONUS_MESSAGES);
            ids.extend(level_growth::NAME_MESSAGES);
            let texts = hud::load_messages_optional(
                source,
                &menu.messages.index,
                menu.messages.ordinal,
                menu.messages.name_hash,
                &ids,
            )?;
            for (r, text) in rows.iter().zip(&texts) {
                if let Some(t) = text {
                    f.item_names.insert(r.id, t.clone());
                }
            }
            let n = rows.len();
            f.gauge_up = texts.get(n).cloned().flatten().unwrap_or_default();
            f.sora_title = texts.get(n + 1).cloned().flatten().unwrap_or_default();
            let text = |k: usize| texts.get(k).cloned().flatten().unwrap_or_default();
            for (i, t) in f.growth.stat.iter_mut().enumerate() {
                *t = text(n + 2 + i);
            }
            for (i, t) in f.growth.bonus.iter_mut().enumerate() {
                *t = text(n + 6 + i);
            }
            for (i, t) in f.growth.names.iter_mut().enumerate() {
                *t = text(n + 12 + i);
            }
            Some(f)
        }
        (f, _) => f,
    };
    // The enemy's character id (objentry +0x4c) by its model's name hash:
    // Wisdom and Final Form EXP count defeated Heartless / Nobodies.
    let enemy_character = config.enemy.as_ref().map_or(0, |e| {
        objects
            .iter()
            .find(|o| {
                let name = format!("obj/{}.mdlx", String::from_utf8_lossy(&o.model_name));
                data::path_lookup::md5(name.as_bytes()) == e.model.name_hash
            })
            .map_or(0, |o| o.character)
    });

    let room = match (room, area_job) {
        (Some(room), _) => room,
        (None, Some(job)) => {
            let (room, src, loaded) = job.join().map_err(|_| "area load thread panicked")??;
            area_load = Some((src, loaded));
            room
        }
        (None, None) => return Err("no room loaded".into()),
    };
    // Floors must carry partyStand and walls partyCollide (public labels).
    let triangles = TriangleWorld::new(
        room.collision_triangles
            .iter()
            .filter(|t| t.surface_flags & (PARTY_STAND | PARTY_COLLIDE) != 0)
            .map(|t| {
                (
                    t.corners.map(|v| v.to_array()),
                    t.surface_flags & PARTY_STAND != 0,
                    t.surface_flags & PARTY_COLLIDE != 0,
                )
            }),
        FLOOR_MIN_NORMAL_Y,
    );
    let (floors, walls) = triangles.counts();
    let world = match &mut area_load {
        Some((_, a)) => RoomWorld::Map(MapWorld::new(a.take_collision())),
        None => RoomWorld::Triangles(triangles),
    };
    let mut flag_counts: HashMap<u32, usize> = HashMap::new();
    for t in &room.collision_triangles {
        *flag_counts.entry(t.surface_flags).or_default() += 1;
    }
    let flag_counts: HashMap<String, usize> = flag_counts
        .into_iter()
        .map(|(f, n)| (format!("{f:#x}"), n))
        .collect();
    let low = room
        .frame_points
        .iter()
        .fold(Vec3::splat(f32::MAX), |a, &b| a.min(b));
    let high = room
        .frame_points
        .iter()
        .fold(Vec3::splat(f32::MIN), |a, &b| a.max(b));
    let levels = floor_levels(&room.collision_triangles);
    let weapon_points: Vec<Vec3> = kit.weapon.poses[0]
        .position_bits
        .iter()
        .map(|p| Vec3::from_array(p.map(f32::from_bits)))
        .collect();
    let weapon_bounds = [
        weapon_points
            .iter()
            .fold(Vec3::splat(f32::MAX), |a, &b| a.min(b))
            .to_array(),
        weapon_points
            .iter()
            .fold(Vec3::splat(f32::MIN), |a, &b| a.max(b))
            .to_array(),
    ];
    let idle_clip = kit
        .actions
        .iter()
        .find(|(a, _)| *a == (Action::Idle, true))
        .map_or(0, |a| a.1);
    let idle_hand = kit.clip_poses[idle_clip].weapon[0].to_cols_array();
    let weapon_root = kit.weapon.poses[0].matrix_bits[0].map(f32::from_bits);
    let [sx, sy, sz] = match (config.spawn, &area_load) {
        (placed, Some((_, a))) => placed.unwrap_or(a.player.0.to_array()),
        (Some(spawn), None) => spawn,
        // Default: the walkable level with the most floor area.
        (None, None) => levels.first().map(|l| l.1).ok_or("no walkable floor")?,
    };
    // An area places Sora at the entrance's authored point without a
    // ground snap (FUN_1403b3200); the first physics frames settle him.
    let (spawn, yaw) = match &area_load {
        Some((_, a)) => (
            config.spawn.map_or(a.player.0, Vec3::from_array),
            config.spawn_yaw.unwrap_or(a.player.1),
        ),
        None => {
            let ground_height = world
                .floor(sx, sz, sy)
                .ok_or("no walkable collision floor below the spawn point")?;
            (Vec3::new(sx, ground_height, sz), 0.)
        }
    };
    let summary = serde_json::json!({"event":"admission","source_qualification":"quarantined_unqualified",
        "params":{"walk_speed":params.walk_speed,"run_speed":params.run_speed,"jump_height":params.jump_height,
            "turn_speed":params.turn_speed,"fall_max":params.fall_max,"gravity":actor_physics::G,"run_magnitude":WALK_THRESHOLD,
            "prty_index":prty_index,"prty_offset":prty_offset,"land_inertia":params.land_inertia,"land_drag":params.land_drag,"sstm":format!("{sstm:?}")},
        "test_room_choices":{"step_height":STEP_HEIGHT,
            "body_radius":BODY_RADIUS,"body_height":BODY_HEIGHT,"floor_min_normal_y":FLOOR_MIN_NORMAL_Y,
            "blend_ticks":"per action (player::blend_for, PTYA +0x38 after attacks)","clock":"fixed 60 Hz, one motion tick per update"},
        "ptya":{"records":kit.set.iter().map(|e| serde_json::json!({"id":e.id,"kind":e.kind,"alternate":e.sub,"combo_offset":e.combo_offset,"flag":e.flag,"motion_id":e.motion_id,"ability":e.ability,"floats":e.float_bits.map(f32::from_bits)})).collect::<Vec<_>>(),
            "legacy_combo_ids":[&config.ground_combo_ids,&config.air_combo_ids]},
        "motions":kit.summary["motions"],"collision":{"floors":floors,"walls":walls,"surface_flags":flag_counts,"bounds":[low.to_array(),high.to_array()],
            "largest_levels":levels.iter().take(8).map(|(area,p)| serde_json::json!({"area":area,"point":p})).collect::<Vec<_>>()},
        "spawn":spawn.to_array(),"pose_evaluator":if basic_fk {"basic public FK (T053)"} else {"motion_eval IK/constraints (T057)"},
        "evaluation_totals":kit.summary["evaluation_totals"],"weapon_joint":weapon_joint,"objentry_joint":objentry_joint,"weapon_bounds":weapon_bounds,"idle_hand_matrix":idle_hand,"weapon_root_matrix":weapon_root,"weapon_bones":kit.weapon.skinning.bones.len(),"object_candidates":object_candidates,
        "object_count":objects.len(),
        "missing":["scale channels","up-vector/path/camera constraint types","limiter damping","root rotation","weapon constraint motion","hit detection","enemies","measured installed timing"],
        "inferred_rules":["direction constraints aim local +X","two-joint IK bends the middle joint about local Z","expression list/condition semantics","Hermite tangents scaled by key span","field/battle mode switch with 6 s disarm timeout"],
        "gameplay_verified":false});
    let mut banks = Banks::default();
    for b in &config.sounds {
        let l = &b.locator;
        banks.insert(sound::load_bank(
            source,
            &l.index,
            l.ordinal,
            l.name_hash,
            b.replacement.as_deref(),
        )?)?;
    }
    // Sora's battle voices: `voice/us/battle/<world>0_sora.win32.scd` (the
    // arena counts as Twilight Town [I]) and 00battle `vtbl`.
    let voice_world = config
        .area
        .as_ref()
        .map_or("tt".to_owned(), |a| a.world.clone());
    let voices = data::path_lookup::Library::open(source)
        .ok()
        .and_then(|mut library| {
            library
                .read(&format!("voice/us/battle/{voice_world}0_sora.win32.scd"))
                .ok()
        })
        .and_then(|bytes| banks.insert(bytes).ok())
        .and_then(|bank| drive_room::Voices::read(bar_entry(&battle, b"vtbl").ok()?, bank));
    // Footsteps: the world's 12soundinfo rows and every Sora step bank
    // they name (se<bank>.win32.scd).
    let step_info = area_load.as_ref().and_then(|(_, a)| {
        let code = ::area::progress::WORLDS.get(a.world as usize)?;
        let mut library = data::path_lookup::Library::open(source).ok()?;
        let bytes = library.read("12soundinfo.bar").ok()?;
        let table = data::bar::read_bar(&bytes).ok()?;
        let entry = table
            .entries
            .iter()
            .find(|e| e.name_bytes[..2] == *code.as_bytes())?;
        let start = entry.offset as usize;
        let info = visual::footsteps::WorldInfo::read(
            bytes.get(start..start.checked_add(entry.size as usize)?)?,
        )?;
        for bank in info.sora_banks().into_iter().chain(info.ambient_banks()) {
            // An ambient bank is a long stream bank, past the 32 MiB default.
            match library.read_limited(&format!("se/se{bank:03}.win32.scd"), SOUND_BANK_LIMIT) {
                Ok(bank_bytes) => {
                    if let Err(error) = banks.insert(bank_bytes) {
                        eprintln!("Sound bank {bank} not registered: {error}");
                    }
                }
                Err(error) => eprintln!("Sound bank {bank} not read: {error}"),
            }
        }
        Some((a.world, info))
    });
    let npc = match &config.npc {
        Some(n) => Some(load_npc(source, base, &config, n)?),
        None => None,
    };
    let effects = match &config.effects {
        Some(e) => {
            use visual::effects::{load_atlas, load_pax};
            let pax = |l: &Locator| load_pax(source, &l.index, l.ordinal, l.name_hash);
            Some(EffectSources {
                weapon: pax(&e.weapon)?,
                enemy: e.enemy.as_ref().map(pax).transpose()?,
                enemy_paxes: data::path_lookup::Library::open(source)
                    .map(|mut library| {
                        enemy_battle::SUPPORTED
                            .iter()
                            .filter_map(|&(id, name)| {
                                drive_room::object_pax(&mut library, name).map(|p| (id, p))
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                system: e.system.as_ref().map(pax).transpose()?,
                atlas: load_atlas(source, &e.atlas.index, e.atlas.ordinal, e.atlas.name_hash)?,
            })
        }
        None => None,
    };
    let camp = match (&config.camp, &config.menu) {
        (Some(l), Some(m)) => Some(load_camp_source(source, l, m, &battle, &system)?),
        _ => None,
    };
    let journal = match (&config.journal, &config.menu, &camp) {
        (Some(j), Some(m), Some(_)) => Some(load_journal(source, j, &m.messages, &system)?),
        _ => None,
    };
    let music = match &config.music {
        Some(m) => Some([
            sound::load_bank(
                source,
                &m.field.index,
                m.field.ordinal,
                m.field.name_hash,
                None,
            )?,
            sound::load_bank(
                source,
                &m.battle.index,
                m.battle.ordinal,
                m.battle.name_hash,
                None,
            )?,
        ]),
        None => None,
    };
    let magic_src = magic_src.map(|mut m| {
        m.target_lift = sora_target_lift;
        m.body_lift = sora_body_lift;
        m.heal_lift = sora_heal_lift;
        m
    });
    // The spells' names for the Magic submenu (cmd +8 message ids).
    let magic_src = match (magic_src, &config.menu) {
        (Some(mut m), Some(menu)) => {
            let mut wanted: Vec<(u16, u32)> = m
                .records
                .iter()
                .filter_map(|r| m.commands.get(&r.command).map(|c| (r.command, c.name)))
                .collect();
            // The party list's names (message 0x851e + the character id).
            wanted.extend((0..3).map(|i| (magic_room::MEMBER_NAMES + i, 0x51f + i as u32)));
            // The item commands (execute 3) for the Items submenu.
            wanted.extend(
                m.commands
                    .values()
                    .filter(|c| c.execute == 3)
                    .map(|c| (c.id, c.name)),
            );
            let ids: Vec<u32> = wanted.iter().map(|w| w.1).collect();
            let texts = hud::load_messages_optional(
                source,
                &menu.messages.index,
                menu.messages.ordinal,
                menu.messages.name_hash,
                &ids,
            )?;
            for ((cmd, _), text) in wanted.into_iter().zip(texts) {
                if let Some(t) = text {
                    m.names.insert(cmd, t);
                }
            }
            Some(m)
        }
        (m, _) => m,
    };
    let enemy = match &config.enemy {
        Some(e) => Some(load_enemy(
            source,
            base,
            e,
            &battle,
            &kit.clip_poses,
            sora_body,
            sora_target_radius,
        )?),
        None => None,
    };
    let area_talk = match (&area_load, &config.menu) {
        (Some((src, area)), Some(menu)) => {
            let loc = |name: &str| src.locate(name).ok_or(format!("{name} is not installed"));
            let (fi, fm) = (loc("msg/us/fontinfo.bar")?, loc("msg/us/fontimage.bar")?);
            let code = ::area::progress::WORLDS[area.world as usize];
            let wm = loc(&format!("msg/us/{code}.bar"))?;
            let sheet = hud::load_window_sheet(
                source,
                (fi.0.as_str(), fi.1, fi.2),
                (wm.0.as_str(), wm.1, wm.2),
            )?;
            let font = hud::load_font(
                source,
                (fm.0.as_str(), fm.1, fm.2),
                (fi.0.as_str(), fi.1, fi.2),
                b"evt\0",
            )?;
            let colour = hud::ftst_colour(
                &read_archive(source, &config.system)?,
                5,
                area.world as usize,
            )?;
            let label = hud::load_messages(
                source,
                &menu.messages.index,
                menu.messages.ordinal,
                menu.messages.name_hash,
                &[TALK_LABEL],
            )?
            .remove(0);
            Some(AreaTalk {
                sheet,
                font,
                colour,
                label,
            })
        }
        _ => None,
    };
    let keyblades = bar_entry(&system, b"went")
        .and_then(|went| {
            crate::keyblade_board::Keyblades::read(
                went,
                bar_entry(&system, b"item")?,
                &objects,
                &config.player_object_name,
            )
        })
        .unwrap_or_else(|e| {
            eprintln!("Keyblade models not read: {e}");
            Default::default()
        });
    eprintln!(
        "Equipped Keyblade {KINGDOM_KEY:#x} model: {:?} (room weapon {}), motions {:?}",
        keyblades.model(KINGDOM_KEY),
        config.object_model_name,
        keyblades
            .model(KINGDOM_KEY)
            .and_then(|m| keyblades.animation(m)),
    );
    let board_motion = config.weapon_motion_set.as_ref().and_then(|set| {
        let selection = motion::MotionSetSelection {
            index: set.index.clone(),
            ordinal: set.ordinal,
            name_hash: set.name_hash,
            battle: true,
            has_weapon: true,
        };
        motion::read_motion_ids(source, &selection, &[0])
            .inspect_err(|e| eprintln!("Keyblade idle motion: {e}"))
            .ok()?
            .pop()
    });
    // The game-over sequence (obj/GAMEOVER.mset, pause.2ld, the death event).
    let game_over = {
        let joint = objects
            .iter()
            .find(|o| o.model_name == b"P_EX100")
            .map_or(1, |o| o.weapon_joint);
        match data::path_lookup::Library::open(source)
            .map_err(|e| e.to_string())
            .and_then(|mut library| {
                game_over::Source::load(source, &mut library, joint).map_err(|e| e.to_string())
            }) {
            Ok(g) => Some(g),
            Err(e) => {
                eprintln!("Game over assets unavailable: {e}");
                None
            }
        }
    };
    Ok(Loaded {
        board_motion,
        keyblades,
        weapon_model: config.object_model_name.clone(),
        room,
        arena_mesh,
        area_talk,
        stay: config.area.as_ref().is_some_and(|a| a.stay),
        params,
        sstm,
        abilities: ability_source,
        magic: magic_src,
        items: item_room::load(&system).ok(),
        party: match config.enemy {
            // The party stands in rooms with a fight to join.
            Some(_) => {
                let mut kinds = Vec::new();
                if let Ok(mut library) = data::path_lookup::Library::open(source) {
                    let lvup = bar_entry(&battle, b"lvup").unwrap_or(&[]);
                    let plrp = bar_entry(&battle, b"plrp").unwrap_or(&[]);
                    for (name, character, records) in [
                        ("P_EX020", 2, &player::party_members::DONALD[..]),
                        ("P_EX030", 3, &player::party_members::GOOFY[..]),
                    ] {
                        match party_room::load(
                            source,
                            &mut library,
                            &system,
                            lvup,
                            plrp,
                            name,
                            character,
                            records,
                        ) {
                            Ok(k) => {
                                eprintln!("party member {} loaded", k.name);
                                kinds.push(std::sync::Arc::new(k));
                            }
                            Err(e) => eprintln!("party member {name}: {e}"),
                        }
                    }
                }
                kinds
            }
            None => Vec::new(),
        },
        forms,
        enemy_character,
        fmab,
        voices,
        step_info,
        kit,
        spawn,
        yaw,
        world,
        area: area_load,
        summary,
        hud,
        enemy,
        banks,
        effects,
        npc,
        camp,
        journal,
        game_over,
        language: match (&config.camp, &config.menu) {
            (Some(camp), Some(menu)) => Some(settings_room::LanguageSources {
                camp: camp.clone(),
                menu: menu.clone(),
                journal: config.journal.clone(),
                battle: config.battle.clone(),
                system: config.system.clone(),
            }),
            _ => None,
        },
        music,
    })
}

/// Shadow (or another enemy) for the test room: model, direct-slot clips,
/// HIT spheres, the Keyblade's attack spheres and the ATKP/ENMP/LVPM/LVUP
/// rows the damage routine reads.
fn load_enemy(
    source: &ProbeSource,
    base: &Path,
    config: &EnemyConfig,
    battle: &[u8],
    clip_poses: &[ClipPoses],
    sora_body: Vec<(usize, Vec3, f32, bool)>,
    sora_target_radius: f32,
) -> Result<EnemySource, Box<dyn std::error::Error>> {
    use data::battle_tables;
    let mut models =
        model_inspection::read_selected_models(source, &base.join(&config.model_selection))?;
    if models.len() != 1 {
        return Err("enemy selection requires exactly one model".into());
    }
    let model = models.remove(0);
    let collision =
        object_collision::read(bar_entry_typed(&read_archive(source, &config.model)?, 23)?)?;
    // FUN_1403d7520: Sora's TARGET radius plus the target's own shape
    // radius (FUN_1403be0f0) [I: the enemy's TARGET entry].
    let target_reach = sora_target_radius
        + collision
            .iter()
            .find(|c| c.kind == object_collision::TARGET)
            .map_or(0., |c| c.radius as f32);
    let mut claws: HashMap<u8, Vec<(usize, Vec3, f32)>> = HashMap::new();
    for e in collision
        .iter()
        .filter(|e| e.kind == object_collision::ATTACK && e.bone >= 0)
    {
        claws.entry(e.group).or_default().push((
            e.bone as usize,
            Vec3::new(
                e.position[0] as f32,
                e.position[1] as f32,
                e.position[2] as f32,
            ),
            e.radius as f32,
        ));
    }
    let body: Vec<_> = collision
        .iter()
        .filter(|e| e.kind == object_collision::HIT && e.group == 0 && e.bone >= 0)
        .map(|e| {
            (
                e.bone as usize,
                Vec3::new(
                    e.position[0] as f32,
                    e.position[1] as f32,
                    e.position[2] as f32,
                ),
                e.radius as f32,
                matches!(e.argument, 0 | 2 | 3 | 4 | 5),
            )
        })
        .collect();
    // TARGET entries on bones: what the attack chooser and the automatic
    // target measure (FUN_1403bdac0 / FUN_1403be4b0).
    let targets: Vec<_> = collision
        .iter()
        .filter(|e| e.kind == object_collision::TARGET && e.bone >= 0)
        .map(|e| {
            (
                e.bone as usize,
                Vec3::new(
                    e.position[0] as f32,
                    e.position[1] as f32,
                    e.position[2] as f32,
                ),
                e.radius as f32,
                e.height as f32,
            )
        })
        .collect();
    let weapon_collision = object_collision::read(bar_entry_typed(
        &read_archive(source, &config.weapon_model)?,
        23,
    )?)?;
    let mut blade: HashMap<u8, Vec<(Vec3, f32)>> = HashMap::new();
    for e in weapon_collision
        .iter()
        .filter(|e| e.kind == object_collision::ATTACK && e.bone == 2)
    {
        blade.entry(e.group).or_default().push((
            Vec3::new(
                e.position[0] as f32,
                e.position[1] as f32,
                e.position[2] as f32,
            ),
            e.radius as f32,
        ));
    }
    let slots = shadow::motion::ALL;
    let selected = motion::read_slots(
        source,
        &config.motion_set.index,
        config.motion_set.ordinal,
        config.motion_set.name_hash,
        &slots,
    )?;
    let mut clips = HashMap::new();
    for sel in &selected {
        let (clip, key_start) = clip_for(sel)?;
        let mut frames = Vec::new();
        let mut bones = Vec::new();
        let mut root = Vec::new();
        let windows: Vec<(u32, u32, u16, u8)> = sel
            .triggers
            .iter()
            .flat_map(|t| &t.records.ranges)
            .filter(|r| r.trigger == 10 && r.parameters.len() >= 2)
            .map(|r| {
                (
                    r.start_frame.max(0) as u32,
                    if r.end_frame < 0 {
                        clip.end
                    } else {
                        r.end_frame as u32
                    },
                    r.parameters[0] as u16,
                    r.parameters[1] as u8,
                )
            })
            .collect();
        // The bones of the HIT shapes, the claws and the TARGET shapes (the
        // lock-on and the party's spells read the last).
        let mut bone_list: Vec<usize> = body
            .iter()
            .map(|b| b.0)
            .chain(claws.values().flatten().map(|c| c.0))
            .chain(targets.iter().map(|t| t.0))
            .collect();
        bone_list.sort_unstable();
        bone_list.dedup();
        for offset in 0..=clip.end {
            let (pose, evaluated) = model
                .skinning
                .sample_evaluated(&sel.motion, (key_start + offset) as f32)?;
            let r = Vec3::from_array(evaluated.root_translation);
            root.push(r);
            frames.push(
                pose.position_bits
                    .iter()
                    .map(|p| Vec3::from_array(p.map(f32::from_bits)) - r)
                    .collect(),
            );
            bones.push(
                bone_list
                    .iter()
                    .filter_map(|&b| {
                        let m = pose.matrix_bits.get(b)?;
                        let mut m = Mat4::from_cols_array(&m.map(f32::from_bits));
                        m.w_axis -= r.extend(0.);
                        Some((b, m))
                    })
                    .collect(),
            );
        }
        clips.insert(
            sel.slot,
            EnemyClip {
                frames,
                bones,
                root,
                loop_start: clip.loop_start,
                windows,
                sounds: sound_triggers(sel, -1),
            },
        );
    }
    let atkp = bar_entry(battle, b"atkp")?;
    let mut steps = HashMap::new();
    let mut hit_sounds = HashMap::new();
    let mut hit_effects = HashMap::new();
    let mut rows = HashMap::new();
    for (_, _, id, _) in clip_poses.iter().flat_map(|c| &c.hits) {
        let all = battle_tables::attack_steps(atkp, *id)?;
        let a = all[0];
        steps.insert((*id, 0), all);
        rows.insert(*id, a);
        hit_sounds.insert(*id, a.hit_sound);
        hit_effects.insert(*id, (a.hit_effect, a.hit_effect_random));
    }
    for (_, _, id, _) in clips.values().flat_map(|c: &EnemyClip| &c.windows) {
        let all = battle_tables::attack_steps(atkp, *id)?;
        let a = all[0];
        steps.insert((*id, 0), all);
        hit_sounds.insert(*id, a.hit_sound);
        rows.insert(*id, a);
    }
    let enemy = battle_tables::enemy(bar_entry(battle, b"enmp")?, config.enmp_id)?;
    let level = if enemy.level == 0 {
        config.battle_level
    } else {
        enemy.level
    };
    let row = battle_tables::enemy_level(bar_entry(battle, b"lvpm")?, level)?;
    let stats = combat::TargetStats::enemy(
        enemy.hp_percent,
        enemy.damage_cap,
        enemy.damage_min,
        enemy.weakness,
        row.hp,
        row.defense,
    );
    // Sora at level 1: LVUP row + PLRP (id 0, char 1, Str byte 6); the
    // equipment's stat rows are added by `Game::refresh_gear`.
    let lv = battle_tables::player_level(bar_entry(battle, b"lvup")?, 1, 1)?;
    let plrp = bar_entry(battle, b"plrp")?;
    let plrp_str = plrp.get(8 + 6).copied().unwrap_or(0) as i32;
    let plrp_mag = plrp.get(8 + 7).copied().unwrap_or(0) as i32;
    let sora = combat::AttackerStats {
        strength: lv.strength as i32 + plrp_str,
        magic: lv.magic as i32 + plrp_mag,
        max: 99999,
        min: 1,
        ..combat::AttackerStats::default()
    };
    // Enemy as attacker: LVPM Str with its max/min clamps; Sora as target:
    // LVUP + PLRP defence, player clamps 99999/1, weaknesses 100.
    let shadow_attacker = combat::AttackerStats {
        strength: row.strength as i32,
        magic: row.strength as i32,
        max: row.max_strength as i32,
        min: row.min_strength as i32,
        ..combat::AttackerStats::default()
    };
    let plrp_hp = plrp.get(8 + 3).copied().unwrap_or(20) as i32;
    let plrp_def = plrp.get(8 + 8).copied().unwrap_or(0) as i32;
    let sora_target = combat::TargetStats {
        max_hp: plrp_hp,
        defense: lv.defense as i32 + plrp_def,
        cap: 99999,
        min: 1,
        weakness: [100; 7],
    };
    // The Shadow's AI dead act scatters przt 2 (ai-01); enmp +0x58 is 0.
    let prize_id = if enemy.prize != 0 { enemy.prize } else { 2 };
    let prize = battle_tables::prize(bar_entry(battle, b"przt")?, prize_id)?;
    let lvup_all = bar_entry(battle, b"lvup")?;
    let sora_levels = (1..=99)
        .map(|l| battle_tables::player_level(lvup_all, 1, l))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(EnemySource {
        sora_target_radius,
        battle_level: config.battle_level,
        exp: (enemy.exp, row.exp),
        prize,
        sora_levels,
        model,
        clips: EnemyClips(clips),
        body,
        targets,
        blade,
        steps,
        atkp: atkp.to_vec(),
        difficulty: match config.difficulty {
            0 => combat::Difficulty::Beginner,
            2 => combat::Difficulty::Proud,
            3 => combat::Difficulty::Critical,
            _ => combat::Difficulty::Standard,
        },
        sora,
        stats,
        offset: Vec3::from_array(config.offset),
        claws,
        shadow_attacker,
        sora_target,
        sora_body,
        target_reach,
        hit_sounds,
        hit_effects,
        rows,
    })
}

/// The single BAR entry of a type.
fn bar_entry_typed(bytes: &[u8], kind: u16) -> Result<&[u8], Box<dyn std::error::Error>> {
    let table = bar::read_bar(bytes)?;
    let e = table
        .entries
        .iter()
        .find(|e| e.entry_type == kind && e.offset > 0 && e.size > 0)
        .ok_or("BAR entry type absent")?;
    bytes
        .get(e.offset as usize..e.offset as usize + e.size as usize)
        .ok_or_else(|| "BAR entry exceeds archive".into())
}

#[derive(Resource)]
struct Startup {
    room: Option<LoadedRoom>,
    arena_mesh: Option<visual::room::ArenaMesh>,
    actor: Option<model_inspection::InspectionModel>,
    weapon: Option<model_inspection::InspectionModel>,
    hud: Option<HudSource>,
    enemy_model: Option<model_inspection::InspectionModel>,
    effect_atlas: Option<visual::effects::Atlas>,
    npc_model: Option<model_inspection::InspectionModel>,
}

/// Spawns one textured mesh per model part under `parent`, in part order.
fn spawn_model(
    commands: &mut Commands,
    model: &model_inspection::InspectionModel,
    pose: &[Vec3],
    parent: Entity,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<FogMaterial>,
    images: &mut Assets<Image>,
) -> Vec<(usize, Option<Entity>)> {
    let mut spawned = Vec::new();
    for (ordinal, part) in model.parts.iter().enumerate() {
        if part.indices.is_empty() {
            spawned.push((ordinal, None));
            continue;
        }
        let positions = part
            .indices
            .iter()
            .map(|&i| pose[i as usize].to_array())
            .collect();
        let mut uvs = vec![[0., 0.]; part.indices.len()];
        let mut texture = None;
        if let Some(set) = &model.textures {
            let route = &model.texture_routes[0].parts[ordinal];
            uvs = route
                .expanded_uv_bits
                .iter()
                .map(|uv| uv.map(f32::from_bits))
                .collect();
            let binding = set
                .bind(route.environment_ordinal, TextureMode::OriginalIndexed)
                .expect("admitted model binding");
            if let Some(image) = binding.image_index {
                let level = &set.images[image].levels[0];
                texture = Some(rgba_image(
                    level.width,
                    level.height,
                    level.pixels_rgba.clone(),
                    images,
                ));
            }
        }
        let entity = commands
            .spawn((
                ChildOf(parent),
                Mesh3d(meshes.add(mesh(positions, uvs, vec![[1.; 4]; part.indices.len()]))),
                MeshMaterial3d(materials.add_fogged(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: texture,
                    unlit: true,
                    cull_mode: None,
                    alpha_mode: AlphaMode::Opaque,
                    ..default()
                })),
                Transform::default(),
            ))
            .id();
        spawned.push((ordinal, Some(entity)));
    }
    spawned
}

// Bevy systems take each resource as a parameter.
#[allow(clippy::too_many_arguments)]
/// Advances the area-change presentation one tick (FUN_14015f690 /
/// FUN_14015f5f0 with the fade task FUN_140156f10).
fn area_tick(area: Option<ResMut<AreaRes>>, hud: Option<ResMut<Hud>>) {
    let Some(mut area) = area else { return };
    // The place name banner lives 120 frames (BILLBOARD_PLACE_M).
    if area.transition.is_none()
        && let Some(b) = area.banner.as_mut()
    {
        b.1 = b.1.saturating_sub(1);
        if b.1 == 0 {
            area.banner = None;
        }
    }
    let AreaRes {
        transition,
        pending,
        ..
    } = &mut *area;
    let Some(t) = transition.as_mut() else { return };
    let seq = hud
        .as_ref()
        .and_then(|h| h.iris.as_ref())
        .map(|s| &s.sequence);
    match t.stage {
        Stage::Closing => match seq {
            Some(seq) => {
                let p = t
                    .iris
                    .get_or_insert_with(|| sprite_sequence::Player::new(seq, 1));
                p.update(seq, 1.0);
                if p.finished() {
                    t.fade.fade_out(1, false);
                    t.stage = Stage::Black;
                }
            }
            None => {
                t.fade.fade_out(1, false);
                t.stage = Stage::Black;
            }
        },
        Stage::Black => {
            t.fade.update(1.0);
            if t.fade.state == fade::State::Held {
                t.stage = Stage::Loading;
                *pending = Some(t.target);
            }
        }
        Stage::Loading => {}
        Stage::Opening => {
            t.fade.update(1.0);
            let iris_done = match (seq, t.iris.as_mut()) {
                (Some(seq), Some(p)) => {
                    p.update(seq, 1.0);
                    p.finished()
                }
                _ => true,
            };
            if iris_done && t.fade.state == fade::State::Idle {
                *transition = None;
            }
        }
    }
    if let Some(mut h) = hud {
        h.updated = true;
    }
}

/// While an area change loads and fades in, a slow frame advances the
/// simulation by at most two ticks instead of catching up in one jump
/// (the iris and fade would skip frames otherwise).
fn transition_time(area: Option<Res<AreaRes>>, mut time: ResMut<Time<Virtual>>) {
    let loading = area.is_some_and(|a| {
        a.transition
            .as_ref()
            .is_some_and(|t| matches!(t.stage, Stage::Loading | Stage::Opening))
    });
    let max = if loading {
        std::time::Duration::from_secs_f64(2. / 60.)
    } else {
        std::time::Duration::from_millis(250)
    };
    if time.max_delta() != max {
        time.set_max_delta(max);
    }
}

/// Frame pacing for the run log: one record per 600 frames (mean, worst
/// and frames over 20 ms) and one per frame slower than 50 ms.
#[derive(Resource)]
struct FrameStats {
    started: Instant,
    first_frame: bool,
    frames: u32,
    sum: f64,
    worst: f64,
    slow: u32,
}

fn frame_stats(
    time: Res<Time<Real>>,
    mut stats: ResMut<FrameStats>,
    mut game: ResMut<Game>,
    area: Option<Res<AreaRes>>,
) {
    if !stats.first_frame {
        stats.first_frame = true;
        let ms = stats.started.elapsed().as_secs_f64() * 1e3;
        eprintln!("First frame {ms:.0} ms after start.");
        game.event(serde_json::json!({"event":"first_frame","ms":ms}));
        return;
    }
    let ms = time.delta_secs_f64() * 1e3;
    let tick = game.tick;
    if ms > 50. {
        let stage = area
            .as_ref()
            .and_then(|a| a.transition.as_ref())
            .map(|t| format!("{:?}", t.stage));
        game.event(serde_json::json!({"event":"hitch","tick":tick,"ms":ms,"transition":stage}));
    }
    stats.frames += 1;
    stats.sum += ms;
    stats.worst = stats.worst.max(ms);
    stats.slow += u32::from(ms > 20.);
    if stats.frames == 600 {
        let mean = stats.sum / 600.;
        game.event(
            serde_json::json!({"event":"frame_stats","tick":tick,"mean_ms":mean,
            "worst_ms":stats.worst,"over_20ms":stats.slow}),
        );
        stats.frames = 0;
        stats.sum = 0.;
        stats.worst = 0.;
        stats.slow = 0;
    }
}

/// Loads the area an exit asked for and swaps the room, the collision, the
/// camera collision and Sora's placement (FUN_140152a90 → FUN_140152be0 →
/// FUN_140152680; the iris presentation is not drawn yet).
#[allow(clippy::too_many_arguments)]
fn area_transition(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut game: ResMut<Game>,
    mut rig: ResMut<CameraRig>,
    area: Option<ResMut<AreaRes>>,
    root: Option<Res<SceneRoot>>,
    parts: Query<Entity, With<RoomPart>>,
    hud: Option<Res<Hud>>,
) {
    let (Some(mut area), Some(root)) = (area, root) else {
        return;
    };
    let Some((world, number, entrance)) = area.pending else {
        return;
    };
    // The screen stays black until the worker has decoded the area.
    let job = match area.transition.as_mut().map(|t| t.job.take()) {
        Some(Some(job)) if !job.is_finished() => {
            area.transition.as_mut().expect("transition").job = Some(job);
            return;
        }
        Some(job) => job,
        None => None,
    };
    area.pending = None;
    let started = Instant::now();
    let mut worker_ms = None;
    let loaded = match job.map(|j| j.join()) {
        Some(Ok(Ok((room, next, fork, ms)))) => {
            area.source.adopt(fork);
            worker_ms = Some(ms);
            Ok((room, next))
        }
        Some(Ok(Err(e))) => Err(e.into()),
        Some(Err(_)) => Err("area load thread panicked".into()),
        None => area.source.load(world, number, entrance, [-1; 3]),
    };
    let (room, mut next) = match loaded {
        Ok(loaded) => loaded,
        Err(e) => {
            area.transition = None;
            eprintln!("Area {world}/{number} entrance {entrance} failed to load: {e}");
            game.event(
                serde_json::json!({"event":"area_error","world":world,"area":number,
                "entrance":entrance,"error":e.to_string()}),
            );
            return;
        }
    };
    let decoded = started.elapsed();
    for part in &parts {
        commands.entity(part).despawn();
    }
    let show = game.show_collision;
    spawn_room(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &room,
        root.0,
        show,
        next.state.setup.bg_mask,
    );
    spawn_bobs(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &next,
        root.0,
        next.state.setup.bg_mask,
    );
    spawn_field_npcs(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &area.source,
        &next,
        root.0,
        0,
    );
    area.rendered = next.npcs.len();
    game.world = RoomWorld::Map(MapWorld::new(next.take_collision()));
    game.step_bank = step_room_bank(&game.step_info, next.world, next.area);
    game.ambient = room_ambient(&game.step_info, next.world, next.area);
    let (position, yaw) = next.player;
    game.player = Player::new(position.to_array(), yaw);
    rig.collision = CameraCollision::new(&room);
    rig.camera = FollowCamera::new(
        follow_camera::OUTDOOR,
        &camera_input(&game.player, game.player.position, game.params.run_speed),
    );
    rig.previous_position = game.player.position;
    game.event(
        serde_json::json!({"event":"area_load","world":world,"area":number,
        "worker_ms":worker_ms,"main_decode_ms":decoded.as_secs_f64() * 1e3,
        "main_total_ms":started.elapsed().as_secs_f64() * 1e3}),
    );
    game.event(serde_json::json!({"event":"area_enter","world":world,"area":number,
        "entrance":entrance,"programs":next.programs,"bg_mask":format!("{:#x}", next.state.setup.bg_mask),
        "spawn":position.to_array(),"yaw":yaw}));
    area.previous = Some(area.current.area);
    area.visited.insert((next.world, next.area));
    area.music = Some(std::mem::take(&mut next.music));
    area.banner = area
        .source
        .place_name(next.world, next.area)
        .map(|t| (t.clone(), 120));
    area.bg_mask = next.state.setup.bg_mask;
    area.current = next;
    // The old area's effects end with it; the new area's start next tick.
    game.fx_clear_map.push(None);
    game.map_fx_pending = true;
    if let Some(t) = area.transition.as_mut() {
        t.stage = Stage::Opening;
        t.iris = hud
            .as_ref()
            .and_then(|h| h.iris.as_ref())
            .map(|s| sprite_sequence::Player::new(&s.sequence, 0));
        t.fade.fade_in(1, false);
    }
}

/// A placed map object and its mesh parts.
#[derive(Component)]
struct BobRoot;
#[derive(Component)]
struct BobPart {
    placement: usize,
    ordinal: usize,
}

/// Creates the meshes of an area's map objects (those whose group bit is
/// set draw; the others are hidden and switch with the mask).
fn spawn_bobs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<FogMaterial>,
    images: &mut Assets<Image>,
    area: &LoadedArea,
    parent: Entity,
    mask: u64,
) {
    for (i, p) in area.bobs.placements.iter().enumerate() {
        let Some(Some(bob)) = area.bobs.models.get(p.model) else {
            continue;
        };
        let bit = p.group;
        let visible = visual::room::chunk_visible(bit, mask);
        let root = commands
            .spawn((
                RoomPart,
                BobRoot,
                BgChunk(bit),
                ChildOf(parent),
                p.transform(),
                if visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
            ))
            .id();
        let pose: Vec<Vec3> = bob.model.poses[0]
            .position_bits
            .iter()
            .map(|v| Vec3::from_array(v.map(f32::from_bits)))
            .collect();
        for (ordinal, entity) in
            spawn_model(commands, &bob.model, &pose, root, meshes, materials, images)
        {
            if let Some(entity) = entity {
                commands.entity(entity).insert((
                    RoomPart,
                    BobPart {
                        placement: i,
                        ordinal,
                    },
                ));
            }
        }
    }
}

/// Plays the map objects' looping motions (one tick per tick, start offset
/// from the record).
fn bob_present(
    game: Res<Game>,
    area: Option<Res<AreaRes>>,
    parts: Query<(&BobPart, &Mesh3d)>,
    added: Query<(), Added<BobPart>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut shown: Local<Option<u64>>,
) {
    let Some(area) = area else { return };
    // The pose changes once per tick; extra frames would re-upload it.
    if *shown == Some(game.tick) && added.is_empty() {
        return;
    }
    *shown = Some(game.tick);
    for (part, handle) in &parts {
        let p = &area.current.bobs.placements[part.placement];
        if p.motion < 0 {
            continue;
        }
        let Some(Some(bob)) = area.current.bobs.models.get(p.model) else {
            continue;
        };
        let Some(anim) = bob.anims.get(p.motion as usize) else {
            continue;
        };
        let tick = (game.tick as f32 + p.start) as usize % anim.frames.len();
        let frame = &anim.frames[tick];
        let positions = bob.model.parts[part.ordinal]
            .indices
            .iter()
            .map(|&i| frame[i as usize].to_array());
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            visual::room::update_positions(&mut mesh, positions);
        }
    }
}

/// Component marking a townsperson's root and its mesh parts.
#[derive(Component)]
struct FieldNpcRoot(usize);
#[derive(Component)]
struct FieldNpcPart {
    npc: usize,
    ordinal: usize,
}

/// Creates the meshes of an area's townspeople under `parent` (tagged
/// `RoomPart` so an area change removes them).
#[allow(clippy::too_many_arguments)]
fn spawn_field_npcs(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<FogMaterial>,
    images: &mut Assets<Image>,
    source: &AreaSource,
    area: &LoadedArea,
    parent: Entity,
    from: usize,
) {
    for (i, npc) in area.npcs.iter().enumerate().skip(from) {
        let kind = &source.kinds[npc.kind];
        let root = commands
            .spawn((
                RoomPart,
                FieldNpcRoot(i),
                ChildOf(parent),
                Transform::from_translation(npc.position)
                    .with_rotation(Quat::from_rotation_y(npc.yaw)),
                Visibility::default(),
            ))
            .id();
        for (ordinal, entity) in spawn_model(
            commands,
            &kind.model,
            npc.frame_positions(&kind.anims),
            root,
            meshes,
            materials,
            images,
        ) {
            if let Some(entity) = entity {
                commands
                    .entity(entity)
                    .insert((RoomPart, FieldNpcPart { npc: i, ordinal }));
            }
        }
    }
}

/// The `--npc-tour`'s capture ticks and end.
const NPC_TOUR_CAPTURES: [u64; 3] = [25, 45, 90];
const NPC_TOUR_END: u64 = 120;

/// One tick of every townsperson, and the choice of whom Talk addresses:
/// the nearest with a script whose reaction sphere holds Sora
/// (FUN_140407de0). Talking NPCs stand still; after a talk they face Sora
/// and wait 60 ticks (rtn after_talk, npc-01 §2.4).
fn field_npc_tick(
    mut game: ResMut<Game>,
    area: Option<ResMut<AreaRes>>,
    talk: Option<ResMut<Talk>>,
) {
    let (Some(mut area), Some(mut talk)) = (area, talk) else {
        return;
    };
    let AreaRes {
        source,
        current,
        talk_npc,
        was_talking,
        ..
    } = &mut *area;
    // The `--npc-tour`: at tick 30 Sora is put 30 units from the first
    // townsperson, so the push shows.
    if game.npc_tour
        && game.tick == 30
        && let Some(n) = current.npcs.iter().find(|n| n.alive)
    {
        game.player.position = [n.position.x + 30., n.position.y, n.position.z];
    }
    let sora = Vec3::from_array(game.player.position);
    let talking = talk.active.is_some();
    if *was_talking
        && !talking
        && let Some(i) = *talk_npc
    {
        current.npcs[i].after_talk(sora);
    }
    *was_talking = talking;
    if !talking {
        let mut best: Option<(f32, usize)> = None;
        for (i, n) in current.npcs.iter().enumerate() {
            if !n.alive
                || n.talk == 0
                || n.reaction != 0x1f
                || !current.scripts.contains_key(&n.talk)
            {
                continue;
            }
            if source.kinds[n.kind].target.is_none() {
                continue;
            }
            let d = (n.position + Vec3::Y * REACTION_SPHERE[2]) - (sora + Vec3::Y * SORA_OBJ[2]);
            let (r, h) = (
                REACTION_SPHERE[0] + SORA_OBJ[0],
                REACTION_SPHERE[1] + SORA_OBJ[1],
            );
            if (d.x * d.x + d.z * d.z) / (r * r) + d.y * d.y / (h * h) < 1. {
                let dist = d.x.hypot(d.z);
                if best.is_none_or(|(b, _)| dist < b) {
                    best = Some((dist, i));
                }
            }
        }
        match best {
            Some((_, i)) => {
                let n = &current.npcs[i];
                let kind = &source.kinds[n.kind];
                let t = kind.target.as_ref().expect("candidate has a target shape");
                talk.position = n.position;
                talk.yaw = n.yaw;
                talk.home_yaw = n.yaw;
                talk.target = Shape {
                    bone: t.bone,
                    offset: t.offset,
                    pos_h: t.pos_h,
                    radius: t.radius,
                    height: t.height,
                };
                talk.bones = vec![
                    n.bone(&kind.anims)
                        .map(|m| (t.bone, m))
                        .into_iter()
                        .collect(),
                ];
                if *talk_npc != Some(i) {
                    talk.script = current.scripts[&n.talk].clone();
                    talk.lines = current.lines.clone();
                    *talk_npc = Some(i);
                }
            }
            None => {
                talk.position = Vec3::splat(1.0e9);
                *talk_npc = None;
            }
        }
    }
    // Zone pop-in / pop-out (FUN_1403ff000 / FUN_1403fe320 case 5 and
    // FUN_140416270): an spawn-type-2 NPC appears when Sora is inside a
    // kind-1 zone of its spawn point and leaves 5 ticks after he is not.
    let engine_sora = [sora.x, -sora.y, -sora.z];
    let mut random = crate::npcs::Lcg(0x9e37 ^ game.tick as u32);
    let mut events = Vec::new();
    for d in &mut current.deferred {
        let inside = current.state.objects[d.spawn.point].zones.iter().any(|z| {
            let raw = &current.state.objects[d.spawn.point].point.activators[z.activator].raw;
            u16::from_le_bytes([raw[0x2c], raw[0x2d]]) == 1 && z.contains(engine_sora)
        });
        if inside {
            d.away = 0;
            match d.npc {
                None => {
                    let mut n = crate::area::build_npc(
                        &current.state,
                        d.kind,
                        &d.spawn,
                        d.spawn.point,
                        &mut random,
                    );
                    n.pop_in();
                    events.push(serde_json::json!({"event":"npc_pop_in","serial":n.serial}));
                    current.npcs.push(n);
                    d.npc = Some(current.npcs.len() - 1);
                }
                Some(i) if !current.npcs[i].alive => {
                    current.npcs[i] = crate::area::build_npc(
                        &current.state,
                        d.kind,
                        &d.spawn,
                        d.spawn.point,
                        &mut random,
                    );
                    current.npcs[i].pop_in();
                    events.push(
                        serde_json::json!({"event":"npc_pop_in","serial":current.npcs[i].serial}),
                    );
                }
                _ => {}
            }
        } else if let Some(i) = d.npc {
            d.away += 1;
            // FUN_140416270: the townsperson fades out over ten ticks and is
            // released at zero (`tick_fade`).
            if d.away > 4
                && !(talking && *talk_npc == Some(i))
                && current.npcs[i].alive
                && current.npcs[i].pop_out()
            {
                events.push(
                    serde_json::json!({"event":"npc_pop_out","serial":current.npcs[i].serial}),
                );
            }
        }
    }
    if !talking {
        crate::npcs::update_meets(
            &mut current.npcs,
            |k| source.kinds[k].body,
            sora,
            &mut random,
        );
    }
    for (i, n) in current.npcs.iter_mut().enumerate() {
        if !n.alive {
            continue;
        }
        let is_target = talking && *talk_npc == Some(i);
        if is_target {
            n.yaw = talk.yaw;
        }
        let kind = &source.kinds[n.kind];
        n.tick(&kind.anims, sora, is_target, &mut random);
        n.update_look(kind, sora);
        if n.tick_fade() {
            events.push(serde_json::json!({"event":"npc_gone","serial":n.serial}));
        }
    }
    // FUN_1403b9090 / FUN_14040ad70: Sora and the townspeople push each
    // other apart (look-02 §2; walls are not consulted).
    let mut at = sora;
    for (i, n) in current.npcs.iter_mut().enumerate() {
        if !n.alive {
            continue;
        }
        let body = source.kinds[n.kind].body;
        let shape = [body[1], body[2], body[0]];
        let movable = !n.immovable(talking && *talk_npc == Some(i));
        if let Some((c, back)) = crate::npcs::push(at, SORA_OBJ, n.position, shape, movable) {
            n.position.x += c[0];
            n.position.z += c[1];
            at.x += back[0];
            at.z += back[1];
        }
    }
    if at != sora {
        events.push(
            serde_json::json!({"event":"npc_push","sora_from":sora.to_array(),
            "sora_to":at.to_array()}),
        );
    }
    game.player.position[0] = at.x;
    game.player.position[2] = at.z;
    for e in events {
        game.event(e);
    }
}

/// Applies an area's music change: slots whose track changed fade out over
/// one second and the new track decodes in the background.
fn area_music(area: Option<ResMut<AreaRes>>, music: Option<ResMut<MusicRes>>, audio: Res<Audio>) {
    let (Some(mut area), Some(mut m)) = (area, music) else {
        return;
    };
    let Some(change) = area.music.take() else {
        return;
    };
    for (k, slot) in change.slots.into_iter().enumerate() {
        let Some(slot) = slot else { continue };
        if let Some(output) = &audio.output
            && let Ok(mut mixer) = output.mixer.lock()
        {
            mixer.fade(MUSIC_KEYS[k], 0., 1., audio::FadeEnd::Stop);
        }
        m.clips[k] = None;
        m.pending[k] = slot.map(sound::decode_music_in_background);
        if k == 0 {
            m.started = false;
        }
    }
}

/// Shows and hides map chunks when the background group mask changes.
fn bg_visibility(
    area: Option<Res<AreaRes>>,
    mut last: Local<Option<u64>>,
    mut chunks: Query<(&BgChunk, &mut Visibility)>,
    added: Query<(), Added<BgChunk>>,
) {
    let Some(area) = area else { return };
    if *last == Some(area.bg_mask) && added.is_empty() {
        return;
    }
    *last = Some(area.bg_mask);
    for (chunk, mut v) in &mut chunks {
        *v = if visual::room::chunk_visible(chunk.0, area.bg_mask) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// Creates meshes for townspeople that appeared since the last frame.
fn field_npc_spawn_new(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut images: ResMut<Assets<Image>>,
    area: Option<ResMut<AreaRes>>,
    root: Option<Res<SceneRoot>>,
) {
    let (Some(mut area), Some(root)) = (area, root) else {
        return;
    };
    if area.transition.is_some() || area.rendered >= area.current.npcs.len() {
        return;
    }
    spawn_field_npcs(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &area.source,
        &area.current,
        root.0,
        area.rendered,
    );
    area.rendered = area.current.npcs.len();
}

/// Places and poses the townspeople.
#[allow(clippy::too_many_arguments)]
fn field_npc_present(
    game: Res<Game>,
    area: Option<Res<AreaRes>>,
    mut roots: Query<(&FieldNpcRoot, &mut Transform, &mut Visibility)>,
    parts: Query<(&FieldNpcPart, &Mesh3d, &MeshMaterial3d<FogMaterial>)>,
    added: Query<(), Added<FieldNpcPart>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut shown: Local<Option<u64>>,
) {
    let Some(area) = area else { return };
    // Townspeople move and pose once per tick.
    if *shown == Some(game.tick) && added.is_empty() {
        return;
    }
    *shown = Some(game.tick);
    let (kinds, npcs) = (&area.source.kinds, &area.current.npcs);
    for (root, mut t, mut v) in &mut roots {
        if let Some(n) = npcs.get(root.0) {
            t.translation = n.position;
            t.rotation = Quat::from_rotation_y(n.yaw);
            *v = if n.alive {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
        }
    }
    for (part, handle, material) in &parts {
        let Some(n) = npcs.get(part.npc).filter(|n| n.alive) else {
            continue;
        };
        // The pop-in / pop-out alpha factor (sys+0xa08 → the colour alpha).
        if let Some(mut m) = materials.get_mut(&material.0)
            && (m.base.base_color.alpha() - n.fade).abs() > 1e-4
        {
            m.base.base_color = Color::WHITE.with_alpha(n.fade);
            m.base.alpha_mode = if n.fade < 1. {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            };
        }
        let kind = &kinds[n.kind];
        let frame = n
            .look_positions
            .as_deref()
            .unwrap_or_else(|| n.frame_positions(&kind.anims));
        let positions = kind.model.parts[part.ordinal]
            .indices
            .iter()
            .map(|&i| frame[i as usize].to_array());
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            visual::room::update_positions(&mut mesh, positions);
        }
    }
}

/// Spawns a room's map models and its collision overlay under `parent`,
/// tagged so an area change can remove them.
#[allow(clippy::too_many_arguments)]
fn spawn_room(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<FogMaterial>,
    images: &mut Assets<Image>,
    room: &LoadedRoom,
    parent: Entity,
    show_collision: bool,
    bg_mask: u64,
) {
    let mut texa_workers = Vec::new();
    // The area's type-14 record: screen clear colour and fog (draw-01 §8.1).
    visual::area_fog::set_current(room.environment.map(|e| e.fog));
    if let Some(e) = &room.environment {
        commands.insert_resource(ClearColor(Color::srgb_u8(
            e.clear[0], e.clear[1], e.clear[2],
        )));
    }
    for part in &room.rooms {
        // Each environment's CLAMP region is baked into a cropped copy with
        // the exe's address modes and linear filter (draw-01 §4.6).
        let regions: Vec<visual::room::EnvRegion> = part
            .textures
            .iter()
            .map(|t| {
                part.regions.get(&t.environment_ordinal).copied().unwrap_or(
                    visual::room::EnvRegion::new(0, 0, [0; 4], [t.width, t.height]),
                )
            })
            .collect();
        let handles: Vec<_> = part
            .textures
            .iter()
            .zip(&regions)
            .map(|(t, r)| {
                let (w, h, px) = r.crop_rgba(t);
                visual::room::region_image(w, h, px, r.wrap, images)
            })
            .collect();
        for t in &part.texa {
            if let Some(i) = part
                .textures
                .iter()
                .position(|x| x.environment_ordinal == t.env)
            {
                texa_workers.push(TexaWorker {
                    image: handles[i].clone(),
                    texa: t.clone(),
                    origin: [regions[i].crop[0], regions[i].crop[1]],
                    player: visual::room::TexaPlayer::default(),
                    shown: None,
                });
            }
        }
        // Chunks drawn with the same texture and blend share one material,
        // so the renderer can batch them; UV-scrolled chunks keep their own.
        let mut shared: HashMap<(Option<usize>, u8), Handle<FogMaterial>> = HashMap::new();
        for (chunk_index, chunk) in part.model.chunks.iter().enumerate() {
            if chunk.indices.is_empty() {
                continue;
            }
            let bit = part
                .chunk_bits
                .get(chunk_index)
                .copied()
                .unwrap_or(visual::room::NO_GROUP);
            let vertex = |i: &u32| &chunk.vertices[*i as usize];
            let positions = chunk
                .indices
                .iter()
                .map(|i| vertex(i).position_bits.map(f32::from_bits))
                .collect();
            let texture_slot = part
                .textures
                .iter()
                .position(|t| t.environment_ordinal as i16 == chunk.texture_index);
            let (scale, offset) = texture_slot.map_or(([1.; 2], [0.; 2]), |i| {
                (regions[i].scale, regions[i].offset)
            });
            let uvs = chunk
                .indices
                .iter()
                .map(|i| {
                    let uv = vertex(i).uv_bits.map(f32::from_bits);
                    [uv[0] * scale[0] + offset[0], uv[1] * scale[1] + offset[1]]
                })
                .collect();
            let colors = chunk
                .indices
                .iter()
                .map(|i| vertex(i).color_bits.map(f32::from_bits))
                .collect();
            let texture = texture_slot.map(|i| handles[i].clone());
            // UVSC: fc bits 1..5 hold the record index + 1; repeat modes only.
            let scroll_id = (chunk.header_words[3] >> 1) & 0x1f;
            let scroll = (scroll_id != 0)
                .then(|| part.uv_scroll.iter().find(|r| r.0 + 1 == scroll_id))
                .flatten()
                .zip(texture_slot);
            // Sky layer 0 is never fogged (draw-01 §4.5).
            let sky0 = part.name == *b"SK0";
            let blend = if chunk.transparency_flag == 0 {
                0
            } else if chunk.header_words[3] & 0x40 != 0 {
                1
            } else {
                2
            };
            let base = StandardMaterial {
                base_color: if texture.is_some() {
                    Color::WHITE
                } else {
                    Color::srgb(1., 0., 1.)
                },
                base_color_texture: texture,
                unlit: true,
                cull_mode: None,
                // GS TEST: opaque pass keeps alpha > 31.5/255
                // (AREF 0x10); the alpha pass blends, additive when
                // the descriptor's fc bit 6 is set (draw-01 §3-4).
                alpha_mode: match blend {
                    0 => AlphaMode::Mask(31.5 / 255.),
                    1 => AlphaMode::Add,
                    _ => AlphaMode::Blend,
                },
                ..default()
            };
            let mut add = |base| {
                if sky0 {
                    materials.add_plain(base)
                } else {
                    materials.add_fogged(base)
                }
            };
            let material = if scroll.is_some() {
                add(base)
            } else {
                shared
                    .entry((texture_slot, blend))
                    .or_insert_with(|| add(base))
                    .clone()
            };
            let mut entity = commands.spawn((
                ChildOf(parent),
                RoomPart,
                BgChunk(bit),
                if visual::room::chunk_visible(bit, bg_mask) {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                },
                Mesh3d(meshes.add(mesh(positions, uvs, colors))),
                MeshMaterial3d(material.clone()),
                Transform::default(),
            ));
            if let Some((record, slot)) = scroll {
                let r = &regions[slot];
                entity.insert(UvScroll {
                    material,
                    speed: [record.1, record.2],
                    ratio: [
                        if r.wrap[0] { r.scroll_ratio[0] } else { 0. },
                        if r.wrap[1] { r.scroll_ratio[1] } else { 0. },
                    ],
                    scale: r.scale,
                });
            }
        }
    }
    // Collision overlay (F1): walkable floors green, walls red.
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    for triangle in &room.collision_triangles {
        let t = &triangle.corners;
        let normal = (t[1] - t[0]).cross(t[2] - t[0]).normalize_or_zero();
        let stand = triangle.surface_flags & PARTY_STAND != 0;
        let collide = triangle.surface_flags & PARTY_COLLIDE != 0;
        let color = if normal.y.abs() >= FLOOR_MIN_NORMAL_Y && stand {
            [0.1, 1., 0.2, 0.35]
        } else if collide {
            [1., 0.15, 0.1, 0.35]
        } else {
            // Not used by the player controller.
            [0.2, 0.4, 1., 0.15]
        };
        for v in t {
            positions.push((*v + Vec3::Y * 2.).to_array());
            colors.push(color);
        }
    }
    if !texa_workers.is_empty() {
        eprintln!("{} TEXA texture animation(s) running", texa_workers.len());
    }
    commands.insert_resource(TexaWorkers {
        list: texa_workers,
        last_tick: u64::MAX,
    });
    let corners = positions.len();
    commands.spawn((
        CollisionOverlay,
        RoomPart,
        ChildOf(parent),
        Mesh3d(meshes.add(mesh(positions, vec![[0., 0.]; corners], colors))),
        MeshMaterial3d(materials.add_fogged(StandardMaterial {
            base_color: Color::WHITE,
            unlit: true,
            cull_mode: None,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::default(),
        if show_collision {
            Visibility::Visible
        } else {
            Visibility::Hidden
        },
    ));
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    mut startup: ResMut<Startup>,
    mut game: ResMut<Game>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<FogMaterial>>,
    mut plain_materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    talk: Option<ResMut<Talk>>,
    mut frame: ResMut<Ui2dFrame>,
    windows: Query<&Window>,
    area: Option<Res<AreaRes>>,
    mut layout_materials: ResMut<Assets<visual::overlay::LayoutMaterial>>,
) {
    let room = startup.room.take().expect("room retained for setup");
    let mut compose = Compose {
        scene: Handle::default(),
        scene_sheet: 0,
        snapshot_sheet: None,
        hud_sheet: None,
        iris_sheet: None,
        white_sheet: None,
        stable_frames: 0,
    };
    let enemy_model = startup.enemy_model.take();
    if let Some(HudSource {
        sheets,
        field,
        iris,
        stats: (hp, mp),
        menu,
    }) = startup.hud.take()
    {
        // Test-room choices for values the opening room does not define:
        // full HP/MP from PLRP, normal drive mode with three full bars.
        let stats = GaugeStats {
            hp,
            max_hp: hp,
            mp,
            max_mp: mp,
            drive_mode: 1,
            drive: 0,
            drive_level: 3,
            drive_max: 3,
            mp_charge: None,
            form_bars: 0,
            form_time: 0.,
            form_max_time: 0.,
        };
        let canvas = Canvas::new(sheets[0].texel_scale as f32);
        let mut image = Image::new(
            Extent3d {
                width: canvas.width as u32,
                height: canvas.height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            canvas.rgba.clone(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::linear();
        let image = images.add(image);
        // The 16:9 HUD canvas over the whole window, composed in gamma space
        // after the scene.
        compose.hud_sheet = Some(frame.add_image(
            image.clone(),
            [canvas.width as u32, canvas.height as u32],
            true,
        ));
        if let Some(i) = &iris {
            compose.iris_sheet = Some(frame.add_sheet(
                &mut images,
                i.texture_rgba.clone(),
                i.texture_size,
                i.texel_scale as f32,
            ));
            compose.white_sheet = Some(frame.add_sheet(&mut images, vec![255; 16], [2, 2], 1.));
        }
        let mut slots = visual::hud_slots::HudSlots::default();
        commands.insert_resource(Hud {
            panels: Default::default(),
            gauge: Gauge::new(
                &sheets.iter().map(|s| &s.sequence).collect::<Vec<_>>(),
                &stats,
            ),
            enemy_gauge: EnemyGauge::new(&sheets[0].sequence, &mut slots),
            scan: Default::default(),
            slots,
            field,
            reticle: Default::default(),
            reticle_draw: None,
            sheets,
            iris,
            menu: menu.map(|(sheet, font, labels)| {
                // Test-room choices for the opening state: Magic and Drive
                // have no entries yet (colour 5), Items is usable.
                let items = labels
                    .into_iter()
                    .zip([(1, 0), (2, 5), (3, 0), (4, 5)])
                    .map(|(label, (icon, colour))| Item {
                        label,
                        icon,
                        colour,
                    })
                    .collect();
                let mut menu = CommandMenu::new(&sheet.sequence, items);
                // The Magic row opens submenu 1 (command-01 section 0.6).
                menu.opens = vec![None, Some(1), Some(2), None];
                MenuState { menu, sheet, font }
            }),
            stats,
            canvas,
            image,
            updated: true,
        });
    }
    // Scene data stays in source coordinates under one root. The exe maps
    // them to the screen without a mirror (model +X on screen right,
    // camp-03 §1.4), as the rotation from_world does.
    let world = commands
        .spawn((Transform::default(), Visibility::default()))
        .id();
    if let Some(arena) = startup.arena_mesh.take() {
        let (w, h, rgba) = arena.texture;
        let texture = rgba_image(w, h, rgba, &mut images);
        commands.spawn((
            ChildOf(world),
            Mesh3d(meshes.add(mesh(arena.positions, arena.uvs, arena.colors))),
            MeshMaterial3d(materials.add_fogged(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(texture),
                unlit: true,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
        ));
    }
    spawn_room(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut images,
        &room,
        world,
        game.show_collision,
        area.as_ref().map_or(!0, |a| a.current.state.setup.bg_mask),
    );
    if let Some(a) = &area {
        spawn_bobs(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut images,
            &a.current,
            world,
            a.current.state.setup.bg_mask,
        );
        spawn_field_npcs(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut images,
            &a.source,
            &a.current,
            world,
            0,
        );
    }
    // A pool of spell-volume placeholders (the most volumes live at once).
    let unit = meshes.add(Sphere::new(1.).mesh().ico(3).unwrap());
    for i in 0..16 {
        commands.spawn((
            SpellVolume(i),
            ChildOf(world),
            Mesh3d(unit.clone()),
            MeshMaterial3d(plain_materials.add(StandardMaterial {
                base_color: Color::srgba(1., 1., 1., 0.3),
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                ..default()
            })),
            Transform::default(),
            Visibility::Hidden,
        ));
    }
    commands.insert_resource(SceneRoot(world));
    let mut actor = startup.actor.take().expect("actor retained for setup");
    let weapon = startup.weapon.take().expect("weapon retained for setup");
    let pose = game.clip_poses[game.poses[&(Action::Idle, false)]].frames[0].clone();
    let root = commands
        .spawn((
            SoraRoot,
            ChildOf(world),
            Transform::from_translation(Vec3::from_array(game.player.position)),
            Visibility::default(),
        ))
        .id();
    let mut part_indices = Vec::new();
    for (ordinal, entity) in spawn_model(
        &mut commands,
        &actor,
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
    // Tz::MenuObj's member 0: Sora's walk (motion 0x8b = slot 6) rendered
    // off-screen on layer 3 and pasted into the camp (camp-03 spec). Menu
    // space: (mx, my, mz + 100) seen from (0, 50, 800) toward (0, 0, -55),
    // i.e. the game's view turned 180° about X (no mirror).
    let walk = game.poses[&(Action::Walk, false)];
    let walk_pose: Vec<Vec3> = {
        let c = &game.clip_poses[walk];
        c.frames[0].iter().map(|p| *p + c.root[0]).collect()
    };
    let menu_root = commands
        .spawn((
            MenuModelRoot,
            Transform::from_xyz(0., 0., 100.),
            Visibility::Hidden,
            RenderLayers::layer(MENU_LAYER),
        ))
        .id();
    for (ordinal, entity) in spawn_model(
        &mut commands,
        &actor,
        &walk_pose,
        menu_root,
        &mut meshes,
        &mut materials,
        &mut images,
    ) {
        if let Some(entity) = entity {
            commands
                .entity(entity)
                .insert((MenuModelPart(ordinal), RenderLayers::layer(MENU_LAYER)));
        }
    }
    let menu_target = images.add(Image::new_target_texture(
        MENU_TARGET[0],
        MENU_TARGET[1],
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -3,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        bevy::camera::RenderTarget::Image(menu_target.clone().into()),
        // A 640x480-style camera with horizontal FOV 40° (focal 879.19):
        // vertical 2·atan(240 / 879.19); squeezed to the layout, the
        // target's 16:9 square pixels match it (camp-03 §1).
        Projection::Perspective(PerspectiveProjection {
            fov: 2. * (240f32 / 879.1928).atan(),
            near: 1.,
            far: 100_000.,
            ..default()
        }),
        Transform::from_xyz(0., 50., 800.).looking_at(Vec3::new(0., 0., -55.), Vec3::Y),
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
        NoIndirectDrawing,
        RenderLayers::layer(MENU_LAYER),
        MenuCamera,
    ));
    commands.insert_resource(MenuModel {
        sheet: frame.add_image(menu_target, MENU_TARGET, true),
        clip: walk,
        indices: actor.parts.iter().map(|p| p.indices.clone()).collect(),
    });
    // Sora's skeleton for the skateboard's rider retarget.
    let skinning = std::mem::take(&mut actor.skinning);
    let rig = skinning
        .bind_pose(data::model_pose::PoseProfile::Legacy)
        .ok()
        .and_then(|bind| {
            let bind = bind
                .matrix_bits
                .iter()
                .map(|m| Mat4::from_cols_array(&m.map(f32::from_bits)))
                .collect();
            let parents = skinning.bones.iter().map(|b| b.header_i32[1]).collect();
            crate::skate_rig::Rig::new(bind, parents)
        });
    if rig.is_none() {
        eprintln!("Sora's skeleton is not the one the skateboard pose expects");
    }
    commands.insert_resource(SoraRig { skinning, rig });
    let hand = commands
        .spawn((
            WeaponRoot,
            ChildOf(root),
            Transform::from_matrix(game.clip_poses[game.poses[&(Action::Idle, false)]].weapon[0]),
            Visibility::default(),
        ))
        .id();
    // The weapon's bind pose places it by its own root joint; express it in
    // that joint's space so the actor's weapon joint replaces it.
    let weapon_root = Mat4::from_cols_array(&weapon.poses[0].matrix_bits[0].map(f32::from_bits));
    let weapon_pose: Vec<Vec3> = weapon.poses[0]
        .position_bits
        .iter()
        .map(|p| {
            weapon_root
                .inverse()
                .transform_point3(Vec3::from_array(p.map(f32::from_bits)))
        })
        .collect();
    // With the weapon's own motion, its posed vertices are already in the
    // hand joint's space; otherwise fall back to the bind pose above.
    let battle_idle = &game.clip_poses[game.poses[&(Action::Idle, true)]];
    let animated = !battle_idle.weapon_frames.is_empty();
    let weapon_pose = if animated {
        battle_idle.weapon_frames[0].clone()
    } else {
        weapon_pose
    };
    let mut weapon_indices = Vec::new();
    for (ordinal, entity) in spawn_model(
        &mut commands,
        &weapon,
        &weapon_pose,
        hand,
        &mut meshes,
        &mut materials,
        &mut images,
    ) {
        weapon_indices.push(weapon.parts[ordinal].indices.clone());
        if let Some(entity) = entity {
            commands.entity(entity).insert(WeaponPart(ordinal));
        }
    }
    game.weapon_indices = weapon_indices;
    if let (Some(model), Some(rt)) = (&enemy_model, game.enemy.as_mut()) {
        let e = &rt.enemy;
        let pose = &rt.clips.0[&e.motion].frames[0];
        let root = commands
            .spawn((
                EnemyRoot,
                ChildOf(world),
                Transform::from_translation(Vec3::from_array(e.position))
                    .with_rotation(Quat::from_rotation_y(e.yaw)),
                Visibility::default(),
            ))
            .id();
        let mut indices = Vec::new();
        for (ordinal, entity) in spawn_model(
            &mut commands,
            model,
            pose,
            root,
            &mut meshes,
            &mut materials,
            &mut images,
        ) {
            indices.push(model.parts[ordinal].indices.clone());
            if let Some(entity) = entity {
                commands.entity(entity).insert(EnemyPart(ordinal));
            }
        }
        rt.indices = indices;
    }
    // Donald and Goofy: one root per member with a mesh per model part.
    for (i, member) in game.party.iter().enumerate() {
        let kind = &member.kind;
        let Some(idle) = kind.motions.get(&0) else {
            continue;
        };
        let root = commands
            .spawn((
                party_room::PartyRoot(i),
                ChildOf(world),
                Transform::from_translation(Vec3::from_array(member.follower.position)),
                Visibility::default(),
            ))
            .id();
        for (ordinal, entity) in spawn_model(
            &mut commands,
            &kind.model,
            &idle.frames[0],
            root,
            &mut meshes,
            &mut materials,
            &mut images,
        ) {
            if let Some(entity) = entity {
                commands.entity(entity).insert(party_room::PartyPart {
                    member: i,
                    ordinal,
                    weapon: false,
                });
            }
        }
        if let (Some(weapon), Some(pose)) = (&kind.weapon, idle.weapon.first()) {
            for (ordinal, entity) in spawn_model(
                &mut commands,
                weapon,
                pose,
                root,
                &mut meshes,
                &mut materials,
                &mut images,
            ) {
                if let Some(entity) = entity {
                    commands.entity(entity).insert(party_room::PartyPart {
                        member: i,
                        ordinal,
                        weapon: true,
                    });
                }
            }
        }
    }
    if let (Some(model), Some(mut talk)) = (startup.npc_model.take(), talk) {
        let root = commands
            .spawn((
                NpcRoot,
                ChildOf(world),
                Transform::from_translation(talk.position)
                    .with_rotation(Quat::from_rotation_y(talk.yaw)),
                Visibility::default(),
            ))
            .id();
        let mut indices = Vec::new();
        for (ordinal, entity) in spawn_model(
            &mut commands,
            &model,
            &talk.frames[0],
            root,
            &mut meshes,
            &mut materials,
            &mut images,
        ) {
            indices.push(model.parts[ordinal].indices.clone());
            if let Some(entity) = entity {
                commands.entity(entity).insert(NpcPart(ordinal));
            }
        }
        talk.indices = indices;
        // The cross-dissolve: a second camera renders the old view into an
        // image, drawn under the HUD while it fades.
        let snapshot = images.add(Image::new_target_texture(
            1280,
            720,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: -1,
                is_active: false,
                ..default()
            },
            bevy::camera::RenderTarget::Image(snapshot.clone().into()),
            Transform::default(),
            Msaa::Off,
            Tonemapping::None,
            DebandDither::Disabled,
            NoIndirectDrawing,
            SnapshotCamera,
        ));
        compose.snapshot_sheet = Some(frame.add_image(snapshot, [1280, 720], true));
    }
    game.weapon_display = weapon_pose;
    game.part_indices = part_indices;
    game.display = pose;
    // Base Sora's model and Keyblade, respawned on a revert.
    if let Some(d) = game.drive.as_mut() {
        d.models.insert(0, (actor, weapon, None));
    }
    // Effect quads: one mesh per blend (GS ALPHA 0x44 blend, 0x48 add).
    if let Some(atlas) = startup.effect_atlas.take() {
        let image = rgba_image(atlas.size[0], atlas.size[1], atlas.rgba, &mut images);
        if let Some(mut i) = images.get_mut(&image) {
            i.sampler = ImageSampler::linear();
        }
        for additive in [false, true] {
            commands.spawn((
                EffectMesh { additive },
                ChildOf(world),
                Mesh3d(meshes.add(mesh(vec![[0.; 3]; 3], vec![[0.; 2]; 3], vec![[0.; 4]; 3]))),
                MeshMaterial3d(materials.add_fogged(StandardMaterial {
                    base_color: Color::WHITE,
                    base_color_texture: Some(image.clone()),
                    unlit: true,
                    cull_mode: None,
                    alpha_mode: if additive {
                        AlphaMode::Add
                    } else {
                        AlphaMode::Blend
                    },
                    ..default()
                })),
                Transform::default(),
            ));
        }
    }
    // The 3D scene renders into an image the 2D pass composes first, so
    // every 2D layer blends in gamma space as the exe's framebuffer does.
    let size = windows
        .single()
        .map(|w| [w.physical_width().max(1), w.physical_height().max(1)])
        .unwrap_or([1280, 720]);
    compose.scene = images.add(Image::new_target_texture(
        size[0],
        size[1],
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    compose.scene_sheet = frame.add_image(compose.scene.clone(), size, true);
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -2,
            ..default()
        },
        bevy::camera::RenderTarget::Image(compose.scene.clone().into()),
        Projection::Perspective(PerspectiveProjection {
            fov: FollowCamera::new(follow_camera::OUTDOOR, &CameraInput::default()).vertical_fov(),
            near: 5.,
            far: 200_000.,
            ..default()
        }),
        Transform::default(),
        Msaa::Off,
        Tonemapping::None,
        DebandDither::Disabled,
        NoIndirectDrawing,
    ));
    // The Drive change's afterimage (drivefx-02 §3): a feedback pass over
    // the scene, idle until a change turns it on.
    let blur = crate::cutscene::blur::Blur::spawn(
        &mut commands,
        &mut meshes,
        &mut layout_materials,
        &mut images,
        &mut frame,
        &compose.scene,
        size,
        Vec::new(),
        [1., 1.],
    );
    commands.insert_resource(Afterimage { blur, on: false });
    commands.insert_resource(compose);
}

/// The camp menu's member model (Tz::MenuObj) and its off-screen target.
const MENU_LAYER: usize = 3;
const MENU_TARGET: [u32; 2] = [1280, 720];

#[derive(Component)]
struct MenuModelRoot;
#[derive(Component)]
struct MenuModelPart(usize);
#[derive(Component)]
struct MenuCamera;

#[derive(Resource)]
struct MenuModel {
    sheet: usize,
    clip: usize,
    indices: Vec<Vec<u32>>,
}

/// While the camp shows its members, render Sora's walk at the menu time
/// (root motion kept, camp-03 §3).
#[allow(clippy::too_many_arguments)]
fn menu_model_present(
    camp: Option<Res<CampRes>>,
    model: Option<Res<MenuModel>>,
    game_over: Option<Res<game_over::GameOverRes>>,
    sora_rig: Option<Res<SoraRig>>,
    game: Res<Game>,
    mut cameras: Query<&mut Camera, With<MenuCamera>>,
    mut roots: Query<&mut Visibility, With<MenuModelRoot>>,
    parts: Query<(&MenuModelPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(model) = model else { return };
    // The game-over list shows Sora on the GAMEOVER set's motion.
    let over = game_over.as_ref().and_then(|g| g.model_time());
    let time = camp
        .and_then(|c| c.open.as_ref().and_then(|m| m.model_time))
        .or(over);
    if let Ok(mut camera) = cameras.single_mut() {
        camera.is_active = time.is_some();
    }
    for mut v in &mut roots {
        let want = if time.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    let Some(t) = time else { return };
    if let (Some(t), Some(g), Some(rig)) = (over, game_over.as_ref(), sora_rig.as_ref()) {
        if let Some(positions) = game_over::pose(g, rig, t) {
            for (part, handle) in &parts {
                let positions = model.indices[part.0].iter().map(|&i| positions[i as usize]);
                if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                    visual::room::update_positions(&mut mesh, positions);
                }
            }
        }
        return;
    }
    // The member model is base Sora's walk; in a Drive form the room's
    // clips are the form's.
    if game.drive.as_ref().is_some_and(|d| d.shown != 0) {
        return;
    }
    let c = &game.clip_poses[model.clip];
    let f = (t as usize).min(c.frames.len() - 1);
    let root = c.root[f.min(c.root.len() - 1)];
    for (part, handle) in &parts {
        let positions = model.indices[part.0]
            .iter()
            .map(|&i| (c.frames[f][i as usize] + root).to_array());
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            visual::room::update_positions(&mut mesh, positions);
        }
    }
}
/// The 2D composition's sheets: the rendered scene, the dissolve snapshot
/// and the HUD canvas.
#[derive(Resource)]
struct Compose {
    scene: Handle<Image>,
    scene_sheet: usize,
    snapshot_sheet: Option<usize>,
    hud_sheet: Option<usize>,
    /// The area-change iris layout and a white sheet for the fade, drawn
    /// over the HUD by the 2D layer.
    iris_sheet: Option<usize>,
    white_sheet: Option<usize>,
    /// Frames since the scene image last changed size (unattended
    /// captures wait for a settled target).
    stable_frames: u64,
}

/// An attack window this frame: key, ATKP and its spheres.
type ActiveVolume = (u64, u16, Vec<(Vec3, f32)>);

/// The afterimage's feedback cameras and whether they ran last frame.
#[derive(Resource)]
struct Afterimage {
    blur: crate::cutscene::blur::Blur,
    on: bool,
}

/// FUN_14015f030 (priority 75000, after the 3D passes, before the 2D
/// sprites): while a Drive change holds the afterimage, the previous
/// picture is drawn over the scene at alpha/128 (0x70 → 0.875 [O]). The
/// first frame on has no history yet and shows the plain scene [I]; the
/// history lacks the HUD the exe's prevFrameTex holds [I].
fn afterimage(
    mut after: ResMut<Afterimage>,
    game: Res<Game>,
    game_over: Option<Res<game_over::GameOverRes>>,
    mut cameras: Query<&mut Camera>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // The death event's SeqBlur (alpha 56/128) feeds the same pass.
    let alpha = game
        .drive
        .as_ref()
        .and_then(drive_room::DriveRt::afterimage)
        .or(game_over.and_then(|g| g.blur()));
    let weight = alpha.map(|a| if after.on { a } else { 0. });
    after.on = alpha.is_some();
    after.blur.feed(weight, &mut cameras, &mut meshes);
}

/// The scene image follows the window's size.
fn resize_scene(
    compose: Option<ResMut<Compose>>,
    windows: Query<&Window>,
    mut images: ResMut<Assets<Image>>,
    mut frame: ResMut<Ui2dFrame>,
    settings: Res<settings_room::SettingsRes>,
) {
    let (Some(mut c), Ok(w)) = (compose, windows.single()) else {
        return;
    };
    // The Resolution setting scales the scene against the window.
    let scale = settings.current.scale();
    let size = [w.physical_width(), w.physical_height()]
        .map(|v| ((v as f32 * scale).round() as u32).clamp(1, 8192));
    c.stable_frames += 1;
    if frame.sheets[c.scene_sheet].size == size {
        return;
    }
    c.stable_frames = 0;
    if let Some(mut image) = images.get_mut(&c.scene) {
        image.resize(Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        });
    }
    frame.sheets[c.scene_sheet].size = size;
}

/// The scene, then the dissolve snapshot fading out over it, then the HUD
/// (hidden while the camp is open [inferred]); the camp draws after.
#[allow(clippy::too_many_arguments)]
fn compose_2d(
    compose: Option<Res<Compose>>,
    talk: Option<Res<Talk>>,
    camp: Option<Res<CampRes>>,
    area: Option<Res<AreaRes>>,
    hud: Option<Res<Hud>>,
    mut frame: ResMut<Ui2dFrame>,
    settings: Res<settings_room::SettingsRes>,
    game: Res<Game>,
    after: Option<Res<Afterimage>>,
    game_over: Option<Res<game_over::GameOverRes>>,
) {
    let Some(c) = compose else { return };
    // Brightness tints the scene; the HUD setting hides the field HUD. A
    // Drive change darkens the scene's light (drive_room::scene_light).
    let b = settings.current.brightness_factor()
        * game
            .drive
            .as_ref()
            .map_or(1., drive_room::DriveRt::scene_light);
    let t = game
        .drive
        .as_ref()
        .map_or([1.; 3], drive_room::DriveRt::scene_tint);
    let scene = after
        .as_ref()
        .filter(|a| a.on)
        .map_or(c.scene_sheet, |a| a.blur.sheet());
    frame.full_screen(scene, [b * t[0], b * t[1], b * t[2], 1.]);
    if let (Some(s), Some(t)) = (c.snapshot_sheet, &talk)
        && t.dissolve > 0
    {
        frame.full_screen(s, [1., 1., 1., t.dissolve as f32 / DISSOLVE_FRAMES as f32]);
    }
    let camp_open = camp.is_some_and(|c| c.menu_up()) || game_over.is_some_and(|g| g.hides_hud());
    if let Some(h) = c.hud_sheet
        && !camp_open
        && settings.current.show_hud
    {
        frame.full_screen(h, [1.; 4]);
    }
    // The area-change iris (task priority 120000) and the fade (330000)
    // draw over the HUD.
    if let Some(t) = area.as_ref().and_then(|a| a.transition.as_ref()) {
        if let (Some(slot), Some(sheet), Some(player)) = (
            c.iris_sheet,
            hud.as_ref().and_then(|h| h.iris.as_ref()),
            t.iris.as_ref(),
        ) {
            let quads =
                sheet
                    .sequence
                    .quads(player, [0., 0.], &sprite_sequence::Modifiers::default());
            frame.draws.push((slot, quads));
        }
        if t.fade.state != fade::State::Idle
            && let Some(white) = c.white_sheet
        {
            frame.fill(white, [0., 0., 0., t.fade.alpha as f32 / 128.]);
        }
    }
}

/// The exe's pad pipeline for the room (bgm-input-01 Part B): PADDATA,
/// raw word, logical bits through pad mode 1 (field) or 7 (a menu). New
/// presses queue the field actions: jump 0x100 (back button), decide 0x4,
/// guard 0x200 (Square), reaction 0x400 (Triangle), talk advance 0x40c and
/// the command menu's up/down 0x4000/0x8000.
#[allow(clippy::too_many_arguments)]
fn read_pad(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<(&Gamepad, Option<&Name>)>,
    camp: Option<Res<CampRes>>,
    debug: Res<debug_menu::DebugMenu>,
    game_over: Option<Res<game_over::GameOverRes>>,
    mut game: ResMut<Game>,
    mut targeting: ResMut<enemy_target::Targeting>,
) {
    if game.autoplay.is_some() {
        return;
    }
    let (data, _, _) = input::read_devices(&keys, &mouse, 0., &pads);
    let menu = camp.is_some_and(|c| c.menu_up()) || game_over.is_some_and(|g| g.menu_up());
    let mode = if menu {
        controller::MODE_TITLE
    } else {
        controller::MODE_FIELD
    };
    game.pad_data = data;
    game.pad.update_raw(controller::raw(&data), mode, 1.);
    if menu || debug.open {
        return;
    }
    let p = game.pad.pressed;
    // L1 held (logical bit 23) shows the shortcut menu: the face buttons
    // (logical bits 24-27) then pick its four slots instead of acting.
    if game.pad.held & 0x80_0000 != 0 {
        for k in 0..4 {
            if p & (1 << (24 + k)) != 0 {
                game.shortcut_queued |= 1 << k;
            }
        }
        return;
    }
    game.back_queued |= p & 0x8 != 0;
    game.jump_queued |= p & 0x100 != 0;
    game.attack_queued |= p & 0x4 != 0;
    game.guard_queued |= p & 0x200 != 0;
    game.rc_queued |= p & 0x400 != 0;
    game.advance_queued |= p & 0x40c != 0;
    game.menu_up_queued |= p & 0x4000 != 0;
    game.menu_down_queued |= p & 0x8000 != 0;
    game.lock_queued |= p & 0x800 != 0;
    game.cycle_queued |= p & 0x30_0000_0000 != 0;
    targeting.queue_cycle(p);
}
/// Scripted input for unattended verification runs.
fn autoplay_input(tick: u64) -> (Vec2, bool, bool) {
    let stick = match tick {
        60..150 => Vec2::new(0., 1.),
        150..240 => Vec2::new(0., 0.35),
        240..270 => Vec2::new(1., 0.),
        _ => Vec2::ZERO,
    };
    // (1520: Aerial Recovery out of the blow struck at 1500.)
    let jump = matches!(tick, 300 | 560 | 1520);
    // (spell slots are scripted in `simulate`)
    let attack = matches!(tick, 400 | 410 | 440 | 470 | 500 | 575 | 590 | 610 | 630);
    (stick, jump, attack)
}
const AUTOPLAY_CAPTURES: &[u64] = &[
    90, 110, 200, 255, 312, 330, 352, 405, 450, 495, 585, 615, 760, 1010, 1100, 1198, 1300, 1500,
    1512, 1530, 1700, 1712, 1730, 1830, 1900, 1930, 1990, 2030, 2050, 2070, 2110, 2230, 2310, 2330,
    2390, 2420, 2450, 2520, 2690, 2715, 2760, 2780, 2840, 2880, 2915, 2920, 2950, 2970, 3000, 3080,
    3110, 3200, 3220, 3230, 3250, 3270, 3300, 3330, 3560, 3590, 3660, 3690, 3720, 3730, 3760, 3800,
    3830, 3860, 3900, 3930, 4060, 4090, 4150, 4180, 4420, 4450, 4480, 4800, 4830, 4880, 4910, 5000,
    5030, 5100, 5130, 5500, 5560, 5590, 5650, 5690, 5740, 5780, 5800, 5840, 5890, 5930, 5940, 6000,
    6030, 6090, 6210, 6270, 6300, 6330, 6390, 6430, 6460, 6475, 6530, 6560, 6600, 6670, 6705, 6725,
    6760, 6790, 6900, 7000, 7160, 7200, 7240, 7330, 7600, 7680, 7710, 7740, 7770, 7800, 7840, 7905,
    7930, 7960, 8000, 8040, 8105, 8150, 8180, 8200, 8230, 8265, 8300, 8330, 8360, 8390, 8650, 8900,
    9100, 9170,
];
/// The talk room: Talk at tick 60, one advance once the text is up, and
/// captures through the prompt, dissolve, window, close and return.
const AUTOPLAY_TALK: [u64; 1] = [60];
const AUTOPLAY_ADVANCE: [u64; 1] = [150];
const AUTOPLAY_TALK_CAPTURES: [u64; 10] = [40, 61, 68, 84, 100, 140, 156, 164, 172, 200];
const AUTOPLAY_TALK_END: u64 = 260;
/// Debug blow hits of the unattended run (see `Game::debug_blow`).
const AUTOPLAY_BLOWS: [u64; 2] = [1500, 1700];
/// Guard presses ahead of the Shadow's claws seen at ticks 1194 and 1361
/// in earlier unattended runs, and Square for Retaliating Slash out of the
/// blow struck at 1700.
const AUTOPLAY_GUARDS: [u64; 3] = [1170, 1340, 1720];
/// The last stretch leaves Sora standing so an enemy can attack him.
/// The index that holds the item pictures.
const PICTURE_INDEX: &str = "Image/dt/kh2_fifth.hed";
const AUTOPLAY_END: u64 = 9400;
/// A shown-pose change above this many units in one frame is logged.
const POSE_JUMP_LOG: f32 = 6.;
const AUTOPLAY_WARMUP_FRAMES: u64 = 30;

#[allow(clippy::too_many_arguments)]
fn simulate(
    mut rig: ResMut<CameraRig>,
    mut game: ResMut<Game>,
    hud: Option<ResMut<Hud>>,
    mut area: Option<ResMut<AreaRes>>,
    skate: Res<crate::skate::Skate>,
    mut camp: Option<ResMut<CampRes>>,
    gallery: Res<enemy_gallery::EnemyGallery>,
    mut targeting: ResMut<enemy_target::Targeting>,
) {
    let tick = game.tick;
    let (up, down) = if game.ability_tour.is_some()
        || game.enemy_tour
        || game.enemy_battle_tour
        || game.magic_tour
    {
        (false, false)
    } else if game.autoplay.is_some() {
        // (7690: back to Attack for the Drive forms' combos.)
        (tick == 7690, tick == 980)
    } else {
        let queued = (game.menu_up_queued, game.menu_down_queued);
        game.menu_up_queued = false;
        game.menu_down_queued = false;
        queued
    };
    let (stick, jump, decide) = if game.enemy_battle_tour {
        (
            Vec2::ZERO,
            false,
            (700..1300).contains(&tick) && tick.is_multiple_of(20),
        )
    } else if game.enemy_tour {
        (Vec2::ZERO, false, false)
    } else if game.magic_tour {
        (
            Vec2::ZERO,
            magic_tour::jump(tick, game.magic_shift),
            magic_tour::attack(tick, game.magic_shift),
        )
    } else if let Some(tour) = &game.ability_tour {
        (
            Vec2::ZERO,
            tick == tour.close + 60,
            tour.attacks.contains(&tick),
        )
    } else if game.autoplay.is_some() {
        if game.autoplay_end == AUTOPLAY_TALK_END {
            // The talk room's unattended run stands still.
            (Vec2::ZERO, false, false)
        } else {
            autoplay_input(tick)
        }
    } else {
        // The left-stick vector past the game's 60-unit dead zone (up is
        // negative in PADDATA), magnitude capped at 1.
        let [x, y, magnitude] = controller::analog(game.pad_data.left);
        let stick = Vec2::new(x, -y).normalize_or_zero() * magnitude;
        let queued = (game.jump_queued, game.attack_queued);
        game.jump_queued = false;
        game.attack_queued = false;
        (stick, queued.0, queued.1)
    };
    // Field talk locks control (player+0xdc8 → +0x40 bit 0); an area
    // change zeroes the pad while DAT_142a10500 is set (FUN_1403b2340).
    // Riding the skateboard hands the pad to the Skate 3 skater.
    let talking =
        game.talking || area.as_deref().is_some_and(|a| a.transition.is_some()) || skate.riding();
    let (stick, jump, decide) = if talking {
        (Vec2::ZERO, false, false)
    } else {
        (stick, jump, decide)
    };
    let (up, down) = if talking { (false, false) } else { (up, down) };
    // Camera-relative stick: forward is the view direction on the ground,
    // right is screen right.
    let eye = from_world(rig.camera.eye);
    let look_at = from_world(rig.camera.look_at);
    let forward = (look_at - eye).xz().normalize_or(Vec2::NEG_Y);
    let right = Vec2::new(-forward.y, forward.x);
    let direction = forward * stick.y + right * stick.x;
    // Unattended area runs head for an exit that does not lead back to the
    // previous area, so each run walks a chain of areas.
    let (direction, stick) = match area
        .as_deref()
        .filter(|a| a.explore && game.autoplay.is_some())
    {
        Some(a) if a.pending.is_none() && a.transition.is_none() => {
            let p = game.player.position;
            match area::exit_target(&a.current.state, a.previous, [p[0], p[2]]) {
                Some(d) => (Vec2::from_array(d), Vec2::Y),
                None => (direction, stick),
            }
        }
        _ => (direction, stick),
    };
    // The decide button executes the command under the cursor; Attack
    // (icon 1) is the only command this room implements.
    let mut cursor_moved = false;
    let back = std::mem::take(&mut game.back_queued) && !talking;
    let mut attack = decide;
    let mut sub_was_open = false;
    let mut menu_cast = None;
    let mut menu_item = None;
    let mut denied = false;
    if let Some(mut h) = hud
        && let Some(m) = h.menu.as_mut()
    {
        // Magic row colour (command-01 section 0.6: 5 without spells, 1
        // when every entry is unusable) and the open submenu's rows.
        if let Some(mg) = game.magic.as_ref() {
            let list = mg.menu_list(&game.player.mp);
            if let Some(row) = m.menu.items.get_mut(1) {
                row.colour = if list.is_empty() {
                    5
                } else if list.iter().all(|c| c.1 >= 4) {
                    1
                } else {
                    0
                };
            }
            if let Some(sub) = m.menu.sub.as_mut()
                && sub.kind == 1
            {
                for (item, (_, c)) in sub.items.iter_mut().zip(&list) {
                    item.colour = *c;
                }
            }
        }
        // Items row: 5 without items, 1 when every row is unusable.
        if let Some(it) = game.items.as_ref() {
            let list = it.menu_list(game.player.in_air());
            if let Some(row) = m.menu.items.get_mut(2) {
                row.colour = if list.is_empty() {
                    5
                } else if list.iter().all(|c| c.1 >= 4) {
                    1
                } else {
                    0
                };
            }
        }
        // A Drive form greys the commands its mask disables (FUN_1403b6cb0:
        // Magic in Valor, Limit and Antiform, Items in Antiform), and
        // Wisdom's Attack row is "Shoot" (0x16d).
        if let Some(d) = game.drive.as_ref() {
            for (row, cmd) in [(1usize, 2u16), (2, 3)] {
                if d.masked(cmd)
                    && let Some(r) = m.menu.items.get_mut(row)
                {
                    r.colour = 5;
                }
            }
            if let (Some(r), Some(label)) = (m.menu.items.get_mut(0), d.attack_label()) {
                r.label = label;
            }
            // Limit Form's Magic slot is "Limit" (FUN_1403f36c0).
            if let Some(r) = m.menu.items.get_mut(1) {
                if d.limit_form() {
                    r.label = d.src.names.get(&0x2c3).cloned().unwrap_or_default();
                    r.colour = if d.limit.is_some() { 5 } else { 0 };
                } else if !d.src.magic_label.is_empty() {
                    r.label = d.src.magic_label.clone();
                }
            }
            // A limit's follow-up is the reaction command (slot 1).
            m.menu.reaction = d.offer.map(|cmd| field_command_menu::Reaction {
                label: d.src.names.get(&cmd).cloned().unwrap_or_default(),
                dimmed: false,
            });
        }
        // The Drive row: "Drive" opening the forms (submenu 3), or "Revert"
        // in a form (FUN_1403f36c0), and the open submenu's colours.
        if let Some((label, colour, opens)) = drive_room::menu_row(&game) {
            if let Some(row) = m.menu.items.get_mut(3) {
                row.label = label;
                row.colour = colour;
            }
            if let Some(o) = m.menu.opens.get_mut(3) {
                *o = opens.then_some(3);
            }
            if let (Some(sub), Some(d)) = (m.menu.sub.as_mut(), game.drive.as_ref())
                && sub.kind == 3
            {
                for (item, (_, _, c)) in sub.items.iter_mut().zip(d.menu(&game.player.drive)) {
                    item.colour = c;
                }
            }
        }
        sub_was_open = m.menu.sub.is_some();
        let before = (m.menu.cursor, m.menu.sub.as_ref().map(|s| s.cursor));
        let decision = m.menu.input_with_back(up, down, decide, back);
        cursor_moved = (m.menu.cursor, m.menu.sub.as_ref().map(|s| s.cursor)) != before;
        attack =
            decision == Decision::Execute(0) && m.menu.items.first().is_some_and(|i| i.icon == 1);
        // The command menu's own sounds (magicsfx-01 §5): a sub-menu push
        // plays se000 #3 (FUN_1403b2800 with its play flag), and backing out
        // of a sub-menu plays #4 (FUN_1403b20f0 with its play flag). Running
        // a command plays no menu sound (#2 is the dialogue choice's decide).
        // Opening and executing reset the cursor, which is not a move.
        match decision {
            Decision::Open(_) => {
                cursor_moved = false;
                game.sound_queue.push((SE_OPEN, "menu_open"));
            }
            Decision::Execute(_) | Decision::ExecuteSub { .. } => cursor_moved = false,
            Decision::Closed => {
                cursor_moved = false;
                game.sound_queue.push((SE_CLOSE, "menu_close"));
            }
            _ => {}
        }
        match decision {
            // Limit Form: the Magic slot is "Limit" (0x2c3, submenu 12).
            Decision::Open(1) if game.drive.as_ref().is_some_and(|d| d.limit_form()) => {
                let (mp, charging) = (game.player.mp.cur, game.player.mp_charging);
                if let Some(d) = game.drive.as_ref() {
                    let items = d
                        .limit_rows(mp, charging)
                        .into_iter()
                        .map(|(cmd, colour)| Item {
                            label: d.src.names.get(&cmd).cloned().unwrap_or_default(),
                            icon: d.src.commands.get(&cmd).map_or(7, |c| c.icon),
                            colour,
                        })
                        .collect();
                    m.menu.open_sub(12, items);
                }
            }
            Decision::ExecuteSub { row: 1, index }
                if game.drive.as_ref().is_some_and(|d| d.limit_form()) =>
            {
                if let Some(d) = game.drive.as_mut() {
                    d.limit_request = player::limit_form::LIMITS.get(index).copied();
                }
            }
            Decision::Open(1) => {
                let mp = game.player.mp;
                if let Some(mg) = game.magic.as_mut() {
                    let list = mg.menu_list(&mp);
                    let items = list
                        .iter()
                        .map(|&(t, colour)| {
                            let cmd = mg.current(t).map(|r| r.command).unwrap_or(0);
                            Item {
                                label: mg.src.names.get(&cmd).cloned().unwrap_or_default(),
                                icon: mg.src.commands.get(&cmd).map_or(0, |c| c.icon),
                                colour,
                            }
                        })
                        .collect();
                    mg.menu_types = list.iter().map(|l| l.0).collect();
                    m.menu.open_sub(1, items);
                }
            }
            Decision::Open(2) => {
                let air = game.player.in_air();
                let g = &mut *game;
                let (items, magic) = (&mut g.items, &g.magic);
                if let (Some(it), Some(mg)) = (items.as_mut(), magic.as_ref()) {
                    let list = it.menu_list(air);
                    let items = list
                        .iter()
                        .map(|&(slot, colour)| {
                            let cmd = it.command_of(slot).unwrap_or(0);
                            Item {
                                label: mg.src.names.get(&cmd).cloned().unwrap_or_default(),
                                icon: mg.src.commands.get(&cmd).map_or(0, |c| c.icon),
                                colour,
                            }
                        })
                        .collect();
                    it.menu_slots = list.iter().map(|l| l.0).collect();
                    m.menu.open_sub(2, items);
                }
            }
            Decision::ExecuteSub { row: 2, index } => {
                menu_item = game
                    .items
                    .as_ref()
                    .and_then(|it| it.menu_slots.get(index).copied());
            }
            Decision::ExecuteSub { row: 1, index } => {
                // A spell whose command opens a submenu (Cure, submenu 4)
                // first asks for the party member (FUN_1403f22e0).
                let picked = game.magic.as_ref().and_then(|mg| {
                    let t = *mg.menu_types.get(index)?;
                    let cmd = mg.src.commands.get(&mg.current(t)?.command)?;
                    Some((t, cmd.submenu))
                });
                match picked {
                    Some((_, 4)) if !game.party.is_empty() => {
                        let mg = game.magic.as_ref().expect("magic");
                        let name = |id: u16| mg.src.names.get(&id).cloned().unwrap_or_default();
                        let mut items = vec![Item {
                            label: name(magic_room::MEMBER_NAMES),
                            icon: 0,
                            colour: 0,
                        }];
                        for (i, member) in game.party.iter().enumerate() {
                            items.push(Item {
                                label: name(magic_room::MEMBER_NAMES + 1 + i as u16),
                                icon: 0,
                                colour: if member.stored || member.hp <= 0 {
                                    5
                                } else {
                                    0
                                },
                            });
                        }
                        game.cure_pick = Some(index);
                        m.menu.open_sub(4, items);
                    }
                    _ => {
                        menu_cast = game
                            .magic
                            .as_ref()
                            .and_then(|mg| mg.menu_types.get(index).copied());
                    }
                }
            }
            // The party list: the picked member is the Cure's target.
            Decision::ExecuteSub { row: 0, index } if game.cure_pick.is_some() => {
                let pick = game.cure_pick.take();
                menu_cast = game
                    .magic
                    .as_ref()
                    .and_then(|mg| mg.menu_types.get(pick?).copied());
                game.cure_target = index;
            }
            // Backing out of the list returns to the Magic list.
            Decision::Closed if game.cure_pick.take().is_some() => {
                let mp = game.player.mp;
                if let Some(mg) = game.magic.as_mut() {
                    let list = mg.menu_list(&mp);
                    let items = list
                        .iter()
                        .map(|&(t, colour)| {
                            let cmd = mg.current(t).map(|r| r.command).unwrap_or(0);
                            Item {
                                label: mg.src.names.get(&cmd).cloned().unwrap_or_default(),
                                icon: mg.src.commands.get(&cmd).map_or(0, |c| c.icon),
                                colour,
                            }
                        })
                        .collect();
                    mg.menu_types = list.iter().map(|l| l.0).collect();
                    m.menu.open_sub(1, items);
                }
            }
            Decision::Open(3) => {
                let gauge = game.player.drive;
                if let Some(d) = game.drive.as_mut() {
                    let list = d.menu(&gauge);
                    let items = list
                        .iter()
                        .map(|&(_, cmd, colour)| Item {
                            label: d.src.names.get(&cmd).cloned().unwrap_or_default(),
                            icon: d.src.commands.get(&cmd).map_or(4, |c| c.icon),
                            colour,
                        })
                        .collect();
                    d.menu_forms = list.iter().map(|l| l.0).collect();
                    m.menu.open_sub(3, items);
                }
            }
            Decision::ExecuteSub { row: 3, index } => {
                if let Some(d) = game.drive.as_mut() {
                    d.request = d.menu_forms.get(index).copied();
                }
            }
            // Revert (Execute 5, FUN_140405440).
            Decision::Execute(3) => {
                if let Some(d) = game.drive.as_mut() {
                    d.request = Some(0);
                }
            }
            Decision::Invalid => denied = true,
            _ => {}
        }
    }
    // Circle backs out of a submenu instead of jumping.
    let jump = jump && !sub_was_open;
    // Cursor moves play system sound 1 (FUN_1403bc640/FUN_1403b11e0).
    if cursor_moved {
        game.sound_queue.push((SE_CURSOR, "menu_cursor"));
    }
    if denied {
        game.sound_queue.push((5, "menu_denied"));
    }
    // Guard (Square): F or the pad's west button; unattended runs guard
    // just before two of the Shadow's claws.
    let guard = if game.autoplay.is_some() {
        !game.magic_tour && AUTOPLAY_GUARDS.contains(&tick)
    } else {
        std::mem::take(&mut game.guard_queued)
    } && !talking;
    // The Jump button up this tick (logical 0x100 not held): cuts a rising
    // jump. Unattended runs hold it.
    let jump_up = game.autoplay.is_none() && game.pad.held & 0x100 == 0;
    let toggle = std::mem::take(&mut game.lock_queued);
    let cycle = std::mem::take(&mut game.cycle_queued);
    let before = targeting.holder.target;
    let mut selected = enemy_target::select(&game, &gallery, &mut targeting, toggle, cycle, 1.);
    if attack {
        selected = enemy_target::acquire(&game, &gallery, &mut targeting);
    }
    if before != selected.id || toggle || cycle {
        game.event(serde_json::json!({"event":"target_changed","tick":tick,"target":format!("{:?}",selected.id),"locked":selected.locked}));
    }
    game.selected_target = selected.target.map(|t| t.centre);
    // The lock plays se000 #7 as it engages (also when the lock moves to
    // another actor) and #8 when the toggle or an untargetable target releases it.
    let other_actor = match (before, selected.id) {
        (
            Some(enemy_target::TargetId::Gallery(a, _)),
            Some(enemy_target::TargetId::Gallery(b, _)),
        ) => a != b,
        (Some(old), Some(new)) => std::mem::discriminant(&old) != std::mem::discriminant(&new),
        _ => false,
    };
    if selected.locked && (!game.target_locked || other_actor) {
        game.sound_queue.push((SE_LOCK, "lock_on"));
    } else if !selected.locked
        && game.target_locked
        && (toggle
            || before.is_some_and(|id| enemy_target::untargetable_but_present(&game, &gallery, id)))
    {
        game.sound_queue.push((SE_UNLOCK, "lock_off"));
    }
    game.target_locked = selected.locked;
    let target = selected.target.map(|s| {
        let p = game.player.position;
        [
            s.centre[0] - p[0],
            s.centre[1] - p[1],
            s.centre[2] - p[2],
            game.enemy
                .as_ref()
                .map_or(s.radius, |rt| match selected.id {
                    Some(enemy_target::TargetId::Legacy(_)) => rt.target_reach,
                    _ => s.radius + rt.sora_target_radius,
                }),
        ]
    });
    let shapes = selected.shapes;
    let mut nearby = [None; 4];
    for (slot, t) in nearby.iter_mut().zip(&shapes) {
        *slot = Some(*t);
    }
    let input = Input {
        direction: direction.to_array(),
        jump,
        jump_up,
        attack,
        guard,
        // Square held (logical 0x200).
        guard_held: game.autoplay.is_none() && game.pad.held & 0x200 != 0,
        target: target.map(|t| [t[0], t[2]]),
        target_rise: target.map_or(0., |t| t[1]),
        target_reach: target.map_or(0., |t| t[3]),
        target_shape: shapes.first().copied(),
        nearby,
        probe_blocked: false,
    };
    // L1 + face button: the shortcut's command (a spell's level-1 command id
    // stands for the spell of its magic type, FUN_1403b68b0). Unattended
    // runs cast Fire, Thunder, Blizzard and Cure on fixed ticks.
    let slot = std::mem::take(&mut game.shortcut_queued);
    let mut cast = None;
    if slot != 0 && !talking {
        let k = slot.trailing_zeros() as usize;
        let cmd = game.shortcuts[k];
        // FUN_1403f29e0: in Limit Form every non-item slot is a limit.
        let item = game
            .magic
            .as_ref()
            .and_then(|m| m.src.commands.get(&cmd))
            .is_some_and(|c| c.execute == 3);
        if !item
            && let Some(d) = game.drive.as_mut()
            && d.limit_form()
        {
            d.limit_request = player::limit_form::LIMITS.get(k).copied();
        }
        let cmd = if game.drive.as_ref().is_some_and(|d| d.limit_form()) && !item {
            0
        } else {
            cmd
        };
        cast = game
            .magic
            .as_ref()
            .and_then(|m| m.src.commands.get(&cmd))
            .filter(|c| c.execute == 2)
            .map(|c| c.argument as u8);
        // An item shortcut uses the slot holding that item's command.
        if cast.is_none() {
            menu_item =
                menu_item.or_else(|| game.items.as_ref().and_then(|it| it.slot_for_command(cmd)));
        }
    }
    if game.magic_tour {
        // The Magic Lock-On probes switch the ability the tick before.
        match magic_tour::lock_on(tick, game.magic_shift) {
            Some(true) => game.player.abilities.set(abilities::bit::MAGIC_LOCK_ON),
            Some(false) => game.player.abilities.clear(abilities::bit::MAGIC_LOCK_ON),
            None => {}
        }
        cast = magic_tour::cast(tick, game.magic_shift);
        game.cure_target = magic_tour::cure_target(tick, game.magic_shift);
    }
    if game.autoplay.is_some()
        && !game.enemy_battle_tour
        && !game.magic_tour
        && game.ability_tour.is_none()
        && game.autoplay_end != AUTOPLAY_TALK_END
    {
        cast = match tick {
            700 => Some(magic::FIRE),
            800 => Some(magic::BLIZZARD),
            900 => Some(magic::THUNDER),
            1000 => Some(magic::MAGNET),
            1100 => Some(magic::REFLECT),
            1250 => Some(magic::CURE),
            _ => None,
        };
    }
    if game.autoplay.is_some()
        && !game.enemy_battle_tour
        && !game.magic_tour
        && game.ability_tour.is_none()
        && game.autoplay_end != AUTOPLAY_TALK_END
    {
        // The unattended run uses the Potion and the Ether.
        menu_item = match tick {
            1400 => Some(0),
            1460 => Some(1),
            _ => menu_item,
        };
    }
    let cast = cast.or(menu_cast);
    // Spells and items a form disables are refused (FUN_1403b6cb0).
    let (cast, menu_item) = match game.drive.as_ref() {
        Some(d) => (
            cast.filter(|_| !d.masked(2)),
            menu_item.filter(|_| !d.masked(3)),
        ),
        None => (cast, menu_item),
    };
    // Magic Lock-On (FUN_1403aa560 from FUN_1403c5170): Fire, Blizzard,
    // Thunder and Magnet without a target choose one as an attack does and
    // lock it.
    if let Some(t) = cast
        && !talking
        && matches!(
            t,
            magic::FIRE | magic::BLIZZARD | magic::THUNDER | magic::MAGNET
        )
        && game.player.abilities.has(abilities::bit::MAGIC_LOCK_ON)
        && game.selected_target.is_none()
    {
        let chosen = enemy_target::acquire(&game, &gallery, &mut targeting);
        game.selected_target = chosen.target.map(|s| s.centre);
        game.target_locked = chosen.locked;
        let count = enemy_target::candidates(&game, &gallery).len();
        let enemy_state = game.enemy.as_ref().map(|r| {
            (
                r.enabled,
                r.enemy.targetable(),
                r.enemy.hp,
                r.enemy.motion,
                r.targets.len(),
                r.clips.0.contains_key(&r.enemy.motion),
            )
        });
        game.event(serde_json::json!({"event":"magic_lock_on","tick":tick,
            "target":format!("{:?}", chosen.id),"locked":chosen.locked,
            "candidates":count,
            "params":gallery.target_params.is_some(),
            "enemy":enemy_state}));
    }
    if let Some(t) = cast
        && !talking
    {
        magic_room::request(&mut game, t, tick);
    }
    if let Some(slot) = menu_item
        && !talking
    {
        item_room::request(&mut game, slot, tick);
    }
    let Game {
        player,
        params,
        clips,
        world,
        ..
    } = &mut *game;
    let step = player.tick(&input, params, clips, world);
    if game.map_fx_pending
        && let Some(a) = area.as_deref()
    {
        game.map_fx_pending = false;
        game.map_pax = a.current.pax.clone();
        if let Some(pax) = a.current.pax.clone() {
            start_map_effects(&mut game, &pax, a.bg_mask, None);
        }
    }
    magic_room::tick(&mut game, tick);
    // Wisdom's shots join the spell volumes before the hits are tested.
    drive_room::shots(&mut game);
    item_room::tick(&mut game, tick);
    // The area's zone task (FUN_1403a4f90, priority 12000) runs after the
    // objects: exits are tested against Sora's new position.
    if let Some(a) = area.as_deref_mut()
        && a.pending.is_none()
        && a.transition.is_none()
    {
        let moving = (direction.length() > 0.).then(|| direction.normalize().to_array());
        let running = stick.length() >= WALK_THRESHOLD;
        let probe = area::probe(
            Vec3::from_array(game.player.position),
            game.player.yaw,
            moving,
            running,
        );
        // A disable reason stays for 6 s (FUN_140167ef0 param 6.0).
        if a.guide_hold > 0 {
            a.guide_hold -= 1;
        } else {
            a.guide = None;
        }
        let blocked = a.source.blocked_destinations();
        let is_blocked = |w: u8, d: u8| blocked.contains(&(w, d));
        for request in area::requests(&mut a.current.state, &probe, is_blocked) {
            if let ::area::Request::Blocked { world, area: dest } = &request {
                a.guide = a.source.blocked_text(*world, *dest);
                a.guide_hold = 360;
            }
            if let ::area::Request::Guide {
                world, area: dest, ..
            } = &request
            {
                let known = a.visited.contains(&(*world, *dest));
                a.guide = if known {
                    a.source.place_name(*world, *dest).cloned()
                } else {
                    Some(a.source.unknown_place.clone())
                };
            }
            if let ::area::Request::Jump {
                world,
                area,
                entrance,
                ..
            } = request
            {
                game.event(serde_json::json!({"event":"exit","tick":tick,"world":world,
                    "area":area,"entrance":entrance}));
                if a.transition.is_none() {
                    a.transition = Some(Transition {
                        stage: Stage::Closing,
                        iris: None,
                        fade: Fade::default(),
                        target: (world, area, entrance),
                        job: start_area_job(&a.source, (world, area, entrance)),
                    });
                }
            } else if let ::area::Request::BgGroups { on, off } = request {
                for (bit, state) in [(on, true), (off, false)] {
                    if let Some(bit) = bit {
                        if state {
                            a.bg_mask |= 1 << bit;
                        } else {
                            a.bg_mask &= !(1u64 << bit);
                        }
                        if let RoomWorld::Map(w) = &mut game.world {
                            w.set_group(bit, state);
                        }
                        // FUN_140162a00 spawns the group's elements,
                        // FUN_1401629b0 destroys them.
                        if state {
                            if let Some(pax) = a.current.pax.clone() {
                                start_map_effects(&mut game, &pax, 1 << bit, Some(bit));
                            }
                        } else {
                            game.fx_clear_map.push(Some(bit));
                        }
                    }
                }
            }
        }
    }
    combat_tick(&mut game, tick);
    prize_room::tick(&mut game, tick);
    if game.magic_tour
        && let Some((member, spell, heal)) = magic_tour::party_cast(tick, game.magic_shift)
    {
        let ordered = party_room::order_cast(&mut game, member, spell, heal);
        game.event(
            serde_json::json!({"event":"party_cast_order","tick":tick,"member":member,
            "spell":format!("{spell:?}"),"heal":heal,"ordered":ordered}),
        );
    }
    party_room::tick(&mut game, tick);
    // Drive: requests, the form change, the form gauge (after the player
    // tick, like FUN_1403a92c0 / FUN_140404a40).
    {
        let mut slots = camp.as_deref_mut().map(|c| &mut c.ability_slots);
        let exp = game
            .drive
            .as_mut()
            .map_or(0, |d| std::mem::take(&mut d.pending_exp));
        drive_room::add_form_exp(&mut game, exp, slots.as_deref_mut());
        drive_room::tick(&mut game, slots.as_deref().map(|s| s.as_slice()));
        let rc = game.drive.as_ref().is_some_and(|d| d.offer.is_some())
            && (std::mem::take(&mut game.rc_queued) || game.autoplay.is_some());
        drive_room::limit_tick(&mut game, rc, None);
        drive_room::motion_effects(&mut game);
    }
    // Camera inputs: Q/E or J/L and the right stick turn, -/= zoom, C
    // recenters (test-room bindings for the original's camera buttons).
    let mut turn = 0f32;
    let mut zoom = 0f32;
    let mut right_stick = 0f32;
    if game.autoplay.is_some() {
        // Unattended runs orbit 180 degrees to check Sora from the front.
        if (660..720).contains(&tick) {
            turn = 1.;
        }
    } else {
        // Camera bits (mode "A", right stick): yaw right 21 / left 20 and
        // height down 19 / up 18 scaled by the right stick's magnitude;
        // R3 (bit 17) recentres. Height drives the follow camera's radius
        // here [test-room mapping of the original height adjust].
        let held = game.pad.held;
        let [_, _, magnitude] = controller::analog(game.pad_data.right);
        right_stick = magnitude;
        let axis = |pos: u64, neg: u64| {
            ((held & pos != 0) as i32 - (held & neg != 0) as i32) as f32 * magnitude
        };
        turn = axis(0x20_0000, 0x10_0000);
        zoom = axis(0x8_0000, 0x4_0000);
        if game.pad.pressed & 0x2_0000 != 0 {
            rig.recenter_queued = true;
        }
        let pending = rig.pending_turn.clamp(-1., 1.);
        if turn == 0. && pending != 0. {
            turn = pending;
            rig.pending_turn -= pending;
        }
    }
    let mut camera = camera_input(&game.player, rig.previous_position, game.params.run_speed);
    camera.stick = [stick.x, -stick.y];
    camera.magnitude = stick.length().min(1.);
    camera.turn = turn.clamp(-1., 1.);
    camera.zoom = zoom.clamp(-1., 1.);
    camera.right_stick = right_stick;
    camera.recenter = std::mem::take(&mut rig.recenter_queued);
    // Pad bit 32 (R2) keeps manual zoom allowed through the follow target's
    // zoom lock; no supported actor sets the lock.
    camera.zoom_override = game.pad.held & (1 << 32) != 0;
    camera.lock_target = if game.target_locked {
        targeting
            .holder
            .target
            .and_then(|id| enemy_target::camera_anchor(&game, &gallery, id))
            .map(to_world)
    } else {
        None
    };
    let CameraRig {
        camera: follow,
        collision,
        collision_hits,
        ..
    } = &mut *rig;
    let hits = std::cell::Cell::new(0);
    let spheres = enemy_target::camera_spheres(&gallery, to_world);
    follow.tick_with(
        &camera,
        &|a, b| {
            let hit = collision.cast(a, b);
            hits.set(hits.get() + hit.is_some() as u64);
            hit
        },
        &spheres,
    );
    *collision_hits += hits.get();
    rig.previous_position = game.player.position;
    game.tick += 1;
    if let Some(action) = step.started {
        let position = game.player.position;
        let armed = game.player.armed;
        game.action_started = game.tick;
        let motion = game.player.motion_id(&game.clips);
        let ptya_id = match action {
            Action::Attack(i) | Action::AirAttack(i) => Some(game.clips.records[i].id),
            _ => None,
        };
        let combo = [game.player.combo.step, game.player.combo.finish];
        game.event(
            serde_json::json!({"event":"action","tick":tick,"action":format!("{action:?}"),"armed":armed,
            "position":position,"landed":step.landed,"motion":motion,"ptya":ptya_id,"combo":combo}),
        );
    }
    sora_sounds(&mut game);
    sora_dust(&mut game);
    if tick.is_multiple_of(60) {
        let (position, yaw, action) = (game.player.position, game.player.yaw, game.player.action);
        let enemy = game.enemy.as_ref().map(|e| {
            serde_json::json!({"position":e.enemy.position,"yaw":e.enemy.yaw,
                "hp":e.enemy.hp,"act":format!("{:?}", e.enemy.current_act()),"motion":e.enemy.motion})
        });
        game.event(
            serde_json::json!({"event":"state","tick":tick,"position":position,"yaw":yaw,
            "action":format!("{action:?}"),"camera_eye":rig.camera.eye,"camera_yaw":rig.camera.yaw,
            "camera_collision_triangles":rig.collision.polygons(),"camera_collision_hits":rig.collision_hits,
            "enemy":enemy,
            "npcs":area.as_deref().map(|a| a.current.npcs.iter().map(|n| serde_json::json!({
                "kind":a.source.kinds[n.kind].name,"position":n.position.to_array(),"yaw":n.yaw,
                "pose":format!("{:?}", n.pose),"talk":n.talk,"reaction":n.reaction,"alive":n.alive,"fade":n.fade,
                "serial":n.serial,"point":n.entity,"act":n.act_name(),
                "script":a.current.scripts.contains_key(&n.talk),
                "target":a.source.kinds[n.kind].target.is_some()})).collect::<Vec<_>>())}),
        );
    }
}

/// The camera's view matrix in game world space (Y down), in the effect
/// VM's convention: view x right, y down, z forward.
fn game_view(eye: [f32; 3], look_at: [f32; 3]) -> ::effect::math::Mat {
    let e = Vec3::from_array(eye);
    let f = (Vec3::from_array(look_at) - e).normalize_or(Vec3::Z);
    let down = (Vec3::Y - f * Vec3::Y.dot(f)).normalize_or(Vec3::Y);
    let right = down.cross(f);
    [
        [right.x, down.x, f.x, 0.],
        [right.y, down.y, f.y, 0.],
        [right.z, down.z, f.z, 0.],
        [-right.dot(e), -down.dot(e), -f.dot(e), 1.],
    ]
}

/// The enemy's object matrix in game world space (scene yaw θ is a game
/// rotation of −θ about Y) composed with a bound element's transform.
fn bound_world(
    position: [f32; 3],
    yaw: f32,
    element: &object_effect::Element,
) -> ::effect::math::Mat {
    let object = ::effect::instance_matrix(to_world(position), [0., -yaw, 0.], [1.; 3]);
    let local = ::effect::instance_matrix(element.translation, element.rotation, element.scale);
    ::effect::math::mul(&object, &local)
}

/// Starts requested effects, steps every running instance one game frame
/// and keeps the quads they drew.
fn effects_tick(mut game: ResMut<Game>, rig: Res<CameraRig>, mut fx: ResMut<Effects>) {
    let tick = game.tick;
    if std::mem::take(&mut game.weapon_pax_changed)
        && let Some(pax) = game.weapon_pax.clone()
    {
        fx.weapon = Some(pax);
    }
    weapon_trail::tick(&mut game, fx.weapon.as_ref());
    let view = game_view(rig.camera.eye, rig.camera.look_at);
    for request in std::mem::take(&mut game.effect_queue) {
        let (pax, element, world, label, bound) = match request {
            EffectRequest::Hit {
                effect,
                random,
                contact,
                direction,
            } => {
                let Some(pax) = fx.weapon.clone() else {
                    continue;
                };
                // mask = (1 << (32 + hitEffect − 1)) | 1. A random count
                // adds an LCG pick (DAT_140783ca0 × 0x10dcd) whose seed is
                // untraced; Sora's rows have none, so those hits are exact.
                if random != 0 {
                    game.event(serde_json::json!({"event":"effect_random_unmodelled","tick":tick,"count":random}));
                }
                let b = effect as u64 - 1;
                let mask = (1u64 << (32 + b)) | 1;
                let Some(element) = pax.by_category(mask).cloned() else {
                    game.event(serde_json::json!({"event":"effect_missing","tick":tick,"mask":format!("{mask:#x}")}));
                    continue;
                };
                let d = to_world(direction.to_array());
                let pitch = (-d[1]).atan2((d[0] * d[0] + d[2] * d[2]).sqrt());
                let yaw = d[0].atan2(d[2]);
                let world = ::effect::instance_matrix(
                    to_world(contact.to_array()),
                    [pitch, yaw, 0.],
                    element.scale,
                );
                (pax, element, world, "hit", None)
            }
            EffectRequest::EnemyHit {
                object,
                effect,
                random,
                contact,
                direction,
            } => {
                let pax = if object == 0 {
                    fx.enemy.clone()
                } else {
                    fx.enemy_paxes.get(&object).cloned()
                };
                let Some(pax) = pax else { continue };
                if random != 0 {
                    game.event(serde_json::json!({"event":"effect_random_unmodelled","tick":tick,"count":random}));
                }
                // mask = (1 << (32 + hitEffect − 1)) | 1 on the attacker's
                // own container; a hit with no such element starts nothing.
                let mask = (1u64 << (32 + effect as u64 - 1)) | 1;
                let Some(element) = pax.by_category(mask) else {
                    game.event(serde_json::json!({"event":"effect_missing","tick":tick,"object":object,"mask":format!("{mask:#x}")}));
                    continue;
                };
                let d = to_world(direction.to_array());
                game.fx_queue.push(drive_fx::Placed {
                    req: drive_fx::FxRequest {
                        pax: pax.clone(),
                        id: element.id,
                        owner: 0xfe,
                    },
                    anchor: drive_fx::Anchor::World {
                        at: contact,
                        pitch: (-d[1]).atan2((d[0] * d[0] + d[2] * d[2]).sqrt()),
                        yaw: d[0].atan2(d[2]),
                    },
                });
                continue;
            }
            EffectRequest::ActorElement { object, actor, id } => {
                let pax = if object == 0 {
                    fx.enemy.clone()
                } else {
                    fx.enemy_paxes.get(&object).cloned()
                };
                let Some(pax) = pax else { continue };
                game.fx_queue.push(drive_fx::Placed {
                    req: drive_fx::FxRequest {
                        pax,
                        id,
                        owner: 0xfa,
                    },
                    anchor: drive_fx::Anchor::Actor(actor),
                });
                continue;
            }
            EffectRequest::ActorElementAt { object, id, at } => {
                let pax = if object == 0 {
                    fx.enemy.clone()
                } else {
                    fx.enemy_paxes.get(&object).cloned()
                };
                let Some(pax) = pax else { continue };
                game.fx_queue.push(drive_fx::Placed {
                    req: drive_fx::FxRequest {
                        pax,
                        id,
                        owner: 0xfa,
                    },
                    anchor: drive_fx::Anchor::World {
                        at,
                        pitch: 0.,
                        yaw: 0.,
                    },
                });
                continue;
            }
            EffectRequest::EnemyDead => {
                // effect_start_bind(obj, 2): the enemy's own element 2,
                // bound to its object, with its own sound.
                let Some(pax) = fx.enemy.clone() else {
                    continue;
                };
                game.fx_queue.push(drive_fx::Placed {
                    req: drive_fx::FxRequest {
                        pax,
                        id: 2,
                        owner: 0xfe,
                    },
                    anchor: drive_fx::Anchor::Enemy,
                });
                continue;
            }
            EffectRequest::System { id, at } => {
                let Some(pax) = fx.system.clone() else {
                    continue;
                };
                let Some(element) = pax.by_id(id).cloned() else {
                    continue;
                };
                let world = ::effect::instance_matrix(
                    to_world(at.to_array()),
                    element.rotation,
                    element.scale,
                );
                (pax, element, world, "system", None)
            }
        };
        // The element's own sound [inferred: played as a plain request id].
        // Only a positive id plays (FUN_1402c58f0); 0 is "no sound".
        if element.sound > 0 {
            game.sound_queue.push((element.sound as u32, "effect_se"));
        }
        let Some(dpx) = pax.effect(element.effect_number) else {
            continue;
        };
        let Some(instance) = ::effect::Instance::new(
            pax.dpds[dpx.dpd].clone(),
            dpx.pdata,
            world,
            element.start_wait,
        ) else {
            continue;
        };
        // At most 8 hit effects live (FUN_1403d9be0 ring).
        if label == "hit"
            && fx.running.iter().filter(|r| r.label == "hit").count() >= 8
            && let Some(oldest) = fx.running.iter().position(|r| r.label == "hit")
        {
            fx.running.remove(oldest);
        }
        game.event(serde_json::json!({"event":"effect","tick":tick,"kind":label,
            "element":element.id,"effect_number":element.effect_number,"dpd":dpx.dpd,"pdata":dpx.pdata,
            "position":[world[3][0],world[3][1],world[3][2]]}));
        fx.running.push(RunningEffect {
            instance,
            label,
            bound,
        });
    }
    let enemy = game
        .enemy
        .as_ref()
        .map(|rt| (rt.enemy.position, rt.enemy.yaw));
    let Effects {
        running,
        rng,
        quads,
        view_inverse,
        reported,
        ..
    } = &mut *fx;
    let mut spread: Vec<(&'static str, f32)> = Vec::new();
    quads.clear();
    for r in running.iter_mut() {
        if let (Some(element), Some((position, yaw))) = (&r.bound, enemy) {
            r.instance.world = bound_world(position, yaw, element);
        }
        let drawn = r.instance.frame(&view, rng);
        // Farthest quad centre from the instance origin (view space).
        let o = ::effect::math::apply(&view, r.instance.world[3]);
        let far = drawn
            .iter()
            .map(|q| {
                let c: [f32; 3] =
                    std::array::from_fn(|i| q.corners.iter().map(|p| p[i]).sum::<f32>() / 4.);
                ((c[0] - o[0]).powi(2) + (c[1] - o[1]).powi(2) + (c[2] - o[2]).powi(2)).sqrt()
            })
            .fold(0f32, f32::max);
        spread.push((r.label, far));
        quads.extend(drawn);
        for &kind in &r.instance.unsupported {
            if !reported.contains(&kind) {
                reported.push(kind);
                game.event(serde_json::json!({"event":"effect_unsupported","tick":tick,"program":kind,"effect":r.label}));
            }
        }
    }
    running.retain(|r| !r.instance.finished());
    *view_inverse = Mat4::from_cols_array_2d(&view).inverse();
    if tick.is_multiple_of(10) && !running.is_empty() {
        game.event(serde_json::json!({"event":"effects_state","tick":tick,"running":running.len(),
            "parts":running.iter().map(|r| r.instance.live_parts()).sum::<usize>(),"quads":quads.len(),
            "spread":spread}));
    }
}

/// Rebuilds the effect meshes from the last tick's quads: view space back
/// to game world through the inverse view, then to scene coordinates.
/// Colours are PS2 gamma-space multipliers, converted to linear.
fn present_effects(
    fx: Res<Effects>,
    targets: Query<(&EffectMesh, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (target, handle) in &targets {
        let mut positions = Vec::new();
        let mut uvs = Vec::new();
        let mut colors = Vec::new();
        for q in fx
            .quads
            .iter()
            .filter(|q| q.tbp == visual::effects::ATLAS_TBP)
            .filter(|q| (q.blend == 0x48) == target.additive)
        {
            let corner = |i: usize| {
                let p = fx
                    .view_inverse
                    .transform_point3(Vec3::from_array(q.corners[i]));
                from_world(p.to_array()).to_array()
            };
            let colour = [
                q.colour[0].max(0.).powf(2.2),
                q.colour[1].max(0.).powf(2.2),
                q.colour[2].max(0.).powf(2.2),
                q.colour[3].clamp(0., 1.),
            ];
            for i in [0, 1, 2, 0, 2, 3] {
                positions.push(corner(i));
                uvs.push([q.uv[i][0] * fx.uv_scale[0], q.uv[i][1] * fx.uv_scale[1]]);
                colors.push(colour);
            }
        }
        if positions.is_empty() {
            positions = vec![[0.; 3]; 3];
            uvs = vec![[0.; 2]; 3];
            colors = vec![[0.; 4]; 3];
        }
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
        }
    }
}

/// A collision shape for the talk camera: bone, offset, height offset,
/// radius and height (model space, Y up).
#[derive(Clone, Copy, Debug)]
struct Shape {
    bone: usize,
    offset: Vec3,
    pos_h: f32,
    radius: f32,
    height: f32,
}

struct NpcSource {
    model: model_inspection::InspectionModel,
    frames: Vec<Vec<Vec3>>,
    bones: Vec<HashMap<usize, Mat4>>,
    loop_start: u32,
    target: Shape,
    sheet: Sheet,
    font: SystemFont,
    lines: HashMap<u32, Vec<u8>>,
    colour: u32,
    label: Vec<u8>,
    offset: Vec3,
    yaw: f32,
    script: Vec<visual::dialogue::Op>,
}

/// Sora's OBJ ellipsoid (P_EX100 type 23 entry 1): radius, height and its
/// centre above the root; the towns-person reaction sphere (composed for
/// Axel): centre 200 up, radius 200, height 200 (dialogue-01 §2.2).
const SORA_OBJ: [f32; 3] = [35., 55., 85.];
const REACTION_SPHERE: [f32; 3] = [200., 200., 200.];
/// cmd 31 "Talk": angle gate cos 135° (+0x10); message 0x81ea.
const TALK_DOT: f32 = -std::f32::consts::FRAC_1_SQRT_2;
const TALK_LABEL: u32 = 0x81ea;
/// Screen cross-dissolve length (FUN_140163040(8.0, 1): 2 × 8 frames) and
/// the talk camera's field of view (0x14071a290, 60°).
const DISSOLVE_FRAMES: u32 = 16;
const TALK_FOV: f32 = std::f32::consts::FRAC_PI_3;

/// The field-talk state: the NPC, the window art and font, the running
/// script and the talk camera (FUN_140401410 / FUN_140400f90).
#[derive(Resource)]
struct Talk {
    position: Vec3,
    yaw: f32,
    home_yaw: f32,
    tick: u32,
    frames: Vec<Vec<Vec3>>,
    bones: Vec<HashMap<usize, Mat4>>,
    loop_start: u32,
    target: Shape,
    indices: Vec<Vec<u32>>,
    sheet: Sheet,
    font: SystemFont,
    lines: HashMap<u32, Vec<u8>>,
    colour: u32,
    label: Vec<u8>,
    script: Vec<visual::dialogue::Op>,
    /// Script position while talking.
    active: Option<usize>,
    current: Option<visual::dialogue::TalkWindow>,
    closing: Vec<visual::dialogue::TalkWindow>,
    wait: f32,
    /// Talk camera eye and target (scene coordinates).
    camera: Option<(Vec3, Vec3)>,
    /// Remaining cross-dissolve frames, and a snapshot to take this frame.
    dissolve: u32,
    snapshot: bool,
    /// Reaction candidate this frame: Some(dimmed).
    reaction: Option<bool>,
}

#[derive(Component)]
struct NpcRoot;
#[derive(Component)]
struct NpcPart(usize);
#[derive(Component)]
struct SnapshotCamera;

fn load_npc(
    source: &ProbeSource,
    base: &Path,
    config: &Config,
    npc: &NpcConfig,
) -> Result<NpcSource, Box<dyn std::error::Error>> {
    let menu = config
        .menu
        .as_ref()
        .ok_or("an NPC needs the menu sources")?;
    let mut models =
        model_inspection::read_selected_models(source, &base.join(&npc.model_selection))?;
    if models.len() != 1 {
        return Err("NPC selection requires exactly one model".into());
    }
    let model = models.remove(0);
    let collision = object_collision::read(bar_entry_typed(
        &read_archive(source, &npc.target_model)?,
        23,
    )?)?;
    let t = collision
        .iter()
        .find(|e| e.kind == object_collision::TARGET && e.bone >= 0)
        .ok_or("no TARGET shape for the talk camera")?;
    let target = match &npc.camera_shape {
        Some(c) => Shape {
            bone: c.bone,
            offset: Vec3::from_array(c.offset),
            pos_h: c.pos_h,
            radius: c.radius,
            height: c.height,
        },
        None => Shape {
            bone: t.bone as usize,
            offset: Vec3::new(
                t.position[0] as f32,
                t.position[1] as f32,
                t.position[2] as f32,
            ),
            pos_h: t.position_height as f32,
            radius: t.radius as f32,
            height: t.height as f32,
        },
    };
    let selected = motion::read_slots(
        source,
        &npc.motion_set.index,
        npc.motion_set.ordinal,
        npc.motion_set.name_hash,
        &[0],
    )?;
    let sel = selected.first().ok_or("NPC idle motion absent")?;
    let (clip, key_start) = clip_for(sel)?;
    let (mut frames, mut bones) = (Vec::new(), Vec::new());
    for offset in 0..=clip.end {
        let (pose, evaluated) = model
            .skinning
            .sample_evaluated(&sel.motion, (key_start + offset) as f32)?;
        let r = Vec3::from_array(evaluated.root_translation);
        frames.push(
            pose.position_bits
                .iter()
                .map(|p| Vec3::from_array(p.map(f32::from_bits)) - r)
                .collect(),
        );
        let mut b = HashMap::new();
        if let Some(m) = pose.matrix_bits.get(target.bone) {
            let mut m = Mat4::from_cols_array(&m.map(f32::from_bits));
            m.w_axis -= r.extend(0.);
            b.insert(target.bone, m);
        }
        bones.push(b);
    }
    fn loc(l: &Locator) -> (&str, usize, [u8; 16]) {
        (l.index.as_str(), l.ordinal, l.name_hash)
    }
    let sheet = hud::load_window_sheet(source, loc(&npc.font_info), loc(&npc.world_messages))?;
    let font = hud::load_font(source, loc(&npc.font_image), loc(&npc.font_info), b"evt\0")?;
    let texts = hud::load_messages(
        source,
        &npc.messages.index,
        npc.messages.ordinal,
        npc.messages.name_hash,
        &npc.script,
    )?;
    let lines = npc.script.iter().copied().zip(texts).collect();
    let colour = hud::ftst_colour(&read_archive(source, &config.system)?, 5, npc.world)?;
    let label = hud::load_messages(
        source,
        &menu.messages.index,
        menu.messages.ordinal,
        menu.messages.name_hash,
        &[TALK_LABEL],
    )?
    .remove(0);
    Ok(NpcSource {
        model,
        frames,
        bones,
        loop_start: clip.loop_start,
        target,
        sheet,
        font,
        lines,
        colour,
        label,
        offset: Vec3::from_array(npc.offset),
        yaw: npc.yaw,
        // Composed libretto script: op 1 per message id.
        script: npc
            .script
            .iter()
            .map(|&id| visual::dialogue::Op::Message(id))
            .collect(),
    })
}

/// FUN_140175e50 (flag 1): the target is the shape centre; the eye sits
/// `radius` from it toward the player, raised by `height`.
fn talk_camera(talk: &Talk, player: Vec3) -> (Vec3, Vec3) {
    let s = talk.target;
    let m = talk.bones[(talk.tick as usize).min(talk.bones.len() - 1)]
        .get(&s.bone)
        .copied()
        .unwrap_or(Mat4::IDENTITY);
    let turn = Quat::from_rotation_y(talk.yaw);
    let centre = talk.position + turn * (m.transform_point3(s.offset) + Vec3::Y * s.pos_h);
    let toward = ((player - centre) * Vec3::new(1., 0., 1.)).normalize_or(Vec3::Z);
    (centre + toward * s.radius + Vec3::Y * s.height, centre)
}

/// One frame of field talk: the NPC's idle, the reaction-command test
/// (FUN_140407de0 / FUN_1404088f0), and while talking the script
/// (FUN_1403ecb60), its windows and Sora turning toward the NPC.
fn talk_tick(mut game: ResMut<Game>, talk: Option<ResMut<Talk>>, hud: Option<ResMut<Hud>>) {
    use visual::dialogue::{Op, TalkWindow};
    let Some(mut talk) = talk else { return };
    let tick = game.tick;
    let autoplay = game.autoplay.is_some();
    let rc = std::mem::take(&mut game.rc_queued) || (autoplay && AUTOPLAY_TALK.contains(&tick));
    let advance =
        std::mem::take(&mut game.advance_queued) || (autoplay && AUTOPLAY_ADVANCE.contains(&tick));
    // The talk motion (0x32) is absent from Axel's set, so he keeps idling.
    talk.tick += 1;
    if talk.tick as usize >= talk.frames.len() {
        talk.tick = talk.loop_start;
    }
    talk.dissolve = talk.dissolve.saturating_sub(1);
    let sora = Vec3::from_array(game.player.position);
    if talk.active.is_none() {
        let sora_centre = sora + Vec3::Y * SORA_OBJ[2];
        let npc_centre = talk.position + Vec3::Y * REACTION_SPHERE[2];
        let d = npc_centre - sora_centre;
        let (r, h) = (
            REACTION_SPHERE[0] + SORA_OBJ[0],
            REACTION_SPHERE[1] + SORA_OBJ[1],
        );
        let inside = (d.x * d.x + d.z * d.z) / (r * r) + d.y * d.y / (h * h) < 1.;
        // cmd flags bit 4: no Talk in battle (an enemy is active).
        let battle = game.enemy.as_ref().is_some_and(|rt| rt.enemy.alive());
        let facing = Vec3::new(game.player.yaw.sin(), 0., game.player.yaw.cos());
        let dir = (d * Vec3::new(1., 0., 1.)).normalize_or_zero();
        talk.reaction = (inside && !battle && !game.player.action.airborne())
            .then(|| dir.dot(facing) < TALK_DOT);
        if rc && talk.reaction == Some(false) {
            // FUN_140401410 → FUN_140400f90: lock Sora, snap the NPC to face
            // him, cross-dissolve to the talk camera.
            talk.active = Some(0);
            game.talking = true;
            let to_sora = sora - talk.position;
            talk.yaw = to_sora.x.atan2(to_sora.z);
            talk.snapshot = true;
            talk.dissolve = DISSOLVE_FRAMES;
            let cam = talk_camera(&talk, sora);
            talk.camera = Some(cam);
            game.event(serde_json::json!({"event":"talk_start","tick":tick,
                "camera_eye":cam.0.to_array(),"camera_target":cam.1.to_array()}));
        }
    }
    if let Some(mut i) = talk.active {
        // Sora turns toward the NPC at his turn rate (FUN_1403b65c0).
        let v = talk.position - sora;
        let want = v.x.atan2(v.z);
        let step = game.params.turn_speed;
        let delta = (want - game.player.yaw + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        game.player.yaw += delta.clamp(-step, step);
        loop {
            let Some(op) = talk.script.get(i).cloned() else {
                // End: restore facing, camera (with a dissolve) and control.
                talk.active = None;
                talk.yaw = talk.home_yaw;
                talk.camera = None;
                talk.snapshot = true;
                talk.dissolve = DISSOLVE_FRAMES;
                game.talking = false;
                game.event(serde_json::json!({"event":"talk_end","tick":tick}));
                break;
            };
            match op {
                Op::Message(id) => match talk.current.as_ref().map(|w| w.state) {
                    None => {
                        let text = talk.lines.get(&id).cloned().unwrap_or_default();
                        let window = TalkWindow::new(
                            &talk.sheet.sequence,
                            text,
                            talk.colour,
                            &talk.font.widths,
                        );
                        game.event(serde_json::json!({"event":"talk_window","tick":tick,
                            "message":id,"size":window.size()}));
                        talk.current = Some(window);
                        break;
                    }
                    Some(3) => {
                        let mut w = talk.current.take().unwrap();
                        w.close();
                        talk.closing.push(w);
                        i += 1;
                    }
                    Some(_) => break,
                },
                Op::Wait(frames) => {
                    if talk.wait <= 0. {
                        talk.wait = frames;
                    }
                    talk.wait -= 1.;
                    if talk.wait > 0. {
                        break;
                    }
                    i += 1;
                }
                Op::Stop => i = usize::MAX,
                Op::Signal(_) | Op::Other(..) => i += 1,
            }
            if talk.active.is_some() {
                talk.active = Some(i);
            }
        }
    }
    let mut sounds = Vec::new();
    if let Some(w) = talk.current.as_mut() {
        sounds.extend(w.step(advance));
    }
    for w in &mut talk.closing {
        sounds.extend(w.step(false));
    }
    talk.closing.retain(|w| !w.deleted());
    for s in sounds {
        game.sound_queue.push((s, "talk"));
    }
    if let Some(mut hud) = hud
        && let Some(m) = hud.menu.as_mut()
    {
        let label = talk.label.clone();
        m.menu.reaction = if game.drive.as_ref().is_some_and(|d| d.offer.is_some()) {
            m.menu.reaction.take()
        } else if game.talking {
            None
        } else {
            talk.reaction
                .map(|dimmed| field_command_menu::Reaction { label, dimmed })
        };
    }
}

fn present_npc(
    talk: Option<Res<Talk>>,
    mut roots: Query<&mut Transform, With<NpcRoot>>,
    parts: Query<(&NpcPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(talk) = talk else { return };
    if let Ok(mut t) = roots.single_mut() {
        t.translation = talk.position;
        t.rotation = Quat::from_rotation_y(talk.yaw);
    }
    let frame = &talk.frames[(talk.tick as usize).min(talk.frames.len() - 1)];
    for (part, handle) in &parts {
        let positions = talk.indices[part.0]
            .iter()
            .map(|&i| frame[i as usize].to_array());
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            visual::room::update_positions(&mut mesh, positions);
        }
    }
}

/// FUN_140163040 / FUN_1401635a0: on a camera cut the old frame is
/// snapshotted and drawn over the new view at alpha falling 1 → 0 over 16
/// frames. The snapshot renders the old view with a second camera before
/// `camera_control` moves the main one.
#[allow(clippy::type_complexity)]
fn dissolve_present(
    talk: Option<ResMut<Talk>>,
    main: Query<(&Transform, &Projection), CameraOnly>,
    mut snapshot: Query<
        (&mut Camera, &mut Transform, &mut Projection),
        (With<SnapshotCamera>, Without<SoraRoot>, Without<WeaponRoot>),
    >,
) {
    let Some(mut talk) = talk else { return };
    let Ok((mut camera, mut transform, mut projection)) = snapshot.single_mut() else {
        return;
    };
    camera.is_active = false;
    if std::mem::take(&mut talk.snapshot)
        && let Ok((t, p)) = main.single()
    {
        *transform = *t;
        *projection = p.clone();
        camera.is_active = true;
    }
}

/// Frame-8 sound triggers of Sora's current clip (the Keyblade's motion)
/// reached since the last scan; a new action or a loop rescans from 0.
fn sora_sounds(game: &mut Game) {
    let clip = game.poses[&(game.player.action, game.player.armed)];
    let frame = game.player.motion_frame(&game.clips);
    let started = game.action_started;
    let from = match game.sound_frame {
        Some((s, f)) if s == started && f <= frame => Some(f),
        _ => None,
    };
    for &(at, id) in &game.clip_poses[clip].sounds {
        if from.is_none_or(|f| at > f) && at <= frame {
            game.sound_queue.push((id, "swing"));
        }
    }
    game.sound_frame = Some((started, frame));
    // Frame trigger 14 (FUN_140404d00 → FUN_1403d69b0): a battle voice,
    // none in Antiform.
    let anti = game
        .drive
        .as_ref()
        .is_some_and(|d| d.active && d.form == player::forms::ANTI);
    let clip = game.player.clip(&game.clips).clone();
    let raw = game.player.frame;
    let key = (started, game.player.action);
    if let Some(v) = game.voices.as_mut() {
        let from = match v.scan {
            Some((s, a, f)) if (s, a) == key && f <= raw => Some(f),
            _ => None,
        };
        v.scan = Some((key.0, key.1, raw));
        if !anti {
            for &(at, n) in &clip.voices {
                if from.is_none_or(|f| at > f)
                    && at <= raw
                    && let Some(id) = v.pick(1, n)
                {
                    game.sound_queue.push((id, "voice"));
                }
            }
        }
    }
    // Frame trigger 8 on Sora's own motion: cloth, jump and item-use sounds.
    // The looped frame, so a walking or running cycle plays its triggers
    // again each time it wraps (a wrap clears the scan, as for swings).
    let looped = game.player.motion_frame(&game.clips);
    let from = match game.move_se_scan {
        Some((s, a, f)) if (s, a) == key && f <= looped => Some(f),
        _ => None,
    };
    game.move_se_scan = Some((key.0, key.1, looped));
    for &(at, id) in &clip.sounds {
        if from.is_none_or(|f| at > f) && at <= looped {
            game.sound_queue.push((id, "move_se"));
        }
    }
    // Frame trigger 2: footsteps on the floor polygon's material. Only map
    // collision has materials; the arena's triangles do not.
    let from = match game.step_scan {
        Some((s, a, f)) if (s, a) == key && f <= looped => Some(f),
        _ => None,
    };
    game.step_scan = Some((key.0, key.1, looped));
    if let RoomWorld::Map(map) = &game.world {
        let material = map.floor_ground();
        for &(at, p0) in &clip.steps {
            if from.is_none_or(|f| at > f)
                && at <= looped
                && let Some(id) = game.steps.id(game.step_bank, p0, material)
            {
                game.sound_queue.push((id, "footstep"));
            }
        }
    }
}

/// The map-effect category bit of a ground attribute (the table at
/// 0x1405d4ff0, 0 = no dust).
fn dust_ground_bit(ground: u8) -> u8 {
    const TABLE: [u8; 28] = [
        0, 1, 0, 2, 3, 4, 5, 0, 0, 0, 6, 7, 8, 9, 10, 11, 0, 0, 0, 0, 0, 0, 12, 0, 0, 0, 0, 13,
    ];
    TABLE.get(ground as usize).copied().unwrap_or(0)
}

/// Frame trigger 19 (FUN_140424c60): the map's category element for this
/// ground and foot starts at the foot, turned along Sora's heading.
fn sora_dust(game: &mut Game) {
    let key = (game.action_started, game.player.action);
    let looped = game.player.motion_frame(&game.clips);
    let from = match game.dust_scan {
        Some((s, a, f)) if (s, a) == key && f <= looped => Some(f),
        _ => None,
    };
    game.dust_scan = Some((key.0, key.1, looped));
    let (Some(pax), RoomWorld::Map(map)) = (game.map_pax.clone(), &game.world) else {
        return;
    };
    let bit = dust_ground_bit(map.floor_ground());
    if bit == 0 {
        return;
    }
    let clip = game.player.clip(&game.clips).clone();
    for &(at, p0) in &clip.dust {
        if !(0..32).contains(&p0) || !from.is_none_or(|f| at > f) || at > looped {
            continue;
        }
        let mask = (1u64 << bit) | (1u64 << (32 + p0 as u64));
        let Some(element) = pax.by_category(mask) else {
            continue;
        };
        let heading = to_world([game.player.yaw.sin(), 0., game.player.yaw.cos()]);
        game.fx_queue.push(drive_fx::Placed {
            req: drive_fx::FxRequest {
                pax: pax.clone(),
                id: element.id,
                owner: 0xfb,
            },
            anchor: drive_fx::Anchor::World {
                at: Vec3::from_array(game.player.position),
                pitch: 0.,
                yaw: heading[0].atan2(heading[2]),
            },
        });
    }
}

/// Plays this tick's requests and keeps the low-HP alarm (system sound 9,
/// looped) in step with the gauge's low-HP state: started when HP <= max/4
/// and stopped when HP recovers or Sora is KO (FUN_14017f050).
fn play_sounds(mut game: ResMut<Game>, mut audio: ResMut<Audio>, hud: Option<Res<Hud>>) {
    let tick = game.tick;
    let mut requests = std::mem::take(&mut game.sound_queue);
    // A map effect that ended fades its sound over half a second
    // (FUN_1401de500 with 0x78 units).
    for id in std::mem::take(&mut game.sound_stops) {
        if let Some(output) = &audio.output
            && let Ok(mut mixer) = output.mixer.lock()
        {
            mixer.fade(id, 0., 0.5, audio::FadeEnd::Stop);
        }
    }
    if let Some(hud) = hud {
        let s = hud.stats;
        let low = s.hp > 0 && s.hp <= s.max_hp / 4;
        if low && !audio.alarm {
            requests.push((SE_LOW_HP, "low_hp_alarm"));
        } else if !low && audio.alarm {
            if let Some(output) = &audio.output
                && let Ok(mut mixer) = output.mixer.lock()
            {
                mixer.stop(SE_LOW_HP);
            }
            game.event(serde_json::json!({"event":"sound_stop","tick":tick,"id":SE_LOW_HP}));
        }
        audio.alarm = low;
    }
    // The room's ambient bed (FUN_140154fd0 / FUN_140155380): a bed that
    // is unchanged continues; a changed one fades out over two seconds
    // while the new one starts at once.
    if game.ambient != audio.ambient {
        if let Some(old) = audio.ambient
            && let Some(output) = &audio.output
            && let Ok(mut mixer) = output.mixer.lock()
        {
            mixer.fade(AMBIENT_KEY | old, 0., 2., audio::FadeEnd::Stop);
            game.event(serde_json::json!({"event":"sound_fade","tick":tick,"id":old}));
        }
        audio.ambient = game.ambient;
        if let Some(id) = game.ambient {
            requests.push((id | AMBIENT_KEY, "ambient"));
        }
    }
    for (id, cause) in requests {
        let Audio { output, banks, .. } = &mut *audio;
        // The ambient bed's mixer key carries a flag; its request id does not.
        let request_id = if cause == "ambient" {
            id & !AMBIENT_KEY
        } else {
            id
        };
        let resolved = match banks.request(request_id) {
            Ok(r) => r,
            Err(error) => {
                eprintln!("Sound {request_id} failed to decode: {error}");
                None
            }
        };
        game.event(serde_json::json!({"event":"sound","tick":tick,"id":request_id,"cause":cause,
            "layers":resolved.as_ref().map(|r| r.layers.len()),
            "volume":resolved.as_ref().map(|r| r.volume),"looped":resolved.as_ref().map(|r| r.looped)}));
        if let (Some(output), Some(r)) = (output.as_ref(), resolved)
            && let Ok(mut mixer) = output.mixer.lock()
        {
            for layer in r.layers {
                mixer.play(layer, id, r.volume, 1.);
            }
        }
    }
}

/// HUD frame: test-room debug keys (F4 blow hit, F5/F6 HP -/+2, F7/F8 MP -/+10, F9 drive +10)
/// or the unattended script change stats, then the gauge logic runs.
fn hud_tick(
    keys: Res<ButtonInput<KeyCode>>,
    mut game: ResMut<Game>,
    hud: Option<ResMut<Hud>>,
    gallery: Res<enemy_gallery::EnemyGallery>,
    targeting: Res<enemy_target::Targeting>,
    rig: Option<Res<CameraRig>>,
) {
    if keys.just_pressed(KeyCode::F4) && game.autoplay.is_none() {
        game.debug_blow = true;
    }
    // Test-room aids for Drive forms: F11 refills the gauge
    // (FUN_1403c2020), F12 lets the missing party count as present.
    if game.autoplay.is_none() && game.drive.is_some() {
        if keys.just_pressed(KeyCode::F11) {
            game.player.drive.refill();
        }
        if keys.just_pressed(KeyCode::F12)
            && let Some(d) = game.drive.as_mut()
        {
            d.party = !d.party;
            eprintln!(
                "Drive: stand-in party {}",
                if d.party { "on" } else { "off" }
            );
        }
    }
    let Some(mut hud) = hud else { return };
    let mut s = hud.stats;
    if let Some(rt) = game.enemy.as_mut() {
        // Combat drives HP; G refills it.
        if keys.just_pressed(KeyCode::F6) {
            rt.sora_hp = rt.sora_target.max_hp;
        }
        s.hp = rt.sora_hp;
        s.max_hp = rt.sora_target.max_hp;
    } else if game.autoplay.is_some() {
        match game.tick {
            420 => s.hp = s.max_hp * 7 / 10,
            640 => s.hp = s.max_hp / 5,
            900 => s.hp = s.max_hp,
            _ => {}
        }
    } else {
        if keys.just_pressed(KeyCode::F5) {
            s.hp = (s.hp - 2).max(0);
        }
        if keys.just_pressed(KeyCode::F6) {
            s.hp = (s.hp + 2).min(s.max_hp);
        }
        if keys.just_pressed(KeyCode::F7) {
            game.player.mp.add(-10);
        }
        if keys.just_pressed(KeyCode::F8) {
            game.player.mp.add(10);
        }
        if keys.just_pressed(KeyCode::F9) {
            game.player.drive.add(10);
        }
    }
    // MP is Sora's (casts spend it, MP Charge refills it).
    s.mp = game.player.mp.cur;
    s.max_mp = game.player.mp.max;
    let mp = game.player.mp;
    s.mp_charge = mp.charging().then_some((mp.remaining, mp.total));
    // The Drive gauge is Sora's (hits, damage and orbs fill it).
    let drive = game.player.drive;
    (s.drive, s.drive_level, s.drive_max) = (drive.partial, drive.level, drive.max);
    s.drive_mode = drive.mode;
    (s.form_bars, s.form_time, s.form_max_time) = (drive.bars, drive.time, drive.max_time);
    hud.stats = s;
    let Hud {
        sheets,
        gauge,
        menu,
        enemy_gauge,
        scan,
        slots,
        ..
    } = &mut *hud;
    if let Some(m) = menu {
        m.menu.update(&m.sheet.sequence);
    }
    gauge.update(&sheets.iter().map(|s| &s.sequence).collect::<Vec<_>>(), &s);
    // The Scan gauge follows the lock manager's target (priority 7000, after
    // the player's gauge).
    scan.step(
        enemy_gauge,
        &sheets[0].sequence,
        slots,
        enemy_target::scan_view(&game, &gallery, &targeting),
    );
    // The reticle (priority 4000) follows the held target's TARGET shape,
    // projected through the camera as last applied.
    let camera = rig.as_ref().map(|rig| rig.camera);
    let held = camera.and_then(|_| enemy_target::reticle_view(&game, &gallery, &targeting));
    let Hud {
        reticle,
        field,
        reticle_draw,
        ..
    } = &mut *hud;
    *reticle_draw = reticle.step(
        &field.sequence,
        held,
        &|at| camera.map_or([0.; 3], |camera| camera.project(at)),
        0,
    );
    // Form levels gained this frame queue their panels (FUN_1403edb90).
    if let Some(d) = game.drive.as_mut() {
        for up in std::mem::take(&mut d.level_ups) {
            for e in drive_room::level_panels(&d.src, &up) {
                hud.panels.push(e);
            }
        }
    }
    if game.growth_tour {
        level_growth::tour(&mut game);
    }
    if game.prize_tour || game.badge_tour {
        prize_room::tour(&mut game);
    }
    // Level-ups and bonus levels queue theirs (FUN_1403eda60 / FUN_1403edb20)
    // and the first level of a kill starts the level-up effect
    // (FUN_1403ecf30: system element 15 on Sora).
    if let Some(rt) = game.enemy.as_mut() {
        let waiting = std::mem::take(&mut rt.panels);
        let effect = std::mem::take(&mut rt.level_effect);
        if let Some(d) = game.drive.as_mut() {
            if effect && let Some(p) = d.src.system_pax.clone() {
                d.fx_requests.push(drive_fx::FxRequest {
                    pax: p,
                    id: 15,
                    owner: 0xff,
                });
            }
            for e in level_growth::entries(&d.src, &waiting) {
                hud.panels.push(e);
            }
        }
    }
    let Hud { sheets, panels, .. } = &mut *hud;
    panels.update(&sheets[0].sequence);
    hud.updated = true;
}

#[allow(clippy::too_many_arguments)]
fn hud_present(
    hud: Option<ResMut<Hud>>,
    talk: Option<Res<Talk>>,
    area: Option<Res<AreaRes>>,
    skate: Res<crate::skate::Skate>,
    debug: Res<debug_menu::DebugMenu>,
    gallery: Res<enemy_gallery::EnemyGallery>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut hud) = hud else { return };
    if !std::mem::take(&mut hud.updated) {
        return;
    }
    let Hud {
        sheets,
        gauge,
        enemy_gauge,
        field,
        reticle_draw,
        canvas,
        image,
        menu,
        panels,
        ..
    } = &mut *hud;
    let mut textures: Vec<Texture> = sheets
        .iter()
        .map(|s| Texture {
            rgba: &s.texture_rgba,
            size: s.texture_size.map(|v| v as usize),
            texel_scale: s.texel_scale as f32,
        })
        .collect();
    let field_sheet = textures.len();
    textures.push(Texture {
        rgba: &field.texture_rgba,
        size: field.texture_size.map(|v| v as usize),
        texel_scale: field.texel_scale as f32,
    });
    let mut lock_on = reticle_draw.take();
    if let Some(Draw::Quads { sheet, .. }) = lock_on.as_mut() {
        *sheet = field_sheet;
    }
    // The command menu (priorities 0x14b5..0x14b7) draws before the gauge
    // (7000).
    let mut draws: Vec<Draw> = Vec::new();
    let mut menu_font = None;
    if let Some(m) = menu {
        let first = textures.len();
        menu_font = Some((first + 1, m.font.widths.clone()));
        textures.push(Texture {
            rgba: &m.sheet.texture_rgba,
            size: m.sheet.texture_size.map(|v| v as usize),
            texel_scale: m.sheet.texel_scale as f32,
        });
        textures.push(Texture {
            rgba: &m.font.rgba,
            size: m.font.size.map(|v| v as usize),
            texel_scale: m.font.texel_scale as f32,
        });
        // While riding, Skate 3's score HUD replaces the command menu and
        // the gauge.
        draws = if skate.replaces_hud() {
            Vec::new()
        } else {
            m.menu.draw(
                &m.sheet.sequence,
                &field_command_menu::Font {
                    widths: &m.font.widths,
                    pad: 1,
                },
                first,
                first + 1,
            )
        };
    }
    if !skate.replaces_hud() {
        // The reticle (priority 4000) draws before the command menu.
        if let Some(reticle) = lock_on {
            draws.insert(0, reticle);
        }
        draws.extend(gauge.draw(&sheets.iter().map(|s| &s.sequence).collect::<Vec<_>>()));
        draws.extend(enemy_gauge.draw(&sheets[0].sequence, 0));
        if let Some((font_sheet, widths)) = menu_font.as_ref() {
            draws.extend(panels.draw(
                &sheets[0].sequence,
                &field_command_menu::Font { widths, pad: 1 },
                0,
                *font_sheet,
            ));
        }
    }
    // Message windows (task priority 25000) draw above the HUD.
    if let Some(t) = &talk {
        let first = textures.len();
        textures.push(Texture {
            rgba: &t.sheet.texture_rgba,
            size: t.sheet.texture_size.map(|v| v as usize),
            texel_scale: t.sheet.texel_scale as f32,
        });
        textures.push(Texture {
            rgba: &t.font.rgba,
            size: t.font.size.map(|v| v as usize),
            texel_scale: t.font.texel_scale as f32,
        });
        for w in t.closing.iter().chain(&t.current) {
            draws.extend(w.draw(first, first + 1));
        }
    }
    // Place name banner and the exit guide (system font, white).
    if let (Some(a), Some((sheet, widths))) = (area.as_ref(), menu_font.as_ref())
        && a.transition.is_none()
    {
        let sheet = *sheet;
        let font = field_command_menu::Font { widths, pad: 1 };
        for (text, y) in [a.banner.as_ref().map(|b| &b.0), a.guide.as_ref()]
            .into_iter()
            .zip([192., 384.])
            .filter_map(|(t, y)| t.map(|t| (t, y)))
        {
            let size = 20.;
            let width = field_command_menu::text_width(text, &font, size);
            let origin = [256. - width / 2., y - 24. * size / 18.];
            draws.push(Draw::Quads {
                quads: field_command_menu::text_quads_sized(text, origin, [1.; 4], &font, size),
                z: 0,
                sheet,
            });
        }
    }
    // The skateboard key's notice (system font, white, upper centre).
    if let (Some((text, _)), Some((sheet, widths))) = (skate.notice.as_ref(), menu_font.as_ref()) {
        let font = field_command_menu::Font { widths, pad: 1 };
        let size = 14.;
        let width = field_command_menu::text_width(text, &font, size);
        let origin = [256. - width / 2., 112. - 24. * size / 18.];
        draws.push(Draw::Quads {
            quads: field_command_menu::text_quads_sized(text, origin, [1.; 4], &font, size),
            z: 0,
            sheet: *sheet,
        });
    }
    if !debug.open
        && let Some((sheet, widths)) = menu_font.as_ref()
    {
        let font = field_command_menu::Font { widths, pad: 1 };
        let status = format!(
            "Enemies ({}/{}) - F3: {}",
            gallery.actors.len(),
            enemy_gallery::MAX_ACTORS,
            gallery.status
        );
        let label = pause_menu::encode_text(&status);
        let size =
            12_f32.min(12. * 480. / field_command_menu::text_width(&label, &font, 12.).max(1.));
        draws.push(Draw::Quads {
            quads: field_command_menu::text_quads_sized(&label, [16., 20.], [1.; 4], &font, size),
            z: 0,
            sheet: *sheet,
        });
    }
    // The debug menu (F3) draws over everything.
    if debug.open
        && let Some((sheet, widths)) = menu_font.as_ref()
    {
        let white = textures.len();
        textures.push(Texture {
            rgba: &debug_menu::WHITE,
            size: [1, 1],
            texel_scale: 1.,
        });
        draws.extend(debug_menu::draws(&debug.view, white, *sheet, widths));
    }
    canvas.render(&draws, &textures);
    if let Some(mut target) = images.get_mut(image.id()) {
        target.data = Some(canvas.rgba.clone());
    }
}

/// One tick of the Keyblade's attack windows against the enemy, then the
/// enemy's own tick. Attack spheres: weapon bone 2 entries of the window's
/// collision group, placed through Sora's hand and swept from last frame;
/// target spheres: the enemy's HIT entries on its animated bones.
/// The enemy's TARGET shapes this tick, in simulation space.
fn enemy_targets(rt: &EnemyRuntime) -> Vec<player::ptya::Target> {
    if !rt.enabled {
        return Vec::new();
    }
    let e = &rt.enemy;
    let turn = Quat::from_rotation_y(e.yaw);
    let at = Vec3::from_array(e.position);
    let Some(ec) = rt.clips.0.get(&e.motion) else {
        return Vec::new();
    };
    let Some(bones) = ec
        .bones
        .get((e.motion_tick as usize).min(ec.bones.len().saturating_sub(1)))
    else {
        return Vec::new();
    };
    rt.targets
        .iter()
        .filter_map(|&(b, offset, radius, height)| {
            let m = bones.get(&b)?;
            Some(player::ptya::Target {
                centre: (at + turn * m.transform_point3(offset)).to_array(),
                radius,
                height,
            })
        })
        .collect()
}

fn combat_tick(game: &mut Game, tick: u64) {
    let Game {
        player,
        clips,
        clip_poses,
        poses,
        enemy,
        params,
        world,
        action_started,
        autoplay,
        log,
        sound_queue,
        effect_queue,
        fx_calls,
        magic,
        debug_blow,
        drive,
        cheats,
        sora_attack,
        hero_hit_tick,
        enemy_battle_tour,
        magic_tour,
        magic_shift,
        ..
    } = game;
    let Some(rt) = enemy.as_mut() else { return };
    let debug_blow = std::mem::take(debug_blow)
        || (autoplay.is_some()
            && !*enemy_battle_tour
            && !*magic_tour
            && AUTOPLAY_BLOWS.contains(&tick));
    let act_at_start = rt.enemy.current_act();
    let place = if *magic_tour {
        magic_tour::place(tick, *magic_shift)
    } else {
        tick == 380
    };
    if autoplay.is_some() && !*enemy_battle_tour && place {
        // Unattended runs place the target just ahead of Sora.
        let f = Vec3::new(player.yaw.sin(), 0., player.yaw.cos());
        let distance = if *magic_tour {
            magic_tour::place_distance(tick, *magic_shift)
        } else {
            110.
        };
        let at = Vec3::from_array(player.position) + f * distance;
        rt.enemy.position = at.to_array();
        rt.enemy.spawn = at.to_array();
    }
    let sora_at = Vec3::from_array(player.position);
    let sora_turn = Quat::from_rotation_y(player.yaw);
    let clip = poses[&(player.action, player.armed)];
    let frame = player.motion_frame(clips) as usize;
    let cp = &clip_poses[clip];
    let e = &rt.enemy;
    let targetable = rt.enabled && e.targetable();
    let enemy_turn = Quat::from_rotation_y(e.yaw);
    let enemy_at = Vec3::from_array(e.position);
    let ec = &rt.clips.0[&e.motion];
    let bones = &ec.bones[(e.motion_tick as usize).min(ec.bones.len() - 1)];
    let body: Vec<(Vec3, f32, bool)> = rt
        .body
        .iter()
        .filter_map(|&(b, offset, r, high)| {
            let m = bones.get(&b)?;
            Some((enemy_at + enemy_turn * m.transform_point3(offset), r, high))
        })
        .collect();
    let mut hit = None;
    // Live guard (ATKP Type 2/3) spheres: centre, radius, ATKP.
    let mut guards: Vec<(Vec3, f32, u16, Option<u64>)> = Vec::new();
    // The attack windows this frame: slot 0's Keyblade, slot 1's (Valor,
    // Master and Final), then Sora's own (the drive motions' ATKP 124 /
    // 125 on his object's attack shapes), each with its spheres.
    let mut active: Vec<ActiveVolume> = Vec::new();
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
        for (w, &(start, end, atkp, group)) in hits.iter().enumerate() {
            if (frame as u32) < start || frame as u32 > end {
                continue;
            }
            let spheres = rt.blade[hand]
                .get(&group)
                .into_iter()
                .flatten()
                .map(|&(offset, radius)| {
                    (
                        sora_at + sora_turn * to_model.transform_point3(offset),
                        radius,
                    )
                })
                .collect();
            active.push((
                (*action_started << 7) | (hand as u64) << 5 | w as u64,
                atkp,
                spheres,
            ));
        }
    }
    if let Some(bones) = cp.body.get(frame.min(cp.body.len().saturating_sub(1))) {
        for (w, &(start, end, atkp, group)) in cp.actor_hits.iter().enumerate() {
            if (frame as u32) < start || frame as u32 > end {
                continue;
            }
            let spheres = sora_attack
                .get(&group)
                .into_iter()
                .flatten()
                .filter_map(|&(bone, offset, radius)| {
                    let m = bones.get(&bone)?;
                    Some((sora_at + sora_turn * m.transform_point3(offset), radius))
                })
                .collect();
            active.push(((*action_started << 7) | 2 << 5 | w as u64, atkp, spheres));
        }
    }
    for (instance, atkp, spheres) in active {
        ensure_attack_rows(rt, atkp, 0);
        // Guard types never test bodies (FUN_1403cff40).
        let guard = rt.rows.get(&atkp).is_some_and(|r| matches!(r.kind, 2 | 3));
        // FUN_1403cf400 / FUN_1403cf490: the attack's id and combo group
        // (ATKP ComboGroup, else a fresh one per attack).
        rt.instance_seen.insert(instance, tick);
        let (aid, _) = *rt.instance_ids.entry(instance).or_insert_with(|| {
            let combo = rt
                .steps
                .get(&(atkp, 0))
                .map_or(0, |r| r[0].combo_group as i32);
            let group = if combo != 0 {
                combo
            } else {
                combat::allocate_group(&mut rt.next_group)
            };
            (rt.ids.allocate(), group)
        });
        // FUN_1403d2060: an attack hits a target once per record bit.
        if !guard && rt.enemy.record.has(aid) {
            continue;
        }
        for (i, (centre, radius)) in spheres.into_iter().enumerate() {
            let previous = rt.previous.insert((instance, i), centre);
            if guard {
                guards.push((centre, radius, atkp, None));
                continue;
            }
            if !targetable {
                continue;
            }
            for c in combat::swept(centre.to_array(), previous.map(|p| p.to_array()), radius) {
                if hit.is_none()
                    && let Some(&(target, target_radius, high)) = body
                        .iter()
                        .find(|(b, r, _)| combat::spheres_overlap(c, radius, b.to_array(), *r))
                {
                    let at = (Vec3::from_array(c), target, target_radius);
                    hit = Some((instance, atkp, 0u8, high, at, None::<Vec3>, None::<u64>));
                }
            }
        }
    }
    // Spell volumes (YS::ATTACK_MAGIC): guards join the clash list, damaging
    // volumes are tested against the Shadow's HIT spheres after the weapon
    // (one body hit per target per frame, FUN_1403cff40).
    if let Some(m) = magic.as_mut() {
        let volumes = m.volumes.clone();
        for v in &volumes {
            ensure_attack_rows(rt, v.atkp, v.sub);
        }
        for v in &volumes {
            let centre = Vec3::from_array(v.centre);
            match v.role {
                player::magic::Role::Guard => {
                    guards.push((centre, v.radius, v.atkp, Some(v.key)));
                }
                player::magic::Role::Damage => {
                    if !targetable || hit.is_some() {
                        continue;
                    }
                    let key = magic_room::KEY_BASE | v.key;
                    rt.instance_seen.insert(key, tick);
                    let (aid, _) = *rt.instance_ids.entry(key).or_insert_with(|| {
                        let combo = rt
                            .steps
                            .get(&(v.atkp, v.sub))
                            .map_or(0, |r| r[0].combo_group as i32);
                        let group = if combo != 0 {
                            combo
                        } else {
                            combat::allocate_group(&mut rt.next_group)
                        };
                        (rt.ids.allocate(), group)
                    });
                    if rt.enemy.record.has(aid) {
                        continue;
                    }
                    for c in combat::swept_volume(v.centre, v.previous, v.radius, v.height) {
                        if hit.is_none()
                            && let Some(&(target, target_radius, high)) =
                                body.iter().find(|(b, r, _)| {
                                    combat::volume_hits_sphere(
                                        c,
                                        v.radius,
                                        v.height,
                                        b.to_array(),
                                        *r,
                                    )
                                })
                        {
                            let at = (Vec3::from_array(c), target, target_radius);
                            let moving = v.moving.map(Vec3::from_array);
                            hit = Some((key, v.atkp, v.sub, high, at, moving, Some(v.key)));
                        }
                    }
                }
            }
        }
    }
    // The enemy's record advances once a tick (FUN_1403cf4b0), like Sora's.
    let in_damage_motion = (10..=39).contains(&player.motion_id(clips));
    rt.sora_record.tick(1., in_damage_motion);
    rt.enemy.record.tick(1., false);
    if let Some((instance, atkp, sub, high, (attack_centre, target, target_radius), moving, key)) =
        hit
        && let Some(rows) = rt.steps.get(&(atkp, sub))
        && let Some(&(aid, group)) = rt.instance_ids.get(&instance)
    {
        // The hit direction (FUN_1404373e0): a moving volume's direction,
        // else toward the target from the volume's centre; the weapon uses
        // Sora's facing toward the Shadow.
        let d = match (moving, key) {
            (Some(m), _) => m,
            (None, Some(_)) => enemy_at - attack_centre,
            (None, None) => enemy_at - sora_at,
        } * Vec3::new(1., 0., 1.);
        let dir = d.normalize_or(Vec3::new(player.yaw.sin(), 0., player.yaw.cos()));
        // FUN_1403cf600: record the hit; the consecutive-hit count of its
        // combo group picks the ATKP level row (FUN_1403eb230).
        let cure = matches!(rows[0].kind, 5 | 6);
        let (counted, row) = rt.enemy.record.hit(aid, group, cure, |count| {
            let r = data::battle_tables::step_index(rows, count);
            (r, rows[r].interval)
        });
        let attack = params_of(&rows[row]);
        // The attacker's ability terms (FUN_1403c12f0).
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
        if !counted {
            // Recorded but not counted: no damage and no reaction.
        } else if let Some(damage) =
            rt.enemy
                .hit(dir.to_array(), high, &attack, &rt.sora, rt.difficulty)
        {
            // FUN_1403d12c0: the attacker's obj+0x120 bit 25, which lets
            // the next press chain the combo.
            player.connected = true;
            // FUN_1403a8800: the row's DriveDrain fills the gauge (the
            // Shadow is a ZAKO).
            let (mp_charging, boost) = (player.mp_charging, player.abilities.drive_boost);
            // FUN_1403e86b0: each hit Sora lands is Valor Form EXP.
            if let Some(d) = drive.as_mut()
                && d.active
                && d.form == player::forms::VALOR
            {
                d.pending_exp += 1;
            }
            player
                .drive
                .hit(rows[row].drive_drain, true, mp_charging, boost);
            // ATKP HitSound (FUN_1403d12c0 → FUN_1403dadf0); 500xxx ids
            // resolve to the equipped weapon's bank, 500 for this Keyblade.
            if let Some(&id) = rt.hit_sounds.get(&atkp)
                && id >= 0
            {
                sound_queue.push((id as u32, "hit"));
            }
            // Spark at the contact point on the target's hit shape
            // (FUN_1403d1b40; a sphere here, so height = radius).
            // A spell's row names its own spark.
            let spark = rt
                .hit_effects
                .get(&atkp)
                .copied()
                .or_else(|| key.map(|_| (rows[row].hit_effect, rows[row].hit_effect_random)));
            if let Some((effect, random)) = spark
                && effect != 0
            {
                let v = attack_centre - target;
                let h = target_radius;
                let y = v.y.clamp(-h, h);
                let across = Vec3::new(v.x, 0., v.z).normalize_or_zero()
                    * target_radius
                    * (1. - (y / h) * (y / h)).max(0.).sqrt();
                let contact = target + across + Vec3::Y * y;
                // A spell's spark comes from the spell's own PAX.
                let spell_spark = key.and_then(|k| {
                    magic.as_mut().and_then(|m| {
                        magic_room::spark(m, k, effect, contact.to_array(), dir.to_array())
                    })
                });
                match spell_spark {
                    Some(calls) => fx_calls.extend(calls),
                    None => effect_queue.push(EffectRequest::Hit {
                        effect,
                        random,
                        contact,
                        direction: dir,
                    }),
                }
            }
            if let (Some(k), Some(m)) = (key, magic.as_mut()) {
                m.feedback.hit.push(k);
                // Reaction 5 (Magnet): held at a radius drawn between KB1
                // and KB2 (DAT_140783ca0 LCG), spinning by ATKP +0x10.
                if combat::reaction_class(attack.reaction) == 5
                    && let Some(v) = m.volumes.iter().find(|v| v.key == k)
                {
                    let r = &rows[row];
                    let hold = rt.rng.range(r.knockback[0] as f32, r.knockback[1] as f32);
                    rt.enemy.capture(
                        k,
                        Vec3::from_array(v.centre).to_array(),
                        hold,
                        r.spin as f32,
                    );
                }
            }
            let record = serde_json::json!({"event":"hit","tick":tick,"atkp":atkp,"sub":sub,"damage":damage,
                "step":row,"combo":player.combo.step,"enemy_hp":rt.enemy.hp,"reaction":format!("{:?}", rt.enemy.reaction),
                "act":format!("{:?}", rt.enemy.current_act()),"high":high});
            if let Some(file) = log.as_mut() {
                let _ = serde_json::to_writer(&mut *file, &record);
                let _ = std::io::Write::write_all(file, b"\n");
            }
        }
    }
    let _ = params;
    // The Shadow's claws against Sora's HIT spheres.
    let e = &rt.enemy;
    let ec = &rt.clips.0[&e.motion];
    let tick_in = e.motion_tick;
    let mut sora_hit = None;
    let mut clash = None;
    // FUN_1403d1f40: R = ReflectRange (0: no test); the attacker of the
    // other attack must lie within R° of this attack's owner's facing.
    let cone = |row: &data::battle_tables::Attack, facing: Vec3, v: Vec3| {
        if row.reflect_range == 0 {
            return true;
        }
        let turn = Quat::from_rotation_y((row.reflect_angle as f32 * 2.).to_radians());
        let f = turn * facing;
        v.dot(f) < -(row.reflect_range as f32).to_radians().cos()
    };
    let sora_facing = Vec3::new(player.yaw.sin(), 0., player.yaw.cos());
    let shadow_facing = Vec3::new(e.yaw.sin(), 0., e.yaw.cos());
    let from_shadow = ((sora_at - enemy_at) * Vec3::new(1., 0., 1.)).normalize_or_zero();
    let clashes = |guard: u16, claw: u16| {
        let (Some(g), Some(c)) = (rt.rows.get(&guard), rt.rows.get(&claw)) else {
            return false;
        };
        // DAT_1405c5e98: the symmetric clash pairs 0-0, 0-2, 0-3, 1-3.
        let pair = (g.kind.min(c.kind), g.kind.max(c.kind));
        matches!(pair, (0, 0) | (0, 2) | (0, 3) | (1, 3))
            && cone(g, sora_facing, from_shadow)
            && cone(c, shadow_facing, -from_shadow)
    };
    if rt.enabled && e.alive() && !e.sunk {
        let bones = &ec.bones[(tick_in as usize).min(ec.bones.len() - 1)];
        let sora_bones = &cp.body[frame.min(cp.body.len().saturating_sub(1))];
        let sora_spheres: Vec<(Vec3, f32, bool)> = rt
            .sora_body
            .iter()
            .filter_map(|&(b, offset, r, high)| {
                let m = sora_bones.get(&b)?;
                Some((sora_at + sora_turn * m.transform_point3(offset), r, high))
            })
            .collect();
        for (w, &(start, end, atkp, group)) in ec.windows.iter().enumerate() {
            if tick_in < start || tick_in > end {
                continue;
            }
            let instance = (1 << 63) | ((e.motion_serial as u64) << 4) | w as u64;
            rt.instance_seen.insert(instance, tick);
            let (aid, _) = *rt.instance_ids.entry(instance).or_insert_with(|| {
                let combo = rt
                    .steps
                    .get(&(atkp, 0))
                    .map_or(0, |r| r[0].combo_group as i32);
                let group = if combo != 0 {
                    combo
                } else {
                    combat::allocate_group(&mut rt.next_group)
                };
                (rt.ids.allocate(), group)
            });
            if rt.sora_record.has(aid) {
                continue;
            }
            for (i, &(b, offset, radius)) in rt.claws.get(&group).into_iter().flatten().enumerate()
            {
                let Some(m) = bones.get(&b) else { continue };
                let centre = enemy_at + enemy_turn * m.transform_point3(offset);
                let previous = rt.claw_previous.insert((instance, i), centre);
                for c in combat::swept(centre.to_array(), previous.map(|p| p.to_array()), radius) {
                    // Attack-vs-attack clashes run before the body pass.
                    if clash.is_none()
                        && let Some(&(g, gr, guard, gkey)) =
                            guards.iter().find(|(g, gr, guard, _)| {
                                combat::spheres_overlap(c, radius, g.to_array(), *gr)
                                    && clashes(*guard, atkp)
                            })
                    {
                        clash = Some((instance, atkp, guard, Vec3::from_array(c), g, gr, gkey));
                    }
                    if sora_hit.is_none()
                        && clash.is_none_or(|k| k.0 != instance)
                        && let Some(&(body, body_radius, high)) = sora_spheres
                            .iter()
                            .find(|(b, r, _)| combat::spheres_overlap(c, radius, b.to_array(), *r))
                    {
                        sora_hit = Some((
                            instance,
                            atkp,
                            high,
                            (Vec3::from_array(c), body, body_radius),
                        ));
                    }
                }
            }
        }
    }
    // A guarded claw: FUN_1403d2140 both ways, Sora's FUN_1403d61d0 and the
    // Shadow's FUN_140411060.
    if let Some((instance, claw, guard, claw_at, g, gr, gkey)) = clash
        && let (Some(&gr_row), Some(&claw_row)) = (rt.rows.get(&guard), rt.rows.get(&claw))
    {
        // attack_get_reflect_count (+0xb0): each clash bumps the guard's
        // counter, which Reflect's counter tiers read.
        if let (Some(k), Some(m)) = (gkey, magic.as_mut()) {
            *m.reflected.entry(k).or_insert(0) += 1;
        }
        if let Some(&(aid, _)) = rt.instance_ids.get(&instance) {
            rt.sora_record.mark(aid);
        }
        // FUN_1403d61d0 first sets obj+0x120 bit 25 ("connected").
        player.connected = true;
        // FUN_1403d0f00: at the guard sphere's surface toward the claw.
        let contact = g + (claw_at - g).normalize_or_zero() * gr;
        let (s, t) = (gr_row.refact_self, claw_row.refact_other);
        let outcome = if s == 2 || t == 2 {
            effect_queue.push(EffectRequest::System {
                id: 13,
                at: contact,
            });
            "pass"
        } else if !player.action.airborne() && (s != 0 || t != 0) {
            // Sora is the player character (0x6c8 bit 7): effect 11.
            effect_queue.push(EffectRequest::System {
                id: 11,
                at: contact,
            });
            if player
                .guard_recoil([from_shadow.x, from_shadow.z], clips)
                .is_some()
            {
                *action_started = tick;
            }
            rt.sora_invulnerable = 80.;
            "guard"
        } else {
            // Bounce (motion 41 + slide 50) is not loaded in this room.
            effect_queue.push(EffectRequest::System {
                id: 13,
                at: contact,
            });
            "bounce"
        };
        if claw_row.refact_self != 2 && claw_row.reflect_motion != 0 {
            rt.enemy.reflect(
                claw_row.reflect_motion as usize,
                claw_row.reflect_hit_back as f32,
            );
        }
        let record = serde_json::json!({"event":"guard","tick":tick,"outcome":outcome,
            "guard_atkp":guard,"claw_atkp":claw,"reflect_motion":claw_row.reflect_motion,
            "contact":contact.to_array()});
        if let Some(file) = log.as_mut() {
            let _ = serde_json::to_writer(&mut *file, &record);
            let _ = std::io::Write::write_all(file, b"\n");
        }
    }
    rt.sora_invulnerable = (rt.sora_invulnerable - 1.).max(0.);
    if debug_blow && rt.sora_hp > 0 {
        // Reaction 4 with KB1 and KB2 both 0 (the SSTM blow), away from
        // the enemy; no damage.
        let d = ((sora_at - enemy_at) * Vec3::new(1., 0., 1.)).normalize_or(Vec3::Z);
        let hit = player::Hit {
            reaction: 4,
            original: 4,
            direction: [d.x, d.z],
            high: true,
            knockback: [0, 0],
        };
        let started = player.react(&hit, &rt.enemy.sstm, clips);
        if started.is_some() {
            *action_started = tick;
        }
        let record = serde_json::json!({"event":"sora_blow_debug","tick":tick,
            "reaction":format!("{started:?}")});
        if let Some(file) = log.as_mut() {
            let _ = serde_json::to_writer(&mut *file, &record);
            let _ = std::io::Write::write_all(file, b"\n");
        }
    }
    // Aerial Recovery's range 27 (obj+0x18c bit 22) takes no damage.
    if player.range_invulnerable(clips) || cheats.invincible {
        rt.sora_invulnerable = rt.sora_invulnerable.max(1.);
    }
    if let Some((instance, atkp, _, _)) = sora_hit
        && rt.sora_invulnerable > 0.
    {
        // FUN_1403d65a0: no damage while +0xd70 runs (the hit is still
        // recorded).
        if let Some(&(aid, group)) = rt.instance_ids.get(&instance) {
            rt.sora_record.hit(aid, group, false, |_| (0, 0));
        }
        let record = serde_json::json!({"event":"sora_hit_invulnerable","tick":tick,"atkp":atkp});
        if let Some(file) = log.as_mut() {
            let _ = serde_json::to_writer(&mut *file, &record);
            let _ = std::io::Write::write_all(file, b"\n");
        }
    } else if let Some((instance, atkp, high, (claw_at, body_at, body_radius))) = sora_hit
        && let Some(rows) = rt.steps.get(&(atkp, 0))
        && let Some(&(aid, group)) = rt.instance_ids.get(&instance)
        && rt.sora_hp > 0
    {
        // FUN_1403cf600 on Sora's record (obj+0xc98): a player-type object
        // locks out other attack groups for 30 ticks.
        let cure = matches!(rows[0].kind, 5 | 6);
        let (counted, row) = rt.sora_record.hit(aid, group, cure, |count| {
            let r = data::battle_tables::step_index(rows, count);
            (r, rows[r].interval)
        });
        let attack = params_of(&rows[row]);
        if let Some(&id) = rt.hit_sounds.get(&atkp)
            && id >= 0
        {
            sound_queue.push((id as u32, "sora_hit"));
        }
        // The claw's spark on Sora's hit shape, from the Shadow's own
        // effects (FUN_1403d1b40; a sphere here, so height = radius).
        if counted && rows[row].hit_effect != 0 {
            let v = claw_at - body_at;
            let y = v.y.clamp(-body_radius, body_radius);
            let across = Vec3::new(v.x, 0., v.z).normalize_or_zero()
                * body_radius
                * (1. - (y / body_radius).powi(2)).max(0.).sqrt();
            let heading = (sora_at - enemy_at) * Vec3::new(1., 0., 1.);
            effect_queue.push(EffectRequest::EnemyHit {
                object: 0,
                effect: rows[row].hit_effect,
                random: rows[row].hit_effect_random,
                contact: body_at + across + Vec3::Y * y,
                direction: heading.normalize_or(Vec3::Z),
            });
        }
        let ab = &player.abilities;
        let state = combat::TargetState {
            hp: rt.sora_hp,
            player_type: true,
            defender: ab.defender as i32,
            damage_control: ab.damage_control,
            damage_control_ratio: 50,
            difficulty: rt.difficulty,
        };
        if counted {
            *hero_hit_tick = Some(tick);
        }
        let damage = if counted {
            combat::damage_with(&rt.shadow_attacker, &attack, &rt.sora_target, &state)
        } else {
            0
        };
        // FUN_1403d6670: Second Chance (ability 127) and Once More (128).
        let veto = combat::sora_veto(ab.has(127), ab.has(128), rt.sora_hp, 0, in_damage_motion);
        rt.sora_hp = combat::apply_damage(
            rt.sora_hp,
            0,
            rt.sora_target.max_hp,
            damage,
            veto && counted,
        );

        let d = (sora_at - enemy_at) * Vec3::new(1., 0., 1.);
        let dir = d.normalize_or(Vec3::Z);
        // FUN_1403d2fe0: the reaction class picks the motion and physics
        // (flinch and slide, air flinch and hop, blow).
        let hit = player::Hit {
            reaction: attack.reaction,
            original: attack.reaction,
            direction: [dir.x, dir.z],
            high,
            knockback: attack.knockback,
        };
        let started = if counted {
            player.react(&hit, &rt.enemy.sstm, clips)
        } else {
            None
        };
        if started.is_some() {
            *action_started = tick;
        }
        // Damage Drive (FUN_1403a85f0): damage taken fills the gauge.
        let damage_drive = player.abilities.damage_drive;
        player
            .drive
            .damaged(damage, rt.sora_target.max_hp, damage_drive);
        // MP Rage (FUN_1403d5e50): damage taken outside MP Charge gives
        // MP by its share of max HP × the ability's value.
        if damage > 0 && !player.mp.charging() && player.abilities.mp_rage > 0. {
            let gain =
                (damage as f32 / rt.sora_target.max_hp as f32 * player.abilities.mp_rage) as i32;
            if gain > 0 {
                player.mp.add(gain);
            }
        }
        // FUN_140404d60: HP 0 plays DEAD.
        if rt.sora_hp == 0 && player.die(clips).is_some() {
            *action_started = tick;
        }
        let record = serde_json::json!({"event":"sora_hit","tick":tick,"atkp":atkp,"damage":damage,
            "counted":counted,"step":row,"sora_hp":rt.sora_hp,"reaction":format!("{started:?}")});
        if let Some(file) = log.as_mut() {
            let _ = serde_json::to_writer(&mut *file, &record);
            let _ = std::io::Write::write_all(file, b"\n");
        }
    }
    if rt.sora_hp == 0 {
        rt.ko_ticks += 1;
        if rt.ko_ticks >= 120 {
            // [test-room] No game-over flow: stand up with full HP.
            rt.sora_hp = rt.sora_target.max_hp;
            rt.ko_ticks = 0;
        }
    }
    // Healed while dead (the camp, a Cure): FUN_1403c3c30 stands him up.
    if player.dead() && rt.sora_hp > 0 {
        player.revive();
        *action_started = tick;
    }
    // A Magnet's hold follows its volume and ends with it.
    if let Some(h) = rt.enemy.held {
        match magic
            .as_ref()
            .and_then(|m| m.volumes.iter().find(|v| v.key == h.key))
        {
            Some(v) => {
                if let Some(held) = rt.enemy.held.as_mut() {
                    held.centre = v.centre;
                }
            }
            None => rt.enemy.release_held(),
        }
    }
    let before = rt.enemy.current_act();
    if rt.enabled && !cheats.enemy_frozen {
        rt.enemy.tick(
            &mut rt.rng,
            &rt.clips,
            &RoomFloor(world),
            player.position,
            player.position,
        );
    }
    // The enemy motion's frame-8 sounds reached this tick.
    let e = &rt.enemy;
    let from = match rt.sound_tick {
        Some((serial, t)) if serial == e.motion_serial && t <= e.motion_tick => Some(t),
        _ => None,
    };
    if !e.gone
        && let Some(c) = rt.clips.0.get(&e.motion)
    {
        for &(at, id) in &c.sounds {
            if from.is_none_or(|t| at > t) && at <= e.motion_tick {
                sound_queue.push((id, "enemy"));
            }
        }
    }
    rt.sound_tick = Some((e.motion_serial, e.motion_tick));
    let after = rt.enemy.current_act();
    if act_at_start != after && after == Some(shadow::ActKind::Dead) {
        effect_queue.push(EffectRequest::EnemyDead);
    }
    if before != after
        && let Some(file) = log.as_mut()
    {
        let record = serde_json::json!({"event":"enemy_act","tick":tick,"act":format!("{after:?}"),
            "motion":rt.enemy.motion,"position":rt.enemy.position});
        let _ = serde_json::to_writer(&mut *file, &record);
        let _ = std::io::Write::write_all(file, b"\n");
    }
}

/// Places the spell-volume placeholders on this tick's volumes.
fn present_spells(
    game: Res<Game>,
    mut volumes: Query<(
        &SpellVolume,
        &mut Transform,
        &mut Visibility,
        &MeshMaterial3d<StandardMaterial>,
    )>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let live: Vec<_> = game
        .magic
        .as_ref()
        .map(|m| m.volumes.clone())
        .unwrap_or_default();
    let shown = game.show_collision;
    for (slot, mut transform, mut visibility, material) in &mut volumes {
        match live.get(slot.0).filter(|_| shown) {
            Some(v) => {
                *visibility = Visibility::Visible;
                transform.translation = Vec3::from_array(v.centre);
                // Horizontal radius r, vertical half-extent h.
                transform.scale = Vec3::new(v.radius, v.height, v.radius);
                if let Some(mut m) = materials.get_mut(&material.0) {
                    let c = spell_tint(v.atkp);
                    m.base_color = Color::srgba(c[0], c[1], c[2], c[3]);
                }
            }
            None => *visibility = Visibility::Hidden,
        }
    }
}

fn present_enemy(
    game: Res<Game>,
    mut roots: Query<(&mut Transform, &mut Visibility), With<EnemyRoot>>,
    parts: Query<(&EnemyPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Some(rt) = &game.enemy else { return };
    let e = &rt.enemy;
    if let Ok((mut t, mut v)) = roots.single_mut() {
        t.translation = Vec3::from_array(e.position);
        t.rotation = Quat::from_rotation_y(e.yaw);
        t.scale = Vec3::new(1., e.scale_y.max(0.01), 1.);
        // Alpha fades are shown as visible/hidden at half alpha.
        *v = if !rt.enabled || e.gone || e.alpha < 0.5 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
    let clip = &rt.clips.0[&e.motion];
    let pose = &clip.frames[(e.motion_tick as usize).min(clip.frames.len() - 1)];
    for (part, handle) in &parts {
        let positions = rt.indices[part.0]
            .iter()
            .map(|&i| pose[i as usize].to_array());
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            visual::room::update_positions(&mut mesh, positions);
        }
    }
}

/// The weapon root, distinct from Sora's root transform.
type WeaponOnly = (With<WeaponRoot>, Without<SoraRoot>);
/// The camera never shares a transform with Sora's hierarchy.
type CameraOnly = (
    With<Camera3d>,
    Without<SoraRoot>,
    Without<WeaponRoot>,
    Without<SnapshotCamera>,
    Without<MenuCamera>,
);

fn camera_control(
    motion: Res<AccumulatedMouseMotion>,
    mut rig: ResMut<CameraRig>,
    mut camera: Query<(&mut Transform, &mut Projection), CameraOnly>,
    talk: Option<Res<Talk>>,
    settings: Res<settings_room::SettingsRes>,
) {
    // Mouse-look turns the camera (the PC layer feeds mouse motion to the
    // right stick; its sensitivity is unresolved): 0.1 at Camera Speed 5.
    let speed = settings.current.camera_factor();
    rig.pending_turn = (rig.pending_turn + motion.delta.x * speed).clamp(-30., 30.);
    let (eye, look_at, fov) = match talk.as_ref().and_then(|t| t.camera) {
        // The talk camera (FUN_140175e50): 60° in the game's horizontal
        // 4:3 sense, converted like the follow camera's.
        Some((eye, target)) => (
            eye,
            target,
            2. * (240. / (320. / (TALK_FOV / 2.).tan())).atan(),
        ),
        None => (
            from_world(rig.camera.eye),
            from_world(rig.camera.look_at),
            rig.camera.vertical_fov(),
        ),
    };
    if let Ok((mut transform, mut projection)) = camera.single_mut() {
        *transform = Transform::from_translation(eye).looking_at(look_at, Vec3::Y);
        if let Projection::Perspective(p) = projection.as_mut()
            && p.fov != fov
        {
            p.fov = fov;
        }
    }
}

/// The skateboard's fixed step: start a ride on the room's collision, step
/// the Skate 3 skater and put Sora where it is. An area change ends the ride.
fn skate_tick(
    mut skate: ResMut<crate::skate::Skate>,
    mut game: ResMut<Game>,
    area: Option<Res<AreaRes>>,
    audio: Option<Res<Audio>>,
) {
    // Skate 3's riding sounds play through the room's mixer.
    if let Some(change) = skate.take_stream_change()
        && let Some(output) = audio.as_ref().and_then(|a| a.output.as_ref())
        && let Ok(mut mixer) = output.mixer.lock()
    {
        mixer.set_stream(change);
    }
    if area.as_deref().is_some_and(|a| a.transition.is_some()) {
        if skate.riding() {
            skate.stop();
        }
        return;
    }
    if skate.wants_start() {
        let triangles = game.world.skate_triangles();
        let position = Vec3::from_array(game.player.position);
        skate.start(&triangles, position, game.player.yaw);
    }
    if let Some(ride) = skate.step() {
        game.player.position = ride.position.to_array();
        game.player.yaw = ride.yaw;
    }
}

/// Riding uses Skate 3's own gameplay camera in place of the follow camera.
fn skate_camera(
    mut skate: ResMut<crate::skate::Skate>,
    mut camera: Query<(&mut Transform, &mut Projection), CameraOnly>,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    if let Projection::Perspective(p) = projection.as_mut() {
        skate.aspect = p.aspect_ratio;
        if let Some((view, fov)) = skate.camera() {
            *transform = view;
            p.fov = fov;
        }
    }
}

#[derive(Component)]
struct SkateBoard;

/// The optional skateboard's presentation (the Skate 3 score HUD).
struct SkatePlugins;

impl Plugin for SkatePlugins {
    fn build(&self, _app: &mut App) {
        #[cfg(feature = "skate3")]
        _app.add_plugins(crate::skate_hud::SkateHudPlugin);
    }
}

type BoardOnly = (With<SkateBoard>, Without<SoraRoot>);

/// Sora's skinning and the rider retarget built on his bind pose.
#[derive(Resource)]
struct SoraRig {
    skinning: data::model_pose::Skinning,
    rig: Option<crate::skate_rig::Rig>,
}

/// The skateboard's models: the equipped Keyblade's, read from the install
/// the first time it is ridden.
#[derive(Resource)]
struct BoardModels {
    keyblades: crate::keyblade_board::Keyblades,
    /// The room's own weapon model (the hand Keyblade's meshes serve it).
    hand: String,
    source: Arc<ProbeSource>,
    /// The Keyblades' shared idle motion (W_EX010.mset motion 0: every
    /// Keyblade model on the install uses that motion set), which swings the
    /// keychain.
    motion: Option<motion::SelectedMotion>,
    /// Model name -> the board model (None: unreadable).
    models: HashMap<String, Option<BoardModel>>,
    /// The model the spawned board shows.
    shown: Option<String>,
}

/// A Keyblade as the board: its model and its vertices for each 60 Hz frame
/// of the idle motion (one bind-pose frame without a usable motion).
struct BoardModel {
    model: model_inspection::InspectionModel,
    frames: Vec<Vec<Vec3>>,
    loop_start: usize,
    /// Its placement on the board frame and how far that raised it.
    placement: Transform,
    lift: f32,
}

/// The rider bone the board is drawn on (skate-sim's skeleton order).
const BOARD_BONE: usize = 25;

impl BoardModel {
    fn frame(&self, tick: u64) -> &[Vec3] {
        let n = self.frames.len() as u64;
        let i = if tick < n {
            tick
        } else {
            let start = (self.loop_start as u64).min(n - 1);
            start + (tick - start) % (n - start)
        };
        &self.frames[i as usize]
    }
}

/// One mesh of the board's Keyblade (its model part ordinal).
#[derive(Component)]
struct BoardPart(usize);

/// `obj/<name>.mdlx` from whichever index holds it, with its vertices in
/// its root joint's space (as the hand Keyblade's bind pose).
fn read_board_model(
    source: &ProbeSource,
    name: &str,
    idle: Option<&motion::SelectedMotion>,
) -> Result<BoardModel, Box<dyn std::error::Error>> {
    let hash = data::name_hash::of_path(&format!("obj/{name}.mdlx"));
    for (index, _) in validation::required_files()
        .into_iter()
        .filter(|(n, _)| n.ends_with(".hed"))
    {
        let checked = source.open_index(&index)?;
        let Some(ordinal) = checked
            .inventory
            .entries
            .iter()
            .position(|e| e.name_hash == hash)
        else {
            continue;
        };
        drop(checked);
        let mut models = model_inspection::read_model_at(source, &index, ordinal, hash)?;
        if models.is_empty() {
            break;
        }
        let model = models.remove(0);
        let pose = model.poses.first().ok_or("model pose absent")?;
        let root = pose.matrix_bits.first().map_or(Mat4::IDENTITY, |m| {
            Mat4::from_cols_array(&m.map(f32::from_bits))
        });
        let inverse = root.inverse();
        let points = pose
            .position_bits
            .iter()
            .map(|p| inverse.transform_point3(Vec3::from_array(p.map(f32::from_bits))))
            .collect();
        // The idle motion's frames, as the hand Keyblade's (its posed
        // vertices are in the grip joint's space).
        let animated = idle.and_then(|idle| {
            let (clip, key_start) = clip_for(idle).ok()?;
            let frames = (0..=clip.end)
                .map(|offset| {
                    let (pose, _) = model
                        .skinning
                        .sample_evaluated(&idle.motion, (key_start + offset) as f32)
                        .ok()?;
                    Some(
                        pose.position_bits
                            .iter()
                            .map(|p| Vec3::from_array(p.map(f32::from_bits)))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Option<Vec<_>>>()?;
            Some((frames, clip.loop_start as usize))
        });
        let (frames, loop_start) = animated.unwrap_or_else(|| {
            eprintln!("Keyblade board {name}: no idle motion, shown still");
            (vec![points], 0)
        });
        let (placement, lift) = crate::keyblade_board::deck_transform(&frames[0]);
        return Ok(BoardModel {
            model,
            frames,
            loop_start,
            placement,
            lift,
        });
    }
    Err(format!("obj/{name}.mdlx is not in the install").into())
}

/// Sora rides the Skate 3 rider's pose, and the Kingdom Key follows the
/// deck as the board.
// Bevy systems take each resource as a parameter.
#[allow(clippy::too_many_arguments)]
fn skate_present(
    mut commands: Commands,
    skate: Res<crate::skate::Skate>,
    game: Res<Game>,
    sora: Option<Res<SoraRig>>,
    mut roots: Query<&mut Transform, (With<SoraRoot>, Without<SkateBoard>)>,
    mut boards: Query<(Entity, &mut Transform), BoardOnly>,
    parts: Query<(&SoraPart, &Mesh3d)>,
    keyblade: Query<(&Mesh3d, &MeshMaterial3d<FogMaterial>), With<WeaponPart>>,
    mut hand: Query<&mut Visibility, (With<WeaponRoot>, Without<SkateBoard>)>,
    camp: Option<Res<CampRes>>,
    mut board_models: ResMut<BoardModels>,
    board_parts: Query<(&BoardPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut fog_materials: ResMut<Assets<FogMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(physics_deck) = skate.deck() else {
        for (entity, _) in &boards {
            commands.entity(entity).despawn();
        }
        board_models.shown = None;
        return;
    };
    // The equipped Keyblade is the board; an equipment change mid-ride
    // swaps it.
    let wanted = camp
        .as_deref()
        .and_then(|c| board_models.keyblades.model(c.equipment.weapon))
        .unwrap_or(&board_models.hand)
        .to_owned();
    if board_models.shown.as_deref() != Some(wanted.as_str()) {
        for (entity, _) in &boards {
            commands.entity(entity).despawn();
        }
    }
    // The Keyblade is the board, so it leaves Sora's hand.
    if let Ok(mut visibility) = hand.single_mut() {
        *visibility = Visibility::Hidden;
    }
    // The board is drawn where Skate 3 draws it: on the rider's animated
    // board bone (SKATEBOARD_ROOT, 25), which carries flips and shuvits; at
    // rest it coincides with the physics deck.
    let deck = skate
        .rider()
        .and_then(|(root, bones, rest)| {
            let (now, rest) = (bones.get(BOARD_BONE)?, rest.get(BOARD_BONE)?);
            let turn = now.to_scale_rotation_translation().1
                * rest.to_scale_rotation_translation().1.inverse();
            Some(Transform {
                translation: root.transform_point(now.w_axis.truncate()),
                rotation: (root.rotation * turn).normalize(),
                scale: Vec3::ONE,
            })
        })
        .unwrap_or(physics_deck);
    // How far the board was raised to clear the ground: Sora rises with it.
    let lift = board_models
        .models
        .get(&wanted)
        .and_then(Option::as_ref)
        .map_or(0., |b| b.lift);
    // With the rider retarget Sora takes the rider's root and pose;
    // otherwise he stands sideways on the deck in his current pose.
    let posed = sora.as_deref().and_then(|s| {
        let (root, bones, rest) = skate.rider()?;
        let matrices = s.rig.as_ref()?.pose(bones, rest)?;
        let rows: Vec<[[f32; 4]; 4]> = matrices.iter().map(Mat4::to_cols_array_2d).collect();
        let positions = s.skinning.positions_with(&rows).ok()?;
        Some((*root, positions))
    });
    if let Ok(mut root) = roots.single_mut() {
        match &posed {
            Some((rider, _)) => {
                *root = *rider;
                root.translation += deck.rotation * Vec3::Y * lift;
            }
            None => {
                root.translation =
                    deck.translation + deck.rotation * Vec3::Y * crate::skate::DECK_TOP;
                root.rotation = deck.rotation * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
            }
        }
    }
    if let Some((_, positions)) = &posed {
        for (part, handle) in &parts {
            let part_positions: Vec<[f32; 3]> = game.part_indices[part.0]
                .iter()
                .filter_map(|&i| positions.get(i as usize).copied())
                .collect();
            if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, part_positions);
            }
        }
    }
    if board_models.shown.as_deref() == Some(wanted.as_str())
        && let Ok((_, mut transform)) = boards.single_mut()
    {
        *transform = deck;
        // The keychain: this tick's idle-motion frame.
        if let Some(Some(b)) = board_models.models.get(&wanted)
            && b.frames.len() > 1
        {
            let frame = b.frame(game.tick);
            for (part, handle) in &board_parts {
                let positions: Vec<[f32; 3]> = b.model.parts[part.0]
                    .indices
                    .iter()
                    .filter_map(|&i| frame.get(i as usize).map(|v| v.to_array()))
                    .collect();
                if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
                }
            }
        }
        return;
    }
    let board = commands
        .spawn((SkateBoard, deck, Visibility::default()))
        .id();
    if !board_models.models.contains_key(&wanted) {
        let model = read_board_model(&board_models.source, &wanted, board_models.motion.as_ref())
            .inspect_err(|e| eprintln!("Keyblade board {wanted}: {e}"))
            .ok();
        board_models.models.insert(wanted.clone(), model);
    }
    match board_models.models.get(&wanted).and_then(Option::as_ref) {
        // The Keyblade's own model, its keychain swinging with the idle
        // motion (placed by the first frame).
        Some(b) => {
            let blade = commands
                .spawn((b.placement, Visibility::default(), ChildOf(board)))
                .id();
            for (ordinal, entity) in spawn_model(
                &mut commands,
                &b.model,
                b.frame(game.tick),
                blade,
                &mut meshes,
                &mut fog_materials,
                &mut images,
            ) {
                if let Some(entity) = entity {
                    commands.entity(entity).insert(BoardPart(ordinal));
                }
            }
        }
        // The room's own Keyblade: its hand meshes.
        None if !keyblade.is_empty() => {
            let blade = commands
                .spawn((
                    crate::keyblade_board::deck_transform(&game.weapon_display).0,
                    Visibility::default(),
                    ChildOf(board),
                ))
                .id();
            for (mesh, material) in &keyblade {
                commands.spawn((mesh.clone(), material.clone(), ChildOf(blade)));
            }
        }
        None => {
            let [w, h, l] = crate::skate::DECK_SIZE;
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(w, h, l))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.12, 0.12, 0.13),
                    perceptual_roughness: 0.9,
                    ..default()
                })),
                ChildOf(board),
            ));
        }
    }
    board_models.shown = Some(wanted);
}

fn present(
    fixed: Res<Time<Fixed>>,
    mut game: ResMut<Game>,
    mut roots: Query<&mut Transform, (With<SoraRoot>, Without<WeaponRoot>)>,
    mut hands: Query<(&mut Transform, &mut Visibility), WeaponOnly>,
    parts: Query<(&SoraPart, &Mesh3d)>,
    weapon_parts: Query<(&WeaponPart, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let action = game.player.action;
    let started = game.action_started;
    let previous = game.shown_action;
    if game.shown_action != Some((action, started)) {
        if game.shown_action.is_some() {
            game.blend_from = Some((game.display.clone(), game.display_hand, game.action_started));
            game.blend_from_joints = std::mem::take(&mut game.display_joints);
        }
        game.shown_action = Some((action, started));
    }
    let frame = game.player.motion_frame(&game.clips) as usize;
    let clip = game.poses[&(action, game.player.armed)];
    let skin = game.clip_poses[clip].skin.clone();
    let target_joints: Option<Vec<data::motion_eval::World>> = {
        let joints = &game.clip_poses[clip].joints;
        joints
            .get(frame.min(joints.len().saturating_sub(1)))
            .cloned()
    };
    let target = &game.clip_poses[clip].frames;
    let target = &target[frame.min(target.len() - 1)];
    // Unattended runs log how far each switch of action moves the shown
    // pose when it is not blended (the first frame of the new motion
    // against the pose left behind).
    let switched = if game.autoplay.is_some()
        && let Some((from, _)) = previous
        && previous != Some((action, started))
        && game.display.len() == target.len()
    {
        let jump = target
            .iter()
            .zip(&game.display)
            .map(|(a, b)| a.distance(*b))
            .fold(0f32, f32::max);
        Some((format!("{from:?}"), jump, game.display_frame))
    } else {
        None
    };
    let mut display = target.clone();
    let weapons = &game.clip_poses[clip].weapon;
    let mut hand = Transform::from_matrix(weapons[frame.min(weapons.len() - 1)]);
    // Crossfade measured in simulation ticks (plus the fixed-step overstep),
    // applied to the body and the Keyblade alike. The reference starts a
    // blend's clock at 2 and counts one tick before the first shown frame
    // (FUN_1403c8a40, FUN_1403c6740), and takes durations of 2 or less as no
    // blend; the weight is that clock over the duration.
    let mut shown_joints = target_joints.clone().unwrap_or_default();
    if let Some((from, from_hand, start)) = &game.blend_from {
        let elapsed = (game.tick - start) as f32 + fixed.overstep_fraction();
        let blend = game.player.blend;
        let weight = if blend > 2. {
            ((elapsed + 3.) / blend).clamp(0., 1.)
        } else {
            1.
        };
        if weight >= 1. {
            game.blend_from = None;
            game.blend_from_joints.clear();
        } else if let (Some(skin), Some(to)) = (&skin, &target_joints)
            && game.blend_from_joints.len() == to.len()
            && let Ok(pose) = {
                shown_joints = data::motion_eval::blend_poses(
                    &skin.parents,
                    &game.blend_from_joints,
                    to,
                    weight,
                );
                skin.skinning.pose_from_worlds(&shown_joints)
            }
        {
            // Joint by joint, as the reference blends (FUN_1401c3970): the
            // body's joints turn between the two poses instead of the
            // skinned points sliding past each other.
            display = pose
                .position_bits
                .iter()
                .map(|p| Vec3::from_array(p.map(f32::from_bits)))
                .collect();
            if let Some(m) = pose.matrix_bits.get(skin.weapon_joint) {
                hand = Transform::from_matrix(Mat4::from_cols_array(&m.map(f32::from_bits)));
            }
        } else {
            for (d, f) in display.iter_mut().zip(from) {
                *d = f.lerp(*d, weight);
            }
            hand = Transform {
                translation: from_hand.translation.lerp(hand.translation, weight),
                rotation: from_hand.rotation.slerp(hand.rotation, weight),
                scale: from_hand.scale.lerp(hand.scale, weight),
            };
            shown_joints.clear();
        }
    }
    game.display_joints = shown_joints;
    game.display_hand = hand;
    if let Ok((mut transform, mut visibility)) = hands.single_mut() {
        *transform = hand;
        // The Keyblade is drawn only in battle mode.
        *visibility = if game.player.armed {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (part, handle) in &parts {
        let positions = game.part_indices[part.0]
            .iter()
            .map(|&i| display[i as usize].to_array());
        if let Some(mut mesh) = meshes.get_mut(&handle.0) {
            visual::room::update_positions(&mut mesh, positions);
        }
    }
    if let Some((from, jump, from_frame)) = switched {
        let (tick, blend) = (game.tick, game.player.blend);
        game.event(serde_json::json!({"event":"pose_switch","tick":tick,"from":from,
            "to":format!("{action:?}"),"jump":jump,"blend":blend,"from_frame":from_frame,"frame":frame}));
    }
    // Unattended runs log every sudden change of the shown pose, with the
    // action, its frame and the blend, to find abrupt motion ends.
    if game.autoplay.is_some() && game.display.len() == display.len() {
        let jump = display
            .iter()
            .zip(&game.display)
            .map(|(a, b)| a.distance(*b))
            .fold(0f32, f32::max);
        if jump > POSE_JUMP_LOG {
            let (tick, frame, blend) = (game.tick, frame, game.player.blend);
            game.event(
                serde_json::json!({"event":"pose_jump","tick":tick,"action":format!("{action:?}"),
                "frame":frame,"jump":jump,"blend":blend}),
            );
        }
    }
    game.display = display;
    game.display_frame = frame;
    // The Keyblade's own motion (key chain and blade) at the same frame.
    let weapon_frames = &game.clip_poses[clip].weapon_frames;
    if !weapon_frames.is_empty() {
        let pose = weapon_frames[frame.min(weapon_frames.len() - 1)].clone();
        for (part, handle) in &weapon_parts {
            let positions = game.weapon_indices[part.0]
                .iter()
                .map(|&i| pose[i as usize].to_array());
            if let Some(mut mesh) = meshes.get_mut(&handle.0) {
                visual::room::update_positions(&mut mesh, positions);
            }
        }
        game.weapon_display = pose;
    }
    if let Ok(mut root) = roots.single_mut() {
        root.translation = Vec3::from_array(game.player.position);
        root.rotation = Quat::from_rotation_y(game.player.yaw);
    }
}

/// Unattended runs start with simulation time paused until rendering has
/// warmed up, so the first capture is not a blank frame.
fn pause_for_autoplay(game: Res<Game>, mut time: ResMut<Time<Virtual>>) {
    if game.autoplay.is_some() {
        time.pause();
    }
}

// Bevy systems take each resource as a parameter.
#[allow(clippy::too_many_arguments)]
fn finish(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut overlay: Query<&mut Visibility, With<CollisionOverlay>>,
    mut game: ResMut<Game>,
    mut time: ResMut<Time<Virtual>>,
    mut closed: MessageReader<bevy::window::WindowCloseRequested>,
    mut exit: MessageWriter<AppExit>,
    music: Option<Res<MusicRes>>,
    compose: Option<Res<Compose>>,
    gallery: Res<enemy_gallery::EnemyGallery>,
) {
    // Unattended runs warm up once the music decodes have finished (they
    // compete with the first pipeline compiles).
    let music_ready = music.is_none_or(|m| {
        m.pending
            .iter()
            .flatten()
            .all(|slot| slot.lock().map_or(true, |r| r.is_some()))
    });
    let settled = compose.is_none_or(|c| c.stable_frames >= AUTOPLAY_WARMUP_FRAMES);
    if music_ready && settled {
        game.rendered_frames += 1;
    }
    if game.autoplay.is_some()
        && time.is_paused()
        && !game.capture_pending
        && game.rendered_frames >= AUTOPLAY_WARMUP_FRAMES
    {
        time.unpause();
    }
    if let Some(dir) = game.autoplay.clone()
        && !game.capture_pending
        && let Some(&at) = game.captures.get(game.captures_taken)
        && game.tick >= at
    {
        game.capture_pending = true;
        // Hold simulation time so the capture shows exactly this tick.
        time.pause();
        let index = game.captures_taken;
        let path = dir.join(format!("test-room-{index:02}.png"));
        let action = format!("{:?}", game.player.action);
        let tick = game.tick;
        commands.spawn(Screenshot::primary_window()).observe(
            move |event: On<ScreenshotCaptured>,
                  mut game: ResMut<Game>,
                  mut time: ResMut<Time<Virtual>>| {
                let image = event
                    .image
                    .clone()
                    .try_into_dynamic()
                    .expect("test room capture");
                let luma = image.to_luma8();
                let (low, high) = luma.pixels().fold((u8::MAX, u8::MIN), |(l, h), p| {
                    (l.min(p.0[0]), h.max(p.0[0]))
                });
                let uniform = high.saturating_sub(low) < 8;
                if uniform {
                    game.autoplay_failed = true;
                }
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .expect("new private capture");
                image
                    .write_to(&mut file, image::ImageFormat::Png)
                    .expect("capture encode");
                file.sync_all().expect("capture flush");
                game.capture_pending = false;
                game.captures_taken += 1;
                time.unpause();
                game.event(
                    serde_json::json!({"event":"capture","index":index,"tick":tick,"action":action,
                    "luma_range":[low,high],"uniform":uniform}),
                );
            },
        );
    }
    if keys.just_pressed(KeyCode::F1) {
        game.show_collision = !game.show_collision;
        if let Ok(mut visibility) = overlay.single_mut() {
            *visibility = if game.show_collision {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
        }
    }
    let autoplay_done = game.autoplay.is_some()
        && game.tick >= game.autoplay_end
        && game.captures_taken == game.captures.len();
    if autoplay_done && game.exit_code.is_none() && game.enemy_tour {
        let pass = gallery.actors.len() == enemy_tour::MODELS.len()
            && gallery
                .actors
                .iter()
                .zip(enemy_tour::MODELS)
                .all(|(a, model)| {
                    gallery.catalog.entries[a.entry].model_name == *model
                        && a.error.is_none()
                        && a.motion_choice.is_some()
                });
        game.autoplay_failed |= !pass;
        game.event(serde_json::json!({"event":"enemy_tour_verdict","pass":pass,
            "actors":gallery.actors.iter().map(|a| serde_json::json!({"id":a.id,
                "model":gallery.catalog.entries[a.entry].model_name,
                "object_id":gallery.catalog.entries[a.entry].object_id,
                "motion_choice":a.motion_choice,"frame":a.frame,"error":a.error})).collect::<Vec<_>>(),
            "installed_comparison":"NOT RUN"}));
    }
    // The badge and journal tours (their Sora has the unattended run's extra
    // ability) have no ability verdict.
    if autoplay_done
        && game.exit_code.is_none()
        && !game.badge_tour
        && !game.journal_tour
        && let Some(tour) = &game.ability_tour
    {
        let expected = game
            .ability_source
            .build(&tour.expected_slots, &game.equipment);
        let pass = game.player.abilities == expected;
        game.autoplay_failed |= !pass;
        let verdict = serde_json::json!({"event":"ability_tour_verdict","pass":pass,
            "growth":game.player.abilities.growth,"ground_combo":game.player.abilities.ground_combo,
            "air_combo":game.player.abilities.air_combo,"finishers":game.player.abilities.finishers});
        game.event(verdict);
    }
    let window_closed = closed.read().count() > 0;
    if keys.just_pressed(KeyCode::Escape)
        || window_closed
        || autoplay_done
        || game.limit.is_some_and(|n| game.tick >= n)
    {
        let code = if game.autoplay.is_some() && (!autoplay_done || game.autoplay_failed) {
            1
        } else {
            0
        };
        if game.exit_code.is_none() {
            game.exit_code = Some(code);
            let tick = game.tick;
            game.event(serde_json::json!({"event":"exit","tick":tick,"code":code,"window_closed":window_closed}));
            exit.write(if code == 0 {
                AppExit::Success
            } else {
                AppExit::Error(std::num::NonZeroU8::new(code).unwrap())
            });
        }
    }
}

/// The camp (pause) menu: its sources, the open menu and the menu pad.
#[derive(Resource)]
struct CampRes {
    source: CampSource,
    open: Option<Camp>,
    /// Cursor memory and the save's config bits / game mode.
    memory: pause_menu::Memory,
    config_flags: u32,
    game_mode: u32,
    pad: Pad,
    /// Sheet slots: camp art per glyph variant, then the font.
    art: [Option<usize>; 4],
    font_sheet: usize,
    icon_sheet: usize,
    glyph: usize,
    keyboard_last: bool,
    /// The room's stock (ROOM_STOCK, then what the camp leaves) and the
    /// stock list's seen marks.
    stock: std::collections::BTreeMap<u16, u8>,
    seen: Vec<u16>,
    /// The items the room starts with (never NEW!).
    starting: Vec<u16>,
    /// Sora's 80 ability slots, seeded from the installed development catalog.
    ability_slots: Vec<u16>,
    /// Shortcut command ids (save+0x36f8; empty in a new save [test-room
    /// value]) and the MP-charge bits (save+0x3744).
    shortcuts: [u16; 4],
    no_charge: u8,
    /// Sora's equipment [test-room values]: the Kingdom Key, one empty
    /// armor and accessory slot, two empty item slots.
    equipment: pause_menu::items::Equipment,
    /// The equipment last logged from the open menu.
    logged_equipment: Option<pause_menu::items::Equipment>,
    /// The menu chain past the camp: the journal and the curtain between.
    journal_source: Option<JournalSource>,
    journal: Option<Journal>,
    curtain: Option<Curtain>,
    journal_sheets: JournalSheets,
    /// The item pictures: the selected source they load from at run time
    /// (`itempic/item-%03d.imd` in kh2_fifth) and their sheets (None:
    /// the install has no such file).
    picture_source: Option<std::sync::Arc<ProbeSource>>,
    /// The journal's seen `char` and `albu` rows (save+0x4c38 / 0x4c7c).
    seen_chars: Vec<u16>,
    seen_albums: Vec<u16>,
    seen_limits: Vec<u16>,
    seen_quests: Vec<u16>,
    seen_minigames: Vec<u16>,
    seen_story: Vec<u16>,
    seen_synthesis: Vec<u8>,
    picture_sheets: HashMap<u16, Option<usize>>,
    notice_sheets: HashMap<String, usize>,
    /// The menus' language code (`menu/<lang>/` notice and photo files).
    lang: String,
}

/// Sheet slots of the journal's images.
#[derive(Default)]
struct JournalSheets {
    book: usize,
    curtain: usize,
    worlds: Vec<usize>,
    tops: HashMap<String, Vec<usize>>,
    /// jmface_<code>.2ld faces by journal world, photos by `albu` row
    /// (loaded when the journal asks).
    faces: HashMap<u8, Vec<usize>>,
    photos: HashMap<u16, usize>,
}

impl CampRes {
    /// A menu (camp, curtain or journal) holds the screen and the pad.
    fn menu_up(&self) -> bool {
        self.open.is_some() || self.journal.is_some() || self.curtain.is_some()
    }
}

/// Unattended camp check after the combat plan: open, down to Status,
/// the character list, the stat browser (column switch, move), back out,
/// down to Config, change Field Camera and Summon Effects, jump to the
/// bottom with R1, R2 to the Graphics page, back out, close with START.
const AUTOPLAY_CAMP: &[(u64, u64)] = &[
    (1810, 0x2),
    (1840, 0x20),
    (1850, 0x20),
    (1860, 0x20),
    (1870, 0x20),
    (1890, 0x4),
    (1940, 0x4),
    (2040, 0x80),
    (2060, 0x20),
    (2090, 0x8),
    (2140, 0x8),
    (2240, 0x20),
    (2250, 0x20),
    (2270, 0x4),
    (2320, 0x80),
    (2340, 0x20),
    (2350, 0x20),
    (2360, 0x20),
    (2380, 0x80),
    (2400, 0x4000),
    // R2: the Graphics settings page.
    (2405, 0x1_0000),
    (2430, 0x8),
    (2480, 0x2),
    // Items: reopen, wrap to Items, Stock, the consumables tab, use an
    // Ether, use another (MP full: the no-effect box), throw it away.
    (2600, 0x2),
    (2640, 0x20),
    (2660, 0x4),
    (2700, 0x20),
    (2730, 0x4),
    (2770, 0x80),
    (2790, 0x1_0000),
    (2830, 0x20),
    (2850, 0x4),
    (2920, 0x4),
    (2960, 0x4),
    (3010, 0x4),
    (3040, 0x4),
    (3080, 0x4),
    (3120, 0x4),
    (3150, 0x8),
    (3180, 0x20),
    (3200, 0x4),
    (3240, 0x40),
    (3260, 0x4),
    (3310, 0x8),
    (3350, 0x8),
    (3430, 0x2),
    // Abilities: reopen, down to Abilities, Sora, the list; unequip Guard
    // with TRIANGLE, re-equip it in the popup, back out, SQUARE to Items
    // and back, close.
    (3490, 0x2),
    (3530, 0x20),
    (3550, 0x4),
    (3610, 0x4),
    (3710, 0x20),
    (3730, 0x10),
    (3750, 0x1000),
    (3780, 0x4),
    (3850, 0x20),
    (3880, 0x4),
    (3950, 0x8),
    (4020, 0x800),
    (4110, 0x800),
    (4200, 0x8),
    (4270, 0x2),
    // Party: reopen, down to Party, the card (nothing to switch), back.
    (4330, 0x2),
    (4370, 0x20),
    (4380, 0x20),
    (4400, 0x4),
    (4470, 0x80),
    (4490, 0x8),
    (4570, 0x2),
    // Customize: reopen, up to Customize, Sora, shortcut 1 → Potion, back
    // out, close.
    (4630, 0x2),
    (4670, 0x10),
    (4690, 0x4),
    (4750, 0x4),
    (4860, 0x4),
    (4930, 0x20),
    (4950, 0x4),
    (5050, 0x8),
    (5150, 0x8),
    (5220, 0x2),
    // Equip: reopen, up to Items, Sora, item slot 1 → Potion, auto-reload
    // on with TRIANGLE, back to Items, close.
    (5290, 0x2),
    (5330, 0x10),
    (5340, 0x10),
    (5360, 0x4),
    (5440, 0x4),
    (5610, 0x20),
    (5620, 0x20),
    (5630, 0x20),
    (5670, 0x4),
    (5760, 0x20),
    (5800, 0x4),
    (5910, 0x1000),
    (5950, 0x8),
    (6050, 0x8),
    (6120, 0x2),
    // Journal: reopen, down to Jiminy's Journal (curtain, Notes of the
    // world), Characters (the free hand walks left and down onto Donald),
    // back, down to Album, the list, a photo, R2, back out, Select World,
    // out (the camp comes back as the curtain lifts), close.
    (6190, 0x2),
    (6230, 0x20),
    (6240, 0x20),
    (6250, 0x20),
    (6260, 0x20),
    (6270, 0x20),
    (6290, 0x4),
    (6360, 0x4),
    (6450, 0x40),
    (6451, 0x40),
    (6452, 0x40),
    (6453, 0x40),
    (6454, 0x40),
    (6455, 0x40),
    (6456, 0x40),
    (6457, 0x40),
    (6458, 0x40),
    (6459, 0x40),
    (6460, 0x40),
    (6461, 0x40),
    (6462, 0x40),
    (6450, 0x20),
    (6451, 0x20),
    (6452, 0x20),
    (6453, 0x20),
    (6454, 0x20),
    (6455, 0x20),
    (6456, 0x20),
    (6457, 0x20),
    (6458, 0x20),
    (6500, 0x4),
    (6540, 0x8),
    (6610, 0x20),
    (6620, 0x20),
    (6650, 0x4),
    (6760, 0x4),
    (6880, 0x1_0000),
    (6980, 0x8),
    (7040, 0x8),
    (7110, 0x8),
    (7180, 0x8),
    (7270, 0x2),
    // Character Files: reopen, Journal (Notes), R1 to Collection, the
    // first option (Character Files), the world menu, the list, a detail page, R2, then
    // back out and close.
    (7450, 0x2),
    (7500, 0x4),
    (7580, 0x4000),
    (7740, 0x4),
    (7820, 0x4),
    (7900, 0x4),
    (7980, 0x1_0000),
    (8120, 0x8),
    (8190, 0x8),
    (8260, 0x8),
    (8400, 0x2),
    (8480, 0x2),
    // Treasures: reopen, Journal (Notes), down to Treasures, the grid, a
    // few cursor moves, START.
    (8550, 0x2),
    (8600, 0x4),
    (8680, 0x20),
    (8700, 0x20),
    (8720, 0x20),
    (8780, 0x4),
    (8960, 0x20),
    (8990, 0x80),
    (9020, 0x80),
    (9050, 0x4000),
    (9200, 0x100),
];
fn camp_setup(
    camp: Option<ResMut<CampRes>>,
    mut frame: ResMut<Ui2dFrame>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(mut c) = camp else { return };
    upload_camp_sheets(&mut c, &mut frame, &mut images);
}

/// The camp's and the journal's images as 2D sheets (again after a text
/// language change).
fn upload_camp_sheets(c: &mut CampRes, frame: &mut Ui2dFrame, images: &mut Assets<Image>) {
    for n in 0..4 {
        if let Some((t, scale)) = c.source.menu.texture_for(0, n) {
            let (rgba, size) = (t.rgba.clone(), t.size);
            c.art[n] = Some(frame.add_sheet(images, rgba, size, scale));
        }
    }
    c.font_sheet = frame.add_sheet(
        images,
        std::mem::take(&mut c.source.font.rgba),
        c.source.font.size,
        c.source.font.texel_scale as f32,
    );
    c.icon_sheet = frame.add_sheet(
        images,
        std::mem::take(&mut c.source.icons.rgba),
        c.source.icons.size,
        c.source.icons.texel_scale as f32,
    );
    if let Some(j) = c.journal_source.as_mut() {
        let mut add = |t: &mut menu_screen::Texture| {
            frame.add_sheet(images, std::mem::take(&mut t.rgba), t.size, t.texel_scale)
        };
        let s = &mut c.journal_sheets;
        let book = j.book.texture_for(0, 0).map(|(_, k)| k);
        if let (Some(t), Some(k)) = (j.book.replacements[0].as_mut(), book) {
            t.texel_scale = k;
            s.book = add(t);
        }
        let curtain = j.curtain.texture_for(0, 0).map(|(_, k)| k);
        if let (Some(t), Some(k)) = (j.curtain.replacements[0].as_mut(), curtain) {
            t.texel_scale = k;
            s.curtain = add(t);
        }
        s.worlds = j.world_icons.iter_mut().map(&mut add).collect();
        for (code, t) in &mut j.tops {
            s.tops
                .insert(code.clone(), t.iter_mut().map(&mut add).collect());
        }
    }
}

fn field_running(
    camp: Option<Res<CampRes>>,
    debug: Res<debug_menu::DebugMenu>,
    game_over: Option<Res<game_over::GameOverRes>>,
) -> bool {
    camp.is_none_or(|c| !c.menu_up()) && !debug.open && game_over.is_none_or(|g| g.field_runs())
}

/// START (logical 0x2 in the field's pad mode) opens the camp
/// (FUN_1401572b0 → FUN_1402e4950(0)); while it runs the field is not
/// stepped, the menu reads pad mode 7 and the frame counter keeps going.
#[allow(clippy::too_many_arguments)]
fn camp_tick(
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    pads: Query<(&Gamepad, Option<&Name>)>,
    camp: Option<ResMut<CampRes>>,
    mut game: ResMut<Game>,
    mut audio: ResMut<Audio>,
    mut hud: Option<ResMut<Hud>>,
    mut frame: ResMut<Ui2dFrame>,
    mut images: ResMut<Assets<Image>>,
    mut settings: ResMut<settings_room::SettingsRes>,
    debug: Res<debug_menu::DebugMenu>,
) {
    let Some(mut c) = camp else { return };
    let c = &mut *c;
    let (data, keyboard, glyph) = input::read_devices(&keys, &mouse, 0., &pads);
    if keyboard {
        c.keyboard_last = true;
    } else if glyph.is_some() {
        c.keyboard_last = false;
    }
    let g = if c.keyboard_last {
        0
    } else {
        glyph.map_or(c.glyph, usize::from)
    };
    if c.art[g].is_some() {
        c.glyph = g;
    }
    let menu = c.menu_up();
    let mode = if menu {
        controller::MODE_TITLE
    } else {
        controller::MODE_FIELD
    };
    // Level-ups change the Status numbers and teach abilities.
    let gear = game.gear;
    if let Some(rt) = game.enemy.as_mut()
        && rt.numbers_dirty
    {
        rt.numbers_dirty = false;
        if let Some(row) = rt.sora_levels.get(rt.level as usize - 1) {
            let n = &mut c.source.numbers;
            n.level = rt.level;
            n.next_exp = row.exp.saturating_sub(rt.exp_total);
            n.strength = rt.sora.strength - i32::from(gear.strength);
            n.magic = rt.sora.magic - i32::from(gear.magic);
            n.defense = rt.sora_target.defense - i32::from(gear.defense);
        }
        for id in rt.learned.drain(..) {
            if !c.ability_slots.iter().any(|&s| s & 0x7fff == id)
                && let Some(slot) = c.ability_slots.iter_mut().find(|s| **s == 0)
            {
                *slot = id;
            }
        }
    }
    let tick = game.tick;
    if game.autoplay.is_some() {
        let logical = if let Some(tour) = &game.ability_tour {
            tour.logical(game.tick)
        } else if game.magic_tour {
            // The magic tour never opens the camp.
            0
        } else {
            AUTOPLAY_CAMP
                .iter()
                .filter(|(at, _)| *at == tick)
                .fold(0, |acc, &(_, b)| {
                    // Menu START is 0x100 in mode 7.
                    acc | if menu && b == 0x2 { 0x100 } else { b }
                })
        };
        let logical = if menu && logical == 2 { 0x100 } else { logical };
        c.pad.update(logical, 1.);
    } else {
        c.pad.update_raw(controller::raw(&data), mode, 1.);
    }
    // The debug menu holds the pad while it is open.
    if debug.open {
        return;
    }
    let mut sounds = Vec::new();
    let layout = &c.source.menu.layout;
    // Test-room values: no munny, level 1, the room's HP/MP; the clock is
    // the room's frame count.
    let (hp, max_hp, mp, max_mp) = match (&game.enemy, &hud) {
        (Some(rt), Some(_)) => (
            rt.sora_hp,
            rt.sora_target.max_hp,
            game.player.mp.cur,
            game.player.mp.max,
        ),
        (None, Some(h)) => (
            h.stats.hp,
            h.stats.max_hp,
            game.player.mp.cur,
            game.player.mp.max,
        ),
        _ => (1, 1, 0, 1),
    };
    let panels = pause_menu::Panels {
        munny: 0,
        time_frames: tick as u32,
        members: vec![pause_menu::Member {
            name: c.source.name.clone(),
            level: 1,
            hp,
            max_hp,
            mp,
            max_mp,
        }],
        labels: c.source.labels.clone(),
    };
    let panels_again = panels.clone();
    // Status values are built when the camp opens (FUN_140354340).
    let gear = game.gear;
    let form_weapon = game.form_weapon();
    // Equip's form rows: the owned Valor, Master and Final Forms and the
    // Keyblade each holds (FUN_1402fc630).
    let form_slots: Vec<(u8, u16)> = game
        .drive
        .as_ref()
        .map(|d| {
            [
                player::forms::VALOR,
                player::forms::MASTER,
                player::forms::FINAL,
            ]
            .into_iter()
            .filter(|&f| d.forms.owned[f as usize])
            .map(|f| (f, d.forms.record(f).map_or(0, |r| r.keyblade)))
            .collect()
        })
        .unwrap_or_default();
    let form_abilities: Vec<u16> = form_slots
        .iter()
        .map(|s| {
            game.ability_source
                .equipment_abilities
                .get(&s.1)
                .copied()
                .unwrap_or(0)
        })
        .collect();
    let camp_data = |c: &CampRes,
                     panels: pause_menu::Panels,
                     battle_items: Option<&item_room::ItemRt>,
                     settings: &settings_room::SettingsRes|
     -> pause_menu::Data {
        // The battle's item slots and stock are the live ones.
        let mut equipment = c.equipment.clone();
        equipment.form_ids = form_slots.iter().map(|s| s.0).collect();
        equipment.forms = form_slots.iter().map(|s| s.1).collect();
        let mut counts = c.stock.clone();
        if let Some(it) = battle_items {
            equipment.items = it.slots.clone();
            equipment.originals = it.originals.clone();
            counts = it.stock.clone();
        }
        // The camp's "new" bits (FUN_140369520): what the stock holds now
        // that the save has not marked seen. The room's starting stock
        // never shows NEW! [I: the new-game save's seen bits are not
        // decoded].
        let new_items = pause_menu::items::new_items(&counts, &c.seen, &c.starting);
        // Level and record numbers plus what the equipment adds (stats
        // +0x188..+0x18e, resists +0x1a8..).
        let mut n = c.source.numbers;
        n.strength += i32::from(gear.strength);
        n.magic += i32::from(gear.magic);
        n.defense += i32::from(gear.defense);
        n.ap += i32::from(gear.ap);
        let sora_abilities = pause_menu::abilities::MemberAbilities {
            slots: c.ability_slots.clone(),
            ap_max: n.ap,
            weapon_ability: c.source.weapon_ability,
            form_weapon_abilities: form_abilities.clone(),
        };
        let ap_used = pause_menu::abilities::used(&pause_menu::abilities::entries(
            &c.source.items.table,
            &sora_abilities,
        ));
        let value = |bit: usize| -> String {
            match bit {
                0 => format!("{}", n.level),
                1 | 7 => "0".into(),
                2 => format!("{}", n.next_exp),
                3 => format!("{hp}/{max_hp}"),
                4 => format!("{mp}/{max_mp}"),
                5 => format!("{ap_used}/{}", n.ap),
                10 => format!("{}", n.strength),
                11 => format!("{}", n.magic),
                12 => format!("{}", n.defense),
                13..=16 => format!("{}%", 100 - i32::from(gear.resist[bit - 12])),
                _ => "0%".into(),
            }
        };
        let mut stats = vec![None; 20];
        for (bit, label, help, style) in &c.source.stats {
            stats[*bit] = Some(pause_menu::Stat {
                label: label.clone(),
                help: help.clone(),
                style: *style,
                value: pause_menu::encode_number(&value(*bit)),
            });
        }
        pause_menu::Data {
            entries: c.source.entries.clone(),
            panels,
            status: vec![pause_menu::StatusEntry {
                bit: 0,
                label: c.source.name.clone(),
                help: c.source.help.clone(),
                stats,
            }],
            banner: c.source.banner.clone(),
            config: c.source.config.clone(),
            config_flags: c.config_flags,
            game_mode: c.game_mode,
            items: pause_menu::items::ItemsData {
                counts,
                equipment: vec![equipment],
                seen: c.seen.clone(),
                new_items,
                ..c.source.items.clone()
            },
            abilities: pause_menu::abilities::AbilitiesData {
                members: vec![sora_abilities.clone()],
                texts: c.source.abilities.clone(),
            },
            party_help: c.source.party_help.clone(),
            equip: pause_menu::equip::EquipData {
                stats: c.source.equip_stats.clone(),
                totals: vec![pause_menu::equip::Totals {
                    ap: n.ap,
                    strength: n.strength,
                    magic: n.magic,
                    defense: n.defense,
                    resists: [
                        gear.resist[1],
                        gear.resist[2],
                        gear.resist[3],
                        gear.resist[4],
                    ],
                }],
                bases: vec![player::equipment_stats::Totals::start(
                    c.source.numbers.strength as u16,
                    c.source.numbers.magic as u16,
                    c.source.numbers.defense as u16,
                    c.source.numbers.ap as u16,
                )],
                form_weapons: vec![form_weapon.unwrap_or(0)],
                texts: c.source.equip_texts.clone(),
            },
            customize: pause_menu::customize::CustomizeData {
                shortcuts: c.shortcuts,
                no_charge: c.no_charge,
                texts: c.source.customize.clone(),
            },
            picture_sequence: c.source.picture_sequence.clone(),
            notice: pause_menu::notice::NoticeData {
                flags: c
                    .journal_source
                    .as_ref()
                    .map_or_else(Vec::new, |j| j.flags.clone()),
                headings: c.source.notice_headings.clone(),
                page: c.source.notice_page.clone(),
                callouts: c.source.notice_callouts.clone(),
            },
            settings: Some(settings.current.clone()),
            languages: settings.languages.clone(),
        }
    };
    // The menu chain (menu mode 10, FUN_14031aac0): the camp; deciding a
    // journal brings the curtain down over the camp's close; the journal
    // runs; then the camp is rebuilt as the curtain lifts.
    if let Some(menu) = c.open.as_mut() {
        // The images loaded so far (FUN_1403018e0's cache).
        menu.pictures.loaded = c
            .picture_sheets
            .iter()
            .filter(|(_, s)| s.is_some())
            .map(|(&id, _)| id)
            .collect();
        sounds = menu.frame(layout, c.pad.pressed, c.pad.repeat, tick as u32);
        game.tick += 1;
        // The settings pages apply as they change.
        if let Some(s) = &menu.data.settings {
            settings.current.clone_from(s);
        }
        // Each equipment change in the open menu, with the totals the
        // Equip screen then shows.
        let shown = menu.data.items.equipment.first().cloned();
        if shown != c.logged_equipment {
            if let Some(e) = &shown {
                let totals = menu.data.equip.totals.first().copied().unwrap_or_default();
                let gg = game.gear;
                game.event(serde_json::json!({"event":"menu_equipment","weapon":e.weapon,
                    "forms":e.forms,"armor":e.armor,"accessories":e.accessories,
                    "ap":totals.ap,"strength":totals.strength,"magic":totals.magic,
                    "defense":totals.defense,"resists":totals.resists,
                    "weapon_ability":menu.data.abilities.members.first().map(|m| m.weapon_ability),
                    "tick":tick,"level_base":[c.source.numbers.strength,c.source.numbers.magic,c.source.numbers.defense],
                    "game_gear":[gg.strength,gg.magic,gg.defense]}));
            }
            c.logged_equipment = shown;
        }
        if menu.journal.is_some()
            && c.curtain.is_none()
            && let Some(j) = &c.journal_source
        {
            c.curtain = Some(Curtain::cover(&j.curtain.layout));
        }
        // FUN_140305480 once the curtain holds black.
        if menu.journal.is_some()
            && !menu.closed
            && c.curtain.as_ref().is_some_and(Curtain::covered)
        {
            menu.kill_all();
        }
        if menu.closed {
            settings.save_if_changed();
            c.memory = menu.memory;
            c.config_flags = menu.data.config_flags;
            c.stock = menu.data.items.counts.clone();
            c.seen = menu.data.items.seen.clone();
            c.shortcuts = menu.data.customize.shortcuts;
            if let Some(e) = menu.data.items.equipment.first() {
                c.equipment = e.clone();
            }
            // The Equip menu's form weapons (FUN_1402fc7c0) and the member's
            // Keyblade: a change replaces the weapon objects.
            let previous_weapon = game.equipment.weapon;
            game.equipment = c.equipment.clone();
            if let Some(d) = game.drive.as_mut() {
                let mut changed = c.equipment.weapon != previous_weapon;
                for (&f, &id) in c.equipment.form_ids.iter().zip(&c.equipment.forms) {
                    if let Some(r) = d.forms.record_mut(f)
                        && r.keyblade != id
                    {
                        r.keyblade = id;
                        changed |= d.active && d.form == f;
                    }
                }
                if changed {
                    d.keyblades_changed(c.equipment.weapon);
                }
            }
            c.source.weapon_ability = game
                .ability_source
                .equipment_abilities
                .get(&c.equipment.weapon)
                .copied()
                .unwrap_or(0);
            c.no_charge = menu.data.customize.no_charge;
            if let Some(it) = game.items.as_mut() {
                it.slots = c.equipment.items.clone();
                it.originals = c.equipment.originals.clone();
                it.stock = c.stock.clone();
            }
            if let Some(m) = menu.data.abilities.members.first() {
                c.ability_slots = m.slots.clone();
            }
            // The battle ability record follows the equipped slots;
            // Guard's ability bit gates PTYA 0x0b (FUN_140402750).
            // In a Drive form the form's own slots are linked as well.
            let slots = c.ability_slots.clone();
            drive_room::rebuild_abilities(&mut game, Some(&slots));
            let equipped = serde_json::json!({"event":"equipped_abilities","growth":game.player.abilities.growth,
                "ground_combo":game.player.abilities.ground_combo,"air_combo":game.player.abilities.air_combo,
                "finishers":game.player.abilities.finishers,"combo_boost":game.player.abilities.combo_boost,
                "air_combo_boost":game.player.abilities.air_combo_boost,"reaction_boost":game.player.abilities.reaction_boost});
            game.event(equipped);
            // Owned spell levels (the element items), the shortcut slots
            // and the MP-charge toggles follow the camp.
            game.shortcuts = c.shortcuts;
            if let Some(m) = game.magic.as_mut() {
                for (t, item) in ELEMENT_ITEMS.iter().enumerate() {
                    m.levels[t] = c.stock.get(item).copied().unwrap_or(0);
                }
                m.dont_charge = c.no_charge;
            }
            // Items used in the camp change the room's HP / MP.
            if let Some(sora) = menu.data.panels.members.first() {
                if let Some(rt) = game.enemy.as_mut() {
                    rt.sora_hp = sora.hp;
                }
                if let Some(h) = hud.as_mut() {
                    h.stats.hp = sora.hp;
                }
                // Items used in the camp restore MP through the charge
                // rules (FUN_1403c0890).
                let delta = sora.mp - game.player.mp.cur;
                if delta != 0 {
                    game.player.mp.add(delta);
                }
            }
            let kind = menu.journal;
            c.open = None;
            game.event(serde_json::json!({"event":"camp_close","tick":tick}));
            if let (Some(kind), Some(j)) = (kind, &c.journal_source) {
                let data = journal::JournalData {
                    kind,
                    // The room stands in for Twilight Town (world 2) and a
                    // save with no story flags [test-room values].
                    world: 2,
                    worlds: j.worlds.clone(),
                    stories: j.stories.clone(),
                    flags: {
                        let mut f = j.flags.clone();
                        // The tour shows everything (story flag 0x4837).
                        if game.journal_tour {
                            f.extend(journal_tour::STORY_FLAGS);
                        }
                        f
                    },
                    // FUN_140339500 counts each `limi` command's limit in
                    // save+0x3d48 (FUN_1403e7c30 / FUN_1403e8150): none
                    // in this save; no reports, pieces, missions, minigames
                    // or synthesis either [test-room values].
                    unlocks: journal::Unlocks {
                        limits: game.journal_tour,
                        ..Default::default()
                    },
                    texts: j.texts.clone(),
                    tops: j.tops.iter().map(|(code, _)| code.clone()).collect(),
                    world_icons: !j.world_icons.is_empty(),
                    chars: j.chars.clone(),
                    diag: j.diag.clone(),
                    albums: j.albums.clone(),
                    reports: j.reports.clone(),
                    treasures: j.treasures.clone(),
                    item_names: j.item_names.clone(),
                    opened_bits: j.opened_bits.clone(),
                    owned_items: j.owned_items.clone(),
                    seen_chars: c.seen_chars.clone(),
                    seen_albums: c.seen_albums.clone(),
                    // No limit has been used in this save [test-room value].
                    limit_counts: (0..j.limits.len())
                        .map(|i| {
                            if game.journal_tour {
                                journal_tour::LIMIT_COUNTS.get(i).copied().unwrap_or(0)
                            } else {
                                0
                            }
                        })
                        .collect(),
                    limits: j.limits.clone(),
                    seen_limits: c.seen_limits.clone(),
                    quests: j.quests.clone(),
                    minigames: j.minigames.clone(),
                    // No minigame has been played and no menu flag is set
                    // in this save [test-room values].
                    minigame_records: if game.journal_tour {
                        journal_tour::minigame_records()
                    } else {
                        Vec::new()
                    },
                    menu_flags: Vec::new(),
                    seen_quests: c.seen_quests.clone(),
                    seen_minigames: c.seen_minigames.clone(),
                    seen_story: c.seen_story.clone(),
                    // No synthesis yet [test-room value]; the tour gives
                    // the Moogle and some finds.
                    synthesis: if game.journal_tour {
                        journal_tour::SYNTHESIS
                    } else {
                        journal::Synthesis::default()
                    },
                    moogle_names: j.moogle_names.clone(),
                    seen_synthesis: c.seen_synthesis.clone(),
                };
                let (opened, s) = Journal::open(&j.book.layout, data);
                game.event(serde_json::json!({"event":"journal_open","tick":tick,
                    "kind":kind,"state":opened.state}));
                c.journal = Some(opened);
                sounds.extend(s);
            }
        }
    } else if let Some(open) = c.journal.as_mut() {
        let j = c.journal_source.as_ref().expect("journal source");
        // The left stick drives the Characters page's free hand.
        let stick = if game.autoplay.is_some() {
            [0.; 3]
        } else {
            controller::analog(data.left)
        };
        sounds = open.frame(
            &j.book.layout,
            c.pad.pressed,
            c.pad.repeat,
            c.pad.held,
            stick,
        );
        game.tick += 1;
        if open.closed {
            (c.seen_chars, c.seen_albums) = open.seen();
            c.seen_limits = open.seen_limits();
            (c.seen_quests, c.seen_minigames) = open.seen_lists();
            c.seen_story = open.seen_story();
            c.seen_synthesis = open.seen_synthesis();
            c.journal = None;
            game.event(serde_json::json!({"event":"journal_close","tick":tick}));
            // FUN_140304ea0: the camp is rebuilt (its open sound) and the
            // curtain lifts (FUN_14031a690).
            let (menu, s) = Camp::open(
                layout,
                camp_data(c, panels, game.items.as_ref(), &settings),
                c.memory,
            );
            c.open = Some(menu);
            sounds.extend(s);
            if let Some(cur) = c.curtain.as_mut() {
                cur.uncover(&j.curtain.layout);
            }
            game.event(serde_json::json!({"event":"camp_open","tick":tick}));
        }
    } else if c.curtain.is_none() && c.pad.pressed & 0x2 != 0 && !game.talking {
        let (menu, s) = Camp::open(
            layout,
            camp_data(c, panels, game.items.as_ref(), &settings),
            c.memory,
        );
        c.open = Some(menu);
        sounds = s;
        game.tick += 1;
        game.event(serde_json::json!({"event":"camp_open","tick":tick}));
    }
    load_wanted_picture(c, &mut frame, &mut images);
    load_wanted_notice(c, &mut frame, &mut images);
    load_wanted_journal(c, &mut frame, &mut images);
    if let (Some(cur), Some(j)) = (c.curtain.as_mut(), c.journal_source.as_ref()) {
        cur.update(&j.curtain.layout);
        if cur.dead {
            c.curtain = None;
        }
    }
    for n in sounds {
        game.event(serde_json::json!({"event":"sound","tick":tick,"id":n,"cause":"camp"}));
        let Audio { output, banks, .. } = &mut *audio;
        if let (Some(output), Ok(Some(r))) = (output.as_ref(), banks.request(n))
            && let Ok(mut mixer) = output.mixer.lock()
        {
            for layer in r.layers {
                mixer.play(layer, n, r.volume, 1.);
            }
        }
    }
    // A new text language: the menus reload in it and the camp comes back
    // on Config with its state.
    if settings.language_pending() && c.open.is_some() {
        match settings.switch_language(c, hud.as_deref_mut(), &mut frame, &mut images) {
            Ok(()) => {
                let mut old = c.open.take().expect("camp open");
                old.commit_config();
                let mut data = camp_data(c, panels_again, game.items.as_ref(), &settings);
                let o = &old.data;
                data.config_flags = o.config_flags;
                data.items.counts.clone_from(&o.items.counts);
                data.items.seen.clone_from(&o.items.seen);
                data.items.equipment.clone_from(&o.items.equipment);
                data.customize.shortcuts = o.customize.shortcuts;
                data.customize.no_charge = o.customize.no_charge;
                data.abilities.members.clone_from(&o.abilities.members);
                data.notice.flags.clone_from(&o.notice.flags);
                for (m, was) in data.panels.members.iter_mut().zip(&o.panels.members) {
                    m.hp = was.hp;
                    m.mp = was.mp;
                }
                let row = old.config_cursor();
                c.open = Some(Camp::open_on_config(
                    &c.source.menu.layout,
                    data,
                    old.memory,
                    row,
                ));
                settings.save_if_changed();
                game.event(serde_json::json!({"event":"language","tick":tick,
                    "language":settings.current.language}));
            }
            Err(e) => {
                eprintln!("Text language not changed: {e}");
                settings.keep_loaded_language();
                if let Some(s) = c.open.as_mut().and_then(|m| m.data.settings.as_mut()) {
                    s.language.clone_from(&settings.current.language);
                }
            }
        }
    }
}

/// FUN_1403018e0's load (here at once): the item picture the camp asks for,
/// from the install (`itempic/item-%03d.imd`).
fn load_wanted_picture(c: &mut CampRes, frame: &mut Ui2dFrame, images: &mut Assets<Image>) {
    let Some(menu) = c.open.as_mut() else { return };
    let Some(id) = menu.pictures.wanted.take() else {
        return;
    };
    let Some(source) = &c.picture_source else {
        return;
    };
    if c.picture_sheets.contains_key(&id) {
        return;
    }
    let path = format!("itempic/item-{id:03}.imd");
    let sheet = match menu_screen::load_imgd(source, PICTURE_INDEX, &path) {
        Ok(Some(t)) => Some(frame.add_sheet(images, t.rgba, t.size, t.texel_scale)),
        Ok(None) => None,
        Err(e) => {
            eprintln!("Item picture {path} rejected: {e}");
            None
        }
    };
    c.picture_sheets.insert(id, sheet);
    if sheet.is_some() {
        menu.pictures.loaded.insert(id);
    }
}

/// The journal's face and photo files, loaded when a page asks
/// (FUN_14032db20 / FUN_14032a670): `menu/jmface_<code>.2ld` and
/// `menu/us/jm_photo/<code><nn>.bin`.
fn load_wanted_journal(c: &mut CampRes, frame: &mut Ui2dFrame, images: &mut Assets<Image>) {
    let (Some(open), Some(j), Some(source)) = (
        c.journal.as_mut(),
        c.journal_source.as_ref(),
        c.picture_source.as_ref(),
    ) else {
        return;
    };
    let code_of = |world: u8| {
        match world {
            0x10 => return Some("ex".to_owned()),
            0x11 => return Some("heart".to_owned()),
            0x12 => return Some("no".to_owned()),
            _ => {}
        }
        j.worlds
            .iter()
            .find(|w| w.world as u8 == world)
            .map(|w| w.code.clone())
    };
    if let Some(key) = open.cache.wanted_face.take() {
        if let std::collections::hash_map::Entry::Vacant(slot) = c.journal_sheets.faces.entry(key) {
            let faces = code_of(key).and_then(|code| {
                let path = format!("menu/jmface_{code}.2ld");
                match menu_screen::load_images_named(source, PICTURE_INDEX, &path) {
                    Ok(f) => f,
                    Err(e) => {
                        eprintln!("Journal faces {path} rejected: {e}");
                        None
                    }
                }
            });
            slot.insert(
                faces
                    .into_iter()
                    .flatten()
                    .map(|t| frame.add_sheet(images, t.rgba, t.size, t.texel_scale))
                    .collect(),
            );
        }
        open.cache.faces.insert(key);
    }
    if let Some(row) = open.cache.wanted_photo
        && c.journal_sheets.photos.contains_key(&row)
    {
        open.cache.wanted_photo = None;
        open.cache.photos.insert(row);
    }
    if let Some(row) = open.cache.wanted_photo.take()
        && !c.journal_sheets.photos.contains_key(&row)
        && let Some(a) = j.albums.get(row as usize)
        && let Some(code) = code_of(a.world as u8)
    {
        let mut path = format!("menu/{}/jm_photo/{code}{}.bin", c.lang, a.suffix);
        if c.lang != "us"
            && !matches!(
                menu_screen::load_imgd(source, PICTURE_INDEX, &path),
                Ok(Some(_))
            )
        {
            path = format!("menu/us/jm_photo/{code}{}.bin", a.suffix);
        }
        match menu_screen::load_imgd(source, PICTURE_INDEX, &path) {
            Ok(Some(t)) => {
                let slot = frame.add_sheet(images, t.rgba, t.size, t.texel_scale);
                c.journal_sheets.photos.insert(row, slot);
                open.cache.photos.insert(row);
            }
            other => {
                if let Err(e) = other {
                    eprintln!("Journal photo {path} rejected: {e}");
                }
            }
        }
    }
}

/// The notice file the camp asks for (FUN_140367150): `menu/us/<name>.bar`.
fn load_wanted_notice(c: &mut CampRes, frame: &mut Ui2dFrame, images: &mut Assets<Image>) {
    let Some(menu) = c.open.as_mut() else { return };
    let Some(file) = menu.notice_files.wanted.take() else {
        return;
    };
    let Some(source) = &c.picture_source else {
        return;
    };
    let mut path = format!("menu/{}/{file}", c.lang);
    if c.lang != "us"
        && !matches!(
            menu_screen::load_imgd(source, PICTURE_INDEX, &path),
            Ok(Some(_))
        )
    {
        path = format!("menu/us/{file}");
    }
    match menu_screen::load_imgd(source, PICTURE_INDEX, &path) {
        Ok(Some(t)) => {
            let slot = frame.add_sheet(images, t.rgba, t.size, t.texel_scale);
            c.notice_sheets.insert(file.clone(), slot);
            menu.notice_files.loaded.insert(file);
        }
        other => {
            if let Err(e) = other {
                eprintln!("Notice {path} rejected: {e}");
            }
            menu.notice_files.missing.insert(file);
        }
    }
}

/// The camp over the frozen field (the field HUD is hidden meanwhile
/// [inferred]), the curtain (0x7554) over it, the journal (0x7557 up) over
/// that.
fn camp_present(
    camp: Option<ResMut<CampRes>>,
    model: Option<Res<MenuModel>>,
    mut frame: ResMut<Ui2dFrame>,
) {
    let Some(mut c) = camp else { return };
    let c = &mut *c;
    let font = field_command_menu::Font {
        widths: &c.source.font.widths,
        // The keyboard or mouse was last used: key caps (pad 0).
        pad: c.glyph as u8,
    };
    if let Some(menu) = c.open.as_mut() {
        let art = c.art[c.glyph].or(c.art[0]).expect("camp art");
        for (sheet, mut quads) in menu.draw(&c.source.menu.layout, &font) {
            let slot = match sheet {
                pause_menu::Sheet::Art => art,
                pause_menu::Sheet::Font => c.font_sheet,
                pause_menu::Sheet::Icon => c.icon_sheet,
                // Only Sora's model is rendered in this room.
                pause_menu::Sheet::Model(i) => {
                    let Some(m) = model.as_ref().filter(|_| i == 0) else {
                        continue;
                    };
                    for q in &mut quads {
                        q.uv = [0, 0, MENU_TARGET[0] as i32, MENU_TARGET[1] as i32];
                    }
                    m.sheet
                }
                pause_menu::Sheet::Picture(id) => {
                    let Some(&Some(s)) = c.picture_sheets.get(&id) else {
                        continue;
                    };
                    s
                }
                pause_menu::Sheet::Notice(name) => {
                    let Some(&s) = c.notice_sheets.get(&name) else {
                        continue;
                    };
                    s
                }
            };
            frame.draws.push((slot, quads));
        }
    }
    let Some(j) = &c.journal_source else { return };
    let s = &c.journal_sheets;
    if let Some(cur) = &c.curtain {
        frame.draws.push((s.curtain, cur.quads(&j.curtain.layout)));
    }
    if let Some(open) = &c.journal {
        let top = open.top.as_ref().and_then(|code| s.tops.get(code));
        for (sheet, quads) in open.draw(&j.book.layout, &font) {
            let slot = match sheet {
                journal::Sheet::Art => Some(s.book),
                journal::Sheet::Font => Some(c.font_sheet),
                journal::Sheet::Icon => Some(c.icon_sheet),
                journal::Sheet::World(i) => s.worlds.get(i).copied(),
                journal::Sheet::Top(i) => top.and_then(|t| t.get(i).copied()),
                journal::Sheet::Face(k, i) => {
                    s.faces.get(&k).and_then(|v| v.get(i as usize).copied())
                }
                journal::Sheet::Photo(r) => s.photos.get(&r).copied(),
            };
            if let Some(slot) = slot {
                frame.draws.push((slot, quads));
            }
        }
    }
}
/// Field (slot 0) and battle (slot 1) music (FUN_140155840 / FUN_140155c30)
/// and the battle state (FUN_1403ab6f0, DAT_142a11404) driving the switch.
#[derive(Resource)]
struct MusicRes {
    pending: [Option<sound::MusicSlot>; 2],
    clips: [Option<sound::Music>; 2],
    started: bool,
    battle: bool,
    counter: u32,
    since_reset: u32,
}

const MUSIC_KEYS: [u32; 2] = [9_000_000, 9_000_001];
/// BGM slot level 0x3000 / 16383.
const BGM_LEVEL: f32 = 0x3000 as f32 / 16383.;
/// Fade units are 1/240 s: 0xf0 = 1 s, 0x1e0 = 2 s, 0x2d0 = 3 s.
const BATTLE_IN: f32 = 1.;
const FIELD_PAUSE: f32 = 2.;
const BATTLE_OUT: f32 = 3.;
const FIELD_RESUME: f32 = 2.;
/// Battle when an enemy is within this horizontal distance; switch after
/// the target differs 45 frames (to battle) or 120 (to field), at once
/// in the first 30 frames after a reset.
const BATTLE_DISTANCE: f32 = 1200.;

fn music_tick(music: Option<ResMut<MusicRes>>, mut game: ResMut<Game>, audio: Res<Audio>) {
    let Some(mut m) = music else { return };
    let m = &mut *m;
    let tick = game.tick;
    for k in 0..2 {
        if let Some(slot) = &m.pending[k]
            && let Some(result) = slot.lock().ok().and_then(|mut r| r.take())
        {
            m.pending[k] = None;
            match result {
                Ok(music) => {
                    game.event(serde_json::json!({"event":"music_decoded","slot":k,"tick":tick,"stats":music.stats}));
                    m.clips[k] = Some(music);
                }
                Err(e) => game.event(
                    serde_json::json!({"event":"music_failed","slot":k,"tick":tick,"error":e}),
                ),
            }
        }
    }
    let Some(output) = &audio.output else { return };
    // Room entry: the field track at full level, no fade.
    if !m.started
        && let Some(field) = &m.clips[0]
    {
        if let Ok(mut mixer) = output.mixer.lock() {
            mixer.play(
                field.clip.clone(),
                MUSIC_KEYS[0],
                BGM_LEVEL * field.volume,
                1.,
            );
        }
        m.started = true;
        game.event(serde_json::json!({"event":"bgm_start","slot":0,"tick":tick}));
    }
    // FUN_1403ab6f0: the target state and its hysteresis.
    let p = game.player.position;
    let target = game.enemy.as_ref().is_some_and(|rt| {
        let e = rt.enemy.position;
        !matches!(
            rt.enemy.current_act(),
            Some(shadow::ActKind::Leave | shadow::ActKind::Dead)
        ) && ((e[0] - p[0]).powi(2) + (e[2] - p[2]).powi(2)).sqrt() < BATTLE_DISTANCE
    });
    m.since_reset = m.since_reset.saturating_add(1);
    if target != m.battle {
        m.counter += 1;
        let need = if target { 45 } else { 120 };
        if m.since_reset <= 30 || m.counter >= need {
            m.battle = target;
            m.counter = 0;
            let (Some(field), Some(battle)) = (&m.clips[0], &m.clips[1]) else {
                return;
            };
            if let Ok(mut mixer) = output.mixer.lock() {
                if target {
                    mixer.fade(MUSIC_KEYS[0], 0., FIELD_PAUSE, audio::FadeEnd::Pause);
                    mixer.stop(MUSIC_KEYS[1]);
                    mixer.play(battle.clip.clone(), MUSIC_KEYS[1], 0., 1.);
                    mixer.fade(
                        MUSIC_KEYS[1],
                        BGM_LEVEL * battle.volume,
                        BATTLE_IN,
                        audio::FadeEnd::Hold,
                    );
                } else {
                    mixer.fade(MUSIC_KEYS[1], 0., BATTLE_OUT, audio::FadeEnd::Stop);
                    mixer.resume(MUSIC_KEYS[0], BGM_LEVEL * field.volume, FIELD_RESUME);
                }
            }
            game.event(serde_json::json!({"event":"battle_music","tick":tick,"battle":target}));
        }
    } else {
        m.counter = 0;
    }
}
/// Double-click launch: with no arguments, read the three required paths
/// (and optional extra options) from a private launch file outside Git.
fn launch_file_args(name: &str) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    #[derive(Deserialize)]
    struct Launch {
        steam_root: PathBuf,
        manifest: PathBuf,
        config: PathBuf,
        #[serde(default)]
        options: Vec<String>,
    }
    // Search next to the checkout first (`.local/test-room/launch.json`, found
    // from the executable in `target/<profile>/`), then LOCALAPPDATA.
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        for up in exe.ancestors().skip(1).take(4) {
            candidates.push(up.join(format!(".local/test-room/{name}.json")));
        }
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates
            .push(PathBuf::from(local).join(format!("KH2FinalRewrite/test-room-{name}.json")));
    }
    let path = candidates.iter().find(|p| p.is_file()).ok_or_else(|| {
        format!(
            "no arguments and no launch file; looked in: {}",
            candidates
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("; ")
        )
    })?;
    let launch: Launch = serde_json::from_slice(&read_limited(path)?)?;
    let mut args = vec![launch.steam_root, launch.manifest, launch.config];
    args.extend(launch.options.into_iter().map(PathBuf::from));
    Ok(args)
}

fn run(mut args: Vec<PathBuf>) -> Result<AppExit, Box<dyn std::error::Error>> {
    // No arguments: the private launch file; `talk`: the talk-room one.
    if args.is_empty() {
        args = launch_file_args("launch")?;
    } else if args.len() == 1 && args[0].as_os_str() == "talk" {
        args = launch_file_args("launch-talk")?;
    }
    let started = Instant::now();
    if args.len() < 3 {
        return Err("usage: room <Steam-root> <manifest> <test-room-config> [--log new.jsonl] [--frames N] [--autoplay <new-capture-dir> | --ability-tour <new-capture-dir> | --gear-tour <new-capture-dir> | --enemy-tour <new-capture-dir>] [--enemy-audit <new-private-report.json>] [--gamepad] [--show-collision] [--basic-fk]".into());
    }
    let mut log_path = None;
    let mut limit = None;
    let mut autoplay = None;
    let mut show_collision = false;
    let mut basic_fk = false;
    let mut ability_tour = false;
    let mut gear_tour = false;
    let mut gear_tour_quick = false;
    let mut enemy_tour = false;
    let mut enemy_battle_tour = false;
    let mut magic_tour = false;
    let mut magic_form = 0u8;
    let mut growth_tour = false;
    let mut prize_tour = false;
    let mut npc_tour = false;
    let mut badge_tour = false;
    let mut journal_tour = false;
    let mut game_over_tour = false;
    let mut enemy_audit = None;
    // Capture ticks replacing the unattended run's own list (research).
    let mut capture_at: Option<Vec<u64>> = None;
    let mut options = args[3..].iter();
    while let Some(option) = options.next() {
        match option.to_str() {
            // Pads are always read now; the flag is accepted for old launch files.
            Some("--gamepad") => {}
            Some("--show-collision") => show_collision = true,
            Some("--basic-fk") => basic_fk = true,
            Some("--enemy-audit") if enemy_audit.is_none() => {
                enemy_audit = Some(options.next().cloned().ok_or("enemy audit output absent")?);
            }
            Some("--log") if log_path.is_none() => log_path = options.next().cloned(),
            Some("--frames") if limit.is_none() => {
                limit = Some(
                    options
                        .next()
                        .and_then(|v| v.to_str())
                        .ok_or("frame limit absent")?
                        .parse::<u64>()?,
                )
            }
            Some("--autoplay") if autoplay.is_none() => autoplay = options.next().cloned(),
            Some("--capture-at") if capture_at.is_none() => {
                capture_at = Some(
                    options
                        .next()
                        .and_then(|v| v.to_str())
                        .ok_or("capture ticks absent")?
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<_, _>>()?,
                )
            }
            Some("--enemy-tour") if autoplay.is_none() => {
                enemy_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("enemy tour capture directory absent")?,
                );
            }
            Some("--enemy-combat-tour") if autoplay.is_none() => {
                enemy_battle_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("enemy combat tour capture directory absent")?,
                );
            }
            Some("--magic-form") => {
                magic_form = options
                    .next()
                    .and_then(|v| v.to_str())
                    .ok_or("magic form absent")?
                    .parse::<u8>()?;
            }
            Some("--magic-tour") if autoplay.is_none() => {
                magic_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("magic tour capture directory absent")?,
                );
            }
            Some(flag @ ("--gear-tour" | "--gear-tour-quick")) if autoplay.is_none() => {
                ability_tour = true;
                gear_tour = true;
                gear_tour_quick = flag == "--gear-tour-quick";
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("gear tour capture directory absent")?,
                );
            }
            Some("--game-over-tour") if autoplay.is_none() => {
                game_over_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("game over tour capture directory absent")?,
                );
            }
            Some("--npc-tour") if autoplay.is_none() => {
                npc_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("npc tour capture directory absent")?,
                );
            }
            Some("--journal-tour") if autoplay.is_none() => {
                ability_tour = true;
                journal_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("journal tour capture directory absent")?,
                );
            }
            Some("--badge-tour") if autoplay.is_none() => {
                ability_tour = true;
                badge_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("badge tour capture directory absent")?,
                );
            }
            Some("--prize-tour") if autoplay.is_none() => {
                prize_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("prize tour capture directory absent")?,
                );
            }
            Some("--growth-tour") if autoplay.is_none() => {
                growth_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("growth tour capture directory absent")?,
                );
            }
            Some("--ability-tour") if autoplay.is_none() => {
                ability_tour = true;
                autoplay = Some(
                    options
                        .next()
                        .cloned()
                        .ok_or("ability tour capture directory absent")?,
                );
            }
            _ => return Err("unknown/duplicate test room option".into()),
        }
    }
    let source = Arc::new(ProbeSource::bind_session(&args[0], &args[1])?);
    if let Some(path) = enemy_audit {
        ProbeSource::bind(&args[0], &args[1])?.verify_full_source()?;
        source.verify_session()?;
        let catalog = enemy_catalog::Catalog::load(&source)?;
        let path = source.output_path(&path)?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let report = catalog.audit(&source, basic_fk);
        serde_json::to_writer_pretty(&mut file, &report)?;
        file.write_all(b"\n")?;
        ProbeSource::bind(&args[0], &args[1])?.verify_full_source()?;
        eprintln!("Enemy corpus inspection complete; see retained per-entry verdicts.");
        return Ok(AppExit::Success);
    }
    let board_source = source.clone();
    // Validate the log path now; the file is created only after admission.
    let log_path = match log_path {
        Some(path) => {
            let path = source.output_path(&path)?;
            if path.exists() {
                return Err("test room log must be new".into());
            }
            Some(path)
        }
        None => None,
    };
    let autoplay = match autoplay {
        Some(dir) => {
            // Guard a file inside the directory: the directory itself exists.
            source.output_path(&dir.join("test-room-00.png"))?;
            if !dir.is_dir() || std::fs::read_dir(&dir)?.next().is_some() {
                return Err("autoplay capture directory must exist and be empty".into());
            }
            Some(dir)
        }
        None => None,
    };
    eprintln!("Validating the selected KH2FM source, then decoding the test room...");
    let mut loaded = load(&source, &args[2], basic_fk)?;
    let catalog = enemy_catalog::Catalog::load(&source)?;
    eprintln!(
        "Enemy catalog: {} entries, {} unclassified candidates, {} classification issues",
        catalog.entries.len(),
        catalog
            .entries
            .iter()
            .filter(|e| e.category == enemy_catalog::Category::Unresolved)
            .count(),
        catalog.issues.len()
    );
    let mut gallery = enemy_gallery::EnemyGallery::new(source.clone(), catalog, basic_fk);
    gallery.target_params = Some(enemy_battle::target_params(&source)?);
    gallery.lock_range = Some(enemy_battle::lock_range(&source)?);
    eprintln!(
        "Decoded in {:.0} ms.",
        started.elapsed().as_secs_f64() * 1e3
    );
    // Beside the room's configuration; unattended runs keep the defaults.
    let settings = settings_room::SettingsRes::new(
        source.clone(),
        autoplay.is_none().then(|| {
            args[2]
                .parent()
                .unwrap_or(Path::new("."))
                .join("kh2fm-settings.json")
        }),
        loaded.language.take(),
    )?;
    let log = match log_path {
        Some(path) => Some(OpenOptions::new().write(true).create_new(true).open(path)?),
        None => None,
    };
    let mut poses = HashMap::new();
    for (action, clip) in &loaded.kit.actions {
        poses.insert(*action, *clip);
    }
    let stay = loaded.stay;
    let step_bank = loaded
        .area
        .as_ref()
        .and_then(|(_, a)| step_room_bank(&loaded.step_info, a.world, a.area));
    let ambient = loaded
        .area
        .as_ref()
        .and_then(|(_, a)| room_ambient(&loaded.step_info, a.world, a.area));
    let area_res = loaded.area.map(|(source, mut current)| AreaRes {
        music: Some(std::mem::take(&mut current.music)),
        rendered: current.npcs.len(),
        bg_mask: current.state.setup.bg_mask,
        banner: source
            .place_name(current.world, current.area)
            .map(|t| (t.clone(), 120)),
        guide: None,
        guide_hold: 0,
        visited: std::iter::once((current.world, current.area)).collect(),
        source,
        current,
        previous: None,
        pending: None,
        transition: None,
        explore: !stay,
        talk_npc: None,
        was_talking: false,
    });
    let talk_room = loaded.npc.is_some();
    let mut player = Player::new(loaded.spawn.to_array(), loaded.yaw);
    let equipment = room_equipment();
    let ability_slots = loaded.abilities.inventory.slots.clone();
    let tour = if ability_tour {
        let camp = loaded
            .camp
            .as_ref()
            .ok_or("ability tour requires camp assets")?;
        Some(if journal_tour {
            journal_tour::tour(&loaded.abilities.inventory)
        } else if badge_tour {
            prize_room::badge_tour(&loaded.abilities.inventory)
        } else if gear_tour {
            gear_tour::tour(
                &camp.items.table,
                &loaded.abilities.owned,
                loaded.abilities.inventory.slots.clone(),
                gear_tour_quick,
            )
        } else {
            ability_inventory::Tour::new(&camp.items.table, &loaded.abilities.inventory)
        })
    } else {
        None
    };
    // A form's change takes about 260 ticks before the first cast.
    let magic_shift = if magic_form != 0 { 260 } else { 0 };
    let captures = if journal_tour {
        journal_tour::CAPTURES.to_vec()
    } else if npc_tour {
        NPC_TOUR_CAPTURES.to_vec()
    } else if magic_tour {
        magic_tour::captures(magic_shift)
    } else if prize_tour {
        prize_room::CAPTURES.to_vec()
    } else if growth_tour {
        level_growth::CAPTURES.to_vec()
    } else if game_over_tour {
        game_over::CAPTURES.to_vec()
    } else if enemy_battle_tour {
        enemy_battle_tour::CAPTURES.to_vec()
    } else if enemy_tour {
        enemy_tour::CAPTURES.to_vec()
    } else {
        tour.as_ref()
            .map(|t| t.captures.clone())
            .unwrap_or_else(|| {
                if talk_room {
                    AUTOPLAY_TALK_CAPTURES.to_vec()
                } else {
                    capture_at
                        .clone()
                        .unwrap_or_else(|| AUTOPLAY_CAPTURES.to_vec())
                }
            })
    };
    let autoplay_end = if journal_tour {
        journal_tour::END
    } else if npc_tour {
        NPC_TOUR_END
    } else if magic_tour {
        magic_tour::END + magic_shift
    } else if prize_tour {
        prize_room::END
    } else if growth_tour {
        level_growth::END
    } else if game_over_tour {
        game_over::END
    } else if enemy_battle_tour {
        enemy_battle_tour::END
    } else if enemy_tour {
        enemy_tour::END
    } else {
        tour.as_ref().map_or(
            if talk_room {
                AUTOPLAY_TALK_END
            } else if let Some(last) = capture_at.as_ref().and_then(|c| c.last()) {
                last + 2
            } else {
                AUTOPLAY_END
            },
            |t| t.end,
        )
    };
    player.abilities = loaded.abilities.build(&ability_slots, &equipment);
    player.fmab = std::mem::take(&mut loaded.fmab);
    if autoplay.is_some() {
        // The unattended run strikes Sora with blows and recovers with
        // Retaliating Slash (ability bit 101), which is unequipped by
        // default.
        player.abilities.set(101);
    }
    if let Some(h) = &loaded.hud {
        player.mp = magic::Mp::new(h.stats.1);
    }
    let spell_fx_resource =
        spell_fx::SpellFx::new(loaded.magic.as_ref().and_then(|m| m.texcommon.clone()));
    // The room's stock with every Keyblade, armor and accessory it owns.
    let owned_stock = gear::with_owned_equipment(room_stock(), &loaded.abilities.owned);
    let mut game = Game {
        ability_source: loaded.abilities,
        equipment,
        gear: player::equipment_stats::Totals::start(0, 0, 0, 0),
        ability_slots,
        ability_tour: tour,
        enemy_tour,
        enemy_battle_tour,
        magic_tour,
        magic_form,
        magic_shift,
        growth_tour,
        prize_tour,
        npc_tour,
        badge_tour,
        journal_tour,
        hero_hit_tick: None,
        enemy_incoming_hits: 0,
        enemy_outgoing_hits: 0,
        lock_queued: false,
        cycle_queued: false,
        selected_target: None,
        target_locked: false,
        player_target_shape: loaded.kit.target_shape,
        params: loaded.params,
        clips: loaded.kit.clips,
        world: loaded.world,
        player,
        poses,
        clip_poses: loaded.kit.clip_poses,
        part_indices: Vec::new(),
        display_frame: 0,
        tick: 0,
        log,
        limit,
        autoplay,
        captures_taken: 0,
        capture_pending: false,
        pad: Pad::default(),
        pad_data: controller::PadData::default(),
        jump_queued: false,
        attack_queued: false,
        guard_queued: false,
        shortcut_queued: 0,
        back_queued: false,
        debug_blow: false,
        cheats: debug_menu::Cheats {
            invincible: magic_tour,
            enemy_frozen: magic_tour,
        },
        shortcuts: ROOM_SHORTCUTS,
        drive: None,
        voices: loaded.voices.take(),
        step_bank,
        ambient,
        step_info: loaded.step_info.take(),
        steps: visual::footsteps::Steps::default(),
        step_scan: None,
        map_pax: None,
        dust_scan: None,
        move_se_scan: None,
        sora_attack: std::mem::take(&mut loaded.kit.attack),
        party: loaded
            .party
            .into_iter()
            .enumerate()
            .map(|(i, kind)| party_room::MemberRt {
                hp: kind.hp,
                max_hp: kind.hp,
                mp: player::magic::Mp::new(kind.mp),
                kind,
                follower: player::party_members::Follower::spawn(
                    loaded.spawn.to_array(),
                    loaded.yaw,
                    i as u8,
                ),
                started: 0,
                stored: false,
                cast: None,
                casts: 0,
                armed: false,
                forced: None,
            })
            .collect(),
        items: loaded.items.map(|src| {
            item_room::ItemRt::new(
                src,
                ROOM_ITEM_SLOTS.to_vec(),
                ROOM_ITEM_SLOTS.to_vec(),
                owned_stock,
            )
        }),
        magic: loaded.magic.map(|src| {
            let mut levels = [0u8; 6];
            for (t, item) in ELEMENT_ITEMS.iter().enumerate() {
                levels[t] = ROOM_STOCK
                    .iter()
                    .find(|(i, _)| i == item)
                    .map_or(0, |&(_, n)| n);
            }
            magic_room::MagicRt::new(src, levels)
        }),
        menu_up_queued: false,
        menu_down_queued: false,
        enemy: None,
        display: Vec::new(),
        blend_from: None,
        display_joints: Vec::new(),
        blend_from_joints: Vec::new(),
        display_hand: Transform::IDENTITY,
        weapon_indices: Vec::new(),
        weapon2_indices: Vec::new(),
        weapon_pax: None,
        weapon_pax_changed: false,
        weapon_display: Vec::new(),
        fx_calls: Vec::new(),
        party_spells: true,
        fx_live: HashMap::new(),
        cure_target: 0,
        cure_pick: None,
        shown_action: None,
        action_started: 0,
        exit_code: None,
        show_collision,
        rendered_frames: 0,
        autoplay_failed: false,
        sound_queue: Vec::new(),
        sound_frame: None,
        effect_queue: Vec::new(),
        fx_queue: Vec::new(),
        actor_poses: HashMap::new(),
        actor_scan: HashMap::new(),
        trail_scan: None,
        trail_length: None,
        fx_clear_map: Vec::new(),
        map_fx_pending: true,
        sound_stops: Vec::new(),
        talking: false,
        rc_queued: false,
        advance_queued: false,
        captures,
        autoplay_end,
    };
    game.event(loaded.summary);
    game.refresh_gear();
    game.event(serde_json::json!({"event":"ability_inventory","slots":game.ability_slots,
        "ap_budget":game.ability_source.inventory.ap,"growth_tiers":game.ability_source.inventory.growth}));
    let drop_view = item_drop_view::load(&source);
    let mut drive_fx = drive_fx::DriveFx::default();
    if let Some(mut src) = loaded.forms.take() {
        drive_fx = drive_fx::DriveFx::new(src.texcommon.take(), src.prize_pax.take());
        let mut d = drive_room::DriveRt::new(src, Arc::clone(&source), loaded.enemy_character);
        if let Some(p) = loaded.kit.pax.take() {
            d.paxes.insert(0, Arc::new(p));
        }
        game.player.drive.max = drive_room::ROOM_DRIVE_MAX;
        // Test-room choice: the gauge starts full (FUN_1403c2020), so every
        // owned form can be tried at once; unattended runs keep it empty.
        if game.autoplay.is_none() {
            game.player.drive.refill();
        }
        game.event(serde_json::json!({"event":"drive_forms","form_objects":d.src.form_objects,
            "commands":d.src.form_command,"owned":d.forms.owned,
            "records":d.forms.records.iter().map(|r| serde_json::json!({"level":r.level,"keyblade":r.keyblade,
                "abilities":r.abilities})).collect::<Vec<_>>(),"enemy_character":loaded.enemy_character}));
        game.drive = Some(d);
    }
    let output = if loaded.banks.numbers().is_empty() {
        None
    } else {
        match audio::Output::open() {
            Ok(output) => Some(output),
            Err(error) => {
                eprintln!("Sound output unavailable: {error}");
                None
            }
        }
    };
    game.event(
        serde_json::json!({"event":"audio","banks":loaded.banks.numbers(),
        "output":output.is_some(),"rate":audio::RATE}),
    );
    let audio = Audio {
        output,
        banks: loaded.banks,
        alarm: false,
        ambient: None,
    };
    let (talk, npc_model) = match loaded.npc {
        Some(n) => {
            let position = loaded.spawn + n.offset;
            game.event(serde_json::json!({"event":"npc","position":position.to_array(),"yaw":n.yaw,
                "frames":n.frames.len(),"target_shape":format!("{:?}", n.target),
                "lines":n.lines.iter().map(|(k, v)| (format!("{k:#x}"), v.len())).collect::<HashMap<_, _>>(),
                "text_colour":format!("{:#010x}", n.colour),"window_texel_scale":n.sheet.texel_scale,
                "font_texel_scale":n.font.texel_scale}));
            (
                Some(Talk {
                    position,
                    yaw: n.yaw,
                    home_yaw: n.yaw,
                    tick: 0,
                    frames: n.frames,
                    bones: n.bones,
                    loop_start: n.loop_start,
                    target: n.target,
                    indices: Vec::new(),
                    sheet: n.sheet,
                    font: n.font,
                    lines: n.lines,
                    colour: n.colour,
                    label: n.label,
                    script: n.script,
                    active: None,
                    current: None,
                    closing: Vec::new(),
                    wait: 0.,
                    camera: None,
                    dissolve: 0,
                    snapshot: false,
                    reaction: None,
                }),
                Some(n.model),
            )
        }
        None => (
            loaded.area_talk.map(|t| Talk {
                // Pointed at the nearest townsperson by field_npc_tick; far
                // away until then so no reaction shows.
                position: Vec3::splat(1.0e9),
                yaw: 0.,
                home_yaw: 0.,
                tick: 0,
                frames: vec![Vec::new()],
                bones: vec![HashMap::new()],
                loop_start: 0,
                target: Shape {
                    bone: 0,
                    offset: Vec3::ZERO,
                    pos_h: 0.,
                    radius: 0.,
                    height: 0.,
                },
                indices: Vec::new(),
                sheet: t.sheet,
                font: t.font,
                lines: HashMap::new(),
                colour: t.colour,
                label: t.label,
                script: Vec::new(),
                active: None,
                current: None,
                closing: Vec::new(),
                wait: 0.,
                camera: None,
                dissolve: 0,
                snapshot: false,
                reaction: None,
            }),
            None,
        ),
    };
    let (effects, effect_atlas) = match loaded.effects {
        Some(e) => {
            game.event(serde_json::json!({"event":"effects","weapon_pax":e.weapon.name,
                "weapon_elements":e.weapon.elements.len(),"enemy_pax":e.enemy.as_ref().map(|p| p.name.clone()),
                "atlas_size":e.atlas.size,"atlas_texels":e.atlas.texel_size,"atlas_hd":e.atlas.hd,
                "atlas_alignment":e.atlas.alignment}));
            let size = if e.atlas.hd {
                e.atlas.size
            } else {
                e.atlas.texel_size
            };
            (
                Effects {
                    weapon: Some(Arc::new(e.weapon)),
                    enemy: e.enemy.map(Arc::new),
                    enemy_paxes: e
                        .enemy_paxes
                        .into_iter()
                        .map(|(id, p)| (id, Arc::new(p)))
                        .collect(),
                    system: e.system.map(Arc::new),
                    running: Vec::new(),
                    rng: ::effect::Rng::default(),
                    quads: Vec::new(),
                    view_inverse: Mat4::IDENTITY,
                    uv_scale: [1. / size[0] as f32, 1. / size[1] as f32],
                    reported: Vec::new(),
                },
                Some(e.atlas),
            )
        }
        None => (
            Effects {
                weapon: None,
                enemy: None,
                enemy_paxes: HashMap::new(),
                system: None,
                running: Vec::new(),
                rng: ::effect::Rng::default(),
                quads: Vec::new(),
                view_inverse: Mat4::IDENTITY,
                uv_scale: [1.; 2],
                reported: Vec::new(),
            },
            None,
        ),
    };
    let mut enemy_model = None;
    if let Some(e) = loaded.enemy {
        let spawn = loaded.spawn + e.offset;
        let to_sora = loaded.spawn - spawn;
        let enemy = Shadow::new(
            spawn.to_array(),
            to_sora.x.atan2(to_sora.z),
            e.stats,
            loaded.sstm,
        );
        game.event(serde_json::json!({"event":"enemy","hp":e.stats.max_hp,"defense":e.stats.defense,
            "damage_cap":e.stats.cap,"damage_min":e.stats.min,"sora_strength":e.sora.strength,
            "hit_spheres":e.body.len(),"blade_groups":e.blade.keys().collect::<Vec<_>>(),"position":spawn,
            "claw_groups":e.claws.iter().map(|(g, v)| (g.to_string(), v.len())).collect::<HashMap<_, _>>(),
            "claw_windows":e.clips.0.iter().filter(|(_, c)| !c.windows.is_empty()).map(|(s, c)| (s.to_string(), c.windows.clone())).collect::<HashMap<_, _>>(),
            "sora_hit_spheres":e.sora_body.len(),"target_reach":e.target_reach,"sora_target":format!("{:?}", e.sora_target),"shadow_attacker":format!("{:?}", e.shadow_attacker)}));
        enemy_model = Some(e.model);
        game.enemy = Some(EnemyRuntime {
            sora_target_radius: e.sora_target_radius,
            enabled: !game.enemy_battle_tour,
            battle_level: e.battle_level,
            clips: e.clips,
            body: e.body,
            targets: e.targets,
            blade: [e.blade.clone(), e.blade],
            steps: e.steps,
            atkp: e.atkp,
            difficulty: e.difficulty,
            sora: e.sora,
            enemy,
            rng: ScriptRandom::default(),
            claws: e.claws,
            shadow_attacker: e.shadow_attacker,
            sora_target: e.sora_target,
            sora_body: e.sora_body,
            target_reach: e.target_reach,
            sora_record: combat::HitRecord::new(true),
            ids: combat::AttackIds::default(),
            instance_ids: HashMap::new(),
            instance_seen: HashMap::new(),
            next_group: 0x100,
            claw_previous: HashMap::new(),
            sora_hp: e.sora_target.max_hp,
            ko_ticks: 0,
            exp_rule: e.exp,
            prize: e.prize,
            sora_levels: e.sora_levels,
            level: 1,
            exp_total: 0,
            munny: 0,
            orbs: Vec::new(),
            drops: Vec::new(),
            next_drop: 0,
            was_dead: false,
            learned: Vec::new(),
            numbers_dirty: false,
            panels: Vec::new(),
            level_effect: false,
            heal_ticks: 0.,
            bonus_taken: Vec::new(),
            previous: HashMap::new(),
            indices: Vec::new(),
            hit_sounds: e.hit_sounds,
            hit_effects: e.hit_effects,
            rows: e.rows,
            sora_invulnerable: 0.,
            sound_tick: None,
        });
        // The combat numbers start without the equipment's share: add it.
        game.gear = player::equipment_stats::Totals::start(0, 0, 0, 0);
        game.refresh_gear();
    }
    let rig = CameraRig {
        camera: FollowCamera::new(
            follow_camera::OUTDOOR,
            &camera_input(&game.player, game.player.position, game.params.run_speed),
        ),
        previous_position: game.player.position,
        pending_turn: 0.,
        recenter_queued: false,
        collision_hits: 0,
        collision: CameraCollision::new(&loaded.room),
    };
    eprintln!(
        "Test room ready (the original PC bindings). WASD moves (Left Alt walks), Space/Backspace jump, Enter/left-click decides (Attack), Up/Down choose a command, right-click/Right Shift guards, F/Right Ctrl reacts (Talk), M opens the menu, the mouse turns the camera, middle-click/Right Alt recentres, F3 opens the debug menu, Esc exits."
    );
    let mut app = App::new();
    if let Some(area) = area_res {
        app.insert_resource(area);
    }
    if let Some(talk) = talk {
        app.insert_resource(talk);
    }
    if let Some(music) = loaded.music {
        let pending = music.map(|bytes| Some(sound::decode_music_in_background(bytes)));
        app.insert_resource(MusicRes {
            pending,
            clips: [None, None],
            started: false,
            battle: false,
            counter: 0,
            since_reset: 0,
        });
    }
    if let Some(mut c) = loaded.camp {
        // Development grant: keep native AP costs/toggles, with enough AP
        // to try every catalog entry. This is not campaign progression.
        c.numbers.ap = c.numbers.ap.max(game.ability_source.inventory.ap);
        game.event(serde_json::json!({"event":"camp_loaded","props":c.menu.layout.props.len(),
            "scenes":c.menu.layout.scenes.len(),"sequences":c.menu.layout.sequences.len(),
            "replacements":c.menu.replacements.iter().map(|r| r.as_ref().map(|t| t.size)).collect::<Vec<_>>(),
            "entries":c.entries.len(),"stats":c.stats.len(),"config_rows":c.config.len(),
            "picture_groups":c.picture_sequence.as_ref().map(|s| s.groups.len())}));
        app.insert_resource(CampRes {
            source: c,
            open: None,
            memory: pause_menu::Memory::default(),
            // A new game's config (FUN_1402e4fc0) and Standard mode
            // (FUN_1401523a0) [test-room save].
            config_flags: 0x40b,
            game_mode: 1,
            pad: Pad::default(),
            art: [None; 4],
            font_sheet: 0,
            icon_sheet: 0,
            glyph: 0,
            keyboard_last: true,
            stock: gear::with_owned_equipment(room_stock(), &game.ability_source.owned),
            starting: gear::with_owned_equipment(room_stock(), &game.ability_source.owned)
                .into_keys()
                .collect(),
            seen: Vec::new(),
            shortcuts: ROOM_SHORTCUTS,
            no_charge: 0,
            equipment: room_equipment(),
            logged_equipment: None,
            ability_slots: game.ability_slots.clone(),
            journal_source: loaded.journal,
            journal: None,
            curtain: None,
            journal_sheets: JournalSheets::default(),
            picture_source: Some(source),
            seen_chars: Vec::new(),
            seen_albums: Vec::new(),
            seen_limits: Vec::new(),
            seen_quests: Vec::new(),
            seen_minigames: Vec::new(),
            seen_story: Vec::new(),
            seen_synthesis: Vec::new(),
            picture_sheets: HashMap::new(),
            notice_sheets: HashMap::new(),
            lang: settings.loaded_or("us"),
        });
    }
    if let Some(src) = loaded.game_over.take() {
        app.insert_resource(game_over::GameOverRes::new(
            src,
            (loaded.spawn.to_array(), loaded.yaw),
            game_over_tour,
        ));
    }
    Ok(app
        .insert_resource(game)
        .insert_resource(Startup {
            room: Some(loaded.room),
            arena_mesh: loaded.arena_mesh,
            actor: Some(loaded.kit.actor),
            weapon: Some(loaded.kit.weapon),
            hud: loaded.hud,
            enemy_model,
            effect_atlas,
            npc_model,
        })
        .insert_resource(rig)
        .insert_resource(FrameStats {
            started,
            first_frame: false,
            frames: 0,
            sum: 0.,
            worst: 0.,
            slow: 0,
        })
        .insert_resource(BoardModels {
            keyblades: loaded.keyblades,
            hand: loaded.weapon_model,
            source: board_source,
            motion: loaded.board_motion,
            models: HashMap::new(),
            shown: None,
        })
        .insert_resource(audio)
        .insert_resource(settings)
        .insert_resource(effects)
        .insert_resource(drive_fx)
        .insert_resource(spell_fx_resource)
        .insert_resource(drop_view)
        .insert_resource(gallery)
        .init_resource::<enemy_target::Targeting>()
        .insert_resource(Time::<Fixed>::from_hz(60.))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Kingdom Hearts II Re:Written (test room; quarantined source) | WASD move, Space jump, Enter/LMB attack, RMB guard, F talk, M menu, F3 debug, Esc".into(),
                resolution: (1280, 720).into(),
                ..default()
            }),
            // Closing the window goes through finish() so the exit code and
            // log record reflect an unfinished unattended run.
            exit_condition: bevy::window::ExitCondition::DontExit,
            close_when_requested: false,
            ..default()
        }))
        .add_plugins(FogPlugin)
        .add_plugins(Ui2dPlugin {
            order: 0,
            overlay: false,
        })
        .init_resource::<crate::skate::Skate>()
        .init_resource::<debug_menu::DebugMenu>()
        .init_resource::<enemy_battle_tour::Checks>()
        .add_plugins(SkatePlugins)
        .add_systems(
            Startup,
            (setup, pause_for_autoplay, camp_setup, game_over::setup),
        )
        .add_systems(Update, crate::skate::toggle)
        .add_systems(Update, settings_room::apply_settings)
        .add_systems(
            FixedUpdate,
            (
                read_pad,
                camp_tick,
                game_over::tick,
                (
                    simulate,
                    skate_tick,
                    area_tick,
                    enemy_battle::tick,
                    enemy_gallery::tick,
                    enemy_battle_tour::tick,
                    enemy_tour::tick,
                    field_npc_tick,
                    talk_tick,
                    effects_tick,
                    drive_fx::tick,
                    spell_fx::tick,
                    hud_tick,
                    play_sounds,
                    music_tick,
                )
                    .chain()
                    .run_if(field_running),
            )
                .chain(),
        )
        .add_systems(
            Update,
            ((
                debug_menu::tick,
                enemy_gallery::service,
                area_transition,
                transition_time,
                bg_visibility,
                area_music,
                uv_scroll_tick,
                texa_tick,
                bob_present,
                field_npc_spawn_new,
                field_npc_present,
                dissolve_present,
                camera_control,
                game_over::camera,
                skate_camera,
            )
                .chain(),
            (
                drive_room::present,
                present,
                drive_room::thrown_keyblade,
                skate_present,
                present_enemy,
                (enemy_gallery::present, present_spells).chain(),
                party_room::present,
                drive_room::party_visibility,
                present_npc,
                present_effects,
                (drive_fx::present, spell_fx::present).chain(),
                item_drop_view::present,
                hud_present,
                resize_scene,
                afterimage.run_if(resource_exists::<Afterimage>),
                compose_2d,
                menu_model_present,
                (camp_present, game_over::present, game_over::snap).chain(),
                finish,
                frame_stats,
            )
                .chain(),
        )
            .chain(),
        )
        .run())
}

/// Runs this program with its arguments; `interactive` = started from the
/// launcher menu (keep the console open on an error).
pub fn main(args: Vec<PathBuf>, interactive: bool) -> std::process::ExitCode {
    match run(args) {
        Ok(AppExit::Success) => std::process::ExitCode::SUCCESS,
        Ok(_) => std::process::ExitCode::FAILURE,
        Err(e) => {
            eprintln!("Test room rejected: {e}");
            // Keep a double-clicked console open long enough to read the error.
            if interactive {
                eprintln!("Press Enter to close.");
                let _ = std::io::stdin().read_line(&mut String::new());
            }
            std::process::ExitCode::FAILURE
        }
    }
}
