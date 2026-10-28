//! assimp's `ScenePreprocessor` and the post-processing steps the game
//! enables, run in `GetPostProcessingStepInstanceList` order:
//! MakeLeftHanded, RemoveRedundantMaterials, FindInstances, Triangulate,
//! OptimizeMeshes, JoinIdenticalVertices, LimitBoneWeights.

use std::collections::HashMap;

use super::aimath::{vlen, vnormalize, vsqlen, vsub};
use super::scene::*;
use super::{super_fast_hash, ImportError};

/// `jassimp.AiPostProcessSteps` (raw `aiProcess_*` bits).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Steps(pub u32);

impl Steps {
    pub const CALC_TANGENT_SPACE: Steps = Steps(0x1);
    pub const JOIN_IDENTICAL_VERTICES: Steps = Steps(0x2);
    pub const MAKE_LEFT_HANDED: Steps = Steps(0x4);
    pub const TRIANGULATE: Steps = Steps(0x8);
    pub const REMOVE_COMPONENT: Steps = Steps(0x10);
    pub const GEN_NORMALS: Steps = Steps(0x20);
    pub const GEN_SMOOTH_NORMALS: Steps = Steps(0x40);
    pub const SPLIT_LARGE_MESHES: Steps = Steps(0x80);
    pub const PRE_TRANSFORM_VERTICES: Steps = Steps(0x100);
    pub const LIMIT_BONE_WEIGHTS: Steps = Steps(0x200);
    pub const VALIDATE_DATA_STRUCTURE: Steps = Steps(0x400);
    pub const IMPROVE_CACHE_LOCALITY: Steps = Steps(0x800);
    pub const REMOVE_REDUNDANT_MATERIALS: Steps = Steps(0x1000);
    pub const FIX_INFACING_NORMALS: Steps = Steps(0x2000);
    pub const SORT_BY_PTYPE: Steps = Steps(0x8000);
    pub const FIND_DEGENERATES: Steps = Steps(0x10000);
    pub const FIND_INVALID_DATA: Steps = Steps(0x20000);
    pub const GEN_UV_COORDS: Steps = Steps(0x40000);
    pub const TRANSFORM_UV_COORDS: Steps = Steps(0x80000);
    pub const FIND_INSTANCES: Steps = Steps(0x100000);
    pub const OPTIMIZE_MESHES: Steps = Steps(0x200000);
    pub const OPTIMIZE_GRAPH: Steps = Steps(0x400000);
    pub const FLIP_UVS: Steps = Steps(0x800000);
    pub const FLIP_WINDING_ORDER: Steps = Steps(0x1000000);
    pub const SPLIT_BY_BONE_COUNT: Steps = Steps(0x2000000);
    pub const DEBONE: Steps = Steps(0x4000000);

    pub fn contains(self, o: Steps) -> bool {
        self.0 & o.0 == o.0
    }
    pub fn insert(&mut self, o: Steps) {
        self.0 |= o.0;
    }
    pub fn remove(&mut self, o: Steps) {
        self.0 &= !o.0;
    }

    /// `AiPostProcessSteps.valueOf(name)`.
    pub fn by_name(name: &str) -> Option<Steps> {
        Some(match name {
            "CALC_TANGENT_SPACE" => Self::CALC_TANGENT_SPACE,
            "JOIN_IDENTICAL_VERTICES" => Self::JOIN_IDENTICAL_VERTICES,
            "MAKE_LEFT_HANDED" => Self::MAKE_LEFT_HANDED,
            "TRIANGULATE" => Self::TRIANGULATE,
            "REMOVE_COMPONENT" => Self::REMOVE_COMPONENT,
            "GEN_NORMALS" => Self::GEN_NORMALS,
            "GEN_SMOOTH_NORMALS" => Self::GEN_SMOOTH_NORMALS,
            "SPLIT_LARGE_MESHES" => Self::SPLIT_LARGE_MESHES,
            "PRE_TRANSFORM_VERTICES" => Self::PRE_TRANSFORM_VERTICES,
            "LIMIT_BONE_WEIGHTS" => Self::LIMIT_BONE_WEIGHTS,
            "VALIDATE_DATA_STRUCTURE" => Self::VALIDATE_DATA_STRUCTURE,
            "IMPROVE_CACHE_LOCALITY" => Self::IMPROVE_CACHE_LOCALITY,
            "REMOVE_REDUNDANT_MATERIALS" => Self::REMOVE_REDUNDANT_MATERIALS,
            "FIX_INFACING_NORMALS" => Self::FIX_INFACING_NORMALS,
            "SORT_BY_PTYPE" => Self::SORT_BY_PTYPE,
            "FIND_DEGENERATES" => Self::FIND_DEGENERATES,
            "FIND_INVALID_DATA" => Self::FIND_INVALID_DATA,
            "GEN_UV_COORDS" => Self::GEN_UV_COORDS,
            "TRANSFORM_UV_COORDS" => Self::TRANSFORM_UV_COORDS,
            "FIND_INSTANCES" => Self::FIND_INSTANCES,
            "OPTIMIZE_MESHES" => Self::OPTIMIZE_MESHES,
            "OPTIMIZE_GRAPH" => Self::OPTIMIZE_GRAPH,
            "FLIP_UVS" => Self::FLIP_UVS,
            "FLIP_WINDING_ORDER" => Self::FLIP_WINDING_ORDER,
            "SPLIT_BY_BONE_COUNT" => Self::SPLIT_BY_BONE_COUNT,
            "DEBONE" => Self::DEBONE,
            _ => return None,
        })
    }

    /// The set `FileTask_LoadMesh` passes for meshes.
    pub fn mesh_default() -> Steps {
        Steps(
            Self::FIND_INSTANCES.0
                | Self::MAKE_LEFT_HANDED.0
                | Self::LIMIT_BONE_WEIGHTS.0
                | Self::TRIANGULATE.0
                | Self::OPTIMIZE_MESHES.0
                | Self::REMOVE_REDUNDANT_MATERIALS.0
                | Self::JOIN_IDENTICAL_VERTICES.0,
        )
    }

    /// The set `FileTask_LoadAnimation` passes.
    pub fn animation_default() -> Steps {
        Steps(Self::MAKE_LEFT_HANDED.0 | Self::REMOVE_REDUNDANT_MATERIALS.0)
    }

    /// `FileTask_LoadMesh.handlePostProcessFlags`: `+NAME;-NAME;...`.
    /// Unknown names are ignored (Java would throw).
    pub fn apply_overrides(&mut self, spec: &str) {
        for flag in spec.split(';') {
            if let Some(n) = flag.strip_prefix('+') {
                if let Some(s) = Steps::by_name(n) {
                    self.insert(s);
                }
            } else if let Some(n) = flag.strip_prefix('-') {
                if let Some(s) = Steps::by_name(n) {
                    self.remove(s);
                }
            }
        }
    }
}

/// Runs the active steps in assimp's order.
pub fn apply(scene: &mut AiScene, steps: Steps) -> Result<(), ImportError> {
    if steps.0 == 0 {
        return Ok(());
    }
    if steps.contains(Steps::MAKE_LEFT_HANDED) {
        make_left_handed(scene);
    }
    if steps.contains(Steps::FLIP_UVS) {
        flip_uvs(scene);
    }
    if steps.contains(Steps::FLIP_WINDING_ORDER) {
        flip_winding_order(scene);
    }
    if steps.contains(Steps::REMOVE_REDUNDANT_MATERIALS) {
        remove_redundant_materials(scene)?;
    }
    if steps.contains(Steps::FIND_INSTANCES) && !steps.contains(Steps::PRE_TRANSFORM_VERTICES) {
        find_instances(scene);
    }
    if steps.contains(Steps::TRIANGULATE) {
        triangulate(scene);
    }
    if steps.contains(Steps::OPTIMIZE_MESHES) {
        optimize_meshes(scene, steps.contains(Steps::SORT_BY_PTYPE))?;
    }
    if steps.contains(Steps::JOIN_IDENTICAL_VERTICES) {
        join_vertices(scene);
    }
    if steps.contains(Steps::LIMIT_BONE_WEIGHTS) {
        limit_bone_weights(scene, 4);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ScenePreprocessor

pub fn preprocess(scene: &mut AiScene) {
    for m in scene.meshes.iter_mut() {
        preprocess_mesh(m);
    }
    for ai in 0..scene.animations.len() {
        preprocess_animation(scene, ai);
    }
    if scene.materials.is_empty() && !scene.meshes.is_empty() {
        let mut helper = AiMaterial::default();
        helper.add_floats("$clr.diffuse", 0, 0, &[0.6, 0.6, 0.6]);
        helper.add_string("?mat.name", 0, 0, "DefaultMaterial");
        for m in scene.meshes.iter_mut() {
            m.material_index = 0;
        }
        scene.materials.push(helper);
    }
}

fn preprocess_mesh(mesh: &mut AiMesh) {
    for i in 0..MAX_UV {
        let n = mesh.vertices.len();
        let Some(uv) = mesh.uvs[i].as_mut() else {
            mesh.num_uv_components[i] = 0;
            continue;
        };
        if mesh.num_uv_components[i] == 0 {
            mesh.num_uv_components[i] = 2;
        }
        match mesh.num_uv_components[i] {
            2 => uv.iter_mut().take(n).for_each(|p| p[2] = 0.0),
            1 => uv.iter_mut().take(n).for_each(|p| {
                p[2] = 0.0;
                p[1] = 0.0
            }),
            3 if uv.iter().take(n).all(|p| p[2] == 0.0) => {
                mesh.num_uv_components[i] = 2;
            }
            _ => {}
        }
    }
    if mesh.primitive_types == 0 {
        for f in &mesh.faces {
            mesh.primitive_types |= match f.len() {
                3 => PT_TRIANGLE,
                2 => PT_LINE,
                1 => PT_POINT,
                _ => PT_POLYGON,
            };
        }
    }
    if let (Some(n), Some(t), None) = (&mesh.normals, &mesh.tangents, &mesh.bitangents) {
        let b = n
            .iter()
            .zip(t.iter())
            .map(|(a, b)| {
                [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ]
            })
            .collect();
        mesh.bitangents = Some(b);
    }
}

fn preprocess_animation(scene: &mut AiScene, ai: usize) {
    let mut first = 10e10f64;
    let mut last = -10e10f64;
    let duration_unset = scene.animations[ai].duration == -1.0;
    for ci in 0..scene.animations[ai].channels.len() {
        let ch = &scene.animations[ai].channels[ci];
        if duration_unset {
            for k in &ch.position_keys {
                first = first.min(k.time);
                last = last.max(k.time);
            }
            for k in &ch.scaling_keys {
                first = first.min(k.time);
                last = last.max(k.time);
            }
            for k in &ch.rotation_keys {
                first = first.min(k.time);
                last = last.max(k.time);
            }
        }
        if ch.rotation_keys.is_empty() || ch.position_keys.is_empty() || ch.scaling_keys.is_empty()
        {
            if let Some(node) = scene.find_node(&ch.node_name) {
                let (s, r, p) = scene.nodes[node].transform.decompose();
                let ch = &mut scene.animations[ai].channels[ci];
                if ch.rotation_keys.is_empty() {
                    ch.rotation_keys.push(AiQuatKey {
                        time: 0.0,
                        value: r,
                    });
                }
                if ch.scaling_keys.is_empty() {
                    ch.scaling_keys.push(AiVectorKey {
                        time: 0.0,
                        value: s,
                    });
                }
                if ch.position_keys.is_empty() {
                    ch.position_keys.push(AiVectorKey {
                        time: 0.0,
                        value: p,
                    });
                }
            }
        }
    }
    if duration_unset {
        scene.animations[ai].duration = last - first.min(0.0);
    }
}

// ---------------------------------------------------------------------------
// MakeLeftHanded / FlipUVs / FlipWindingOrder

pub fn make_left_handed(scene: &mut AiScene) {
    for n in scene.nodes.iter_mut() {
        let t = &mut n.transform.m;
        t[2][0] = -t[2][0];
        t[2][1] = -t[2][1];
        t[2][2] = -t[2][2];
        t[2][3] = -t[2][3];
        t[0][2] = -t[0][2];
        t[1][2] = -t[1][2];
        t[2][2] = -t[2][2];
        t[3][2] = -t[3][2];
    }
    for m in scene.meshes.iter_mut() {
        for v in m.vertices.iter_mut() {
            v[2] *= -1.0;
        }
        if let Some(ns) = m.normals.as_mut() {
            for v in ns.iter_mut() {
                v[2] *= -1.0;
            }
        }
        if let (Some(ts), Some(bs)) = (m.tangents.as_mut(), m.bitangents.as_mut()) {
            for v in ts.iter_mut() {
                v[2] *= -1.0;
            }
            for v in bs.iter_mut() {
                v[2] *= -1.0;
            }
        }
        for b in m.bones.iter_mut() {
            let o = &mut b.offset.m;
            o[0][2] = -o[0][2];
            o[1][2] = -o[1][2];
            o[3][2] = -o[3][2];
            o[2][0] = -o[2][0];
            o[2][1] = -o[2][1];
            o[2][3] = -o[2][3];
        }
        if let (Some(_), Some(bs)) = (&m.tangents, m.bitangents.as_mut()) {
            for v in bs.iter_mut() {
                *v = [-v[0], -v[1], -v[2]];
            }
        }
    }
    for mat in scene.materials.iter_mut() {
        for p in mat.properties.iter_mut() {
            if p.key == "$tex.mapaxis" && p.data.len() >= 12 {
                let z = -f32::from_le_bytes(p.data[8..12].try_into().unwrap());
                p.data[8..12].copy_from_slice(&z.to_le_bytes());
            }
        }
    }
    for a in scene.animations.iter_mut() {
        for ch in a.channels.iter_mut() {
            for k in ch.position_keys.iter_mut() {
                k.value[2] *= -1.0;
            }
            for k in ch.rotation_keys.iter_mut() {
                k.value.x *= -1.0;
                k.value.y *= -1.0;
            }
        }
    }
}

pub fn flip_uvs(scene: &mut AiScene) {
    for m in scene.meshes.iter_mut() {
        for i in 0..MAX_UV {
            let Some(uv) = m.uvs[i].as_mut() else { break };
            for v in uv.iter_mut() {
                v[1] = 1.0 - v[1];
            }
        }
    }
}

pub fn flip_winding_order(scene: &mut AiScene) {
    for m in scene.meshes.iter_mut() {
        for f in m.faces.iter_mut() {
            let n = f.len();
            for b in 0..n / 2 {
                f.swap(b, n - 1 - b);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// RemoveRedundantMaterials

fn material_hash(mat: &AiMaterial) -> u32 {
    let mut hash = 1503u32;
    for p in &mat.properties {
        if p.key.starts_with('?') {
            continue;
        }
        hash = super_fast_hash(p.key.as_bytes(), hash);
        hash = super_fast_hash(&p.data, hash);
        hash = super_fast_hash(&p.semantic.to_le_bytes(), hash);
        hash = super_fast_hash(&p.index.to_le_bytes(), hash);
    }
    hash
}

fn remove_redundant_materials(scene: &mut AiScene) -> Result<(), ImportError> {
    let n = scene.materials.len();
    if n == 0 {
        return Ok(());
    }
    let mut referenced = vec![false; n];
    for m in &scene.meshes {
        if let Some(r) = referenced.get_mut(m.material_index as usize) {
            *r = true;
        }
    }
    let mut mapping = vec![0u32; n];
    let mut hashes = vec![0u32; n];
    let mut keep = vec![false; n];
    let mut new_num = 0u32;
    for i in 0..n {
        if !referenced[i] {
            continue;
        }
        let mut me = material_hash(&scene.materials[i]);
        hashes[i] = me;
        for a in 0..i {
            if referenced[a] && me == hashes[a] {
                me = 0;
                mapping[i] = mapping[a];
                break;
            }
        }
        if me != 0 {
            mapping[i] = new_num;
            keep[i] = true;
            new_num += 1;
        }
    }
    if new_num < 1 {
        return Err(ImportError("No materials remaining".into()));
    }
    if new_num as usize != n {
        let mut out: Vec<Option<AiMaterial>> = vec![None; new_num as usize];
        for p in 0..n {
            if !referenced[p] {
                continue;
            }
            let idx = mapping[p] as usize;
            if out[idx].is_some() {
                let o = out[idx].as_mut().unwrap();
                if o.get_string("?mat.name", 0, 0).is_none() {
                    o.add_string("?mat.name", 0, 0, &format!("JoinedMaterial_#{}", p));
                }
            } else if keep[p] {
                out[idx] = Some(scene.materials[p].clone());
            }
        }
        for m in scene.meshes.iter_mut() {
            m.material_index = mapping[m.material_index as usize];
        }
        scene.materials = out.into_iter().map(|m| m.unwrap_or_default()).collect();
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// FindInstances

fn vformat_unique(m: &AiMesh) -> u32 {
    let mut r = 1u32;
    if m.has_normals() {
        r |= 0x2;
    }
    if m.has_tangents() {
        r |= 0x4;
    }
    let mut p = 0;
    while p < MAX_UV && m.has_uv(p) {
        r |= 0x100 << p;
        if m.num_uv_components[p] == 3 {
            r |= 0x10000 << p;
        }
        p += 1;
    }
    p = 0;
    while p < MAX_COLORS && m.has_colors(p) {
        r |= 0x1000000 << p;
        p += 1;
    }
    r
}

fn mesh_hash(m: &AiMesh) -> u64 {
    let f = vformat_unique(m) as u64;
    let nb = m.bones.len() as u32;
    let nv = m.vertices.len() as u32;
    let nf = m.faces.len() as u32;
    (f << 32)
        | ((nb << 16) ^ nv ^ (nf << 4) ^ (m.material_index << 15) ^ (m.primitive_types << 28))
            as u64
}

fn array_bounds(v: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let mut mn = [1e10f32; 3];
    let mut mx = [-1e10f32; 3];
    for p in v {
        for i in 0..3 {
            mn[i] = mn[i].min(p[i]);
            mx[i] = mx[i].max(p[i]);
        }
    }
    (mn, mx)
}

fn position_epsilon(m: &AiMesh) -> f32 {
    let (mn, mx) = array_bounds(&m.vertices);
    vlen(vsub(mx, mn)) * 1e-4
}

fn compare_arrays(a: &[[f32; 3]], b: &[[f32; 3]], e: f32) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| vsqlen(vsub(*x, *y)) < e)
}

fn color_diff(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let c = [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]];
    c[0] * c[0] + c[1] * c[1] + c[2] * c[2] + c[3] * c[3]
}

fn compare_bones(orig: &AiMesh, inst: &AiMesh) -> bool {
    for (a, o) in orig.bones.iter().zip(inst.bones.iter()) {
        if a.weights.len() != o.weights.len() || a.offset != o.offset {
            return false;
        }
        for (wa, wo) in a.weights.iter().zip(o.weights.iter()) {
            // assimp's comparison is inverted: a matching weight makes the
            // bones count as different.
            if wa.0 != wo.0 || (wa.1 - wo.1) < 10e-3 {
                return false;
            }
        }
    }
    true
}

fn find_instances(scene: &mut AiScene) {
    let n = scene.meshes.len();
    if n == 0 {
        return;
    }
    let mut hashes = vec![0u64; n];
    let mut remapping = vec![0u32; n];
    let mut removed = vec![false; n];
    let mut num_out = 0u32;
    for i in 0..n {
        hashes[i] = mesh_hash(&scene.meshes[i]);
        let mut eps = position_epsilon(&scene.meshes[i]);
        eps *= eps;
        let inst = &scene.meshes[i];
        for a in (0..i).rev() {
            if hashes[i] != hashes[a] || removed[a] {
                continue;
            }
            let orig = &scene.meshes[a];
            if orig.bones.len() != inst.bones.len()
                || orig.faces.len() != inst.faces.len()
                || orig.vertices.len() != inst.vertices.len()
                || orig.material_index != inst.material_index
                || orig.primitive_types != inst.primitive_types
            {
                continue;
            }
            if !orig.vertices.is_empty() && !compare_arrays(&orig.vertices, &inst.vertices, eps) {
                continue;
            }
            if orig.has_normals()
                && !compare_arrays(
                    orig.normals.as_ref().unwrap(),
                    inst.normals.as_ref().unwrap(),
                    eps,
                )
            {
                continue;
            }
            if orig.has_tangents()
                && (!compare_arrays(
                    orig.tangents.as_ref().unwrap(),
                    inst.tangents.as_ref().unwrap(),
                    eps,
                ) || !compare_arrays(
                    orig.bitangents.as_ref().unwrap(),
                    inst.bitangents.as_ref().unwrap(),
                    eps,
                ))
            {
                continue;
            }
            let uv_eps = 10e-4f32;
            let mut bad = false;
            for j in 0..orig.num_uv_channels() {
                if let (Some(a), Some(b)) = (orig.uvs[j].as_ref(), inst.uvs[j].as_ref()) {
                    if !compare_arrays(a, b, uv_eps) {
                        bad = true;
                        break;
                    }
                }
            }
            if bad {
                continue;
            }
            for j in 0..orig.num_color_channels() {
                if let (Some(a), Some(b)) = (orig.colors[j].as_ref(), inst.colors[j].as_ref()) {
                    if !a
                        .iter()
                        .zip(b.iter())
                        .all(|(x, y)| color_diff(x, y) < uv_eps)
                    {
                        bad = true;
                        break;
                    }
                }
            }
            if bad {
                continue;
            }
            if !compare_bones(orig, inst) {
                continue;
            }
            let nv = orig.vertices.len();
            let mut fo = vec![0u32; nv];
            let mut fi = vec![0u32; nv];
            for (tt, (f1, f2)) in orig.faces.iter().zip(inst.faces.iter()).enumerate() {
                for &x in f1 {
                    if let Some(v) = fo.get_mut(x as usize) {
                        *v = tt as u32;
                    }
                }
                for &x in f2 {
                    if let Some(v) = fi.get_mut(x as usize) {
                        *v = tt as u32;
                    }
                }
            }
            if fo != fi {
                continue;
            }
            remapping[i] = remapping[a];
            removed[i] = true;
            break;
        }
        if !removed[i] {
            remapping[i] = num_out;
            num_out += 1;
        }
    }
    if num_out as usize != n {
        let meshes = std::mem::take(&mut scene.meshes);
        scene.meshes = meshes
            .into_iter()
            .zip(removed.iter())
            .filter(|(_, r)| !**r)
            .map(|(m, _)| m)
            .collect();
        for node in scene.nodes.iter_mut() {
            for m in node.meshes.iter_mut() {
                *m = remapping[*m as usize];
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Triangulate

fn triangulate(scene: &mut AiScene) {
    for m in scene.meshes.iter_mut() {
        triangulate_mesh(m);
    }
}

struct NgonEncoder {
    last: u32,
}

impl NgonEncoder {
    fn same(&self, t: &[u32]) -> bool {
        t[0] == self.last
    }
    fn triangle(&mut self, t: &mut [u32]) {
        if self.same(t) {
            t.swap(0, 2);
            t.swap(1, 2);
        }
        self.last = t[0];
    }
    fn quad(&mut self, t1: &mut [u32], t2: &mut [u32]) {
        if self.same(t1) {
            t1.swap(0, 2);
            t1.swap(1, 2);
            t2.swap(1, 2);
            t2.swap(0, 2);
        }
        self.last = t1[0];
    }
}

fn area2d(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> f64 {
    // GetArea2D: 0.5 * (v1.x*(v3.y-v2.y) + v2.x*(v1.y-v3.y) + v3.x*(v2.y-v1.y))
    let (v1, v2, v3) = (p0, p1, p2);
    0.5 * (v1[0] as f64 * (v3[1] as f64 - v2[1] as f64)
        + v2[0] as f64 * (v1[1] as f64 - v3[1] as f64)
        + v3[0] as f64 * (v2[1] as f64 - v1[1] as f64))
}

fn on_left_side(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2]) -> i32 {
    let area = area2d(p0, p2, p1);
    if area.abs() < 1e-6 {
        0
    } else if area > 0.0 {
        1
    } else {
        -1
    }
}

fn point_in_triangle(p0: [f32; 2], p1: [f32; 2], p2: [f32; 2], pp: [f32; 2]) -> bool {
    on_left_side(p0, p1, pp) >= 0 && on_left_side(p1, p2, pp) >= 0 && on_left_side(p2, p0, pp) >= 0
}

fn normalize2(v: [f32; 2]) -> [f32; 2] {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if l == 0.0 {
        return v;
    }
    // aiVector2D::Normalize: *this /= Length() (component division)
    [v[0] / l, v[1] / l]
}

fn triangulate_mesh(m: &mut AiMesh) -> bool {
    if m.primitive_types == 0 {
        if m.faces.iter().all(|f| f.len() == 3) {
            return false;
        }
    } else if m.primitive_types & PT_POLYGON == 0 {
        return false;
    }
    m.primitive_types |= PT_TRIANGLE;
    m.primitive_types &= !PT_POLYGON;
    m.primitive_types |= PT_NGON_ENCODING;
    let mut enc = NgonEncoder { last: u32::MAX };
    let verts = m.vertices.clone();
    let mut out: Vec<Vec<u32>> = Vec::new();
    for face in std::mem::take(&mut m.faces) {
        let num = face.len();
        if num <= 3 {
            let mut f = face;
            if f.len() == 3 {
                enc.triangle(&mut f);
            }
            out.push(f);
            continue;
        }
        if num == 4 {
            let mut start = 0usize;
            for i in 0..4 {
                let v0 = verts[face[(i + 3) % 4] as usize];
                let v1 = verts[face[(i + 2) % 4] as usize];
                let v2 = verts[face[(i + 1) % 4] as usize];
                let v = verts[face[i] as usize];
                let left = vnormalize(vsub(v0, v));
                let diag = vnormalize(vsub(v1, v));
                let right = vnormalize(vsub(v2, v));
                let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
                let angle = dot(left, diag).acos() + dot(right, diag).acos();
                if angle > std::f32::consts::PI {
                    start = i;
                    break;
                }
            }
            let t = [face[0], face[1], face[2], face[3]];
            let mut a = vec![t[start], t[(start + 1) % 4], t[(start + 2) % 4]];
            let mut b = vec![t[start], t[(start + 2) % 4], t[(start + 3) % 4]];
            enc.quad(&mut a, &mut b);
            out.push(a);
            out.push(b);
            continue;
        }
        // General polygon: ear clipping on the dominant-axis projection.
        let max = num;
        let pts: Vec<[f32; 3]> = face.iter().map(|&i| verts[i as usize]).collect();
        let n = newell_normal(&pts);
        let (ax, ay, az) = (n[0].abs(), n[1].abs(), n[2].abs());
        let (mut ac, mut bc) = (0usize, 1usize);
        let mut inv = n[2];
        if ax > ay {
            if ax > az {
                ac = 1;
                bc = 2;
                inv = n[0];
            }
        } else if ay > az {
            ac = 2;
            bc = 0;
            inv = n[1];
        }
        if inv < 0.0 {
            std::mem::swap(&mut ac, &mut bc);
        }
        let tv: Vec<[f32; 2]> = pts.iter().map(|p| [p[ac], p[bc]]).collect();
        let mut done = vec![false; max];
        let mut tris: Vec<[usize; 3]> = Vec::new();
        let (mut prev, mut next) = (max - 1, 0usize);
        let mut num_left = num;
        while num_left > 3 {
            let mut num_found = 0;
            let mut ear = next;
            loop {
                next = ear + 1;
                loop {
                    if next >= max {
                        next = 0;
                    }
                    if !done[next] {
                        break;
                    }
                    next += 1;
                }
                if next < ear {
                    num_found += 1;
                    if num_found == 2 {
                        break;
                    }
                }
                let p1 = tv[ear];
                let p0 = tv[prev];
                let p2 = tv[next];
                let mut ok = true;
                if on_left_side(p0, p2, p1) == 1 {
                    ok = false;
                }
                if ok {
                    let l = normalize2([p0[0] - p1[0], p0[1] - p1[1]]);
                    let r = normalize2([p2[0] - p1[0], p2[1] - p1[1]]);
                    let mul = l[0] * r[0] + l[1] * r[1];
                    if (mul - 1.0).abs() < 1e-6 || (mul + 1.0).abs() < 1e-6 {
                        ok = false;
                    }
                }
                if ok {
                    for &vt in tv.iter() {
                        if vt != p1 && vt != p2 && vt != p0 && point_in_triangle(p0, p1, p2, vt) {
                            ok = false;
                            break;
                        }
                    }
                }
                if ok {
                    break;
                }
                prev = ear;
                ear = next;
            }
            if num_found == 2 {
                num_left = 0;
                break;
            }
            tris.push([prev, ear, next]);
            done[ear] = true;
            num_left -= 1;
        }
        if num_left > 0 {
            let mut t = 0;
            while done[t] {
                t += 1;
            }
            let a = t;
            t += 1;
            while done[t] {
                t += 1;
            }
            let b = t;
            t += 1;
            while done[t] {
                t += 1;
            }
            tris.push([a, b, t]);
        }
        for t in tris {
            let mut f = vec![face[t[0]], face[t[1]], face[t[2]]];
            enc.triangle(&mut f);
            out.push(f);
        }
    }
    m.faces = out;
    true
}

fn newell_normal(p: &[[f32; 3]]) -> [f32; 3] {
    // NewellNormal<3,3,3>: the arrays are padded with the first two points.
    let num = p.len();
    let at = |i: usize| p[i % num];
    let (mut sx, mut sy, mut sz) = (0.0f32, 0.0f32, 0.0f32);
    for t in 0..num {
        let prev = at(t);
        let cur = at(t + 1);
        let nxt = at(t + 2);
        sx += cur[1] * (nxt[2] - prev[2]);
        sy += cur[2] * (nxt[0] - prev[0]);
        sz += cur[0] * (nxt[1] - prev[1]);
    }
    [sx, sy, sz]
}

// ---------------------------------------------------------------------------
// OptimizeMeshes

fn optimize_meshes(scene: &mut AiScene, pts: bool) -> Result<(), ImportError> {
    let num_old = scene.meshes.len();
    if num_old <= 1 {
        return Ok(());
    }
    let mut instance_cnt = vec![0u32; num_old];
    let mut output_id = vec![u32::MAX; num_old];
    for nid in scene.preorder() {
        for &m in &scene.nodes[nid].meshes {
            instance_cnt[m as usize] += 1;
        }
    }
    let vformat: Vec<u32> = scene.meshes.iter().map(vformat_unique).collect();
    let mut src: Vec<Option<AiMesh>> = std::mem::take(&mut scene.meshes)
        .into_iter()
        .map(Some)
        .collect();
    // Instanced meshes go first, shared by every node referencing them.
    let mut output: Vec<AiMesh> = Vec::new();
    let mut n = 0u32;
    for i in 0..num_old {
        if instance_cnt[i] > 1 && output_id[i] == u32::MAX {
            output_id[i] = n;
            n += 1;
            output.push(src[i].clone().unwrap());
        }
    }
    let can_join = |src: &Vec<Option<AiMesh>>, a: usize, b: usize| -> bool {
        if vformat[a] != vformat[b] {
            return false;
        }
        let (ma, mb) = (src[a].as_ref().unwrap(), src[b].as_ref().unwrap());
        if ma.material_index != mb.material_index || ma.bones.is_empty() != mb.bones.is_empty() {
            return false;
        }
        if pts && ma.primitive_types != mb.primitive_types {
            return false;
        }
        ma.bones.is_empty()
    };
    for nid in scene.preorder() {
        let mut i = 0;
        while i < scene.nodes[nid].meshes.len() {
            let im = scene.nodes[nid].meshes[i] as usize;
            if instance_cnt[im] > 1 {
                scene.nodes[nid].meshes[i] = output_id[im];
            } else {
                let mut merge: Vec<usize> = Vec::new();
                let mut a = i + 1;
                while a < scene.nodes[nid].meshes.len() {
                    let am = scene.nodes[nid].meshes[a] as usize;
                    if instance_cnt[am] == 1 && can_join(&src, im, am) {
                        merge.push(am);
                        let last = *scene.nodes[nid].meshes.last().unwrap();
                        scene.nodes[nid].meshes[a] = last;
                        scene.nodes[nid].meshes.pop();
                        continue;
                    }
                    a += 1;
                }
                if !merge.is_empty() {
                    merge.push(im);
                    let list: Vec<AiMesh> = merge.iter().map(|&k| src[k].take().unwrap()).collect();
                    output.push(merge_meshes(list));
                } else {
                    output.push(src[im].take().unwrap_or_default());
                }
                scene.nodes[nid].meshes[i] = (output.len() - 1) as u32;
            }
            i += 1;
        }
    }
    if output.is_empty() {
        return Err(ImportError("OptimizeMeshes: No meshes remaining".into()));
    }
    scene.meshes = output;
    Ok(())
}

/// `SceneCombiner::MergeMeshes` for meshes without bones.
fn merge_meshes(list: Vec<AiMesh>) -> AiMesh {
    let first = &list[0];
    let mut out = AiMesh {
        material_index: first.material_index,
        ..Default::default()
    };
    out.name = list
        .iter()
        .map(|m| m.name.as_str())
        .collect::<Vec<_>>()
        .join(".");
    for m in &list {
        out.primitive_types |= m.primitive_types;
    }
    let total: usize = list.iter().map(|m| m.vertices.len()).sum();
    if total > 0 {
        if !first.vertices.is_empty() {
            for m in &list {
                out.vertices.extend_from_slice(&m.vertices);
            }
        }
        if first.has_normals() {
            let mut v = Vec::with_capacity(total);
            for m in &list {
                match &m.normals {
                    Some(n) => v.extend_from_slice(n),
                    None => v.extend(std::iter::repeat_n([0.0; 3], m.vertices.len())),
                }
            }
            out.normals = Some(v);
        }
        if first.has_tangents() {
            let mut t = Vec::new();
            let mut b = Vec::new();
            for m in &list {
                match (&m.tangents, &m.bitangents) {
                    (Some(a), Some(c)) => {
                        t.extend_from_slice(a);
                        b.extend_from_slice(c);
                    }
                    _ => {
                        t.extend(std::iter::repeat_n([0.0; 3], m.vertices.len()));
                        b.extend(std::iter::repeat_n([0.0; 3], m.vertices.len()));
                    }
                }
            }
            out.tangents = Some(t);
            out.bitangents = Some(b);
        }
        let mut n = 0;
        while n < MAX_UV && first.has_uv(n) {
            out.num_uv_components[n] = first.num_uv_components[n];
            let mut v = Vec::with_capacity(total);
            for m in &list {
                match &m.uvs[n] {
                    Some(u) => v.extend_from_slice(u),
                    None => v.extend(std::iter::repeat_n([0.0; 3], m.vertices.len())),
                }
            }
            out.uvs[n] = Some(v);
            n += 1;
        }
        n = 0;
        while n < MAX_COLORS && first.has_colors(n) {
            let mut v = Vec::with_capacity(total);
            for m in &list {
                match &m.colors[n] {
                    Some(u) => v.extend_from_slice(u),
                    None => v.extend(std::iter::repeat_n([0.0; 4], m.vertices.len())),
                }
            }
            out.colors[n] = Some(v);
            n += 1;
        }
    }
    let mut ofs = 0u32;
    for m in &list {
        for f in &m.faces {
            out.faces.push(f.iter().map(|&i| i + ofs).collect());
        }
        ofs += m.vertices.len() as u32;
    }
    out
}

// ---------------------------------------------------------------------------
// JoinIdenticalVertices

#[derive(Clone)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    tangent: [f32; 3],
    bitangent: [f32; 3],
    texcoords: [[f32; 3]; MAX_UV],
    colors: [[f32; 4]; MAX_COLORS],
}

impl Vertex {
    fn new(m: &AiMesh, i: usize) -> Vertex {
        let mut v = Vertex {
            position: m.vertices[i],
            normal: [0.0; 3],
            tangent: [0.0; 3],
            bitangent: [0.0; 3],
            texcoords: [[0.0; 3]; MAX_UV],
            colors: [[0.0; 4]; MAX_COLORS],
        };
        if let Some(n) = &m.normals {
            v.normal = n[i];
        }
        if let (Some(t), Some(b)) = (&m.tangents, &m.bitangents) {
            v.tangent = t[i];
            v.bitangent = b[i];
        }
        let mut a = 0;
        while a < MAX_UV && m.has_uv(a) {
            v.texcoords[a] = m.uvs[a].as_ref().unwrap()[i];
            a += 1;
        }
        a = 0;
        while a < MAX_COLORS && m.has_colors(a) {
            v.colors[a] = m.colors[a].as_ref().unwrap()[i];
            a += 1;
        }
        v
    }

    fn equal(&self, o: &Vertex, nuv: usize, ncol: usize) -> bool {
        let sq = 1e-5f32 * 1e-5f32;
        if vsqlen(vsub(self.position, o.position)) > sq
            || vsqlen(vsub(self.normal, o.normal)) > sq
            || vsqlen(vsub(self.tangent, o.tangent)) > sq
            || vsqlen(vsub(self.bitangent, o.bitangent)) > sq
        {
            return false;
        }
        for i in 0..nuv {
            if vsqlen(vsub(self.texcoords[i], o.texcoords[i])) > sq {
                return false;
            }
        }
        for i in 0..ncol {
            if color_diff(&self.colors[i], &o.colors[i]) > sq {
                return false;
            }
        }
        true
    }
}

/// `std::hash<float>` treats +0 and -0 alike; positions with different
/// bits land in different buckets, so only bit-equal positions can join.
/// libc++ puts a new node at the front of its bucket, so lookups see the
/// newest matching vertex first.
fn pos_key(p: [f32; 3]) -> [u32; 3] {
    let k = |f: f32| if f == 0.0 { 0 } else { f.to_bits() };
    [k(p[0]), k(p[1]), k(p[2])]
}

fn join_vertices(scene: &mut AiScene) {
    for m in scene.meshes.iter_mut() {
        join_mesh(m);
    }
}

const JOINED: u32 = 0x8000_0000;

fn join_mesh(m: &mut AiMesh) {
    if m.vertices.is_empty() || m.faces.is_empty() {
        return;
    }
    let nv = m.vertices.len();
    let mut used = vec![false; nv];
    for f in &m.faces {
        for &i in f {
            used[i as usize] = true;
        }
    }
    let nuv = m.num_uv_channels();
    let ncol = m.num_color_channels();
    let mut unique: Vec<Vertex> = Vec::with_capacity(nv);
    let mut replace = vec![0xffff_ffffu32; nv];
    let mut buckets: HashMap<[u32; 3], Vec<u32>> = HashMap::new();
    for a in 0..nv {
        if !used[a] {
            continue;
        }
        let v = Vertex::new(m, a);
        let key = pos_key(v.position);
        let found = buckets.get(&key).and_then(|list| {
            list.iter()
                .rev()
                .copied()
                .find(|&u| unique[u as usize].equal(&v, nuv, ncol))
        });
        match found {
            Some(u) => replace[a] = u | JOINED,
            None => {
                let ni = unique.len() as u32;
                buckets.entry(key).or_default().push(ni);
                replace[a] = ni;
                unique.push(v);
            }
        }
    }
    let n = unique.len();
    m.vertices = unique.iter().map(|v| v.position).collect();
    if m.normals.is_some() {
        m.normals = Some(unique.iter().map(|v| v.normal).collect());
    }
    if m.tangents.is_some() {
        m.tangents = Some(unique.iter().map(|v| v.tangent).collect());
    }
    if m.bitangents.is_some() {
        m.bitangents = Some(unique.iter().map(|v| v.bitangent).collect());
    }
    let mut a = 0;
    while a < MAX_COLORS && m.colors[a].is_some() && n > 0 {
        m.colors[a] = Some(unique.iter().map(|v| v.colors[a]).collect());
        a += 1;
    }
    a = 0;
    while a < MAX_UV && m.uvs[a].is_some() && n > 0 {
        m.uvs[a] = Some(unique.iter().map(|v| v.texcoords[a]).collect());
        a += 1;
    }
    for f in m.faces.iter_mut() {
        for i in f.iter_mut() {
            *i = replace[*i as usize] & !JOINED;
        }
    }
    for b in m.bones.iter_mut() {
        let nw: Vec<(u32, f32)> = b
            .weights
            .iter()
            .filter(|w| replace[w.0 as usize] & JOINED == 0)
            .map(|w| (replace[w.0 as usize], w.1))
            .collect();
        if !nw.is_empty() {
            b.weights = nw;
        }
    }
}

// ---------------------------------------------------------------------------
// LimitBoneWeights

#[derive(Clone, Copy)]
struct Weight {
    bone: u32,
    weight: f32,
}

fn wless(a: &Weight, b: &Weight) -> bool {
    a.weight > b.weight
}

/// libc++ `std::sort` for the small arrays this sees: the sorting
/// networks for 2-5 elements and insertion sort above that (exact tie
/// order matters for which weight is dropped).
fn libcxx_sort(v: &mut [Weight]) {
    let n = v.len();
    match n {
        0 | 1 => {}
        2 => {
            if wless(&v[1], &v[0]) {
                v.swap(0, 1);
            }
        }
        3 => sort3(v, 0, 1, 2),
        4 => sort4(v, 0, 1, 2, 3),
        5 => sort5(v, 0, 1, 2, 3, 4),
        _ => {
            sort3(v, 0, 1, 2);
            for i in 3..n {
                let mut j = i;
                if wless(&v[j], &v[j - 1]) {
                    let t = v[j];
                    loop {
                        v[j] = v[j - 1];
                        j -= 1;
                        if j == 0 || !wless(&t, &v[j - 1]) {
                            break;
                        }
                    }
                    v[j] = t;
                }
            }
        }
    }
}

fn sort3(v: &mut [Weight], x: usize, y: usize, z: usize) {
    if !wless(&v[y], &v[x]) {
        if !wless(&v[z], &v[y]) {
            return;
        }
        v.swap(y, z);
        if wless(&v[y], &v[x]) {
            v.swap(x, y);
        }
        return;
    }
    if wless(&v[z], &v[y]) {
        v.swap(x, z);
        return;
    }
    v.swap(x, y);
    if wless(&v[z], &v[y]) {
        v.swap(y, z);
    }
}

fn sort4(v: &mut [Weight], a: usize, b: usize, c: usize, d: usize) {
    sort3(v, a, b, c);
    if wless(&v[d], &v[c]) {
        v.swap(c, d);
        if wless(&v[c], &v[b]) {
            v.swap(b, c);
            if wless(&v[b], &v[a]) {
                v.swap(a, b);
            }
        }
    }
}

fn sort5(v: &mut [Weight], a: usize, b: usize, c: usize, d: usize, e: usize) {
    sort4(v, a, b, c, d);
    if wless(&v[e], &v[d]) {
        v.swap(d, e);
        if wless(&v[d], &v[c]) {
            v.swap(c, d);
            if wless(&v[c], &v[b]) {
                v.swap(b, c);
                if wless(&v[b], &v[a]) {
                    v.swap(a, b);
                }
            }
        }
    }
}

fn limit_bone_weights(scene: &mut AiScene, max_weights: usize) {
    for m in scene.meshes.iter_mut() {
        if m.bones.is_empty() {
            continue;
        }
        let nv = m.vertices.len();
        let mut per_vertex: Vec<Vec<Weight>> = vec![Vec::new(); nv];
        let mut max_seen = 0;
        for (b, bone) in m.bones.iter().enumerate() {
            for &(v, w) in &bone.weights {
                if (v as usize) >= nv {
                    continue;
                }
                per_vertex[v as usize].push(Weight {
                    bone: b as u32,
                    weight: w,
                });
                max_seen = max_seen.max(per_vertex[v as usize].len());
            }
        }
        if max_seen <= max_weights {
            continue;
        }
        for vw in per_vertex.iter_mut() {
            if vw.len() <= max_weights {
                continue;
            }
            libcxx_sort(vw);
            vw.truncate(max_weights);
            let sum: f32 = vw.iter().fold(0.0f32, |s, w| s + w.weight);
            if sum != 0.0 {
                let inv = 1.0 / sum;
                for w in vw.iter_mut() {
                    w.weight *= inv;
                }
            }
        }
        for b in m.bones.iter_mut() {
            b.weights.clear();
        }
        for (a, vw) in per_vertex.iter().enumerate() {
            for w in vw {
                m.bones[w.bone as usize].weights.push((a as u32, w.weight));
            }
        }
        // AI_CONFIG_IMPORT_REMOVE_EMPTY_BONES defaults to on.
        m.bones.retain(|b| !b.weights.is_empty());
    }
}
