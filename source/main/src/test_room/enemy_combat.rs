//! Combat assets from the selected original archives. Enemy parameter sheets
//! are supplied by a qualified script initialization, never guessed from names.
use super::enemy_catalog::{Entry, Kind};
use bevy::prelude::*;
use data::{
    bar, battle_tables, motion::SelectedMotion, object_collision, objentry, path_lookup::Library,
    probe_source::ProbeSource,
};
use player::{actor_physics, combat};
use std::collections::HashMap;

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug)]
pub struct EnemyBinding {
    pub enmp_id: u16,
    /// A source script's scatter-prize argument, when established. None uses
    /// the ENMP prize field; zero means no scatter rather than a fake drop.
    pub prize_id: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct HitShape {
    pub ordinal: usize,
    /// Record group byte (the lock reference's key for the camera anchor).
    pub group: u8,
    pub bone: usize,
    pub offset: Vec3,
    pub position_height: f32,
    pub radius: f32,
    pub height: f32,
    pub shape: u8,
    pub parts: u8,
    pub high: bool,
}

#[derive(Clone, Debug)]
pub struct WorldShape {
    pub ordinal: usize,
    pub group: u8,
    pub centre: Vec3,
    pub radius: f32,
    pub height: f32,
    pub shape: u8,
    pub parts: u8,
    pub high: bool,
}

pub struct ShapePose {
    pub camera: Vec<WorldShape>,
    /// Live kind-18 camera-collision spheres (see `camera_spheres`).
    pub spheres: Vec<WorldShape>,
    pub body: Vec<WorldShape>,
    pub targets: Vec<WorldShape>,
    pub attacks: HashMap<u8, Vec<WorldShape>>,
}

#[derive(Clone, Copy, Debug)]
pub struct AttackWindow {
    pub start: f32,
    pub end: f32,
    pub id: u16,
    pub group: u8,
}
impl AttackWindow {
    /// Native range dispatch uses a half-open interval. Stored key time and
    /// the event clock are separate; the caller supplies the event clock.
    pub fn active(self, frame: f32) -> bool {
        self.start <= frame && frame < self.end
    }
}

pub struct EnemyCombat {
    pub camera: Vec<HitShape>,
    /// Kind-18 shapes the camera ray is cut at (FUN_1403c9c00 via
    /// FUN_140167c20).
    pub spheres: Vec<HitShape>,
    pub object_id: u32,
    pub character: u16,
    pub enmp_id: u16,
    pub level: u16,
    pub stats: combat::TargetStats,
    pub attacker: combat::AttackerStats,
    pub sstm: actor_physics::Sstm,
    pub difficulty: combat::Difficulty,
    /// Global native manual/cycle lock range, SSTM record +0x4c.
    pub lock_range: f32,
    pub body: Vec<HitShape>,
    pub targets: Vec<HitShape>,
    pub attacks: HashMap<u8, Vec<HitShape>>,
    pub exp: (u16, u16),
    pub prize: Option<battle_tables::Prize>,
    atkp: Vec<u8>,
}

fn payload<'a>(bytes: &'a [u8], name: &[u8; 4]) -> Result<&'a [u8]> {
    let table = bar::read_bar(bytes).map_err(|e| e.to_string())?;
    let entries: Vec<_> = table
        .entries
        .iter()
        .filter(|e| &e.name_bytes == name && e.size != 0)
        .collect();
    if entries.len() != 1 {
        return Err(format!(
            "combat BAR entry {} missing or ambiguous",
            String::from_utf8_lossy(name)
        ));
    }
    entry_payload(bytes, entries[0])
}
fn entry_payload<'a>(bytes: &'a [u8], entry: &bar::BarEntry) -> Result<&'a [u8]> {
    let start = usize::try_from(entry.offset).map_err(|_| "negative combat BAR payload")?;
    let end = start
        .checked_add(entry.size as usize)
        .ok_or("combat BAR range overflow")?;
    bytes
        .get(start..end)
        .ok_or_else(|| "combat BAR payload outside archive".into())
}

impl EnemyCombat {
    pub fn summary(&self) -> serde_json::Value {
        serde_json::json!({
            "object_id":self.object_id,"character":self.character,"enmp":self.enmp_id,
            "level":self.level,"difficulty":format!("{:?}",self.difficulty),
            "hp":self.stats.max_hp,"defense":self.stats.defense,
            "damage_cap":self.stats.cap,"damage_min":self.stats.min,
            "weakness":self.stats.weakness,"strength":self.attacker.strength,
            "body_shapes":self.body.len(),"target_shapes":self.targets.len(),
            "camera_shapes":self.camera.len(),
            "attack_shapes":self.attacks.values().map(Vec::len).sum::<usize>(),
            "exp":self.exp,"has_prize":self.prize.is_some(),
            "sstm_fall_max":self.sstm.fall_max,
            "lock_range":self.lock_range,
        })
    }

    pub fn load(
        source: &ProbeSource,
        entry: &Entry,
        kind: &Kind,
        binding: EnemyBinding,
        battle_level: u16,
        difficulty: combat::Difficulty,
    ) -> Result<Self> {
        source.verify_session().map_err(|e| e.to_string())?;
        let mut library = Library::open(source).map_err(|e| e.to_string())?;
        let objects = objentry::read(&library.read("00objentry.bin").map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let object = objects
            .iter()
            .find(|o| o.object_id == entry.object_id)
            .ok_or("combat object id absent")?;
        if object.model_name != entry.model_name.as_bytes()
            || object.character != entry.character
            || object.animation_name != entry.animation_name.as_bytes()
        {
            return Err("combat object differs from selected source catalog".into());
        }
        let archive = library
            .read(&format!("obj/{}.mdlx", entry.model_name))
            .map_err(|e| e.to_string())?;
        let table = bar::read_bar(&archive).map_err(|e| e.to_string())?;
        let collision_entries: Vec<_> = table
            .entries
            .iter()
            .filter(|e| e.entry_type == 23 && e.size != 0)
            .collect();
        if collision_entries.len() != 1 {
            return Err("combat collision entry absent or ambiguous".into());
        }
        let collision = object_collision::read(entry_payload(&archive, collision_entries[0])?)
            .map_err(|e| e.to_string())?;
        let (body, targets, attacks) =
            collision_shapes(&collision, kind.model.skinning.bones.len())?;
        let camera_entries = collision
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == 7 && e.bone >= 0)
            .collect::<Vec<_>>();
        // Source order is kept; the lock anchor picks the record whose group
        // equals the held TARGET record's group.
        let camera = camera_entries
            .into_iter()
            .map(|(ordinal, e)| {
                let bone = e.bone as usize;
                if bone != 0x4000 && bone >= kind.model.skinning.bones.len() {
                    return Err("camera anchor references an absent bone".into());
                }
                Ok(HitShape {
                    ordinal,
                    group: e.group,
                    bone,
                    offset: Vec3::from_array(e.position.map(f32::from)),
                    position_height: e.position_height as f32,
                    radius: e.radius as f32,
                    height: e.height as f32,
                    shape: e.shape,
                    parts: e.parts,
                    high: false,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let spheres = camera_spheres(&collision);
        let battle = library.read("00battle.bin").map_err(|e| e.to_string())?;
        let enemy = battle_tables::enemy(payload(&battle, b"enmp")?, binding.enmp_id)
            .map_err(|e| e.to_string())?;
        let level = if enemy.level == 0 {
            battle_level
        } else {
            enemy.level
        };
        let row = battle_tables::enemy_level(payload(&battle, b"lvpm")?, level)
            .map_err(|e| e.to_string())?;
        let stats = combat::TargetStats::enemy(
            enemy.hp_percent,
            enemy.damage_cap,
            enemy.damage_min,
            // The installed sheet initializer reads only the low byte of
            // each preserved u16 ENMP weakness field.
            enemy.weakness.map(|value| value & 0xff),
            row.hp,
            row.defense,
        );
        if stats.max_hp <= 0 {
            return Err(
                "enemy primary HP pool is empty; multipart/script combat unsupported".into(),
            );
        }
        let system = library.read("03system.bin").map_err(|e| e.to_string())?;
        let sstm_bytes = payload(payload(&system, b"pref")?, b"sstm")?;
        let sstm_offset = u32::from_le_bytes(
            sstm_bytes
                .get(4..8)
                .ok_or("combat SSTM pointer absent")?
                .try_into()
                .map_err(|_| "combat SSTM pointer invalid")?,
        ) as usize;
        let sstm = sstm_bytes
            .get(sstm_offset..)
            .and_then(actor_physics::Sstm::read)
            .ok_or("combat SSTM record truncated")?;
        let lock_at = sstm_offset
            .checked_add(0x4c)
            .ok_or("combat lock range offset overflow")?;
        let lock_range = f32::from_le_bytes(
            sstm_bytes
                .get(lock_at..lock_at + 4)
                .ok_or("combat SSTM lock range truncated")?
                .try_into()
                .map_err(|_| "combat lock range invalid")?,
        );
        if !lock_range.is_finite() || lock_range <= 0. {
            return Err("combat SSTM lock range must be finite and positive".into());
        }
        if [
            sstm.fall_max,
            sstm.blow_brake_xz,
            sstm.blow_min_xz,
            sstm.blow_brake_up,
            sstm.blow_up,
            sstm.blow_speed,
            sstm.blow_to_hit_back,
            sstm.hit_back,
            sstm.hit_back_small,
            sstm.hit_back_to_jump,
        ]
        .iter()
        .any(|v| !v.is_finite())
        {
            return Err("combat SSTM contains a nonfinite parameter".into());
        }
        let prize_id = binding.prize_id.unwrap_or(enemy.prize);
        let prize = if prize_id == 0 {
            None
        } else {
            Some(
                battle_tables::prize(payload(&battle, b"przt")?, prize_id)
                    .map_err(|e| e.to_string())?,
            )
        };
        let atkp = payload(&battle, b"atkp")?.to_vec();
        source.verify_session().map_err(|e| e.to_string())?;
        Ok(Self {
            camera,
            spheres,
            object_id: object.object_id,
            character: object.character,
            enmp_id: binding.enmp_id,
            level,
            stats,
            attacker: combat::AttackerStats {
                strength: row.strength as i32,
                magic: row.strength as i32,
                max: row.max_strength as i32,
                min: row.min_strength as i32,
                ..Default::default()
            },
            sstm,
            difficulty,
            lock_range,
            body,
            targets,
            attacks,
            exp: (enemy.exp, row.exp),
            prize,
            atkp,
        })
    }

    pub fn attack_steps(&self, id: u16, sub: u8) -> Result<Vec<battle_tables::Attack>> {
        battle_tables::attack_steps_sub(&self.atkp, id, sub).map_err(|e| e.to_string())
    }

    pub fn windows(&self, motion: &SelectedMotion) -> Result<Vec<AttackWindow>> {
        let mut windows = Vec::new();
        for range in motion
            .triggers
            .iter()
            .flat_map(|t| &t.records.ranges)
            .filter(|r| r.trigger == 10)
        {
            if range.parameters.len() != 2 {
                return Err("attack window parameter layout unsupported".into());
            }
            let id = range.parameters[0] as u16;
            let group = u8::try_from(range.parameters[1])
                .map_err(|_| "attack collision group outside byte range")?;
            if !self.attacks.contains_key(&group) {
                return Err(format!(
                    "source attack window group {group} has no supported collision shapes"
                ));
            }
            self.attack_steps(id, 0)?;
            let start = range.start_frame as u16 as f32;
            let end = range.end_frame as u16 as f32;
            if !end.is_finite() || end < start {
                return Err("attack window frame interval invalid".into());
            }
            windows.push(AttackWindow {
                start,
                end,
                id,
                group,
            });
        }
        Ok(windows)
    }

    /// Evaluate only the active pose. Root policy must match the mesh renderer;
    /// no full animation set's matrix/vertex frames are retained here.
    pub fn sample_shapes(
        &self,
        kind: &Kind,
        motion: &SelectedMotion,
        frame: f32,
        position: Vec3,
        yaw: f32,
        apply_root: bool,
    ) -> Result<ShapePose> {
        if !position.is_finite() || !yaw.is_finite() || !frame.is_finite() {
            return Err("combat pose inputs must be finite".into());
        }
        let (pose, evaluated) = kind
            .model
            .skinning
            .sample_evaluated(&motion.motion, frame)
            .map_err(|e| e.to_string())?;
        let matrices: Vec<_> = pose
            .matrix_bits
            .iter()
            .map(|m| Mat4::from_cols_array(&m.map(f32::from_bits)))
            .collect();
        let root = if apply_root {
            Vec3::ZERO
        } else {
            Vec3::from_array(evaluated.root_translation)
        };
        self.transform_shapes(&matrices, root, position, yaw)
    }

    pub fn transform_shapes(
        &self,
        matrices: &[Mat4],
        removed_root: Vec3,
        position: Vec3,
        yaw: f32,
    ) -> Result<ShapePose> {
        if !removed_root.is_finite() || !position.is_finite() || !yaw.is_finite() {
            return Err("combat transform inputs must be finite".into());
        }
        let turn = Quat::from_rotation_y(yaw);
        let transform = |shapes: &[HitShape]| -> Result<Vec<WorldShape>> {
            shapes
                .iter()
                .map(|shape| {
                    let matrix = if shape.bone == 0x4000 {
                        Mat4::IDENTITY
                    } else {
                        *matrices
                            .get(shape.bone)
                            .ok_or("combat shape references absent evaluated bone")?
                    };
                    if !matrix.is_finite() {
                        return Err("combat bone matrix nonfinite".into());
                    }
                    let root = if shape.bone == 0x4000 {
                        Vec3::ZERO
                    } else {
                        removed_root
                    };
                    let local = matrix.transform_point3(shape.offset) - root
                        + Vec3::Y * shape.position_height;
                    let centre = position + turn * local;
                    if !centre.is_finite() {
                        return Err("combat shape centre nonfinite".into());
                    }
                    Ok(WorldShape {
                        ordinal: shape.ordinal,
                        group: shape.group,
                        centre,
                        radius: shape.radius,
                        height: shape.height,
                        shape: shape.shape,
                        parts: shape.parts,
                        high: shape.high,
                    })
                })
                .collect()
        };
        Ok(ShapePose {
            camera: transform(&self.camera)?,
            spheres: transform(&self.spheres)?,
            body: transform(&self.body)?,
            targets: transform(&self.targets)?,
            attacks: self
                .attacks
                .iter()
                .map(|(&group, shapes)| Ok((group, transform(shapes)?)))
                .collect::<Result<_>>()?,
        })
    }
}

/// The kind-18 entries that are live without script help: FUN_1403c9c00
/// zeroes a sphere unless its bone field carries bit 14 (or the actor's flag
/// +0x9b8 bit 10 is set, which no supported actor sets), and only the plain
/// actor-origin attachment (0x4000) is modelled; the other 0x4001..0x4009
/// attachments need the executable's bone-pair anchors (FUN_1403b5450).
fn camera_spheres(entries: &[object_collision::Entry]) -> Vec<HitShape> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == 18 && e.bone == 0x4000 && e.radius > 0)
        .map(|(ordinal, e)| HitShape {
            ordinal,
            group: e.group,
            bone: 0x4000,
            offset: Vec3::from_array(e.position.map(f32::from)),
            position_height: e.position_height as f32,
            radius: e.radius as f32,
            height: e.height as f32,
            shape: e.shape,
            parts: e.parts,
            high: false,
        })
        .collect()
}

type CollisionShapes = (Vec<HitShape>, Vec<HitShape>, HashMap<u8, Vec<HitShape>>);

fn collision_shapes(entries: &[object_collision::Entry], bones: usize) -> Result<CollisionShapes> {
    let mut body = Vec::new();
    let mut targets = Vec::new();
    let mut attacks: HashMap<u8, Vec<HitShape>> = HashMap::new();
    for (ordinal, e) in entries.iter().enumerate().filter(|(_, e)| {
        matches!(
            e.kind,
            object_collision::HIT | object_collision::TARGET | object_collision::ATTACK
        )
    }) {
        if e.bone < 0 {
            continue;
        }
        let bone = e.bone as usize;
        if bone != 0x4000 && bone >= bones {
            return Err(format!("collision shape {ordinal} bone outside skeleton"));
        }
        if !matches!(e.shape, 0 | object_collision::SPHERE) || e.radius <= 0 || e.height <= 0 {
            return Err(format!("collision shape {ordinal} geometry unsupported"));
        }
        if e.parts != 0 {
            return Err(format!(
                "collision shape {ordinal} uses unimplemented HP pool {}",
                e.parts
            ));
        }
        if e.kind == object_collision::HIT && e.group != 0 {
            return Err(format!(
                "collision shape {ordinal} requires unresolved HIT-group activation"
            ));
        }
        let shape = HitShape {
            ordinal,
            group: e.group,
            bone,
            offset: Vec3::from_array(e.position.map(|n| n as f32)),
            position_height: e.position_height as f32,
            radius: e.radius as f32,
            height: e.height as f32,
            shape: e.shape,
            parts: e.parts,
            high: matches!(e.argument, 0 | 2 | 3 | 4 | 5),
        };
        match e.kind {
            object_collision::HIT => body.push(shape),
            object_collision::TARGET => targets.push(shape),
            _ => attacks.entry(e.group).or_default().push(shape),
        }
    }
    if body.is_empty() || targets.is_empty() {
        return Err("enemy has no enabled primary HIT/TARGET shapes".into());
    }
    Ok((body, targets, attacks))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(kind: u8, shape: u8) -> object_collision::Entry {
        object_collision::Entry {
            group: 0,
            parts: 0,
            argument: 1,
            kind,
            shape,
            bone: 0,
            position: [1, 2, 3],
            position_height: 0,
            radius: 20,
            height: 25,
        }
    }
    #[test]
    fn preserves_non_sphere_dimensions_and_rejects_unbound_combat_pools() {
        let mut entries = vec![
            entry(object_collision::HIT, 0),
            entry(object_collision::TARGET, 3),
        ];
        let (body, targets, _) = collision_shapes(&entries, 1).unwrap();
        assert_eq!(
            (body[0].shape, body[0].radius, body[0].height),
            (0, 20., 25.)
        );
        assert_eq!(targets.len(), 1);
        entries[0].parts = 1;
        assert!(collision_shapes(&entries, 1).is_err());
        entries[0].parts = 0;
        entries[0].bone = 1;
        assert!(collision_shapes(&entries, 1).is_err());
        entries[0].bone = 0;
        entries[0].shape = 2;
        assert!(collision_shapes(&entries, 1).is_err());
    }
    #[test]
    fn attack_window_is_half_open_and_rejects_nonfinite_time() {
        let w = AttackWindow {
            start: 4.,
            end: 8.,
            id: 1,
            group: 1,
        };
        assert!(!w.active(3.));
        assert!(w.active(4.));
        assert!(w.active(7.9));
        assert!(!w.active(8.));
        assert!(!w.active(f32::NAN));
    }

    #[test]
    fn collision_pose_tracks_bone_root_policy_and_actor_transform() {
        let (body, targets, attacks) = collision_shapes(
            &[
                entry(object_collision::HIT, 3),
                entry(object_collision::TARGET, 0),
            ],
            1,
        )
        .unwrap();
        // Only kind-18 entries attached to the actor origin (0x4000) are live;
        // a plain bone stays inert without the actor flag.
        let mut root_sphere = entry(18, 3);
        root_sphere.bone = 0x4000;
        root_sphere.position = [0, 0, 200];
        root_sphere.position_height = 7;
        let spheres = camera_spheres(&[root_sphere, entry(18, 3), entry(object_collision::HIT, 3)]);
        assert_eq!(spheres.len(), 1);
        let data = EnemyCombat {
            camera: Vec::new(),
            spheres,
            object_id: 1,
            character: 1,
            enmp_id: 1,
            level: 1,
            stats: combat::TargetStats::enemy(100, 10000, 0, [100; 7], 20, 2),
            attacker: combat::AttackerStats::default(),
            sstm: actor_physics::Sstm::read(&[0; 0x3c]).unwrap(),
            difficulty: combat::Difficulty::Standard,
            lock_range: 1000.,
            body,
            targets,
            attacks,
            exp: (0, 0),
            prize: None,
            atkp: Vec::new(),
        };
        let matrices = [Mat4::from_translation(Vec3::new(10., 20., 30.))];
        let pose = data
            .transform_shapes(
                &matrices,
                Vec3::new(5., 10., 15.),
                Vec3::new(100., 200., 300.),
                0.,
            )
            .unwrap();
        assert_eq!(pose.body[0].centre, Vec3::new(106., 212., 318.));
        assert_eq!(pose.targets[0].centre, pose.body[0].centre);
        // The origin attachment ignores the bone matrices and the removed
        // root, and follows the actor's position and yaw.
        assert_eq!(pose.spheres[0].centre, Vec3::new(100., 207., 500.));
        assert_eq!(pose.spheres[0].radius, 20.);
        let turned = data
            .transform_shapes(
                &matrices,
                Vec3::ZERO,
                Vec3::new(100., 200., 300.),
                std::f32::consts::FRAC_PI_2,
            )
            .unwrap();
        assert!((turned.spheres[0].centre - Vec3::new(300., 207., 300.)).length() < 1e-3);
        assert!(
            data.transform_shapes(&[], Vec3::ZERO, Vec3::ZERO, 0.)
                .is_err()
        );
        assert!(
            data.transform_shapes(&matrices, Vec3::ZERO, Vec3::ZERO, f32::NAN)
                .is_err()
        );
    }
}
