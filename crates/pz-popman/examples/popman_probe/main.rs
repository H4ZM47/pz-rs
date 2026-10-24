//! Differential probe for the `PZPopMan` port.
//!
//! `run <script> <out>` executes a script of native calls (one per line,
//! tab-separated) against [`Native`] and writes what the library reports
//! (zombies handed to the game, radar positions, path results, save-file
//! hashes). A local, uncommitted JVM harness replays the same script against
//! the shipped library; the two outputs are compared line by line.
//!
//! `gen <game dir> <scenario> <save dir> <script>` writes a scenario script:
//! it drives this port while recording every call, reacting to the zombies it
//! hands back the way the game would (registering them, reporting their
//! positions, virtualizing them when their chunk unloads).

use pz_popman::native::{Native, Zombie};
use std::cell::Cell;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn fbits(v: f32) -> String {
    format!("{:08x}", v.to_bits())
}

fn f(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

fn d(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).unwrap())
}

fn ints(s: &str) -> Vec<i32> {
    if s.is_empty() {
        return Vec::new();
    }
    s.split(',').map(|v| v.parse().unwrap()).collect()
}

struct Exec {
    n: Native,
    clock: Rc<Cell<i64>>,
    out: String,
    squares: Vec<(i32, i32, u8)>,
}

impl Exec {
    fn new() -> Exec {
        let clock = Rc::new(Cell::new(0i64));
        let mut n = Native::new(5489);
        let c = clock.clone();
        n.gs.clock = Box::new(move || c.get());
        Exec { n, clock, out: String::new(), squares: Vec::new() }
    }

    /// Runs one script line; returns what it printed.
    fn line(&mut self, line: &str) -> String {
        let start = self.out.len();
        if line.is_empty() || line.starts_with('#') {
            return String::new();
        }
        let t: Vec<&str> = line.split('\t').collect();
        let n = &mut self.n;
        let i = |k: usize| t[k].parse::<i32>().unwrap();
        match t[0] {
            "seed" => n.rand = pz_popman::native::rand::NativeRand::new(t[1].parse::<i64>().unwrap() as u32),
            "time" => self.clock.set(t[1].parse().unwrap()),
            "gsb" => {
                n.mcd_set_game_state_bool(t[1], t[2] == "1").unwrap();
            }
            "gsi" => n.mcd_set_game_state_int(t[1], i(2)).unwrap(),
            "gss" => n.mcd_set_game_state_string(t[1], t[2]).unwrap(),
            "metagrid" => n.mcd_init_meta_grid(i(1), i(2), i(3), i(4)),
            "metacell" => n.mcd_init_meta_cell(i(1), i(2), (t[3] != "-").then(|| PathBuf::from(t[3]))),
            "metachunks" => {
                let v = ints(t[3]);
                for wy in 0..32 {
                    for wx in 0..32 {
                        n.mcd_init_meta_chunk(i(1), i(2), wx, wy, v[(wx + wy * 32) as usize] as u8);
                    }
                }
            }
            "mcdinit" => n.mcd_init(i(1), i(2), i(3), i(4)),
            "init" => n.n_init(false, false, i(1), i(2), i(3), i(4)),
            "cfgf" => n.n_config_float(t[1], f(t[2])).unwrap(),
            "cfgi" => n.n_config_int(t[1], i(2)).unwrap(),
            "outfits" => {
                let names: Vec<&str> = if t[1].is_empty() { Vec::new() } else { t[1].split(',').collect() };
                n.n_set_outfit_names(&names);
            }
            "origins" => n.n_set_spawn_origins(&ints(t[1])).unwrap(),
            "areas" => {
                let a = ints(t[2]);
                n.n_loaded_areas(a.len() as i32 / 4, &a, t[1] == "1").unwrap();
            }
            "loadchunk" => n.n_load_chunk(i(1), i(2), t[3] == "1"),
            "add" => {
                n.n_add_zombie(f(t[1]), f(t[2]), f(t[3]), i(4) as u8, i(5), i(6), i(7), i(8), i(9) as u32);
            }
            "reg" => {
                let id = n.n_register_zombie(f(t[1]), f(t[2]), f(t[3]), i(4) as u8, i(5), i(6), i(7) as u32);
                writeln!(self.out, "regid\t{}", id as i32).unwrap();
            }
            "rel" => n.n_release_zombie(i(1) as u32),
            "aggro" => n.n_aggro_target(i(1), i(2), i(3)),
            "horde" => n.n_spawn_horde(i(1), i(2), i(3), i(4), f(t[5]), f(t[6]), i(7)),
            "sound" => {
                // `POPNOREUSE=x_y,...`: those sounds get a fresh allocation
                // (the original's malloc reuse varies from run to run).
                let key = format!("{}_{}", t[1], t[2]);
                if std::env::var("POPNOREUSE").is_ok_and(|v| v.split(',').any(|k| k == key)) {
                    n.reuse_freed_sound = false;
                }
                n.n_world_sound(i(1), i(2), i(3), i(4))
            }
            "real" => {
                let ids = ints(t[1]);
                let pos: Vec<f32> = if t[2].is_empty() { Vec::new() } else { t[2].split(',').map(f).collect() };
                n.n_update_real_zombies(ids.len() as i32 / 2, &ids, &pos).unwrap();
            }
            "debug" => n.n_debug_command(i(1), i(2), i(3)),
            "chunkupd" => {
                let b: Vec<u8> = (0..64).map(|k| u8::from_str_radix(&t[3][k * 2..k * 2 + 2], 16).unwrap()).collect();
                n.mcd_chunk_update_task(i(1), i(2), &b).unwrap();
            }
            "square" => self.squares.push((i(1), i(2), i(3) as u8)),
            "squareflush" => {
                for (x, y, b) in std::mem::take(&mut self.squares) {
                    n.mcd_square_update_task(x, y, b);
                }
            }
            "main" => {
                n.n_update_main(f(t[1]), d(t[2]));
                let total = n.n_get_add_zombie_count();
                let mut off = 0;
                while off < total {
                    let mut recs = Vec::new();
                    let k = n.n_get_add_zombie_data(off, 1024, &mut recs).unwrap();
                    off += k;
                    for z in recs {
                        writeln!(
                            self.out,
                            "Z\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                            fbits(z.x),
                            fbits(z.y),
                            fbits(z.z),
                            z.dir as i8,
                            z.outfit,
                            z.state,
                            z.path_x,
                            z.path_y,
                            z.id as i32
                        )
                        .unwrap();
                    }
                }
                if n.n_has_radar_data() {
                    let mut xy = vec![0f32; 20000];
                    let k = n.n_get_radar_zombie_data(&mut xy).unwrap();
                    // The original's JNI copy is released with JNI_ABORT, so
                    // Java only ever sees the count.
                    writeln!(self.out, "R\t{k}").unwrap();
                }
            }
            "hasdata" => {
                let a = n.n_has_data_for_thread();
                let b = n.mcd_has_data_for_thread();
                writeln!(self.out, "hd\t{a}\t{b}").unwrap();
            }
            "mcdupdate" => n.mcd_n_update(),
            "thread" => n.n_update_thread(),
            "radarreq" => n.n_request_radar_data(),
            "save" => n.n_save(),
            "mcdsave" => n.mcd_save(),
            "savecell" => n.n_save_cell(i(1), i(2)),
            "files" => {
                let root = Path::new(t[1]);
                let mut files = Vec::new();
                walk(root, &mut files);
                files.sort();
                for p in files {
                    let data = std::fs::read(&p).unwrap();
                    writeln!(
                        self.out,
                        "F\t{}\t{}\t{}",
                        p.strip_prefix(root).unwrap().display(),
                        data.len(),
                        sha256_hex(&data)
                    )
                    .unwrap();
                }
            }
            "pathtask" => {
                let (r, _, _) = n.mcd_path_task(i(1), i(2), i(3), i(4));
                writeln!(self.out, "P\t{r}").unwrap();
            }
            "shouldwait" => {
                let a = n.n_should_wait();
                let b = n.mcd_should_wait();
                writeln!(self.out, "W\t{a}\t{b}").unwrap();
            }
            "ready" => {
                let a = n.n_ready_to_pause();
                writeln!(self.out, "RP\t{a}").unwrap();
            }
            "stop" => n.n_stop(),
            "mcdstop" => n.mcd_stop(),
            "copy" => {
                let _ = std::fs::copy(t[1], t[2]);
            }
            "mark" => writeln!(self.out, "M\t{}", t[1]).unwrap(),
            "rng" => {
                let (st, i) = n.rand.mt.raw();
                let sum = st.iter().fold(0u32, |a, b| a.wrapping_add(*b));
                writeln!(self.out, "G\t{i}\t{sum:08x}").unwrap();
            }
            other => panic!("unknown command {other}"),
        }
        self.out[start..].to_string()
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// SHA-256 (FIPS 180-4), for comparing save files.
fn sha256_hex(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
        0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
        0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
        0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
        0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
        0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
        0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] =
        [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut m = data.to_vec();
    let bits = (data.len() as u64) * 8;
    m.push(0x80);
    while m.len() % 64 != 56 {
        m.push(0);
    }
    m.extend_from_slice(&bits.to_be_bytes());
    for chunk in m.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut a = h;
        for i in 0..64 {
            let s1 = a[4].rotate_right(6) ^ a[4].rotate_right(11) ^ a[4].rotate_right(25);
            let ch = (a[4] & a[5]) ^ (!a[4] & a[6]);
            let t1 = a[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a[0].rotate_right(2) ^ a[0].rotate_right(13) ^ a[0].rotate_right(22);
            let maj = (a[0] & a[1]) ^ (a[0] & a[2]) ^ (a[1] & a[2]);
            let t2 = s0.wrapping_add(maj);
            a = [t1.wrapping_add(t2), a[0], a[1], a[2], a[3].wrapping_add(t1), a[4], a[5], a[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(a[i]);
        }
    }
    h.iter().map(|v| format!("{v:08x}")).collect()
}

mod scenario;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("run") => {
            let script = std::fs::read_to_string(&args[2]).unwrap();
            let mut e = Exec::new();
            for l in script.lines() {
                e.line(l);
            }
            std::fs::write(&args[3], &e.out).unwrap();
        }
        Some("gen") => {
            let mut e = Exec::new();
            let mut script = Vec::new();
            scenario::generate(&mut e, &args[2..], &mut |e: &mut Exec, l: String| {
                let o = e.line(&l);
                script.push(l);
                o
            });
            let mut f = std::fs::File::create(args.last().unwrap()).unwrap();
            for l in script {
                writeln!(f, "{l}").unwrap();
            }
        }
        _ => eprintln!("usage: popman_probe run <script> <out> | gen <game dir> <scenario> <save dir> <script>"),
    }
}

#[allow(dead_code)]
fn zombie_line(z: &Zombie) -> String {
    format!("{} {} {}", z.x, z.y, z.id)
}
