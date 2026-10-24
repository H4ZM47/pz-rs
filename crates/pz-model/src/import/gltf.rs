//! glTF 2 files (`.glb`, or `.gltf` with the buffer embedded in a GLB
//! chunk) through assimp 5.3's `glTF2Importer`: primitives become meshes
//! (vertices remapped in index order, V flipped), skins become bones with
//! the inverse bind matrices as offsets, and animation channels are keyed
//! in milliseconds (1000 ticks per second).

use serde_json::Value;

use super::aimath::{AiMat4, AiQuat};
use super::scene::*;
use super::ImportError;

fn err<T>(m: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(format!("GLTF: {}", m.into())))
}

struct Accessor<'b> {
    data: &'b [u8],
    count: usize,
    elem_size: usize,
    stride: usize,
    comp_type: u64,
    num_comp: usize,
}

impl<'b> Accessor<'b> {
    fn elem(&self, i: usize) -> Result<&'b [u8], ImportError> {
        let s = i * self.stride;
        self.data.get(s..s + self.elem_size).ok_or_else(|| ImportError("GLTF: accessor index out of range".into()))
    }

    /// `Accessor::ExtractData`: raw element bytes (optionally remapped),
    /// zero-padded to `target` bytes.
    fn extract(&self, target: usize, remap: Option<&[u32]>) -> Result<Vec<Vec<u8>>, ImportError> {
        if self.elem_size > target {
            return err("elemSize > targetElemSize");
        }
        let n = remap.map_or(self.count, |r| r.len());
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let src = remap.map_or(i, |r| r[i] as usize);
            let mut e = self.elem(src)?.to_vec();
            e.resize(target, 0);
            out.push(e);
        }
        Ok(out)
    }

    fn floats(&self, n: usize, remap: Option<&[u32]>) -> Result<Vec<Vec<f32>>, ImportError> {
        Ok(self
            .extract(n * 4, remap)?
            .into_iter()
            .map(|e| e.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
            .collect())
    }

    fn uint(&self, i: usize) -> Result<u32, ImportError> {
        let e = self.elem(i)?;
        Ok(match self.comp_type {
            5121 | 5120 => e[0] as u32,
            5123 | 5122 => u16::from_le_bytes([e[0], e[1]]) as u32,
            _ => u32::from_le_bytes(e[0..4].try_into().unwrap()),
        })
    }
}

struct Glb {
    json: Value,
    bin: Vec<u8>,
}

impl Glb {
    fn parse(data: &[u8]) -> Result<Glb, ImportError> {
        if data.len() < 20 || &data[0..4] != b"glTF" {
            let json: Value = serde_json::from_slice(data).map_err(|e| ImportError(format!("GLTF: {e}")))?;
            return Ok(Glb { json, bin: Vec::new() });
        }
        let mut off = 12;
        let mut json = None;
        let mut bin = Vec::new();
        while off + 8 <= data.len() {
            let len = u32::from_le_bytes(data[off..off + 4].try_into().unwrap()) as usize;
            let ty = u32::from_le_bytes(data[off + 4..off + 8].try_into().unwrap());
            let body = &data[off + 8..(off + 8 + len).min(data.len())];
            if ty == 0x4E4F_534A {
                json = Some(serde_json::from_slice(body).map_err(|e| ImportError(format!("GLTF: {e}")))?);
            } else if ty == 0x004E_4942 {
                bin = body.to_vec();
            }
            off += 8 + len;
        }
        let Some(json) = json else { return err("no JSON chunk") };
        Ok(Glb { json, bin })
    }

    fn arr(&self, k: &str) -> &[Value] {
        self.json.get(k).and_then(|v| v.as_array()).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn accessor(&self, idx: u64) -> Result<Accessor<'_>, ImportError> {
        let a = self.arr("accessors").get(idx as usize).ok_or_else(|| ImportError("GLTF: bad accessor".into()))?;
        let comp_type = a["componentType"].as_u64().unwrap_or(5126);
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let num_comp = match a["type"].as_str().unwrap_or("SCALAR") {
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" | "MAT2" => 4,
            "MAT3" => 9,
            "MAT4" => 16,
            _ => 1,
        };
        let csize = match comp_type {
            5120 | 5121 => 1,
            5122 | 5123 => 2,
            _ => 4,
        };
        let elem_size = num_comp * csize;
        let Some(bv) = a.get("bufferView").and_then(|v| v.as_u64()) else {
            return err("accessor without data");
        };
        let view = self.arr("bufferViews").get(bv as usize).ok_or_else(|| ImportError("GLTF: bad bufferView".into()))?;
        if view["buffer"].as_u64().unwrap_or(0) != 0 || self.bin.is_empty() {
            return err("only the GLB binary chunk is supported");
        }
        let vo = view["byteOffset"].as_u64().unwrap_or(0) as usize;
        let vl = view["byteLength"].as_u64().unwrap_or(0) as usize;
        let ao = a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let vs = view["byteStride"].as_u64().unwrap_or(0) as usize;
        let stride = if vs != 0 { vs } else { elem_size };
        let start = vo + ao;
        let end = (vo + vl).min(self.bin.len());
        if start > end {
            return err("accessor out of range");
        }
        Ok(Accessor { data: &self.bin[start..end], count, elem_size, stride, comp_type, num_comp })
    }
}

fn node_name(n: &Value, i: usize) -> String {
    match n["name"].as_str() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => format!("nodes[{i}]"),
    }
}

/// `CopyValue(mat4, aiMatrix4x4)`: glTF matrices are column-major.
fn mat_from_cols(v: &[f32]) -> AiMat4 {
    let mut m = AiMat4::IDENTITY;
    for c in 0..4 {
        for r in 0..4 {
            m.m[r][c] = v[c * 4 + r];
        }
    }
    m
}

fn fvec(v: &Value) -> Option<Vec<f32>> {
    v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect())
}

/// `GetNodeTransform`.
fn node_transform(n: &Value) -> AiMat4 {
    if let Some(m) = fvec(&n["matrix"]) {
        if m.len() == 16 {
            return mat_from_cols(&m);
        }
    }
    let mut out = AiMat4::IDENTITY;
    if let Some(t) = fvec(&n["translation"]) {
        out = out.mul(&AiMat4::translation([t[0], t[1], t[2]]));
    }
    if let Some(r) = fvec(&n["rotation"]) {
        let q = AiQuat { x: r[0], y: r[1], z: r[2], w: r[3] };
        out = out.mul(&AiMat4::from_3x3(&q.get_matrix()));
    }
    if let Some(s) = fvec(&n["scale"]) {
        out = out.mul(&AiMat4::scaling([s[0], s[1], s[2]]));
    }
    out
}

/// Iteration order of a libc++ `std::unordered_map<unsigned, T>` filled
/// by `operator[]` with `keys` (identity hash; bucket counts 2, 5, 11,
/// 23, ...): `GatherSamplers` returns one, and the channel order follows it.
pub(crate) fn libcxx_unordered_order(keys: &[u32]) -> Vec<u32> {
    #[derive(Clone, Copy, PartialEq)]
    enum Pred {
        Head,
        Node(usize),
    }
    fn next_prime(n: usize) -> usize {
        let mut p = n.max(2);
        loop {
            if (2..).take_while(|d| d * d <= p).all(|d| p % d != 0) {
                return p;
            }
            p += 1;
        }
    }
    let constrain = |h: usize, bc: usize| if bc.is_power_of_two() { h & (bc - 1) } else { h % bc };
    let mut key: Vec<usize> = Vec::new();
    let mut next: Vec<Option<usize>> = Vec::new();
    let mut head: Option<usize> = None;
    let mut buckets: Vec<Option<Pred>> = Vec::new();
    let get_next = |p: Pred, head: &Option<usize>, next: &Vec<Option<usize>>| match p {
        Pred::Head => *head,
        Pred::Node(i) => next[i],
    };
    for &k in keys {
        let k = k as usize;
        if key.contains(&k) {
            continue;
        }
        let bc = buckets.len();
        let size = key.len();
        if bc == 0 || (size + 1) as f32 > bc as f32 {
            let pow2 = bc > 2 && bc & (bc - 1) == 0;
            let mut n = (2 * bc + usize::from(!pow2)).max(size + 1);
            if n == 1 {
                n = 2;
            } else if n & (n - 1) != 0 {
                n = next_prime(n);
            }
            if n > bc {
                // __do_rehash
                buckets = vec![None; n];
                let mut pp = Pred::Head;
                if let Some(cp0) = head {
                    let mut phash = constrain(key[cp0], n);
                    buckets[phash] = Some(Pred::Head);
                    pp = Pred::Node(cp0);
                    let mut cp = next[cp0];
                    while let Some(c) = cp {
                        let ch = constrain(key[c], n);
                        if ch == phash {
                            pp = Pred::Node(c);
                        } else if buckets[ch].is_none() {
                            buckets[ch] = Some(pp);
                            pp = Pred::Node(c);
                            phash = ch;
                        } else {
                            let Pred::Node(ppi) = pp else { unreachable!() };
                            next[ppi] = next[c];
                            let bp = buckets[ch].unwrap();
                            next[c] = get_next(bp, &head, &next);
                            match bp {
                                Pred::Head => head = Some(c),
                                Pred::Node(b) => next[b] = Some(c),
                            }
                        }
                        cp = match pp {
                            Pred::Node(i) => next[i],
                            Pred::Head => head,
                        };
                    }
                }
                let _ = pp;
            }
        }
        let bc = buckets.len();
        let id = key.len();
        key.push(k);
        next.push(None);
        let ch = constrain(k, bc);
        match buckets[ch] {
            None => {
                next[id] = head;
                head = Some(id);
                buckets[ch] = Some(Pred::Head);
                if let Some(nx) = next[id] {
                    buckets[constrain(key[nx], bc)] = Some(Pred::Node(id));
                }
            }
            Some(p) => {
                next[id] = get_next(p, &head, &next);
                match p {
                    Pred::Head => head = Some(id),
                    Pred::Node(b) => next[b] = Some(id),
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut c = head;
    while let Some(i) = c {
        out.push(key[i] as u32);
        c = next[i];
    }
    out
}

struct Ctx<'g> {
    g: &'g Glb,
    mesh_offsets: Vec<usize>,
    remaps: Vec<Option<Vec<u32>>>,
}

fn import_node(c: &Ctx, out: &mut AiScene, idx: usize, parent: Option<usize>, depth: usize) -> Result<usize, ImportError> {
    if depth > 1000 {
        return err("recursive node reference");
    }
    let g = c.g;
    let nodes = g.arr("nodes");
    let n = nodes.get(idx).ok_or_else(|| ImportError("GLTF: bad node".into()))?;
    let id = out.add_node(&node_name(n, idx), AiMat4::IDENTITY, parent);
    if let Some(ch) = n["children"].as_array() {
        for ci in ch.iter().filter_map(|v| v.as_u64()) {
            import_node(c, out, ci as usize, Some(id), depth + 1)?;
        }
    }
    out.nodes[id].transform = node_transform(n);
    let Some(mi) = n["mesh"].as_u64() else { return Ok(id) };
    let mi = mi as usize;
    let (s, e) = (c.mesh_offsets[mi], c.mesh_offsets[mi + 1]);
    if let Some(si) = n["skin"].as_u64() {
        let skin = g.arr("skins").get(si as usize).ok_or_else(|| ImportError("GLTF: bad skin".into()))?;
        let joints: Vec<usize> =
            skin["joints"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect()).unwrap_or_default();
        let Some(ibm_acc) = skin["inverseBindMatrices"].as_u64() else { return err("skin without inverseBindMatrices") };
        let ibm = g.accessor(ibm_acc)?.floats(16, None)?;
        let prims = g.arr("meshes")[mi]["primitives"].as_array().cloned().unwrap_or_default();
        for (pn, ai) in (s..e).enumerate() {
            let nb = joints.len();
            let mut weighting: Vec<Vec<(u32, f32)>> = vec![Vec::new(); nb];
            let attr = &prims[pn]["attributes"];
            let r = c.remaps[ai].as_deref();
            let mut wsets = Vec::new();
            let mut jsets = Vec::new();
            for k in 0.. {
                let (Some(w), Some(j)) = (attr[format!("WEIGHTS_{k}")].as_u64(), attr[format!("JOINTS_{k}")].as_u64()) else {
                    break;
                };
                wsets.push(g.accessor(w)?);
                jsets.push(g.accessor(j)?);
            }
            // BuildVertexWeightMapping
            if !wsets.is_empty() && wsets[0].count == jsets[0].count {
                let ws: Vec<Vec<Vec<f32>>> = wsets.iter().map(|a| a.floats(4, r)).collect::<Result<_, _>>()?;
                let small = jsets[0].elem_size == 4;
                let js: Vec<Vec<Vec<u8>>> =
                    jsets.iter().map(|a| a.extract(if small { 4 } else { 8 }, r)).collect::<Result<_, _>>()?;
                let num_vertices = ws.last().map_or(0, |w| w.len());
                for w in 0..ws.len() {
                    for i in 0..num_vertices {
                        for jj in 0..4 {
                            let bone = if small {
                                js[w][i][jj] as usize
                            } else {
                                u16::from_le_bytes([js[w][i][2 * jj], js[w][i][2 * jj + 1]]) as usize
                            };
                            let weight = ws[w][i][jj];
                            if weight > 0.0 && bone < nb {
                                weighting[bone].push((i as u32, weight));
                            }
                        }
                    }
                }
            }
            let mut bones = Vec::new();
            for (i, &jn) in joints.iter().enumerate() {
                let name = match nodes.get(jn).and_then(|j| j["name"].as_str()) {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => format!("bone_{i}"),
                };
                let mut weights = weighting[i].clone();
                if weights.is_empty() {
                    // assimp gives a joint without weights one zero weight
                    weights.push((0, 0.0));
                }
                let offset = ibm.get(i).map(|m| mat_from_cols(m)).unwrap_or_default();
                bones.push(AiBone { name, weights, offset });
            }
            out.meshes[ai].bones = bones;
        }
    }
    out.nodes[id].meshes = (s..e).map(|v| v as u32).collect();
    Ok(id)
}

pub fn import(data: &[u8]) -> Result<AiScene, ImportError> {
    let g = Glb::parse(data)?;
    let mut out = AiScene::default();
    // ImportMaterials: the file's materials, then a default one. Only
    // equality matters downstream (RemoveRedundantMaterials), so a
    // material is keyed by its JSON without the name.
    for m in g.arr("materials") {
        let mut am = AiMaterial::default();
        if let Some(n) = m["name"].as_str() {
            am.add_string("?mat.name", 0, 0, n);
        }
        let mut body = m.clone();
        if let Some(o) = body.as_object_mut() {
            o.remove("name");
        }
        am.add_string("$gltf.material", 0, 0, &body.to_string());
        out.materials.push(am);
    }
    let mut def = AiMaterial::default();
    def.add_string("$gltf.material", 0, 0, "{}");
    out.materials.push(def);
    let default_mat = (out.materials.len() - 1) as u32;

    // ImportMeshes
    let mut mesh_offsets = Vec::new();
    let mut remaps: Vec<Option<Vec<u32>>> = Vec::new();
    for (mi, mesh) in g.arr("meshes").iter().enumerate() {
        mesh_offsets.push(out.meshes.len());
        let prims = mesh["primitives"].as_array().cloned().unwrap_or_default();
        for (p, prim) in prims.iter().enumerate() {
            let attr = &prim["attributes"];
            let pos = attr["POSITION"].as_u64().map(|i| g.accessor(i)).transpose()?;
            let num_all = pos.as_ref().map_or(0, |a| a.count);
            let mut index_buffer = Vec::new();
            let mut remap: Option<Vec<u32>> = None;
            if let Some(ii) = prim["indices"].as_u64() {
                let acc = g.accessor(ii)?;
                let mut rev: Vec<u32> = Vec::new();
                let mut table = Vec::new();
                for i in 0..acc.count {
                    let index = acc.uint(i)?;
                    if index as usize >= num_all {
                        index_buffer.push(index);
                        continue;
                    }
                    if index as usize >= rev.len() {
                        rev.resize(index as usize + 1, u32::MAX);
                    }
                    if rev[index as usize] == u32::MAX {
                        rev[index as usize] = table.len() as u32;
                        table.push(index);
                    }
                    index_buffer.push(rev[index as usize]);
                }
                remap = Some(table);
            }
            let r = remap.as_deref();
            let mut aim = AiMesh::default();
            let base = match mesh["name"].as_str() {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => format!("meshes[{mi}]"),
            };
            aim.name = if prims.len() > 1 { format!("{base}-{p}") } else { base };
            let mode = prim["mode"].as_u64().unwrap_or(4);
            aim.primitive_types |= match mode {
                0 => PT_POINT,
                1..=3 => PT_LINE,
                _ => PT_TRIANGLE,
            };
            if let Some(a) = &pos {
                aim.vertices = a.floats(3, r)?.into_iter().map(|v| [v[0], v[1], v[2]]).collect();
            }
            if let Some(ni) = attr["NORMAL"].as_u64() {
                let a = g.accessor(ni)?;
                if a.count == num_all {
                    let ns: Vec<[f32; 3]> = a.floats(3, r)?.into_iter().map(|v| [v[0], v[1], v[2]]).collect();
                    if let Some(ti) = attr["TANGENT"].as_u64() {
                        let t = g.accessor(ti)?;
                        if t.count == num_all {
                            let ts = t.floats(4, r)?;
                            aim.tangents = Some(ts.iter().map(|v| [v[0], v[1], v[2]]).collect());
                            aim.bitangents = Some(
                                ns.iter()
                                    .zip(ts.iter())
                                    .map(|(a, b)| {
                                        let c = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
                                        [c[0] * b[3], c[1] * b[3], c[2] * b[3]]
                                    })
                                    .collect(),
                            );
                        }
                    }
                    aim.normals = Some(ns);
                }
            }
            for c in 0..MAX_COLORS {
                let Some(ci) = attr[format!("COLOR_{c}")].as_u64() else { break };
                let a = g.accessor(ci)?;
                if a.count != num_all {
                    continue;
                }
                let cols: Vec<[f32; 4]> = match a.comp_type {
                    5126 => a.floats(4, r)?.into_iter().map(|v| [v[0], v[1], v[2], v[3]]).collect(),
                    5121 | 5123 => {
                        let (max, cs) = if a.comp_type == 5121 { (255.0f32, 1) } else { (65535.0, 2) };
                        a.extract(4 * cs, r)?
                            .into_iter()
                            .map(|e| {
                                let gv = |i: usize| if cs == 1 { e[i] as f32 } else { u16::from_le_bytes([e[2 * i], e[2 * i + 1]]) as f32 };
                                [gv(0) / max, gv(1) / max, gv(2) / max, gv(3) / max]
                            })
                            .collect()
                    }
                    _ => continue,
                };
                aim.colors[c] = Some(cols);
            }
            for tc in 0..MAX_UV {
                let Some(ti) = attr[format!("TEXCOORD_{tc}")].as_u64() else { break };
                let a = g.accessor(ti)?;
                if a.count != num_all {
                    continue;
                }
                let mut uv: Vec<[f32; 3]> = a.floats(3, r)?.into_iter().map(|v| [v[0], v[1], v[2]]).collect();
                for v in uv.iter_mut() {
                    v[1] = 1.0 - v[1];
                }
                aim.uvs[tc] = Some(uv);
                aim.num_uv_components[tc] = a.num_comp as u32;
            }
            let nv = aim.vertices.len() as u32;
            let idx: Vec<u32> = if remap.is_some() { index_buffer } else { (0..nv).collect() };
            let count = idx.len();
            let mut faces: Vec<Vec<u32>> = Vec::new();
            let mut push = |f: Vec<u32>| {
                if f.iter().all(|&i| i < nv) {
                    faces.push(f);
                }
            };
            match mode {
                0 => idx.iter().for_each(|&i| push(vec![i])),
                1 => (0..count / 2).for_each(|i| push(vec![idx[2 * i], idx[2 * i + 1]])),
                2 | 3 => {
                    if count >= 2 {
                        push(vec![idx[0], idx[1]]);
                        for i in 2..count {
                            push(vec![idx[i - 1], idx[i]]);
                        }
                        if mode == 2 {
                            push(vec![idx[count - 1], idx[0]]);
                        }
                    }
                }
                5 => {
                    for i in 0..count.saturating_sub(2) {
                        if (i + 1) % 2 == 0 {
                            push(vec![idx[i + 1], idx[i], idx[i + 2]]);
                        } else {
                            push(vec![idx[i], idx[i + 1], idx[i + 2]]);
                        }
                    }
                }
                6 => {
                    if count >= 3 {
                        push(vec![idx[0], idx[1], idx[2]]);
                        for i in 1..count - 2 {
                            push(vec![idx[0], idx[i + 1], idx[i + 2]]);
                        }
                    }
                }
                _ => (0..count / 3).for_each(|i| push(vec![idx[3 * i], idx[3 * i + 1], idx[3 * i + 2]])),
            }
            if faces.is_empty() {
                return err(format!("Mesh \"{}\" has no faces", aim.name));
            }
            aim.faces = faces;
            aim.material_index = prim["material"].as_u64().map(|m| m as u32).unwrap_or(default_mat);
            out.meshes.push(aim);
            remaps.push(remap);
        }
    }
    mesh_offsets.push(out.meshes.len());
    let ctx = Ctx { g: &g, mesh_offsets, remaps };

    // ImportNodes
    let scene_idx = g.json["scene"].as_u64().unwrap_or(0) as usize;
    let Some(scene) = g.arr("scenes").get(scene_idx) else { return err("No scene") };
    let roots: Vec<usize> =
        scene["nodes"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64()).map(|v| v as usize).collect()).unwrap_or_default();
    if roots.len() == 1 {
        out.root = import_node(&ctx, &mut out, roots[0], None, 0)?;
    } else {
        out.root = out.add_node("ROOT", AiMat4::IDENTITY, None);
        let root = out.root;
        for &r in &roots {
            import_node(&ctx, &mut out, r, Some(root), 0)?;
        }
    }

    // ImportAnimations
    let nodes = g.arr("nodes");
    for anim in g.arr("animations") {
        let samplers = anim["samplers"].as_array().cloned().unwrap_or_default();
        let channels = anim["channels"].as_array().cloned().unwrap_or_default();
        let mut per_node: std::collections::HashMap<u32, [Option<usize>; 4]> = std::collections::HashMap::new();
        let mut order = Vec::new();
        for ch in &channels {
            let Some(si) = ch["sampler"].as_u64().map(|v| v as usize) else { continue };
            let Some(s) = samplers.get(si) else { continue };
            let (Some(inp), Some(outp)) = (s["input"].as_u64(), s["output"].as_u64()) else { continue };
            if g.accessor(inp)?.count > g.accessor(outp)?.count {
                continue;
            }
            let Some(node) = ch["target"]["node"].as_u64() else { continue };
            let node = node as u32;
            order.push(node);
            let e = per_node.entry(node).or_insert([None; 4]);
            match ch["target"]["path"].as_str().unwrap_or("") {
                "translation" => e[0] = Some(si),
                "rotation" => e[1] = Some(si),
                "scale" => e[2] = Some(si),
                "weights" => e[3] = Some(si),
                _ => {}
            }
        }
        let read = |si: usize, comps: usize| -> Result<(Vec<f32>, Vec<Vec<f32>>, bool), ImportError> {
            let s = &samplers[si];
            let times = g.accessor(s["input"].as_u64().unwrap_or(0))?.floats(1, None)?.into_iter().map(|v| v[0]).collect();
            let vals = g.accessor(s["output"].as_u64().unwrap_or(0))?.floats(comps, None)?;
            Ok((times, vals, s["interpolation"].as_str() == Some("CUBICSPLINE")))
        };
        let mut na = AiAnimation { name: anim["name"].as_str().unwrap_or("").to_string(), ..Default::default() };
        for node in libcxx_unordered_order(&order) {
            let smp = per_node[&node];
            if smp[0].is_none() && smp[1].is_none() && smp[2].is_none() {
                continue;
            }
            let n = &nodes[node as usize];
            let mut ch = AiNodeAnim { node_name: node_name(n, node as usize), ..Default::default() };
            let ms = |t: f32| (t * 1000.0f32) as f64;
            if let Some(si) = smp[0] {
                let (t, v, cubic) = read(si, 3)?;
                let mut ii = cubic as usize;
                for &time in &t {
                    ch.position_keys.push(AiVectorKey { time: ms(time), value: [v[ii][0], v[ii][1], v[ii][2]] });
                    ii += if cubic { 3 } else { 1 };
                }
            } else if let Some(tv) = fvec(&n["translation"]) {
                ch.position_keys.push(AiVectorKey { time: 0.0, value: [tv[0], tv[1], tv[2]] });
            }
            if let Some(si) = smp[1] {
                let (t, v, cubic) = read(si, 4)?;
                let mut ii = cubic as usize;
                for &time in &t {
                    let q = &v[ii];
                    ch.rotation_keys.push(AiQuatKey { time: ms(time), value: AiQuat { x: q[0], y: q[1], z: q[2], w: q[3] } });
                    ii += if cubic { 3 } else { 1 };
                }
            } else if let Some(rv) = fvec(&n["rotation"]) {
                ch.rotation_keys.push(AiQuatKey { time: 0.0, value: AiQuat { x: rv[0], y: rv[1], z: rv[2], w: rv[3] } });
            }
            if let Some(si) = smp[2] {
                let (t, v, cubic) = read(si, 3)?;
                let mut ii = cubic as usize;
                for &time in &t {
                    ch.scaling_keys.push(AiVectorKey { time: ms(time), value: [v[ii][0], v[ii][1], v[ii][2]] });
                    ii += if cubic { 3 } else { 1 };
                }
            } else if let Some(sv) = fvec(&n["scale"]) {
                ch.scaling_keys.push(AiVectorKey { time: 0.0, value: [sv[0], sv[1], sv[2]] });
            }
            na.channels.push(ch);
        }
        let mut max_d = 0.0f64;
        for c in &na.channels {
            for t in [c.position_keys.last(), c.scaling_keys.last()].into_iter().flatten().map(|k| k.time)
                .chain(c.rotation_keys.last().map(|k| k.time))
            {
                if t > max_d {
                    max_d = t;
                }
            }
        }
        na.duration = max_d;
        na.ticks_per_second = 1000.0;
        out.animations.push(na);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::libcxx_unordered_order;

    #[test]
    fn unordered_map_order() {
        // one key: itself; 2 then 0 land in buckets 0/0 of 2 buckets
        assert_eq!(libcxx_unordered_order(&[5]), vec![5]);
        // with 2 buckets, 1 then 3: same bucket, newest first
        assert_eq!(libcxx_unordered_order(&[1, 3]).len(), 2);
    }
}
