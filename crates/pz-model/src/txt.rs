//! The legacy text mesh format (`media/models/*.txt`, vehicles and a few
//! old characters): `ModelLoader.loadTxt` and `ModelTxt`, read line by
//! line exactly as the Java state machine does.

use std::collections::HashMap;

use crate::processed::LMat;
use crate::{AnimationClip, Keyframe, Mat4, Quat, SkinningData};

/// `ModelTxt`. Vertices keep the file's layout: position, normal, tangent,
/// uv (11 floats) for static meshes, plus 4 weights and 4 bone indices
/// (19 floats) for skinned ones.
#[derive(Clone, Debug, Default)]
pub struct ModelTxt {
    pub is_static: bool,
    pub reverse: bool,
    pub vertices: Vec<f32>,
    pub elements: Vec<u32>,
    pub bone_indices: HashMap<String, i32>,
    pub skeleton_hierarchy: Vec<i32>,
    pub bind_pose: Vec<Mat4>,
    pub inv_bind_pose: Vec<Mat4>,
    pub skin_offset: Vec<Mat4>,
    pub clips: HashMap<String, AnimationClip>,
}

impl ModelTxt {
    pub fn stride(&self) -> usize {
        if self.is_static {
            11
        } else {
            19
        }
    }

    /// `ModelLoader.applyToMesh`: the skinning data a non-static mesh gets.
    pub fn skinning_data(&self) -> SkinningData {
        SkinningData {
            bind_pose: self.bind_pose.clone(),
            inverse_bind_pose: self.inv_bind_pose.clone(),
            bone_offset: self.skin_offset.iter().take(self.skeleton_hierarchy.len()).cloned().collect(),
            skeleton_hierarchy: self.skeleton_hierarchy.clone(),
            bone_indices: self.bone_indices.clone(),
            animation_clips: self.clips.clone(),
        }
    }
}

#[derive(Debug)]
pub struct TxtError(pub String);

impl std::fmt::Display for TxtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TxtError {}

/// `BufferedReader.readLine` over a byte buffer (`\n`, `\r`, `\r\n`).
struct Lines<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Lines<'a> {
    fn read(&mut self) -> Option<String> {
        if self.p >= self.b.len() {
            return None;
        }
        let s = self.p;
        while self.p < self.b.len() && self.b[self.p] != b'\n' && self.b[self.p] != b'\r' {
            self.p += 1;
        }
        let line = String::from_utf8_lossy(&self.b[s..self.p]).into_owned();
        if self.p < self.b.len() {
            if self.b[self.p] == b'\r' && self.b.get(self.p + 1) == Some(&b'\n') {
                self.p += 2;
            } else {
                self.p += 1;
            }
        }
        Some(line)
    }
}

/// `String.split(",")`: trailing empty strings removed.
fn jsplit(s: &str) -> Vec<&str> {
    let mut v: Vec<&str> = s.split(',').collect();
    while v.len() > 1 && v.last() == Some(&"") {
        v.pop();
    }
    v
}

/// `Float.parseFloat` (leading/trailing whitespace allowed, f/F/d/D suffix).
fn pf(s: &str) -> Result<f32, TxtError> {
    let t = s.trim_matches(|c: char| c <= ' ');
    let t = t.strip_suffix(['f', 'F', 'd', 'D']).unwrap_or(t);
    t.parse::<f32>().map_err(|_| TxtError(format!("NumberFormatException: \"{s}\"")))
}

/// `Integer.parseInt` (no whitespace allowed).
fn pi(s: &str) -> Result<i32, TxtError> {
    s.parse::<i32>().map_err(|_| TxtError(format!("NumberFormatException: \"{s}\"")))
}

fn field<'s>(v: &[&'s str], i: usize) -> Result<&'s str, TxtError> {
    v.get(i).copied().ok_or_else(|| TxtError("ArrayIndexOutOfBounds".into()))
}

fn need(l: Option<String>) -> Result<String, TxtError> {
    l.ok_or_else(|| TxtError("NullPointerException: unexpected end of file".into()))
}

fn matrix(l1: &str, l2: &str, l3: &str, l4: &str) -> Result<Mat4, TxtError> {
    let mut m = LMat::IDENTITY;
    for (i, l) in [l1, l2, l3, l4].iter().enumerate() {
        let s = jsplit(l);
        for j in 0..4 {
            m.m[i][j] = pf(field(&s, j)?)?;
        }
    }
    Ok(m.to_mat4())
}

/// Iteration order of a `java.util.HashMap<String, ?>` whose keys were
/// put in `keys` order (no removals): buckets by `(h ^ h >>> 16) &
/// (capacity - 1)`, insertion order within a bucket.
pub fn java_hashmap_order(keys: &[String]) -> Vec<String> {
    let mut uniq: Vec<&String> = Vec::new();
    for k in keys {
        if !uniq.contains(&k) {
            uniq.push(k);
        }
    }
    let mut cap = 16usize;
    while uniq.len() as f64 > cap as f64 * 0.75 {
        cap *= 2;
    }
    let hash = |s: &str| {
        let mut h: i32 = 0;
        for u in s.encode_utf16() {
            h = h.wrapping_mul(31).wrapping_add(u as i32);
        }
        let h = h as u32;
        (h ^ (h >> 16)) as usize & (cap - 1)
    };
    let mut buckets: Vec<Vec<&String>> = vec![Vec::new(); cap];
    for k in uniq {
        buckets[hash(k)].push(k);
    }
    buckets.into_iter().flatten().cloned().collect()
}

/// `ModelLoader.loadTxt(filename, bStatic, bReverse, skinnedTo)`.
pub fn load_txt(data: &[u8], is_static: bool, reverse: bool, skinned_to: Option<&SkinningData>) -> Result<ModelTxt, TxtError> {
    let mut mt = ModelTxt { is_static, reverse, ..Default::default() };
    let stride = mt.stride();
    let mut r = Lines { b: data, p: 0 };
    let mut mode = 0u32;
    let mut skip = 0i32;
    let mut num_vertices = 0usize;
    let mut num_elements = 0usize;
    let mut num_bones = 0usize;
    let mut has_tangent = false;
    // insertion order of boneIndices puts, for the HashMap iteration
    let mut put_order: Vec<String> = Vec::new();
    while let Some(mut line) = r.read() {
        if line.starts_with('#') {
            continue;
        }
        if line.contains("Tangent") {
            if is_static {
                skip += 2;
            }
            has_tangent = true;
        }
        if skip > 0 {
            skip -= 1;
            continue;
        }
        match mode {
            0 => mode = 1,
            1 => mode = 2,
            2 => {
                mode = 5;
                skip = if is_static { 7 } else { 13 };
            }
            5 => {
                num_vertices = pi(&line)? as usize;
                mode = 6;
                mt.vertices = vec![0.0; num_vertices * stride];
            }
            6 => {
                for n in 0..num_vertices {
                    let s = jsplit(&line);
                    let (x, y, z) = (pf(field(&s, 0)?)?, pf(field(&s, 1)?)?, pf(field(&s, 2)?)?);
                    line = need(r.read())?;
                    let s = jsplit(&line);
                    let (nx, ny, nz) = (pf(field(&s, 0)?)?, pf(field(&s, 1)?)?, pf(field(&s, 2)?)?);
                    let (mut tx, mut ty, mut tz) = (0.0, 0.0, 0.0);
                    if has_tangent {
                        line = need(r.read())?;
                        let s = jsplit(&line);
                        tx = pf(field(&s, 0)?)?;
                        ty = pf(field(&s, 1)?)?;
                        tz = pf(field(&s, 2)?)?;
                    }
                    line = need(r.read())?;
                    let s = jsplit(&line);
                    let (u, v) = (pf(field(&s, 0)?)?, pf(field(&s, 1)?)?);
                    let mut w = [0.0f32; 4];
                    let mut bi = [0i32; 4];
                    if !is_static {
                        line = need(r.read())?;
                        let s = jsplit(&line);
                        for k in 0..4 {
                            w[k] = pf(field(&s, k)?)?;
                        }
                        line = need(r.read())?;
                        let s = jsplit(&line);
                        for k in 0..4 {
                            bi[k] = pi(field(&s, k)?.trim())?;
                        }
                    }
                    line = r.read().unwrap_or_default();
                    let o = &mut mt.vertices[n * stride..(n + 1) * stride];
                    o[0..11].copy_from_slice(&[x, y, z, nx, ny, nz, tx, ty, tz, u, v]);
                    if !is_static {
                        o[11..15].copy_from_slice(&w);
                        for k in 0..4 {
                            o[15 + k] = bi[k] as f32;
                        }
                    }
                }
                mode = 7;
            }
            7 => {
                num_elements = pi(&line)? as usize;
                mt.elements = vec![0; num_elements * 3];
                mode = 8;
            }
            8 => {
                for n in 0..num_elements {
                    let s = jsplit(&line);
                    let x = pi(field(&s, 0)?.trim())? as u32;
                    let y = pi(field(&s, 1)?.trim())? as u32;
                    let z = pi(field(&s, 2)?.trim())? as u32;
                    if reverse {
                        mt.elements[n * 3..n * 3 + 3].copy_from_slice(&[z, y, x]);
                    } else {
                        mt.elements[n * 3..n * 3 + 3].copy_from_slice(&[x, y, z]);
                    }
                    line = r.read().unwrap_or_default();
                }
                mode = 9;
            }
            9 => {
                num_bones = pi(&line)? as usize;
                mode = 10;
            }
            10 => {
                for _ in 0..num_bones {
                    let index = pi(&line)?;
                    line = need(r.read())?;
                    let pindex = pi(&line)?;
                    line = need(r.read())?;
                    let bone = line.clone();
                    line = r.read().unwrap_or_default();
                    mt.skeleton_hierarchy.push(pindex);
                    mt.bone_indices.insert(bone.clone(), index);
                    put_order.push(bone);
                }
                mode = 11;
            }
            11 | 12 | 13 => {
                for _ in 0..num_bones {
                    let l1 = need(r.read())?;
                    let l2 = need(r.read())?;
                    let l3 = need(r.read())?;
                    let l4 = need(r.read())?;
                    let m = matrix(&l1, &l2, &l3, &l4)?;
                    match mode {
                        11 => mt.bind_pose.push(m),
                        12 => mt.inv_bind_pose.push(m),
                        _ => mt.skin_offset.push(m),
                    }
                    r.read();
                }
                mode = if mode == 13 { 14 } else { mode + 1 };
            }
            14 => mode = 15,
            15 => {
                let anim_name = line.clone();
                line = need(r.read())?;
                let duration = pf(&line)?;
                line = need(r.read())?;
                let n_frames = pi(&line)? as usize;
                line = need(r.read())?;
                let mut frames = Vec::with_capacity(n_frames);
                for n in 0..n_frames {
                    let bindex = pi(&line)?;
                    line = need(r.read())?;
                    let bone = line.clone();
                    line = need(r.read())?;
                    let t = pf(&line)?;
                    line = need(r.read())?;
                    let line2 = need(r.read())?;
                    let s = jsplit(&line);
                    let pos = [pf(field(&s, 0)?)?, pf(field(&s, 1)?)?, pf(field(&s, 2)?)?];
                    let s = jsplit(&line2);
                    let rot = Quat { x: pf(field(&s, 0)?)?, y: pf(field(&s, 1)?)?, z: pf(field(&s, 2)?)?, w: pf(field(&s, 3)?)? };
                    if n + 1 < n_frames {
                        line = need(r.read())?;
                    }
                    frames.push(Keyframe { bone: bindex, bone_name: bone, time: t, position: pos, rotation: rot, scale: [1.0; 3] });
                }
                mt.clips.insert(
                    anim_name.clone(),
                    AnimationClip { name: anim_name, duration, keep_last_frame: false, is_ragdoll: false, keyframes: frames },
                );
            }
            _ => {}
        }
    }
    if !is_static {
        if let Some(st) = skinned_to {
            remap_to(&mut mt, st, &put_order);
        }
    }
    Ok(mt)
}

/// The `skinnedTo` remapping: bones are renumbered to the animations
/// mesh's skeleton, new ones appended (Java catches and ignores errors
/// here, leaving the model partly remapped; this stops at the first one).
fn remap_to(mt: &mut ModelTxt, st: &SkinningData, put_order: &[String]) {
    let n = mt.bone_indices.len();
    let mut remap = vec![0i32; n];
    let orig_h = mt.skeleton_hierarchy.clone();
    let mut new_idx = st.bone_indices.clone();
    let mut new_h = st.skeleton_hierarchy.clone();
    for name in java_hashmap_order(put_order) {
        let oi = mt.bone_indices[&name];
        let mut ri = new_idx.get(&name).copied().unwrap_or(-1);
        if ri == -1 {
            ri = new_idx.len() as i32;
            new_idx.insert(name.clone(), ri);
            let Some(&op) = orig_h.get(oi as usize) else { return };
            if op >= 0 {
                let Some(&rp) = remap.get(op as usize) else { return };
                new_h.push(rp);
            }
        }
        let Some(slot) = remap.get_mut(oi as usize) else { return };
        *slot = ri;
    }
    mt.bone_indices = new_idx;
    mt.skeleton_hierarchy = new_h;
    let stride = mt.stride();
    for v in mt.vertices.chunks_mut(stride) {
        for k in 0..4 {
            let b = v[15 + k] as i32;
            if b >= 0 {
                let Some(&r) = remap.get(b as usize) else { return };
                v[15 + k] = r as f32;
            }
        }
    }
    for clip in mt.clips.values_mut() {
        for kf in clip.keyframes.iter_mut() {
            let Some(&r) = remap.get(kf.bone as usize) else { return };
            kf.bone = r;
        }
    }
    let count = mt.bone_indices.len();
    let remap_mats = |orig: &[Mat4]| -> Option<Vec<Mat4>> {
        let mut out = vec![LMat::IDENTITY.to_mat4(); count];
        for (i, &r) in remap.iter().enumerate() {
            *out.get_mut(r as usize)? = *orig.get(i)?;
        }
        Some(out)
    };
    if let Some(m) = remap_mats(&mt.skin_offset) {
        mt.skin_offset = m;
    } else {
        return;
    }
    if let Some(m) = remap_mats(&mt.bind_pose) {
        mt.bind_pose = m;
    } else {
        return;
    }
    if let Some(m) = remap_mats(&mt.inv_bind_pose) {
        mt.inv_bind_pose = m;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hashmap_iteration() {
        // "a" (97) and "b" (98) land in buckets 1 and 2 of 16.
        let keys = vec!["b".to_string(), "a".to_string()];
        assert_eq!(java_hashmap_order(&keys), vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn static_wheel() {
        let txt = "# Project Zomboid Mesh\n# File Version:\n1.00000000\n# Model Name:\nWheel\n\
# Vertex Stride Element Count:\n3\n# Vertex Stride Size (in bytes):\n76\n# Vertex Stride Data:\n\
0\nVertexArray\n12\nNormalArray\n24\nTextureCoordArray\n# Vertex Count:\n3\n# Vertex Buffer:\n\
0.0, 0.0, 0.0\n0.0, 1.0, 0.0\n0.25, 0.5\n\
1.0, 0.0, 0.0\n0.0, 1.0, 0.0\n0.75, 0.5\n\
0.0, 0.0, 1.0\n0.0, 1.0, 0.0\n0.25, 1.0\n\
# Number of Faces:\n1\n# Face Data:\n0, 1, 2\n";
        let mt = load_txt(txt.as_bytes(), true, false, None).unwrap();
        assert_eq!(mt.vertices.len(), 3 * 11);
        assert_eq!(&mt.vertices[11..22], &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.75, 0.5]);
        assert_eq!(mt.elements, vec![0, 1, 2]);
    }
}
