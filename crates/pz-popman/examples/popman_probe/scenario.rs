//! Scenario scripts for `popman_probe gen`.

use super::{fbits, Exec};
use pz_fs::GameFs;
use pz_popman::map_collision::meta_init;
use pz_world::{CreateParams, GameRand, IsoMetaGrid, WorldGenBounds};
use std::collections::BTreeSet;

struct Lcg(u64);
impl GameRand for Lcg {
    fn next(&mut self, max: i32) -> i32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        if max <= 0 {
            0
        } else {
            ((self.0 >> 33) % max as u64) as i32
        }
    }
}

#[derive(Clone, Copy)]
struct Real {
    id: i32,
    x: f32,
    y: f32,
    z: f32,
    dir: i32,
    outfit: i32,
    state: i32,
    px: i32,
    py: i32,
}

type Emit<'a> = &'a mut dyn FnMut(&mut Exec, String) -> String;

const GRID: i32 = 13;

fn area_chunks(px: i32, py: i32) -> (i32, i32, BTreeSet<(i32, i32)>) {
    let wx0 = px.div_euclid(8) - GRID / 2;
    let wy0 = py.div_euclid(8) - GRID / 2;
    let mut s = BTreeSet::new();
    for y in 0..GRID {
        for x in 0..GRID {
            s.insert((wx0 + x, wy0 + y));
        }
    }
    (wx0, wy0, s)
}

pub fn generate(e: &mut Exec, args: &[String], emit: Emit) {
    let game_dir = &args[0];
    let scenario = args[1].as_str();
    let save_dir = args[2].trim_end_matches('/').to_string();
    let (seed, ticks, start, step_ms): (u32, i32, (i32, i32), i64) = match scenario {
        "town" => (12345, 3000, (10630, 9800), 120),
        "long" => (777, 12000, (10820, 10100), 400),
        "edge" => (4242, 2500, (10500, 9500), 60),
        _ => (1, 600, (10630, 9800), 120),
    };
    let fs = GameFs::new(game_dir);
    let mut rand = Lcg(42);
    let grid = IsoMetaGrid::create(
        &fs,
        CreateParams { game_map: "Muldraugh, KY", world_gen: WorldGenBounds::default(), rand: &mut rand, voronois: Vec::new() },
    )
    .expect("meta grid");
    let meta = meta_init(&grid);
    let mut time: i64 = 1_000_000;
    let mut hours: f64 = 7.0;
    let mut out = |e: &mut Exec, s: String| emit(e, s);
    out(e, format!("seed\t{seed}"));
    out(e, format!("time\t{time}"));
    for (k, v) in [
        ("Core.GameMode", "Sandbox"),
        ("Core.GameSaveWorld", "w"),
        ("GameWindow.CacheDir", save_dir.as_str()),
        ("GameWindow.GameModeCacheDir", &format!("{save_dir}/")),
        ("GameWindow.SaveDir", save_dir.as_str()),
        ("SavefileNaming.SUBDIR_CHUNKDATA", "chunkdata"),
        ("SavefileNaming.SUBDIR_ZPOP", "zpop"),
    ] {
        out(e, format!("gss\t{k}\t{v}"));
    }
    out(e, "gsb\tCore.bLastStand\t0".into());
    out(e, "gsb\tCore.noSave\t0".into());
    out(e, "gsi\tSandboxOptions.Distribution\t1".into());
    out(e, "gsi\tSandboxOptions.Zombies\t4".into());
    out(e, "gsb\tWorld.ZombiesDisabled\t0".into());
    out(e, "gsb\tPAUSED\t1".into());
    out(e, format!("metagrid\t{}\t{}\t{}\t{}", meta.min_x, meta.min_y, meta.width, meta.height));
    for c in &meta.cells {
        for f in &c.chunkdata {
            match f {
                Some(p) => out(e, format!("metacell\t{}\t{}\t{}", c.cx, c.cy, p.display())),
                None => out(e, format!("metacell\t{}\t{}\t-", c.cx, c.cy)),
            };
        }
        if let Some(v) = &c.intensity {
            let csv: Vec<String> = v.iter().map(|b| b.to_string()).collect();
            out(e, format!("metachunks\t{}\t{}\t{}", c.cx, c.cy, csv.join(",")));
        }
    }
    out(e, format!("mcdinit\t{}\t{}\t{}\t{}", meta.min_x, meta.min_y, meta.width, meta.height));
    out(e, format!("init\t{}\t{}\t{}\t{}", meta.min_x, meta.min_y, meta.width, meta.height));
    let respawn = if scenario == "long" { 0.5f32 } else { 2.0 };
    for (k, v) in [
        ("PopulationMultiplier", 1.0f32),
        ("PopulationStartMultiplier", 1.0),
        ("PopulationPeakMultiplier", 1.5),
        ("RespawnHours", respawn),
        ("RespawnUnseenHours", 1.0),
        ("RespawnMultiplier", 0.1),
        ("RedistributeHours", 1.0),
        ("MinZombiesPerChunk", 0.0),
        ("MaxZombiesPerChunk", 255.0),
        ("UniformZombiesPerChunk", 0.2),
    ] {
        out(e, format!("cfgf\t{k}\t{}", fbits(v)));
    }
    out(e, "cfgi\tPopulationPeakDay\t28".into());
    out(e, "cfgi\tFollowSoundDistance\t100".into());
    out(e, "outfits\tbandit,doctor,police,generic01".into());
    if scenario == "edge" {
        out(e, format!("origins\t{},{},{},{}", start.0 - 300, start.1 - 300, 40, 40));
    }
    // A few direct path walks.
    for k in 0..12 {
        let sx = start.0 - 60 + k * 9;
        let sy = start.1 - 40 + (k * 37) % 80;
        out(e, format!("pathtask\t{}\t{}\t{}\t{}", sx, sy, start.0 + (k * 13) % 50, start.1 + (k * 7) % 50));
    }
    let mult = 1.0f32;
    let mut reals: Vec<Real> = Vec::new();
    let mut loaded: BTreeSet<(i32, i32)> = BTreeSet::new();
    let mut last_area = (i32::MIN, i32::MIN);
    out(e, "main\t3f800000\t401c000000000000".into());
    for tick in 0..ticks {
        time += step_ms + (tick % 7) as i64;
        hours += 1.0 / 60.0;
        out(e, format!("time\t{time}"));
        if tick == 1 {
            out(e, "gsb\tPAUSED\t0".into());
        }
        // The player walks a loop around the start.
        let phase = tick as f32 / ticks as f32 * std::f32::consts::TAU;
        let r = if scenario == "long" { 300.0 } else { 120.0 };
        let px = start.0 + (phase.cos() * r) as i32 - r as i32;
        let py = start.1 + (phase.sin() * r) as i32;
        let (wx0, wy0, now) = area_chunks(px, py);
        // Chunks leaving: virtualize their real zombies first.
        let gone: Vec<(i32, i32)> = loaded.difference(&now).copied().collect();
        for &(wx, wy) in &gone {
            let mut keep = Vec::new();
            for z in reals.drain(..) {
                if (z.x as i32).div_euclid(8) == wx && (z.y as i32).div_euclid(8) == wy {
                    let (ppx, ppy) = if z.z == 0.0 && z.px != i32::MIN { (z.px, z.py) } else { (i32::MIN, i32::MIN) };
                    out(
                        e,
                        format!(
                            "add\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                            fbits(z.x),
                            fbits(z.y),
                            fbits(z.z),
                            z.dir,
                            z.outfit,
                            z.state | 1,
                            ppx,
                            ppy,
                            z.id
                        ),
                    );
                    out(e, format!("rel\t{}", z.id));
                } else {
                    keep.push(z);
                }
            }
            reals = keep;
            out(e, format!("loadchunk\t{wx}\t{wy}\t0"));
        }
        for &(wx, wy) in now.difference(&loaded) {
            out(e, format!("loadchunk\t{wx}\t{wy}\t1"));
        }
        loaded = now;
        if (wx0, wy0) != last_area {
            last_area = (wx0, wy0);
            out(e, format!("areas\t0\t{wx0},{wy0},{GRID},{GRID}"));
        }
        // Real zombies shamble toward their targets.
        for z in reals.iter_mut() {
            if z.px != i32::MIN {
                let dx = z.px as f32 + 0.5 - z.x;
                let dy = z.py as f32 + 0.5 - z.y;
                let d = (dx * dx + dy * dy).sqrt();
                if d > 0.5 {
                    z.x += dx / d * 0.04;
                    z.y += dy / d * 0.04;
                } else {
                    z.px = i32::MIN;
                    z.py = i32::MIN;
                }
            }
        }
        if tick % 30 == 0 && !reals.is_empty() {
            let ids: Vec<String> = reals.iter().flat_map(|z| [z.id.to_string(), (z.state | 1).to_string()]).collect();
            let pos: Vec<String> = reals.iter().flat_map(|z| [fbits(z.x), fbits(z.y), fbits(z.z)]).collect();
            out(e, format!("real\t{}\t{}", ids.join(","), pos.join(",")));
        }
        if tick % 200 == 50 {
            out(e, format!("sound\t{}\t{}\t{}\t{}", px + 40, py - 25, 100, 100));
        }
        if tick == 100 || tick == 900 {
            out(e, format!("horde\t{}\t{}\t0\t0\t{}\t{}\t{}", px - 150, py + 60, fbits(px as f32), fbits(py as f32), 30));
        }
        if tick == 450 {
            out(e, format!("horde\t{}\t{}\t20\t15\t{}\t{}\t{}", px + 120, py - 140, fbits(px as f32 + 0.5), fbits(py as f32), 12));
        }
        if tick == 300 {
            out(e, format!("aggro\t0\t{px}\t{py}"));
        }
        if tick == 420 {
            out(e, "aggro\t0\t-1\t-1".into());
        }
        if tick == 700 {
            out(e, format!("debug\t3\t{}\t{}", px.div_euclid(256), py.div_euclid(256)));
        }
        if tick == 1500 {
            out(e, format!("debug\t5\t{}\t{}", px.div_euclid(256), py.div_euclid(256)));
        }
        if tick % 20 == 5 {
            out(e, "radarreq".into());
        }
        if tick % 97 == 13 {
            // A wall appears and disappears near the player.
            let b = if tick % 2 == 0 { 3 } else { 0 };
            for k in 0..4 {
                out(e, format!("square\t{}\t{}\t{}", px + k, py + 3, b));
            }
        }
        if tick % 400 == 399 {
            out(e, "save".into());
            out(e, "mcdsave".into());
            out(e, format!("files\t{save_dir}"));
            out(e, "shouldwait".into());
            out(e, "ready".into());
        }
        // Game thread: ZombiePopulationManager.updateMain.
        let o = out(e, format!("main\t{}\t{:016x}", fbits(mult), hours.to_bits()));
        for l in o.lines() {
            let t: Vec<&str> = l.split('\t').collect();
            if t[0] != "Z" {
                continue;
            }
            let f = |s: &str| f32::from_bits(u32::from_str_radix(s, 16).unwrap());
            let z = Real {
                x: f(t[1]),
                y: f(t[2]),
                z: f(t[3]),
                dir: t[4].parse().unwrap(),
                outfit: t[5].parse().unwrap(),
                state: t[6].parse().unwrap(),
                px: t[7].parse().unwrap(),
                py: t[8].parse().unwrap(),
                id: t[9].parse().unwrap(),
            };
            let wx = (z.x as i32).div_euclid(8);
            let wy = (z.y as i32).div_euclid(8);
            if !loaded.contains(&(wx, wy)) {
                out(
                    e,
                    format!(
                        "add\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        fbits(z.x),
                        fbits(z.y),
                        fbits(z.z),
                        z.dir,
                        z.outfit,
                        z.state,
                        z.px,
                        z.py,
                        z.id
                    ),
                );
                continue;
            }
            let reg = out(
                e,
                format!("reg\t{}\t{}\t{}\t{}\t{}\t{}\t{}", fbits(z.x), fbits(z.y), fbits(z.z), z.dir, z.outfit, z.state | 1, z.id),
            );
            let id: i32 = reg.trim().split('\t').nth(1).unwrap().parse().unwrap();
            reals.push(Real { id, ..z });
        }
        out(e, "squareflush".into());
        out(e, "hasdata".into());
        // The collision/population thread.
        out(e, "mcdupdate".into());
        out(e, "thread".into());
    }
    out(e, "save".into());
    out(e, "mcdsave".into());
    out(e, format!("files\t{save_dir}"));
    out(e, "stop".into());
    out(e, "mcdstop".into());
    out(e, format!("files\t{save_dir}"));
}
