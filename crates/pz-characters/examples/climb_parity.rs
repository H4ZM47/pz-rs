//! `getClimbingFailChanceFloat`, `getClimbRopeSpeed` and
//! `ClimbOverWallState.setParams`' draws of [`ClimbSkills`] /
//! [`wall_draws`] against the same cases evaluated by the original game
//! (a local harness on a machine with the game installed writes them; no
//! game data is part of this repository).
//!
//! ```text
//! climb_parity <oracle file>
//! ```
//!
//! Each `climb` line is `fitness strength nimble endurance drunk heavy pain
//! gloves awkward traits zombiesBits failBits upBits downBits` (`traits`:
//! obese overweight clumsy allThumbs dextrous burglar gymnast as 0/1), and
//! the `wall` line after it `seed struggle success nextDraw` for a climb
//! north with that character and `RandStandard` seeded with `seed`.

use pz_characters::character::states::climb_wall::wall_draws;
use pz_characters::character::states::sheet_rope::ClimbSkills;

fn main() {
    let path = std::env::args().nth(1).expect("usage: climb_parity <oracle file>");
    let text = std::fs::read_to_string(path).expect("oracle file");
    let (mut cases, mut bad, mut walls, mut bad_walls) = (0, 0, 0, 0);
    let mut last: Option<(ClimbSkills, f32)> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        match f[0] {
            "climb" => {
                let n = |i: usize| f[i].parse::<i32>().unwrap();
                let bits = |i: usize| f32::from_bits(f[i].parse::<i32>().unwrap() as u32);
                let t: Vec<bool> = f[10].bytes().map(|b| b == b'1').collect();
                let k = ClimbSkills {
                    fitness: n(1),
                    strength: n(2),
                    nimble: n(3),
                    endurance: n(4),
                    drunk: n(5),
                    heavy_load: n(6),
                    pain: n(7),
                    gloves: n(8) == 1,
                    awkward_gloves: n(9) == 1,
                    obese: t[0],
                    overweight: t[1],
                    clumsy: t[2],
                    all_thumbs: t[3],
                    dextrous: t[4],
                    burglar: t[5],
                    gymnast: t[6],
                };
                let zombies = bits(11);
                let got = (k.fail_chance(zombies), k.rope_speed(false), k.rope_speed(true));
                let want = (bits(12), bits(13), bits(14));
                cases += 1;
                if got.0.to_bits() != want.0.to_bits() || got.1.to_bits() != want.1.to_bits() || got.2.to_bits() != want.2.to_bits() {
                    bad += 1;
                    if bad <= 10 {
                        println!("climb mismatch: {line}: rust {got:?}");
                    }
                }
                last = Some((k, want.0));
            }
            "wall" => {
                let (k, fail) = last.expect("climb line first");
                let seed: i64 = f[1].parse().unwrap();
                let r = pz_core::random::Rand::new(seed);
                let (struggle, success) = wall_draws(fail as i32, k.heavy_load, k.strength, false, |n| r.next(n));
                let after = r.next(1 << 30);
                let got = format!("{} {} {after}", struggle as i32, success as i32);
                let want = f[2..5].join(" ");
                walls += 1;
                if got != want {
                    bad_walls += 1;
                    if bad_walls <= 10 {
                        println!("wall mismatch: {line}: rust {got}");
                    }
                }
            }
            _ => {}
        }
    }
    println!("climb cases {cases}, mismatches {bad}; wall draws {walls}, mismatches {bad_walls}");
    if bad + bad_walls > 0 {
        std::process::exit(1);
    }
}
