//! Constants world generation takes from method bodies of the installed
//! game's classes (not static initializers): the fixed road nexus points in
//! `RoadGenerator.genRoadDelaunayPoints` and the defaults of
//! `SandboxOptions`' double options.

use std::collections::HashMap;

use pz_classfile::class::{ClassFile, Constant};
use pz_classfile::Jar;

/// Length of the instruction at `pc` (JVMS §6.5).
fn insn_len(code: &[u8], pc: usize) -> usize {
    let op = code[pc];
    match op {
        0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 2,
        0x11
        | 0x13
        | 0x14
        | 0x84
        | 0x99..=0xa8
        | 0xb2..=0xb8
        | 0xbb
        | 0xbd
        | 0xc0
        | 0xc1
        | 0xc6
        | 0xc7 => 3,
        0xc5 => 4,
        0xb9 | 0xba | 0xc8 | 0xc9 => 5,
        0xc4 => {
            if code.get(pc + 1) == Some(&0x84) {
                6
            } else {
                4
            }
        }
        0xaa | 0xab => {
            let base = (pc + 4) & !3;
            let rd = |o: usize| {
                code.get(o..o + 4)
                    .map(|b| i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                    .unwrap_or(0)
            };
            if op == 0xaa {
                let (lo, hi) = (rd(base + 4), rd(base + 8));
                base + 12 + 4 * (hi - lo + 1).max(0) as usize - pc
            } else {
                base + 8 + 8 * rd(base + 4).max(0) as usize - pc
            }
        }
        _ => 1,
    }
}

/// A decoded constant push or call.
#[derive(Clone, Debug, PartialEq)]
enum Insn {
    Double(f64),
    Str(String),
    Invoke(String),
    Other,
}

fn decode(cf: &ClassFile, code: &[u8]) -> Vec<Insn> {
    let mut out = Vec::new();
    let mut pc = 0;
    while pc < code.len() {
        let op = code[pc];
        let idx = |wide: bool| {
            if wide {
                u16::from_be_bytes([code[pc + 1], code[pc + 2]])
            } else {
                code[pc + 1] as u16
            }
        };
        let insn = match op {
            0x0e => Insn::Double(0.0),
            0x0f => Insn::Double(1.0),
            0x12..=0x14 => match cf.constant(idx(op != 0x12)) {
                Ok(Constant::Double(d)) => Insn::Double(*d),
                Ok(Constant::String(s)) => cf
                    .utf8(*s)
                    .map_or(Insn::Other, |s| Insn::Str(s.to_string())),
                _ => Insn::Other,
            },
            0xb6..=0xb9 => cf
                .member_ref(idx(true))
                .map_or(Insn::Other, |m| Insn::Invoke(m.name)),
            _ => Insn::Other,
        };
        out.push(insn);
        pc += insn_len(code, pc);
    }
    out
}

fn method_code<'a>(cf: &'a ClassFile, name: &str) -> Result<&'a [u8], String> {
    cf.methods
        .iter()
        .find(|m| m.name == name && m.code.is_some())
        .and_then(|m| m.code.as_deref())
        .ok_or(format!("{}.{name} not found", cf.this_class))
}

/// The `double` constants a method pushes, in bytecode order.
pub fn method_doubles(jar: &Jar, class: &str, method: &str) -> Result<Vec<f64>, String> {
    let cf = pz_classfile::load_class(jar, class).map_err(|e| format!("{e:?}"))?;
    let code = method_code(&cf, method)?;
    Ok(decode(&cf, code)
        .into_iter()
        .filter_map(|i| match i {
            Insn::Double(d) => Some(d),
            _ => None,
        })
        .collect())
}

/// `RoadGenerator.genRoadDelaunayPoints`' fixed points: the `(x, y)` pairs
/// of `double` constants it passes to `new Vector2D`.
pub fn road_fixed_points(jar: &Jar) -> Result<Vec<(f64, f64)>, String> {
    let d = method_doubles(
        jar,
        "zombie/iso/worldgen/roads/RoadGenerator",
        "genRoadDelaunayPoints",
    )?;
    if d.len() % 2 != 0 {
        return Err(format!("odd number of road point constants: {d:?}"));
    }
    Ok(d.chunks(2).map(|c| (c[0], c[1])).collect())
}

/// The default value of every `SandboxOptions` double option: in the
/// constructor, `newDoubleOption(name, min, max, default)` calls.
pub fn sandbox_double_defaults(jar: &Jar) -> Result<HashMap<String, f64>, String> {
    let cf =
        pz_classfile::load_class(jar, "zombie/SandboxOptions").map_err(|e| format!("{e:?}"))?;
    let code = method_code(&cf, "<init>")?;
    let insns = decode(&cf, code);
    let mut out = HashMap::new();
    for (i, insn) in insns.iter().enumerate() {
        let Insn::Str(name) = insn else { continue };
        let tail = &insns[i + 1..insns.len().min(i + 5)];
        if let [Insn::Double(_), Insn::Double(_), Insn::Double(def), Insn::Invoke(m)] = tail {
            if m == "newDoubleOption" {
                out.insert(name.clone(), *def);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(insn_len(&[0x14, 0, 1], 0), 3);
        assert_eq!(insn_len(&[0xb9, 0, 1, 1, 0], 0), 5);
        assert_eq!(insn_len(&[0x00], 0), 1);
        assert_eq!(insn_len(&[0xc4, 0x84, 0, 0, 0, 0], 0), 6);
    }
}
