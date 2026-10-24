//! `ProcessedAiScene`, `ImportedSkeleton`, `ImportedSkinnedMesh` and
//! `ImportedStaticMesh`: how the game turns an imported [`AiScene`] into
//! vertex arrays, a skeleton and animation clips.
//!
//! The Java side does its matrix math with `org.lwjgl.util.vector`
//! (`Matrix4f`, `Quaternion`); [`LMat`] and the quaternion helpers here
//! port those operations as written so results round the same way.
//! Matrices leave this module as [`Mat4`] (column-major, column vectors),
//! which is the transpose of LWJGL's field layout.

use std::collections::HashMap;

use crate::import::aimath::AiMat4;
use crate::import::scene::{AiAnimation, AiNodeAnim, AiScene, MAX_UV};
use crate::{AnimationClip, Keyframe, Mat4, Quat, SkinningData};

/// `JAssImpImporter.LoadMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadMode {
    Normal,
    StaticMesh,
    AnimationOnly,
}

/// `ProcessedAiSceneParams`.
#[derive(Clone, Debug)]
pub struct ProcessParams<'a> {
    pub mode: LoadMode,
    pub skinned_to: Option<&'a SkinningData>,
    pub mesh_name: Option<String>,
    pub anim_bones_scale_modifier: f32,
    pub anim_bones_rotate_modifier: Option<Quat>,
    pub all_meshes: bool,
}

impl Default for ProcessParams<'_> {
    fn default() -> Self {
        ProcessParams {
            mode: LoadMode::Normal,
            skinned_to: None,
            mesh_name: None,
            anim_bones_scale_modifier: 1.0,
            anim_bones_rotate_modifier: None,
            all_meshes: false,
        }
    }
}

/// The rotation `FileTask_LoadMesh/LoadAnimation` apply to FBX (and glTF
/// animation) bones: `setFromAxisAngle((1, 0, 0, -PI/2))`.
pub fn fbx_bone_rotate_modifier() -> Quat {
    let (ax, ay, az, aw) = (1.0f32, 0.0f32, 0.0f32, -1.570_796_4_f32);
    let n = ((ax * ax + ay * ay + az * az) as f64).sqrt() as f32;
    let s = ((0.5 * aw as f64).sin() / n as f64) as f32;
    Quat { x: ax * s, y: ay * s, z: az * s, w: (0.5 * aw as f64).cos() as f32 }
}

/// `ImportedStaticMesh`: position, normal and one (u, v) pair per UV set
/// (`2 * num_uvs` floats) per vertex.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedStaticMesh {
    pub num_uvs: usize,
    pub vertices: Vec<f32>,
    pub elements: Vec<u32>,
    pub min_xyz: [f32; 3],
    pub max_xyz: [f32; 3],
    pub transform: Option<Mat4>,
}

impl ImportedStaticMesh {
    pub fn stride(&self) -> usize {
        6 + 2 * self.num_uvs
    }
}

/// `ImportedSkinnedMesh`: position, normal, 4 weights, 4 bone indices (as
/// floats), then the UV sets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImportedSkinnedMesh {
    pub name: String,
    pub num_uvs: usize,
    pub vertices: Vec<f32>,
    pub elements: Vec<u32>,
    pub transform: Option<Mat4>,
}

impl ImportedSkinnedMesh {
    pub fn stride(&self) -> usize {
        14 + 2 * self.num_uvs
    }
}

/// `ProcessedAiScene` after `process`.
#[derive(Clone, Debug, Default)]
pub struct ProcessedAiScene {
    /// The skeleton and its clips (`ImportedSkeleton`), as `SkinningData`.
    pub skeleton: Option<SkinningData>,
    pub skinned_mesh: Option<ImportedSkinnedMesh>,
    pub static_meshes: Vec<ImportedStaticMesh>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessError(pub String);

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProcessError {}

// ---------------------------------------------------------------------------
// LWJGL math

/// `org.lwjgl.util.vector.Matrix4f` fields: `m[i][j]` is `mij`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LMat {
    pub m: [[f32; 4]; 4],
}

impl LMat {
    pub const IDENTITY: LMat = LMat {
        m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
    };

    /// `JAssImpImporter.getMatrixFromAiMatrix`: `mij = ai.get(i, j)`.
    pub fn from_ai(a: &AiMat4) -> LMat {
        LMat { m: a.m }
    }

    /// To a column-major, column-vector [`Mat4`].
    pub fn to_mat4(&self) -> Mat4 {
        let mut r = [[0.0f32; 4]; 4];
        for (i, row) in self.m.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                r[j][i] = *v;
            }
        }
        r
    }

    pub fn from_mat4(c: &Mat4) -> LMat {
        let mut r = [[0.0f32; 4]; 4];
        for (i, row) in r.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = c[j][i];
            }
        }
        LMat { m: r }
    }

    /// `Matrix4f.mul(left, right, dest)`.
    pub fn mul(left: &LMat, right: &LMat) -> LMat {
        let l = &left.m;
        let r = &right.m;
        let mut d = [[0.0f32; 4]; 4];
        for i in 0..4 {
            for j in 0..4 {
                d[i][j] = l[0][j] * r[i][0] + l[1][j] * r[i][1] + l[2][j] * r[i][2] + l[3][j] * r[i][3];
            }
        }
        LMat { m: d }
    }

    pub fn determinant(&self) -> f32 {
        let m = &self.m;
        let mut f = m[0][0]
            * (m[1][1] * m[2][2] * m[3][3] + m[1][2] * m[2][3] * m[3][1] + m[1][3] * m[2][1] * m[3][2]
                - m[1][3] * m[2][2] * m[3][1]
                - m[1][1] * m[2][3] * m[3][2]
                - m[1][2] * m[2][1] * m[3][3]);
        f -= m[0][1]
            * (m[1][0] * m[2][2] * m[3][3] + m[1][2] * m[2][3] * m[3][0] + m[1][3] * m[2][0] * m[3][2]
                - m[1][3] * m[2][2] * m[3][0]
                - m[1][0] * m[2][3] * m[3][2]
                - m[1][2] * m[2][0] * m[3][3]);
        f += m[0][2]
            * (m[1][0] * m[2][1] * m[3][3] + m[1][1] * m[2][3] * m[3][0] + m[1][3] * m[2][0] * m[3][1]
                - m[1][3] * m[2][1] * m[3][0]
                - m[1][0] * m[2][3] * m[3][1]
                - m[1][1] * m[2][0] * m[3][3]);
        f -= m[0][3]
            * (m[1][0] * m[2][1] * m[3][2] + m[1][1] * m[2][2] * m[3][0] + m[1][2] * m[2][0] * m[3][1]
                - m[1][2] * m[2][1] * m[3][0]
                - m[1][0] * m[2][2] * m[3][1]
                - m[1][1] * m[2][0] * m[3][2]);
        f
    }

    /// `Matrix4f.invert()`: unchanged when singular.
    pub fn invert(&self) -> LMat {
        let det = self.determinant();
        if det == 0.0 {
            return *self;
        }
        let inv = 1.0 / det;
        let s = &self.m;
        let d3 = |t00: f32, t01: f32, t02: f32, t10: f32, t11: f32, t12: f32, t20: f32, t21: f32, t22: f32| {
            t00 * (t11 * t22 - t12 * t21) + t01 * (t12 * t20 - t10 * t22) + t02 * (t10 * t21 - t11 * t20)
        };
        let t00 = d3(s[1][1], s[1][2], s[1][3], s[2][1], s[2][2], s[2][3], s[3][1], s[3][2], s[3][3]);
        let t01 = -d3(s[1][0], s[1][2], s[1][3], s[2][0], s[2][2], s[2][3], s[3][0], s[3][2], s[3][3]);
        let t02 = d3(s[1][0], s[1][1], s[1][3], s[2][0], s[2][1], s[2][3], s[3][0], s[3][1], s[3][3]);
        let t03 = -d3(s[1][0], s[1][1], s[1][2], s[2][0], s[2][1], s[2][2], s[3][0], s[3][1], s[3][2]);
        let t10 = -d3(s[0][1], s[0][2], s[0][3], s[2][1], s[2][2], s[2][3], s[3][1], s[3][2], s[3][3]);
        let t11 = d3(s[0][0], s[0][2], s[0][3], s[2][0], s[2][2], s[2][3], s[3][0], s[3][2], s[3][3]);
        let t12 = -d3(s[0][0], s[0][1], s[0][3], s[2][0], s[2][1], s[2][3], s[3][0], s[3][1], s[3][3]);
        let t13 = d3(s[0][0], s[0][1], s[0][2], s[2][0], s[2][1], s[2][2], s[3][0], s[3][1], s[3][2]);
        let t20 = d3(s[0][1], s[0][2], s[0][3], s[1][1], s[1][2], s[1][3], s[3][1], s[3][2], s[3][3]);
        let t21 = -d3(s[0][0], s[0][2], s[0][3], s[1][0], s[1][2], s[1][3], s[3][0], s[3][2], s[3][3]);
        let t22 = d3(s[0][0], s[0][1], s[0][3], s[1][0], s[1][1], s[1][3], s[3][0], s[3][1], s[3][3]);
        let t23 = -d3(s[0][0], s[0][1], s[0][2], s[1][0], s[1][1], s[1][2], s[3][0], s[3][1], s[3][2]);
        let t30 = -d3(s[0][1], s[0][2], s[0][3], s[1][1], s[1][2], s[1][3], s[2][1], s[2][2], s[2][3]);
        let t31 = d3(s[0][0], s[0][2], s[0][3], s[1][0], s[1][2], s[1][3], s[2][0], s[2][2], s[2][3]);
        let t32 = -d3(s[0][0], s[0][1], s[0][3], s[1][0], s[1][1], s[1][3], s[2][0], s[2][1], s[2][3]);
        let t33 = d3(s[0][0], s[0][1], s[0][2], s[1][0], s[1][1], s[1][2], s[2][0], s[2][1], s[2][2]);
        let mut d = [[0.0f32; 4]; 4];
        d[0][0] = t00 * inv;
        d[1][1] = t11 * inv;
        d[2][2] = t22 * inv;
        d[3][3] = t33 * inv;
        d[0][1] = t10 * inv;
        d[1][0] = t01 * inv;
        d[2][0] = t02 * inv;
        d[0][2] = t20 * inv;
        d[1][2] = t21 * inv;
        d[2][1] = t12 * inv;
        d[0][3] = t30 * inv;
        d[3][0] = t03 * inv;
        d[1][3] = t31 * inv;
        d[3][1] = t13 * inv;
        d[3][2] = t23 * inv;
        d[2][3] = t32 * inv;
        LMat { m: d }
    }
}

/// `Quaternion.mul(left, right, dest)`.
pub fn quat_mul(l: &Quat, r: &Quat) -> Quat {
    Quat {
        x: l.x * r.w + l.w * r.x + l.y * r.z - l.z * r.y,
        y: l.y * r.w + l.w * r.y + l.z * r.x - l.x * r.z,
        z: l.z * r.w + l.w * r.z + l.x * r.y - l.y * r.x,
        w: l.w * r.w - l.x * r.x - l.y * r.y - l.z * r.z,
    }
}

/// `Quaternion.mulInverse(left, right, dest)`.
pub fn quat_mul_inverse(l: &Quat, r: &Quat) -> Quat {
    let mut n = r.x * r.x + r.y * r.y + r.z * r.z + r.w * r.w;
    n = if n as f64 == 0.0 { n } else { 1.0 / n };
    Quat {
        x: (l.x * r.w - l.w * r.x - l.y * r.z + l.z * r.y) * n,
        y: (l.y * r.w - l.w * r.y - l.z * r.x + l.x * r.z) * n,
        z: (l.z * r.w - l.w * r.z - l.x * r.y + l.y * r.x) * n,
        w: (l.w * r.w + l.x * r.x + l.y * r.y + l.z * r.z) * n,
    }
}

/// `HelperFunctions.transform(Quaternion, Vector3f, Vector3f)`; it
/// normalises the quaternion in place first.
pub fn quat_transform(q: &mut Quat, v: [f32; 3]) -> [f32; 3] {
    let len = ((q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w) as f64).sqrt() as f32;
    if len != 0.0 {
        let l = 1.0 / len;
        q.x *= l;
        q.y *= l;
        q.z *= l;
        q.w *= l;
    }
    let s = q.w;
    let (vx, vy, vz) = (q.x, q.y, q.z);
    let s2 = s * s;
    let v2 = vx * vx + vy * vy + vz * vz;
    let (px, py, pz) = (v[0], v[1], v[2]);
    let cx = vy * pz - vz * py;
    let cy = vz * px - vx * pz;
    let cz = vx * py - vy * px;
    let dot = px * vx + py * vy + pz * vz;
    [
        (s2 - v2) * px + 2.0 * s * cx + 2.0 * vx * dot,
        (s2 - v2) * py + 2.0 * s * cy + 2.0 * vy * dot,
        (s2 - v2) * pz + 2.0 * s * cz + 2.0 * vz * dot,
    ]
}

fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && (a.to_bits() == (-0.0f32).to_bits() || b.to_bits() == (-0.0f32).to_bits()) {
        return -0.0;
    }
    if a <= b {
        a
    } else {
        b
    }
}

fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && (a.to_bits() == 0 || b.to_bits() == 0) {
        return 0.0;
    }
    if a >= b {
        a
    } else {
        b
    }
}

// ---------------------------------------------------------------------------
// ProcessedAiScene

impl ProcessedAiScene {
    /// `ProcessedAiScene.process`.
    pub fn process(scene: &AiScene, params: &ProcessParams) -> Result<ProcessedAiScene, ProcessError> {
        let mut out = ProcessedAiScene::default();
        if params.all_meshes {
            if params.mode != LoadMode::StaticMesh {
                return Ok(out);
            }
            for mi in 0..scene.meshes.len() {
                let mut sm = imported_static_mesh(scene, mi);
                sm.transform = init_mesh_transform(scene, mi).map(|m| m.to_mat4());
                out.static_meshes.push(sm);
            }
            return Ok(out);
        }
        let Some(mi) = find_mesh(scene, params.mesh_name.as_deref()) else {
            // "No such mesh": nothing is produced.
            return Ok(out);
        };
        let mesh = &scene.meshes[mi];
        if params.mode == LoadMode::StaticMesh || mesh.bones.is_empty() {
            let mut sm = imported_static_mesh(scene, mi);
            sm.transform = init_mesh_transform(scene, mi).map(|m| m.to_mat4());
            out.static_meshes.push(sm);
        } else {
            let sk = ImportedSkeleton::process(scene, mi, params)?;
            if params.mode != LoadMode::AnimationOnly {
                let mut skm = imported_skinned_mesh(scene, mi, &sk)?;
                skm.transform = init_mesh_transform(scene, mi).map(|m| m.to_mat4());
                out.skinned_mesh = Some(skm);
            }
            out.skeleton = Some(sk.into_skinning_data());
        }
        Ok(out)
    }
}

fn find_mesh(scene: &AiScene, name: Option<&str>) -> Option<usize> {
    if scene.meshes.is_empty() {
        return None;
    }
    let name = name.filter(|n| !n.trim().is_empty());
    let Some(name) = name else {
        if let Some(i) = scene.meshes.iter().position(|m| !m.bones.is_empty()) {
            return Some(i);
        }
        return Some(0);
    };
    if let Some(i) = scene.meshes.iter().position(|m| java_equals_ignore_case(&m.name, name)) {
        return Some(i);
    }
    let node = find_node_below(scene, scene.root, name)?;
    if scene.nodes[node].meshes.len() == 1 {
        return Some(scene.nodes[node].meshes[0] as usize);
    }
    None
}

fn java_equals_ignore_case(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    a.len() == b.len()
        && a.iter().zip(b.iter()).all(|(x, y)| {
            x == y
                || x.to_uppercase().eq(y.to_uppercase())
                || x.to_lowercase().eq(y.to_lowercase())
        })
}

/// `JAssImpImporter.FindNode`: children only, each before its subtree.
fn find_node_below(scene: &AiScene, node: usize, name: &str) -> Option<usize> {
    for &c in &scene.nodes[node].children {
        if scene.nodes[c].name == name {
            return Some(c);
        }
        if let Some(n) = find_node_below(scene, c, name) {
            return Some(n);
        }
    }
    None
}

fn find_parent_node_for_mesh(scene: &AiScene, mesh: u32, node: usize) -> Option<usize> {
    if scene.nodes[node].meshes.contains(&mesh) {
        return Some(node);
    }
    for &c in &scene.nodes[node].children {
        if let Some(n) = find_parent_node_for_mesh(scene, mesh, c) {
            return Some(n);
        }
    }
    None
}

fn init_mesh_transform(scene: &AiScene, mesh: usize) -> Option<LMat> {
    let node = find_parent_node_for_mesh(scene, mesh as u32, scene.root)?;
    let mut t = LMat::from_ai(&scene.nodes[node].transform);
    let mut p = scene.nodes[node].parent;
    while let Some(pi) = p {
        let px = LMat::from_ai(&scene.nodes[pi].transform);
        t = LMat::mul(&px, &t);
        p = scene.nodes[pi].parent;
    }
    Some(t)
}

fn num_uvs(m: &crate::import::scene::AiMesh) -> usize {
    (0..MAX_UV).filter(|&i| m.uvs[i].is_some()).count()
}

fn face_vertex(f: &[u32], i: usize) -> u32 {
    f.get(i).copied().unwrap_or(0)
}

fn imported_static_mesh(scene: &AiScene, mi: usize) -> ImportedStaticMesh {
    let mesh = &scene.meshes[mi];
    let nuv = num_uvs(mesh);
    let stride = 6 + 2 * nuv;
    let n = mesh.vertices.len();
    let mut out = ImportedStaticMesh {
        num_uvs: nuv,
        vertices: vec![0.0; n * stride],
        elements: vec![0; mesh.faces.len() * 3],
        min_xyz: [f32::MAX; 3],
        max_xyz: [-f32::MAX; 3],
        transform: None,
    };
    for i in 0..n {
        let p = mesh.vertices[i];
        for k in 0..3 {
            out.min_xyz[k] = java_min(out.min_xyz[k], p[k]);
            out.max_xyz[k] = java_max(out.max_xyz[k], p[k]);
        }
        let v = &mut out.vertices[i * stride..(i + 1) * stride];
        v[0..3].copy_from_slice(&p);
        match &mesh.normals {
            Some(ns) => v[3..6].copy_from_slice(&ns[i]),
            None => v[3..6].copy_from_slice(&[0.0, 1.0, 0.0]),
        }
        let mut k = 0;
        for set in 0..MAX_UV {
            if let Some(uv) = &mesh.uvs[set] {
                v[6 + 2 * k] = uv[i][0];
                v[7 + 2 * k] = 1.0 - uv[i][1];
                k += 1;
            }
        }
    }
    for (f, face) in mesh.faces.iter().enumerate() {
        out.elements[f * 3 + 2] = face_vertex(face, 0);
        out.elements[f * 3 + 1] = face_vertex(face, 1);
        out.elements[f * 3] = face_vertex(face, 2);
    }
    out
}

fn imported_skinned_mesh(scene: &AiScene, mi: usize, sk: &ImportedSkeleton) -> Result<ImportedSkinnedMesh, ProcessError> {
    let mesh = &scene.meshes[mi];
    let n = mesh.vertices.len();
    let mut ids = vec![0u32; n * 4];
    let mut weights = vec![0.0f32; n * 4];
    for bone in &mesh.bones {
        let Some(&bi) = sk.bone_indices.get(&bone.name) else {
            return Err(ProcessError(format!("mesh bone \"{}\" is not in the skeleton", bone.name)));
        };
        'w: for &(vid, w) in &bone.weights {
            let start = vid as usize * 4;
            for k in 0..4 {
                if weights[start + k] != 0.0 {
                    continue;
                }
                weights[start + k] = w;
                ids[start + k] = bi as u32;
                continue 'w;
            }
        }
    }
    for i in 0..n {
        let mut sum = 0.0f32;
        for k in 0..4 {
            sum += weights[i * 4 + k];
        }
        if sum == 0.0 {
            continue;
        }
        let inv = 1.0 / sum;
        for k in 0..4 {
            weights[i * 4 + k] *= inv;
        }
    }
    let nuv = num_uvs(mesh);
    let stride = 14 + 2 * nuv;
    let mut out = ImportedSkinnedMesh {
        name: mesh.name.clone(),
        num_uvs: nuv,
        vertices: vec![0.0; n * stride],
        elements: vec![0; mesh.faces.len() * 3],
        transform: None,
    };
    for i in 0..n {
        let v = &mut out.vertices[i * stride..(i + 1) * stride];
        v[0..3].copy_from_slice(&mesh.vertices[i]);
        match &mesh.normals {
            Some(ns) => v[3..6].copy_from_slice(&ns[i]),
            None => v[3..6].copy_from_slice(&[0.0, 1.0, 0.0]),
        }
        v[6..10].copy_from_slice(&weights[i * 4..i * 4 + 4]);
        for k in 0..4 {
            v[10 + k] = ids[i * 4 + k] as i32 as f32;
        }
        let mut k = 0;
        for set in 0..MAX_UV {
            if let Some(uv) = &mesh.uvs[set] {
                v[14 + 2 * k] = uv[i][0];
                v[15 + 2 * k] = 1.0 - uv[i][1];
                k += 1;
            }
        }
    }
    for (f, face) in mesh.faces.iter().enumerate() {
        out.elements[f * 3 + 2] = face_vertex(face, 0);
        out.elements[f * 3 + 1] = face_vertex(face, 1);
        out.elements[f * 3] = face_vertex(face, 2);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// ImportedSkeleton

struct ImportedSkeleton {
    bone_indices: HashMap<String, i32>,
    skeleton_hierarchy: Vec<i32>,
    bind_pose: Vec<LMat>,
    inv_bind_pose: Vec<LMat>,
    skin_offset: Vec<LMat>,
    clips: HashMap<String, AnimationClip>,
}

impl ImportedSkeleton {
    fn into_skinning_data(self) -> SkinningData {
        SkinningData {
            bind_pose: self.bind_pose.iter().map(LMat::to_mat4).collect(),
            inverse_bind_pose: self.inv_bind_pose.iter().map(LMat::to_mat4).collect(),
            bone_offset: self.skin_offset.iter().take(self.skeleton_hierarchy.len()).map(LMat::to_mat4).collect(),
            skeleton_hierarchy: self.skeleton_hierarchy,
            bone_indices: self.bone_indices,
            animation_clips: self.clips,
        }
    }

    fn process(scene: &AiScene, mi: usize, params: &ProcessParams) -> Result<ImportedSkeleton, ProcessError> {
        let root = scene.root;
        let mut vehicle = false;
        let mut root_bone = find_node_below(scene, root, "Dummy01");
        if root_bone.is_none() {
            root_bone = find_node_below(scene, root, "VehicleSkeleton");
            vehicle = true;
        }
        while let Some(rb) = root_bone {
            match scene.nodes[rb].parent {
                Some(p) if p != root => root_bone = Some(p),
                _ => break,
            }
        }
        let root_bone = root_bone.unwrap_or(root);
        let mut bone_nodes = Vec::new();
        collect_bone_nodes(scene, root_bone, &mut bone_nodes);
        if let Some(t) = find_node_below(scene, root, "Translation_Data") {
            bone_nodes.push(t);
            let mut p = scene.nodes[t].parent;
            while let Some(pi) = p {
                if pi == root {
                    break;
                }
                bone_nodes.push(pi);
                p = scene.nodes[pi].parent;
            }
        }
        let mut sk = ImportedSkeleton {
            bone_indices: HashMap::new(),
            skeleton_hierarchy: Vec::new(),
            bind_pose: Vec::new(),
            inv_bind_pose: Vec::new(),
            skin_offset: Vec::new(),
            clips: HashMap::new(),
        };
        if let Some(st) = params.skinned_to {
            sk.bone_indices.extend(st.bone_indices.iter().map(|(k, v)| (k.clone(), *v)));
            sk.skeleton_hierarchy.extend_from_slice(&st.skeleton_hierarchy);
        }
        for &n in &bone_nodes {
            let name = &scene.nodes[n].name;
            if sk.bone_indices.contains_key(name) {
                continue;
            }
            let index = sk.bone_indices.len() as i32;
            sk.bone_indices.insert(name.clone(), index);
            if n == root_bone {
                sk.skeleton_hierarchy.push(-1);
                continue;
            }
            let mut p = scene.nodes[n].parent;
            while let Some(pi) = p {
                if sk.bone_indices.contains_key(&scene.nodes[pi].name) {
                    break;
                }
                p = scene.nodes[pi].parent;
            }
            match p {
                Some(pi) => sk.skeleton_hierarchy.push(sk.bone_indices[&scene.nodes[pi].name]),
                None => sk.skeleton_hierarchy.push(0),
            }
        }
        let nb = sk.bone_indices.len();
        sk.bind_pose = vec![LMat::IDENTITY; nb];
        sk.skin_offset = vec![LMat::IDENTITY; nb];
        let bones = &scene.meshes[mi].bones;
        // jassimp only fills a bone's offset matrix inside its weight loop,
        // so bones without weights reach Java with a null offset and are
        // skipped here.
        let find_bone = |name: &str| bones.iter().find(|b| b.name == name).filter(|b| !b.weights.is_empty());
        for &n in &bone_nodes {
            let name = &scene.nodes[n].name;
            let Some(bone) = find_bone(name) else { continue };
            let skin_offset = LMat::from_ai(&bone.offset);
            let skin_offset_i = skin_offset.invert();
            let Some(parent) = scene.nodes[n].parent else {
                return Err(ProcessError(format!("bone node \"{}\" has no parent", name)));
            };
            let mut skin_offset_parent = LMat::IDENTITY;
            if let Some(pb) = find_bone(&scene.nodes[parent].name) {
                skin_offset_parent = LMat::from_ai(&pb.offset);
            }
            let skin_offset_parent_i = skin_offset_parent.invert();
            let bind = LMat::mul(&skin_offset_i, &skin_offset_parent_i).invert();
            let bi = sk.bone_indices[name] as usize;
            sk.bind_pose[bi] = bind;
            sk.skin_offset[bi] = skin_offset;
        }
        sk.inv_bind_pose = sk.bind_pose.iter().map(|b| b.invert()).collect();
        for anim in &scene.animations {
            if vehicle {
                sk.process_animation(anim, true, 1.0, None);
            } else {
                sk.process_animation(anim, false, params.anim_bones_scale_modifier, params.anim_bones_rotate_modifier);
            }
        }
        Ok(sk)
    }

    fn parent_bone_idx(&self, bone: i32) -> i32 {
        if bone > -1 {
            self.skeleton_hierarchy.get(bone as usize).copied().unwrap_or(-1)
        } else {
            -1
        }
    }

    fn process_animation(&mut self, anim: &AiAnimation, vehicle: bool, scale_mod: f32, rot_mod: Option<Quat>) {
        let duration = anim.duration as f32;
        let tps = anim.ticks_per_second as f32;
        let duration_seconds = duration / tps;
        let nb = self.bone_indices.len();
        let mut frame_times: Vec<Option<Vec<f32>>> = vec![None; nb];
        let mut channels_per_bone: Vec<Option<Vec<&AiNodeAnim>>> = vec![None; nb];
        for ch in &anim.channels {
            let Some(&bi) = self.bone_indices.get(&ch.node_name) else { continue };
            let bi = bi as usize;
            if bi >= nb {
                continue;
            }
            channels_per_bone[bi].get_or_insert_with(Vec::new).push(ch);
            let ft = frame_times[bi].get_or_insert_with(Vec::new);
            for t in ch
                .position_keys
                .iter()
                .map(|k| k.time as f32)
                .chain(ch.rotation_keys.iter().map(|k| k.time as f32))
                .chain(ch.scaling_keys.iter().map(|k| k.time as f32))
            {
                // TFloatArrayList.contains compares with ==.
                if !ft.iter().any(|&x| x == t) {
                    ft.push(t);
                }
            }
        }
        let apply_rot = rot_mod.is_some();
        let mut rot_inv = rot_mod.map(|m| quat_mul_inverse(&Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }, &m));
        let mut frames: Vec<Keyframe> = Vec::new();
        for bone in 0..nb {
            let Some(chans) = &channels_per_bone[bone] else {
                if bone != 0 || rot_mod.is_none() {
                    continue;
                }
                let rotation = rot_mod.unwrap();
                for time in [0.0, duration_seconds] {
                    frames.push(Keyframe {
                        bone: bone as i32,
                        bone_name: "RootNode".into(),
                        time,
                        position: [0.0; 3],
                        rotation,
                        scale: [1.0; 3],
                    });
                }
                continue;
            };
            match frame_times[bone].as_mut() {
                Some(times) => times.sort_by(|a, b| a.total_cmp(b)),
                None => continue,
            }
            let parent = self.parent_bone_idx(bone as i32);
            let parent_adjusted = apply_rot && (parent == 0 || self.bone_has_frames(&frame_times, &channels_per_bone, parent));
            let times = frame_times[bone].clone().unwrap();
            for &ft in &times {
                let secs = ft / tps;
                for a in chans {
                    let (mut position, mut rotation, scale) = if !vehicle {
                        (key_position(a, ft), key_rotation(a, ft), key_scale(a, ft))
                    } else {
                        (
                            vehicle_key_position(a, ft, anim.duration),
                            vehicle_key_rotation(a, ft, anim.duration),
                            vehicle_key_scale(a, ft),
                        )
                    };
                    position = [position[0] * scale_mod, position[1] * scale_mod, position[2] * scale_mod];
                    if let (true, Some(m)) = (apply_rot, rot_mod) {
                        if parent_adjusted {
                            let inv = rot_inv.as_mut().unwrap();
                            rotation = quat_mul(inv, &rotation);
                            if !starts_with_ignore_case(&a.node_name, "Translation_Data") {
                                position = quat_transform(inv, position);
                            }
                        }
                        rotation = quat_mul(&rotation, &m);
                    }
                    frames.push(Keyframe {
                        bone: bone as i32,
                        bone_name: a.node_name.clone(),
                        time: secs,
                        position,
                        rotation,
                        scale,
                    });
                }
            }
        }
        let mut name = anim.name.as_str();
        if let Some(p) = name.find('|') {
            if p > 0 {
                name = &name[p + 1..];
            }
        }
        let name = java_trim(name).to_string();
        self.clips.insert(
            name.clone(),
            AnimationClip { name, duration: duration_seconds, keep_last_frame: true, is_ragdoll: false, keyframes: frames },
        );
    }

    fn bone_has_frames(&self, ft: &[Option<Vec<f32>>], ch: &[Option<Vec<&AiNodeAnim>>], bone: i32) -> bool {
        if bone < 0 {
            return false;
        }
        let b = bone as usize;
        match (&ft.get(b), &ch.get(b)) {
            (Some(Some(t)), Some(Some(c))) => !t.is_empty() && !c.is_empty(),
            _ => false,
        }
    }
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

/// `String.trim()`: strips chars <= ' '.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

fn collect_bone_nodes(scene: &AiScene, n: usize, out: &mut Vec<usize>) {
    out.push(n);
    for &c in &scene.nodes[n].children {
        collect_bone_nodes(scene, c, out);
    }
}

// JAssImpImporter.GetKeyFrame*: exact key or linear blend of the
// bracketing keys (rotations are lerped component-wise, not normalised).

fn key_position(a: &AiNodeAnim, time: f32) -> [f32; 3] {
    let keys = &a.position_keys;
    let mut sidx: i32 = -1;
    for (k, key) in keys.iter().enumerate() {
        let t = key.time as f32;
        if t > time {
            break;
        }
        sidx = k as i32;
        if t == time {
            return key.value;
        }
    }
    if sidx < 0 {
        return [0.0; 3];
    }
    let s = sidx as usize;
    if keys.len() > s + 1 {
        let t1 = keys[s].time as f32;
        let t2 = keys[s + 1].time as f32;
        let r = t2 - t1;
        let f = (time - t1) / r;
        let (a, b) = (keys[s].value, keys[s + 1].value);
        return [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1]), a[2] + f * (b[2] - a[2])];
    }
    keys[s].value
}

fn key_scale(a: &AiNodeAnim, time: f32) -> [f32; 3] {
    let keys = &a.scaling_keys;
    let mut sidx: i32 = -1;
    for (k, key) in keys.iter().enumerate() {
        let t = key.time as f32;
        if t > time {
            break;
        }
        sidx = k as i32;
        if t == time {
            return key.value;
        }
    }
    if sidx < 0 {
        return [1.0; 3];
    }
    let s = sidx as usize;
    if keys.len() > s + 1 {
        let t1 = keys[s].time as f32;
        let t2 = keys[s + 1].time as f32;
        let r = t2 - t1;
        let f = (time - t1) / r;
        let (a, b) = (keys[s].value, keys[s + 1].value);
        return [a[0] + f * (b[0] - a[0]), a[1] + f * (b[1] - a[1]), a[2] + f * (b[2] - a[2])];
    }
    keys[s].value
}

fn q(k: &crate::import::aimath::AiQuat) -> Quat {
    Quat { x: k.x, y: k.y, z: k.z, w: k.w }
}

fn key_rotation(a: &AiNodeAnim, time: f32) -> Quat {
    let keys = &a.rotation_keys;
    let mut sidx: i32 = -1;
    for (k, key) in keys.iter().enumerate() {
        let t = key.time as f32;
        if t > time {
            break;
        }
        sidx = k as i32;
        if t == time {
            return q(&key.value);
        }
    }
    if sidx < 0 {
        return Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
    }
    let s = sidx as usize;
    if keys.len() > s + 1 {
        let t1 = keys[s].time as f32;
        let t2 = keys[s + 1].time as f32;
        let r = t2 - t1;
        let f = (time - t1) / r;
        let (a, b) = (q(&keys[s].value), q(&keys[s + 1].value));
        return Quat {
            x: a.x + f * (b.x - a.x),
            y: a.y + f * (b.y - a.y),
            z: a.z + f * (b.z - a.z),
            w: a.w + f * (b.w - a.w),
        };
    }
    q(&keys[s].value)
}

fn vehicle_frame(times: impl Iterator<Item = f64> + Clone, n: usize, time: f32) -> usize {
    let t: Vec<f64> = times.collect();
    let mut frame = 0;
    while frame < n - 1 && (time as f64) >= t[frame + 1] {
        frame += 1;
    }
    frame
}

fn vehicle_key_position(a: &AiNodeAnim, time: f32, duration: f64) -> [f32; 3] {
    let keys = &a.position_keys;
    if keys.is_empty() {
        return [0.0; 3];
    }
    let frame = vehicle_frame(keys.iter().map(|k| k.time), keys.len(), time);
    let next = (frame + 1) % keys.len();
    let t1 = keys[frame].time as f32;
    let t2 = keys[next].time as f32;
    let mut diff = t2 - t1;
    if diff < 0.0 {
        diff = (diff as f64 + duration) as f32;
    }
    if diff > 0.0 {
        let r = t2 - t1;
        let s = (time - t1) / r;
        let (a, b) = (keys[frame].value, keys[next].value);
        [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1]), a[2] + s * (b[2] - a[2])]
    } else {
        keys[frame].value
    }
}

fn vehicle_key_rotation(a: &AiNodeAnim, time: f32, duration: f64) -> Quat {
    let keys = &a.rotation_keys;
    if keys.is_empty() {
        return Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };
    }
    let frame = vehicle_frame(keys.iter().map(|k| k.time), keys.len(), time);
    let next = (frame + 1) % keys.len();
    let t1 = keys[frame].time as f32;
    let t2 = keys[next].time as f32;
    let mut diff = t2 - t1;
    if diff < 0.0 {
        diff = (diff as f64 + duration) as f32;
    }
    if diff > 0.0 {
        let factor = (time - t1) / diff;
        let s = keys[frame].value;
        let e = keys[next].value;
        let mut cosom = (s.x * e.x + s.y * e.y + s.z * e.z + s.w * e.w) as f64;
        let mut end = Quat { x: e.x, y: e.y, z: e.z, w: e.w };
        if cosom < 0.0 {
            cosom = -cosom;
            end = Quat { x: -end.x, y: -end.y, z: -end.z, w: -end.w };
        }
        let (sclp, sclq);
        if 1.0 - cosom > 1.0e-4 {
            let omega = cosom.acos();
            let sinom = omega.sin();
            sclp = ((1.0 - factor as f64) * omega).sin() / sinom;
            sclq = (factor as f64 * omega).sin() / sinom;
        } else {
            sclp = 1.0 - factor as f64;
            sclq = factor as f64;
        }
        Quat {
            x: (sclp * s.x as f64 + sclq * end.x as f64) as f32,
            y: (sclp * s.y as f64 + sclq * end.y as f64) as f32,
            z: (sclp * s.z as f64 + sclq * end.z as f64) as f32,
            w: (sclp * s.w as f64 + sclq * end.w as f64) as f32,
        }
    } else {
        q(&keys[frame].value)
    }
}

fn vehicle_key_scale(a: &AiNodeAnim, time: f32) -> [f32; 3] {
    let keys = &a.scaling_keys;
    if keys.is_empty() {
        return [1.0; 3];
    }
    let frame = vehicle_frame(keys.iter().map(|k| k.time), keys.len(), time);
    keys[frame].value
}

impl ImportedStaticMesh {
    /// The render layout (`MeshKind::Static`: position, normal, the first
    /// UV set; zeros when the mesh has none).
    pub fn to_mesh_data(&self, id: u64) -> crate::MeshData {
        let s = self.stride();
        let mut v = Vec::with_capacity(self.vertices.len() / s.max(1) * 8);
        for c in self.vertices.chunks_exact(s) {
            v.extend_from_slice(&c[0..6]);
            if self.num_uvs > 0 {
                v.extend_from_slice(&c[6..8]);
            } else {
                v.extend_from_slice(&[0.0, 0.0]);
            }
        }
        crate::MeshData { id, kind: crate::MeshKind::Static, vertices: v, indices: self.elements.clone() }
    }
}

impl ImportedSkinnedMesh {
    /// The render layout (`MeshKind::Skinned`, first UV set).
    pub fn to_mesh_data(&self, id: u64) -> crate::MeshData {
        let s = self.stride();
        let mut v = Vec::with_capacity(self.vertices.len() / s.max(1) * 16);
        for c in self.vertices.chunks_exact(s) {
            v.extend_from_slice(&c[0..14]);
            if self.num_uvs > 0 {
                v.extend_from_slice(&c[14..16]);
            } else {
                v.extend_from_slice(&[0.0, 0.0]);
            }
        }
        crate::MeshData { id, kind: crate::MeshKind::Skinned, vertices: v, indices: self.elements.clone() }
    }
}

impl ProcessedAiScene {
    /// `ProcessedAiScene.applyToMesh`: the first static mesh (bounds and
    /// transform) or the skinned mesh, plus the skeleton. `id` tags the
    /// mesh for GPU caching.
    pub fn into_model_mesh(self, id: u64) -> Option<crate::ModelMesh> {
        let mut min = [f32::MAX; 3];
        let mut max = [-f32::MAX; 3];
        let mut transform = None;
        let mut mesh = None;
        if let Some(sm) = self.static_meshes.first() {
            min = sm.min_xyz;
            max = sm.max_xyz;
            transform = sm.transform;
            mesh = Some(sm.to_mesh_data(id));
        }
        if let Some(sk) = &self.skinned_mesh {
            if sk.transform.is_some() {
                transform = sk.transform;
            }
            mesh = Some(sk.to_mesh_data(id));
        }
        Some(crate::ModelMesh {
            mesh: std::rc::Rc::new(mesh?),
            min_xyz: min,
            max_xyz: max,
            transform,
            skinning: self.skeleton,
        })
    }
}
