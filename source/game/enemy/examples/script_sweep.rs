//! Private research tool: runs object scripts (raw BAR type 3 payloads, one
//! file each) in a synthetic arena and reports script faults, traps with no
//! translation and engine services recorded as unsupported.
//!
//! cargo run -p enemy --example script_sweep --release --locked -- \
//!     <script-dir> [ticks] [names-file] [entries-file]
//!
//! The scripts come from a private extract of the installed game; nothing from
//! them is printed beyond file names, trap numbers and counts.
//!
//! The arena: a flat floor, every motion 30 ticks long, Sora standing 400
//! units in front of the first object. A script that searches for another
//! object (by entry, serial, part or unit argument) and finds none gets a
//! partner object of that description on the next round, so the run shows
//! what the script does once the objects it expects exist. With an entries
//! file (`entry id<TAB>script file stem` per line) the partners and the
//! objects `sysobj_appear` makes run their own scripts.
//!
//! `--implemented` lists the trap slots that have a translation instead.
use enemy::{
    env::{Clips, Env},
    motion::MotionEvents,
    sysobj::{Sysobj, Vec3},
    world::{AppearRequest, Appeared, Event, Object, Spawner, World},
};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::PathBuf,
};

struct ArenaClips;

impl Clips for ArenaClips {
    fn length(&self, _: usize) -> u32 {
        30
    }
    fn loop_start(&self, _: usize) -> u32 {
        0
    }
    fn root_delta(&self, _: usize, _: u32) -> Vec3 {
        [0.; 3]
    }
    fn events(&self, _: usize) -> Option<&MotionEvents> {
        None
    }
}

struct Arena;

impl Env for Arena {
    fn floor(&self, _: f32, _: f32, _: f32) -> Option<f32> {
        Some(0.)
    }
    fn clips(&self, _: u32) -> &dyn Clips {
        &ArenaClips
    }
    fn player_position(&self) -> Vec3 {
        [400., 0., 0.]
    }
    fn target_position(&self, _: u32) -> Option<Vec3> {
        Some([400., 0., 0.])
    }
    fn check_step(&self, _: Vec3, _: Vec3, _: f32, _: f32, _: f32) -> Option<u8> {
        Some(0)
    }
    fn no_leave(&self) -> bool {
        true
    }
    fn area_world(&self) -> i32 {
        2
    }
}

/// Builds objects for `sysobj_appear` from a private extract: an entry table
/// (entry id, script file stem) and the script directory.
struct ExtractSpawner {
    dir: PathBuf,
    entries: HashMap<u32, String>,
}

impl Spawner for ExtractSpawner {
    fn appear(&mut self, request: &AppearRequest) -> Option<Appeared> {
        let payload = self
            .entries
            .get(&request.entry)
            .and_then(|stem| fs::read(self.dir.join(format!("{stem}.bdx"))).ok());
        let mut sys = Sysobj::bare();
        sys.appear_way = 2;
        Some(Appeared { payload, sys })
    }
}

fn load_entries(path: &str) -> HashMap<u32, String> {
    let text = fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some((f.next()?.parse().ok()?, f.next()?.to_string()))
        })
        .collect()
}

/// An object a script looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Partner {
    Entry(u32),
    Serial(u16),
    Part(u16),
    UnitArg(usize, i32),
}

#[derive(Default)]
struct Outcome {
    faults: Vec<String>,
    other_faults: usize,
    /// The first fault of a partner object: its entry id and the fault.
    other_first: Option<String>,
    missing: BTreeSet<(u16, u16)>,
    unsupported: BTreeSet<&'static str>,
    wanted: BTreeSet<Partner>,
    acts: usize,
    /// The last traps of the tick in which the first fault happened.
    fault_trace: Vec<(u16, u16)>,
}

struct Setup<'a> {
    dir: &'a PathBuf,
    entries: &'a HashMap<u32, String>,
}

fn run(
    setup: &Setup<'_>,
    payload: &[u8],
    ticks: u32,
    partners: &BTreeSet<Partner>,
) -> Result<Outcome, String> {
    let mut world = World::default();
    world.tolerate_missing = true;
    world.trace = Some(Vec::new());
    world.act_log = Some(Vec::new());
    world.spawner = Some(Box::new(ExtractSpawner {
        dir: setup.dir.clone(),
        entries: setup.entries.clone(),
    }));
    let env = Arena;
    // The partners exist before the object's OBJ_INIT runs, which is where
    // scripts usually look for them.
    for partner in partners {
        match *partner {
            Partner::Entry(e) => {
                world.appear(&env, 0, e);
            }
            other => {
                let id = world.objects.len().max(1) as u32;
                let mut o = Object::bare(id);
                match other {
                    Partner::Serial(s) => o.sys.spawn_serial = s,
                    Partner::Part(p) => o.sys.entry_part = p,
                    Partner::UnitArg(i, v) => o.sys.spawn_args[i.min(7)] = v,
                    Partner::Entry(_) => {}
                }
                world.objects.resize_with(id as usize + 1, || None);
                world.objects[id as usize] = Some(Box::new(o));
            }
        }
    }
    let mut sys = Sysobj::bare();
    sys.appear_way = 2;
    let mut object = Object::spawn(&mut world, &env, 0, payload, sys)
        .map_err(|e| format!("script rejected: {e:?}"))?;
    let mut out = Outcome::default();
    let mut seen_acts = BTreeSet::new();
    for s in world.searches.drain(..).filter(|s| !s.found) {
        out.wanted.insert(match s.kind {
            "entry" => Partner::Entry(s.value as u32),
            "serial" => Partner::Serial(s.value as u16),
            "part" => Partner::Part(s.value as u16),
            _ => Partner::UnitArg(s.index.clamp(0, 7) as usize, s.value),
        });
    }
    for _ in 0..ticks {
        if let Some(t) = world.trace.as_mut() {
            t.clear();
        }
        object.tick(&mut world, &env);
        if std::env::var_os("SWEEP_TRACE").is_some() {
            eprintln!("-- tick");
            for (t, i) in world.trace.iter().flatten() {
                let name = enemy::trap_table::lookup(*t, *i).map_or("?", |t| t.name);
                eprintln!("trap {t}:{i} {name}");
            }
            for s in &world.searches {
                eprintln!("search {s:?}");
            }
            for o in world.objects.iter().flatten() {
                eprintln!(
                    "object {} entry {} part {} serial {}",
                    o.id, o.sys.entry_id, o.sys.entry_part, o.sys.spawn_serial
                );
            }
            if !object.script.faults.is_empty() {
                eprintln!("FAULT {:?}", object.script.faults);
            }
        }
        if out.fault_trace.is_empty() && !object.script.faults.is_empty() {
            out.fault_trace = world
                .trace
                .as_ref()
                .map(|t| t.iter().rev().take(10).rev().copied().collect())
                .unwrap_or_default();
        }
        world.tick_attacks(&env);
        world.tick_objects(&env, Some(0));
        world.deliver_shouts(&env, Some(&mut object));
        world.follow_attachments(&env, Some(&mut object));
        for event in std::mem::take(&mut world.events) {
            match event {
                Event::Unsupported(name) => {
                    out.unsupported.insert(name);
                }
                Event::Presentation { name, .. } => {
                    out.unsupported.insert(name);
                }
                _ => {}
            }
        }
        if let Some(log) = world.act_log.as_mut() {
            seen_acts.extend(log.drain(..));
        }
        for s in world.searches.drain(..).filter(|s| !s.found) {
            out.wanted.insert(match s.kind {
                "entry" => Partner::Entry(s.value as u32),
                "serial" => Partner::Serial(s.value as u16),
                "part" => Partner::Part(s.value as u16),
                _ => Partner::UnitArg(s.index.clamp(0, 7) as usize, s.value),
            });
        }
        let tracing_fault =
            std::env::var_os("SWEEP_TRACE").is_some() && !object.script.faults.is_empty();
        if object.sys.gone || tracing_fault {
            break;
        }
    }
    out.acts = seen_acts.len();
    out.faults = object
        .script
        .faults
        .iter()
        .map(|(pc, f)| format!("word {pc}: {f:?}"))
        .collect();
    out.other_faults = world
        .objects
        .iter()
        .flatten()
        .map(|o| o.script.faults.len())
        .sum();
    out.other_first = world.objects.iter().flatten().find_map(|o| {
        o.script
            .faults
            .first()
            .map(|(pc, f)| format!("entry {} word {pc}: {f:?}", o.sys.entry_id))
    });
    out.missing = world.missing.iter().copied().collect();
    Ok(out)
}

/// Round after round, give the script the partners it searched for.
fn sweep(
    setup: &Setup<'_>,
    payload: &[u8],
    ticks: u32,
) -> Result<(Outcome, BTreeSet<Partner>), String> {
    let mut partners = BTreeSet::new();
    for _ in 0..6 {
        let out = run(setup, payload, ticks, &partners)?;
        let before = partners.len();
        partners.extend(out.wanted.iter().copied());
        if partners.len() == before || partners.len() > 12 {
            return Ok((out, partners));
        }
    }
    Ok((run(setup, payload, ticks, &partners)?, partners))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--implemented") {
        for t in enemy::trap_table::TRAPS {
            if enemy::traps::lookup(t.table, t.index).is_some() {
                println!("{}:{}", t.table, t.index);
            }
        }
        return Ok(());
    }
    let dir = PathBuf::from(
        args.first()
            .ok_or("usage: script_sweep <dir> [ticks] [names] [entries]")?,
    );
    let ticks: u32 = args.get(1).map_or(Ok(300), |t| t.parse())?;
    let names: Option<BTreeSet<String>> = match args.get(2).filter(|p| *p != "-") {
        Some(p) => Some(
            fs::read_to_string(p)?
                .lines()
                .map(|l| l.split_whitespace().next().unwrap_or("").to_string())
                .collect(),
        ),
        None => None,
    };
    let entries = args.get(3).map(|p| load_entries(p)).unwrap_or_default();
    let setup = Setup {
        dir: &dir,
        entries: &entries,
    };
    let mut files: Vec<_> = fs::read_dir(&dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "bdx"))
        .collect();
    files.sort();
    let (mut clean, mut total) = (0, 0);
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let model = name.split('.').next().unwrap_or("").to_string();
        if names.as_ref().is_some_and(|n| !n.contains(&model)) {
            continue;
        }
        total += 1;
        let payload = fs::read(&path)?;
        match std::panic::catch_unwind(|| sweep(&setup, &payload, ticks)) {
            Ok(Ok((o, partners))) => {
                let ok = o.faults.is_empty() && o.missing.is_empty();
                clean += usize::from(ok);
                println!(
                    "{model}\t{}\tacts={}\tfaults={}\tafter={:?}\tpartner_faults={} {}\tmissing={:?}\tunsupported={:?}\tpartners={partners:?}",
                    if ok { "clean" } else { "NOT-CLEAN" },
                    o.acts,
                    o.faults.first().map_or(String::new(), Clone::clone),
                    o.fault_trace,
                    o.other_faults,
                    o.other_first.as_deref().unwrap_or(""),
                    o.missing,
                    o.unsupported,
                );
            }
            Ok(Err(e)) => println!("{model}\tREJECTED\t{e}"),
            Err(_) => println!("{model}\tPANIC"),
        }
    }
    println!("{clean} of {total} clean");
    Ok(())
}
