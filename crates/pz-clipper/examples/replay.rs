//! Replays a line-based script of `Clipper` / `ClipperOffset` calls and
//! prints every observable result. Used for differential testing against
//! the game's native library: an equivalent Java harness (kept out of the
//! repository) runs the same script through `zombie.vehicles.Clipper`, and
//! the two outputs must be byte-identical.
//!
//! Usage: `cargo run --release -p pz-clipper --example replay -- SCRIPT`
//!
//! Script format (one call per line, floats as 8-digit hex bit patterns,
//! doubles as 16-digit hex bit patterns):
//!
//! ```text
//! # ...                  echoed verbatim
//! C | c                  new Clipper | clear()
//! P clip closed n cap f… addPath(n, bb[cap floats], clip, closed)
//! L|A|R x1 y1 x2 y2      addLine | addAABB | clipAABB
//! B x1 y1 x2 y2 r        addAABBBevel
//! Q x1 y1 … x4 y4        addPolygon
//! G ct delta jt          generatePolygons(ct, delta, jt)
//! T cx cy                generateTriangulatePolygons(cx, cy)
//! X cap                  getPolygon/triangulate/triangulate2 for every
//!                        polygon (and one index past the end), cap bytes
//! O | o                  new ClipperOffset | clear()
//! p jt et n cap f…       ClipperOffset.addPath
//! e delta                ClipperOffset.execute
//! Y cap                  getPolygonCount + getPolygon for every polygon
//! ```

use pz_clipper::{Clipper, ClipperError, ClipperOffset};
use std::fmt::Write as _;
use std::io::{BufRead, Write};

fn f(tok: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(tok, 16).expect("float hex"))
}

fn d(tok: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(tok, 16).expect("double hex"))
}

fn i(tok: &str) -> i32 {
    tok.parse().expect("int")
}

fn exc(e: ClipperError) -> &'static str {
    match e {
        ClipperError::BufferUnderflow => "EXC java.nio.BufferUnderflowException",
        ClipperError::BufferOverflow => "EXC java.nio.BufferOverflowException",
        // libc++ `out_of_range("vector")` / `length_error("vector")`, both
        // rethrown to Java as `java.lang.Error`.
        ClipperError::IndexOutOfRange | ClipperError::LengthError => "EXC java.lang.Error:vector",
    }
}

fn hex_trimmed(buf: &[u8]) -> String {
    let end = buf.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
    let mut s = String::with_capacity(end * 2);
    for b in &buf[..end] {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn result_line(tag: &str, idx: i32, r: Result<i32, ClipperError>, buf: &[u8]) -> String {
    match r {
        Ok(v) => format!("{tag} {idx} {v} {}", hex_trimmed(buf)),
        Err(e) => format!("{tag} {idx} {} {}", exc(e), hex_trimmed(buf)),
    }
}

fn main() {
    let path = std::env::args().nth(1).expect("script path");
    let file = std::fs::File::open(path).expect("open script");
    let out = std::io::stdout();
    let mut out = std::io::BufWriter::new(out.lock());
    let mut clipper = Clipper::new();
    let mut offset = ClipperOffset::new();
    let mut last_count = 0i32;
    for line in std::io::BufReader::new(file).lines() {
        let line = line.expect("read");
        if line.starts_with('#') {
            writeln!(out, "{line}").unwrap();
            continue;
        }
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        match t[0] {
            "C" => {
                clipper = Clipper::new();
                last_count = 0;
            }
            "c" => clipper.clear(),
            "P" => {
                let cap: usize = t[4].parse().unwrap();
                let floats: Vec<f32> = t[5..5 + cap].iter().map(|s| f(s)).collect();
                let r = clipper.add_path(i(t[3]), &floats, t[1] == "1", t[2] == "1");
                if let Err(e) = r {
                    writeln!(out, "P {}", exc(e)).unwrap();
                }
            }
            "L" => clipper.add_line(f(t[1]), f(t[2]), f(t[3]), f(t[4])),
            "A" => clipper.add_aabb(f(t[1]), f(t[2]), f(t[3]), f(t[4])),
            "R" => clipper.clip_aabb(f(t[1]), f(t[2]), f(t[3]), f(t[4])),
            "B" => clipper.add_aabb_bevel(f(t[1]), f(t[2]), f(t[3]), f(t[4]), f(t[5])),
            "Q" => clipper.add_polygon(
                f(t[1]),
                f(t[2]),
                f(t[3]),
                f(t[4]),
                f(t[5]),
                f(t[6]),
                f(t[7]),
                f(t[8]),
            ),
            "G" => {
                last_count = clipper.generate_polygons_full(i(t[1]), d(t[2]), i(t[3]));
                writeln!(out, "G {last_count}").unwrap();
            }
            "T" => {
                last_count = clipper.generate_triangulate_polygons(i(t[1]), i(t[2]));
                writeln!(out, "T {last_count}").unwrap();
            }
            "X" => {
                let cap: usize = t[1].parse().unwrap();
                for idx in 0..=last_count.max(0) {
                    let mut buf = vec![0u8; cap];
                    let r = clipper.get_polygon_into(idx, &mut buf);
                    writeln!(out, "{}", result_line("g", idx, r, &buf)).unwrap();
                    if idx == last_count {
                        break;
                    }
                    let mut buf = vec![0u8; cap];
                    let r = clipper.triangulate_into(idx, &mut buf);
                    writeln!(out, "{}", result_line("t", idx, r, &buf)).unwrap();
                    let mut buf = vec![0u8; cap];
                    let r = clipper.triangulate2_into(idx, &mut buf);
                    writeln!(out, "{}", result_line("u", idx, r, &buf)).unwrap();
                }
            }
            "O" => offset = ClipperOffset::new(),
            "o" => offset.clear(),
            "p" => {
                let cap: usize = t[4].parse().unwrap();
                let floats: Vec<f32> = t[5..5 + cap].iter().map(|s| f(s)).collect();
                let r = offset.add_path(i(t[3]), &floats, i(t[1]), i(t[2]));
                if let Err(e) = r {
                    writeln!(out, "p {}", exc(e)).unwrap();
                }
            }
            "e" => offset.execute(d(t[1])),
            "Y" => {
                let cap: usize = t[1].parse().unwrap();
                let n = offset.get_polygon_count();
                writeln!(out, "Y {n}").unwrap();
                for idx in 0..=n {
                    let mut buf = vec![0u8; cap];
                    let r = offset.get_polygon_into(idx, &mut buf);
                    writeln!(out, "{}", result_line("h", idx, r, &buf)).unwrap();
                }
            }
            other => panic!("unknown op {other}"),
        }
    }
    out.flush().unwrap();
}
