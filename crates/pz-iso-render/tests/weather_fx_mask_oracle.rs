//! Compares `RasterizeBounds` and `Rasterize.scanTriangle` with a dump of
//! the original classes over camera offsets and triangles drawn from a
//! seeded `java.util.Random`. Runs only when `PZ_WEATHERFX_MASK_ORACLE`
//! names the dump (`PZ_WEATHERFX_MASK_SEED`, default 31, must match); the
//! harness that produces it is kept outside the repository.
//!
//! Dump lines: `size w h` (the offscreen size), then `rb offX offY z <x1..y4 as float bits> <cx1..cy4>` then
//! `cells x,y ...` for the view's two triangles; `tri <6 float bits>` then
//! `cells ...` for a free triangle scanned over rows 0..40.

use pz_core::random::JavaRandom;
use pz_iso_render::weather_fx_mask::{scan_triangle, RasterizeBounds};

fn fb(f: f32) -> String {
    format!("{:x}", f.to_bits())
}

fn cells(out: &mut String) -> impl FnMut(i32, i32) + '_ {
    move |x, y| out.push_str(&format!(" {x},{y}"))
}

#[test]
fn rasterize_matches_the_original() {
    let Ok(path) = std::env::var("PZ_WEATHERFX_MASK_ORACLE") else {
        return;
    };
    let seed: i64 = std::env::var("PZ_WEATHERFX_MASK_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(31);
    let java = std::fs::read_to_string(path).expect("oracle dump");
    let mut lines = java.lines();
    let size: Vec<i32> = lines
        .next()
        .expect("size")
        .split(' ')
        .skip(1)
        .map(|v| v.parse().unwrap())
        .collect();
    let rnd = JavaRandom::new(seed);
    let mut out = Vec::new();
    for _ in 0..40 {
        let ox = (rnd.next_float() * 40000.0 - 5000.0) as i32;
        let oy = (rnd.next_float() * 20000.0 - 2000.0) as i32;
        let z = (rnd.next_float() * 4.0) as i32;
        let rb = RasterizeBounds::calculate(ox as f32, oy as f32, size[0], size[1], z);
        let mut s = format!("rb {ox} {oy} {z}");
        for i in 0..4 {
            s += &format!(" {} {}", fb(rb.x[i]), fb(rb.y[i]));
        }
        for i in 0..4 {
            s += &format!(" {} {}", rb.cx[i], rb.cy[i]);
        }
        out.push(s);
        let mut c = String::from("cells");
        scan_triangle(
            rb.x[0],
            rb.y[0],
            rb.x[1],
            rb.y[1],
            rb.x[3],
            rb.y[3],
            0,
            100_000,
            cells(&mut c),
        );
        scan_triangle(
            rb.x[1],
            rb.y[1],
            rb.x[2],
            rb.y[2],
            rb.x[3],
            rb.y[3],
            0,
            100_000,
            cells(&mut c),
        );
        out.push(c);
    }
    for i in 0..200 {
        let mut v = [0f32; 6];
        for x in &mut v {
            *x = rnd.next_float() * 60.0 - 10.0;
        }
        if i % 5 == 0 {
            v[3] = v[1];
        }
        let mut c = String::from("cells");
        scan_triangle(v[0], v[1], v[2], v[3], v[4], v[5], 0, 40, cells(&mut c));
        out.push(format!(
            "tri {}",
            v.iter().map(|f| fb(*f)).collect::<Vec<_>>().join(" ")
        ));
        out.push(c);
    }
    if let Ok(p) = std::env::var("PZ_WEATHERFX_RUST_OUT") {
        std::fs::write(p, out.join("\n") + "\n").expect("write");
    }
    let java: Vec<&str> = lines.collect();
    assert_eq!(java.len(), out.len());
    let bad: Vec<String> = java
        .iter()
        .zip(&out)
        .enumerate()
        .filter(|(_, (j, r))| **j != r.as_str())
        .map(|(i, (j, r))| format!("line {i}\n java {j}\n rust {r}"))
        .take(5)
        .collect();
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
