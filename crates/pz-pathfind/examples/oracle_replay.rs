//! Replays recorded native pathfinder runs and compares the results bit
//! for bit.
//!
//! ```text
//! cargo run --release -p pz-pathfind --example oracle_replay -- <oracle-dir> [region ...] [--show N] [--only INDEX]
//! ```
//!
//! `<oracle-dir>` holds `regions/<name>/{world.txt,chunks.bin,requests.bin,results.txt}`
//! recorded from the original library; nothing is read from this
//! repository. Each region is replayed in a fresh world: `initWorld` with
//! the arguments in `world.txt`, every `chunks.bin` record through
//! `updateChunk` in file order, one `update()`, then every request in
//! index order. The raw result (return code, node count, raw float bits of
//! x / y / z+32 and node flags) is compared with `results.txt`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use pz_pathfind::{World, REQUEST_SIZE};

/// One expected or actual result.
#[derive(PartialEq, Eq, Clone, Debug)]
struct Outcome {
    ret: i32,
    nodes: Vec<(u32, u32, u32, i32)>,
}

fn parse_results(text: &str) -> Vec<Outcome> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split('|');
        let head: Vec<&str> = parts.next().unwrap().split_whitespace().collect();
        let ret = head[1].parse::<i32>().expect("ret");
        let mut nodes = Vec::new();
        for p in parts {
            let f: Vec<&str> = p.split_whitespace().collect();
            let hex = |s: &str| u32::from_str_radix(s, 16).expect("hex");
            nodes.push((
                hex(f[0]),
                hex(f[1]),
                hex(f[2]),
                f[3].parse().expect("flags"),
            ));
        }
        out.push(Outcome { ret, nodes });
    }
    out
}

fn decode_path(ret: i32, buf: &[u8]) -> Outcome {
    let mut nodes = Vec::new();
    if ret == 1 && buf.len() >= 2 {
        let count = i16::from_be_bytes([buf[0], buf[1]]) as usize;
        for i in 0..count {
            let o = 2 + i * 16;
            let w = |k: usize| u32::from_be_bytes(buf[o + k..o + k + 4].try_into().unwrap());
            nodes.push((w(0), w(4), w(8), w(12) as i32));
        }
    }
    Outcome { ret, nodes }
}

fn fmt(o: &Outcome) -> String {
    let mut s = format!("ret={} n={}", o.ret, o.nodes.len());
    for n in &o.nodes {
        s += &format!(
            " | {:08x} {:08x} {:08x} {} ({} {} {})",
            n.0,
            n.1,
            n.2,
            n.3,
            f32::from_bits(n.0),
            f32::from_bits(n.1),
            f32::from_bits(n.2)
        );
    }
    s
}

fn describe_request(r: &[u8]) -> String {
    let f = |o: usize| f32::from_be_bytes(r[o..o + 4].try_into().unwrap());
    let i = |o: usize| i32::from_be_bytes(r[o..o + 4].try_into().unwrap());
    format!(
        "start ({} {} {}) target ({} {} {}) mover {} flags {:?} levels {}..{} tail {:?}",
        f(0),
        f(4),
        f(8) - 32.0,
        f(12),
        f(16),
        f(20) - 32.0,
        i(24),
        &r[28..36],
        i(36),
        i(40),
        &r[44..47]
    )
}

struct Opts {
    show: usize,
    only: Option<usize>,
}

/// Replays one region; returns (requests, mismatches).
fn replay(dir: &Path, opts: &Opts) -> (usize, usize) {
    let world_txt = fs::read_to_string(dir.join("world.txt")).expect("world.txt");
    let args: Vec<i32> = world_txt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .expect("world args")
        .split_whitespace()
        .map(|s| s.parse().expect("int"))
        .collect();
    let mut world = World::new(args[0], args[1], args[2], args[3], args[4] != 0);

    let chunks = fs::read(dir.join("chunks.bin")).expect("chunks.bin");
    let mut o = 0;
    let le = |b: &[u8], o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    while o + 16 <= chunks.len() {
        let (load_id, wx, wy, len) = (
            le(&chunks, o),
            le(&chunks, o + 4),
            le(&chunks, o + 8),
            le(&chunks, o + 12) as usize,
        );
        o += 16;
        world
            .update_chunk(load_id, wx, wy, &chunks[o..o + len])
            .unwrap_or_else(|e| panic!("updateChunk {wx},{wy}: {e:?}"));
        o += len;
    }
    world.update();

    let requests = fs::read(dir.join("requests.bin")).expect("requests.bin");
    let expected =
        parse_results(&fs::read_to_string(dir.join("results.txt")).expect("results.txt"));
    let n = requests.len() / REQUEST_SIZE;
    assert_eq!(n, expected.len(), "request / result count");
    let mut mismatches = 0;
    let mut shown = 0;
    let mut out = Vec::new();
    for (i, exp) in expected.iter().enumerate() {
        if opts.only.is_some_and(|only| only != i) {
            continue;
        }
        let req = &requests[i * REQUEST_SIZE..(i + 1) * REQUEST_SIZE];
        let ret = world
            .find_path_into(req, &mut out, 3072)
            .unwrap_or_else(|e| panic!("request {i}: {e:?}"));
        let got = decode_path(ret, &out);
        if &got != exp {
            mismatches += 1;
            if shown < opts.show {
                shown += 1;
                println!("  #{i}: {}", describe_request(req));
                println!("    native: {}", fmt(exp));
                println!("    port:   {}", fmt(&got));
            }
        }
    }
    (if opts.only.is_some() { 1 } else { n }, mismatches)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let root = match args.next() {
        Some(r) => PathBuf::from(r),
        None => {
            eprintln!("usage: oracle_replay <oracle-dir> [region ...] [--show N] [--only INDEX]");
            return ExitCode::from(2);
        }
    };
    let mut regions = Vec::new();
    let mut opts = Opts {
        show: 5,
        only: None,
    };
    while let Some(a) = args.next() {
        match a.as_str() {
            "--show" => opts.show = args.next().and_then(|v| v.parse().ok()).expect("--show N"),
            "--only" => opts.only = args.next().and_then(|v| v.parse().ok()),
            _ => regions.push(a),
        }
    }
    let regions_dir = root.join("regions");
    if regions.is_empty() {
        let mut names: Vec<String> = fs::read_dir(&regions_dir)
            .expect("regions dir")
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("requests.bin").exists())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        regions = names;
    }
    let (mut total, mut bad) = (0, 0);
    for r in &regions {
        let t = std::time::Instant::now();
        let (n, m) = replay(&regions_dir.join(r), &opts);
        println!(
            "{r}: {n} requests, {m} mismatches ({:.1}s)",
            t.elapsed().as_secs_f32()
        );
        total += n;
        bad += m;
    }
    println!("total: {total} requests, {bad} mismatches");
    if bad == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
