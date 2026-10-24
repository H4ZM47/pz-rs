//! The native half of model loading: what the game gets from
//! `Jassimp.importFile` (assimp 5.3.0). Readers for the formats the game
//! ships (`.x`, binary and ASCII `.fbx`, `.glb`) build an [`AiScene`] the
//! way assimp's importers do; [`postprocess`] then runs the steps the
//! game asks for, in assimp's fixed order.

pub mod aimath;
pub mod fbx;
pub mod gltf;
pub mod postprocess;
pub mod scene;
pub mod x;

pub use postprocess::Steps;
pub use scene::AiScene;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportError(pub String);

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ImportError {}

/// The importer assimp picks for a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    X,
    Fbx,
    Glb,
}

impl Format {
    /// By extension, as `Importer::ReadFile` does first.
    pub fn from_path(path: &str) -> Option<Format> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".x") {
            Some(Format::X)
        } else if lower.ends_with(".fbx") {
            Some(Format::Fbx)
        } else if lower.ends_with(".glb") || lower.ends_with(".gltf") {
            Some(Format::Glb)
        } else {
            None
        }
    }
}

/// `Importer::ReadFile`: import, `ScenePreprocessor`, then the active
/// post-processing steps.
pub fn import(data: &[u8], format: Format, steps: Steps) -> Result<AiScene, ImportError> {
    let mut scene = match format {
        Format::X => x::import(data)?,
        Format::Fbx => fbx::import(data)?,
        Format::Glb => gltf::import(data)?,
    };
    postprocess::preprocess(&mut scene);
    postprocess::apply(&mut scene, steps)?;
    Ok(scene)
}

/// assimp's `fast_atoreal_move<float>`: returns the value and the number
/// of bytes consumed.
pub fn fast_atoreal(s: &[u8], check_comma: bool) -> Result<(f32, usize), ImportError> {
    let at = |i: usize| s.get(i).copied().unwrap_or(0);
    let mut c = 0usize;
    let mut f: f32 = 0.0;
    let inv = at(c) == b'-';
    if inv || at(c) == b'+' {
        c += 1;
    }
    let lower3: Vec<u8> = (0..3).map(|i| at(c + i).to_ascii_lowercase()).collect();
    if lower3 == b"nan" {
        return Ok((f32::NAN, c + 3));
    }
    if lower3 == b"inf" {
        let mut out = f32::INFINITY;
        if inv {
            out = -out;
        }
        c += 3;
        let l5: Vec<u8> = (0..5).map(|i| at(c + i).to_ascii_lowercase()).collect();
        if l5 == b"inity" {
            c += 5;
        }
        return Ok((out, c));
    }
    let digit = |b: u8| b.is_ascii_digit();
    if !digit(at(c)) && !((at(c) == b'.' || (check_comma && at(c) == b',')) && digit(at(c + 1))) {
        return Err(ImportError(format!(
            "Cannot parse string \"{}\" as a real number",
            String::from_utf8_lossy(&s[..s.len().min(32)])
        )));
    }
    if at(c) != b'.' && (!check_comma || at(c) != b',') {
        let (v, n) = strtoul10_64(&s[c..], None);
        f = v as f32;
        c += n;
    }
    if (at(c) == b'.' || (check_comma && at(c) == b',')) && digit(at(c + 1)) {
        c += 1;
        let mut diff = 15u32;
        let (v, n) = strtoul10_64(&s[c..], Some(&mut diff));
        c += n;
        let mut pl = v as f64;
        pl *= FAST_ATOF_TABLE[diff as usize];
        f += pl as f32;
    } else if at(c) == b'.' {
        c += 1;
    }
    if at(c) == b'e' || at(c) == b'E' {
        c += 1;
        let einv = at(c) == b'-';
        if einv || at(c) == b'+' {
            c += 1;
        }
        let (v, n) = strtoul10_64(&s[c..], None);
        c += n;
        let mut exp = v as f32;
        if einv {
            exp = -exp;
        }
        f *= 10.0f32.powf(exp);
    }
    if inv {
        f = -f;
    }
    Ok((f, c))
}

const FAST_ATOF_TABLE: [f64; 16] = [
    0.0,
    0.1,
    0.01,
    0.001,
    0.0001,
    0.00001,
    0.000001,
    0.0000001,
    0.00000001,
    0.000000001,
    0.0000000001,
    0.00000000001,
    0.000000000001,
    0.0000000000001,
    0.00000000000001,
    0.000000000000001,
];

/// assimp's `strtoul10_64`: (value, bytes consumed). With `max`, stops
/// accumulating after that many digits (skipping the rest) and writes
/// back the digit count when it ends earlier.
pub fn strtoul10_64(s: &[u8], max: Option<&mut u32>) -> (u64, usize) {
    let mut cur = 0u32;
    let mut value: u64 = 0;
    let mut i = 0usize;
    let limit = max.as_ref().map(|m| **m);
    loop {
        let c = s.get(i).copied().unwrap_or(0);
        if !c.is_ascii_digit() {
            break;
        }
        let nv = value.wrapping_mul(10).wrapping_add((c - b'0') as u64);
        if nv < value {
            return (0, i);
        }
        value = nv;
        i += 1;
        cur += 1;
        if limit == Some(cur) {
            while s.get(i).is_some_and(|c| c.is_ascii_digit()) {
                i += 1;
            }
            return (value, i);
        }
    }
    if let Some(m) = max {
        *m = cur;
    }
    (value, i)
}

/// assimp's `SuperFastHash` (Paul Hsieh), byte-for-byte.
/// (`len == 0` means `strlen` in assimp; callers here always pass the
/// exact slice.)
pub fn super_fast_hash(data: &[u8], mut hash: u32) -> u32 {
    let len = data.len() as u32;
    let g16 = |d: &[u8], i: usize| (d[i + 1] as u32) << 8 | d[i] as u32;
    let rem = len & 3;
    let mut n = len >> 2;
    let mut p = 0usize;
    while n > 0 {
        hash = hash.wrapping_add(g16(data, p));
        let tmp = (g16(data, p + 2) << 11) ^ hash;
        hash = (hash << 16) ^ tmp;
        p += 4;
        hash = hash.wrapping_add(hash >> 11);
        n -= 1;
    }
    match rem {
        3 => {
            hash = hash.wrapping_add(g16(data, p));
            hash ^= hash << 16;
            hash ^= ((data[p + 2] as i8 as i32).unsigned_abs()) << 18;
            hash = hash.wrapping_add(hash >> 11);
        }
        2 => {
            hash = hash.wrapping_add(g16(data, p));
            hash ^= hash << 11;
            hash = hash.wrapping_add(hash >> 17);
        }
        1 => {
            hash = hash.wrapping_add(data[p] as i8 as i32 as u32);
            hash ^= hash << 10;
            hash = hash.wrapping_add(hash >> 1);
        }
        _ => {}
    }
    hash ^= hash << 3;
    hash = hash.wrapping_add(hash >> 5);
    hash ^= hash << 4;
    hash = hash.wrapping_add(hash >> 17);
    hash ^= hash << 25;
    hash = hash.wrapping_add(hash >> 6);
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atof_matches_assimp_rules() {
        assert_eq!(fast_atoreal(b"1.5;", true).unwrap(), (1.5, 3));
        assert_eq!(fast_atoreal(b"-0.25,", true).unwrap(), (-0.25, 5));
        // a comma directly after the integer part is a decimal separator
        assert_eq!(fast_atoreal(b"1,5", true).unwrap(), (1.5, 3));
        assert_eq!(fast_atoreal(b"1,5", false).unwrap(), (1.0, 1));
        assert_eq!(fast_atoreal(b"2e2", true).unwrap(), (200.0, 3));
        assert!(fast_atoreal(b"x", true).is_err());
    }
}
