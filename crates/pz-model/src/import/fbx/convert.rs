//! `FBXConverter`: the FBX DOM to an [`AiScene`] — node chains for
//! pivots (`$AssimpFbx$` helper nodes), meshes split by material, skin
//! clusters to bones, materials (for the steps that compare them) and
//! animation stacks.

use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use super::document::*;
use crate::import::aimath::{AiMat4, AiQuat};
use crate::import::scene::*;
use crate::import::ImportError;

const MAGIC_NODE_TAG: &str = "_$AssimpFbx$";
const FBX_TIME: f64 = 46186158000.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
enum Comp {
    GeometricScalingInverse = 0,
    GeometricRotationInverse,
    GeometricTranslationInverse,
    Translation,
    RotationOffset,
    RotationPivot,
    PreRotation,
    Rotation,
    PostRotation,
    RotationPivotInverse,
    ScalingOffset,
    ScalingPivot,
    Scaling,
    ScalingPivotInverse,
    GeometricTranslation,
    GeometricRotation,
    GeometricScaling,
}

const COMP_MAX: usize = 17;

const ALL_COMPS: [Comp; COMP_MAX] = [
    Comp::GeometricScalingInverse,
    Comp::GeometricRotationInverse,
    Comp::GeometricTranslationInverse,
    Comp::Translation,
    Comp::RotationOffset,
    Comp::RotationPivot,
    Comp::PreRotation,
    Comp::Rotation,
    Comp::PostRotation,
    Comp::RotationPivotInverse,
    Comp::ScalingOffset,
    Comp::ScalingPivot,
    Comp::Scaling,
    Comp::ScalingPivotInverse,
    Comp::GeometricTranslation,
    Comp::GeometricRotation,
    Comp::GeometricScaling,
];

fn comp_name(c: Comp) -> &'static str {
    match c {
        Comp::Translation => "Translation",
        Comp::RotationOffset => "RotationOffset",
        Comp::RotationPivot => "RotationPivot",
        Comp::PreRotation => "PreRotation",
        Comp::Rotation => "Rotation",
        Comp::PostRotation => "PostRotation",
        Comp::RotationPivotInverse => "RotationPivotInverse",
        Comp::ScalingOffset => "ScalingOffset",
        Comp::ScalingPivot => "ScalingPivot",
        Comp::Scaling => "Scaling",
        Comp::ScalingPivotInverse => "ScalingPivotInverse",
        Comp::GeometricScaling => "GeometricScaling",
        Comp::GeometricRotation => "GeometricRotation",
        Comp::GeometricTranslation => "GeometricTranslation",
        Comp::GeometricScalingInverse => "GeometricScalingInverse",
        Comp::GeometricRotationInverse => "GeometricRotationInverse",
        Comp::GeometricTranslationInverse => "GeometricTranslationInverse",
    }
}

fn comp_property(c: Comp) -> &'static str {
    match c {
        Comp::Translation => "Lcl Translation",
        Comp::Rotation => "Lcl Rotation",
        Comp::Scaling => "Lcl Scaling",
        _ => comp_name(c),
    }
}

fn chain_node_name(name: &str, c: Comp) -> String {
    format!("{name}{MAGIC_NODE_TAG}_{}", comp_name(c))
}

fn sq(v: [f32; 3]) -> f32 {
    v[0] * v[0] + v[1] * v[1] + v[2] * v[2]
}

fn sub1(v: [f32; 3]) -> [f32; 3] {
    [v[0] - 1.0, v[1] - 1.0, v[2] - 1.0]
}

/// `FBXConverter::GetRotationMatrix` (degrees, assimp's DEG_TO_RAD).
fn rotation_matrix(mode: i32, rot: [f32; 3]) -> AiMat4 {
    if mode == 6 {
        return AiMat4::IDENTITY;
    }
    let eps = f32::EPSILON;
    let mut is_id = [true; 3];
    let mut temp = [AiMat4::IDENTITY; 3];
    let d2r = |x: f32| x * 0.017_453_292_5_f32;
    if rot[2].abs() > eps {
        temp[2] = AiMat4::rotation_z(d2r(rot[2]));
        is_id[2] = false;
    }
    if rot[1].abs() > eps {
        temp[1] = AiMat4::rotation_y(d2r(rot[1]));
        is_id[1] = false;
    }
    if rot[0].abs() > eps {
        temp[0] = AiMat4::rotation_x(d2r(rot[0]));
        is_id[0] = false;
    }
    let order = match mode {
        0 => [2, 1, 0],
        1 => [1, 2, 0],
        2 => [0, 2, 1],
        3 => [2, 0, 1],
        4 => [1, 0, 2],
        _ => [0, 1, 2],
    };
    let mut out = AiMat4::IDENTITY;
    if !is_id[order[0]] {
        out = temp[order[0]];
    }
    if !is_id[order[1]] {
        out = out.mul(&temp[order[1]]);
    }
    if !is_id[order[2]] {
        out = out.mul(&temp[order[2]]);
    }
    out
}

fn m3(m: &AiMat4) -> [[f32; 3]; 3] {
    [
        [m.m[0][0], m.m[0][1], m.m[0][2]],
        [m.m[1][0], m.m[1][1], m.m[1][2]],
        [m.m[2][0], m.m[2][1], m.m[2][2]],
    ]
}

fn euler_to_quat(rot: [f32; 3], order: i32) -> AiQuat {
    AiQuat::from_matrix(&m3(&rotation_matrix(order, rot)))
}

fn rotation_order(m: &Model) -> Result<i32, ImportError> {
    let v = m.props.get_or::<i32>("RotationOrder", 0)?;
    Ok(if (0..7).contains(&v) { v } else { 0 })
}

fn fix_node_name(name: &str) -> String {
    name.strip_prefix("Model::").unwrap_or(name).to_string()
}

type KeyFrameList = (Vec<i64>, Vec<f32>, usize);

pub struct Converter<'a, 'd, 'b> {
    doc: &'b Document<'a, 'd>,
    out: AiScene,
    node_names: HashMap<String, u32>,
    meshes_converted: HashMap<u64, Vec<u32>>,
    materials_converted: HashMap<u64, u32>,
    textures_converted: HashMap<u64, u32>,
    num_textures: u32,
    default_material: Option<u32>,
    node_anim_chain_bits: HashMap<String, u32>,
    anim_fps: f64,
}

impl<'a, 'd, 'b> Converter<'a, 'd, 'b> {
    pub fn convert(doc: &'b Document<'a, 'd>) -> Result<AiScene, ImportError> {
        let mut c = Converter {
            doc,
            out: AiScene::default(),
            node_names: HashMap::new(),
            meshes_converted: HashMap::new(),
            materials_converted: HashMap::new(),
            textures_converted: HashMap::new(),
            num_textures: 0,
            default_material: None,
            node_anim_chain_bits: HashMap::new(),
            anim_fps: 1.0,
        };
        c.convert_animations()?;
        let rn = c.unique_name("RootNode");
        c.out.root = c.out.add_node(&rn, AiMat4::IDENTITY, None);
        let root = c.out.root;
        c.convert_nodes(0, root)?;
        Ok(c.out)
    }

    fn unique_name(&mut self, name: &str) -> String {
        if !self.node_names.contains_key(name) {
            self.node_names.insert(name.to_string(), 0);
            return name.to_string();
        }
        let mut i = *self.node_names.get(name).unwrap();
        loop {
            i += 1;
            let u = format!("{name}{i:03}");
            if !self.node_names.contains_key(&u) {
                self.node_names.insert(name.to_string(), i);
                self.node_names.insert(u.clone(), 0);
                return u;
            }
        }
    }

    fn convert_nodes(&mut self, id: u64, parent: usize) -> Result<(), ImportError> {
        let conns = self.doc.by_dest(id, &["Model"]);
        let mut firsts: Vec<usize> = Vec::new();
        for con in conns {
            if !con.prop.is_empty() {
                continue;
            }
            let Some(obj) = self.doc.get(con.src)? else { continue };
            let Some(model) = obj.as_model() else { continue };
            let new_abs = self.out.nodes[parent].transform;
            let node_name = fix_node_name(&model.name);
            let (mut chain, post, need_additional) = self.transformation_chain(model, &node_name)?;
            if need_additional {
                chain.push((node_name.clone(), AiMat4::IDENTITY));
            }
            // Link the chain: the first node is attached to `parent` at the
            // end (in nodes order), the rest hang below each other.
            let mut ids = Vec::new();
            let mut last: Option<usize> = None;
            for (n, t) in &chain {
                let id = self.out.nodes.len();
                self.out.nodes.push(AiNode { name: n.clone(), transform: *t, parent: Some(last.unwrap_or(parent)), ..Default::default() });
                if let Some(l) = last {
                    self.out.nodes[l].children.push(id);
                }
                ids.push(id);
                last = Some(id);
            }
            let main = *ids.last().unwrap();
            self.convert_model(model, main, &new_abs)?;
            let mut last_parent = main;
            let child_conns = self.doc.by_dest(model.id, &["Model"]);
            if !child_conns.is_empty() {
                for (n, t) in &post {
                    let id = self.out.nodes.len();
                    self.out.nodes.push(AiNode { name: n.clone(), transform: *t, parent: Some(last_parent), ..Default::default() });
                    self.out.nodes[last_parent].children.push(id);
                    last_parent = id;
                }
            }
            self.convert_nodes(model.id, last_parent)?;
            firsts.push(ids[0]);
        }
        self.out.nodes[parent].children = firsts;
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    fn transformation_chain(
        &mut self,
        model: &Model,
        name: &str,
    ) -> Result<(Vec<(String, AiMat4)>, Vec<(String, AiMat4)>, bool), ImportError> {
        let props = &model.props;
        let rot = rotation_order(model)?;
        let mut chain = [AiMat4::IDENTITY; COMP_MAX];
        let mut bits = 0u32;
        let eps = f32::EPSILON;
        let g = |n: &str| props.get_ok::<[f32; 3]>(n, false);
        let b = |c: Comp| 1u32 << (c as usize);
        if let Some(v) = g("PreRotation")? {
            if sq(v) > eps {
                bits |= b(Comp::PreRotation);
                chain[Comp::PreRotation as usize] = rotation_matrix(0, v);
            }
        }
        if let Some(v) = g("PostRotation")? {
            if sq(v) > eps {
                bits |= b(Comp::PostRotation);
                chain[Comp::PostRotation as usize] = rotation_matrix(0, v);
            }
        }
        if let Some(v) = g("RotationPivot")? {
            if sq(v) > eps {
                bits |= b(Comp::RotationPivot) | b(Comp::RotationPivotInverse);
                chain[Comp::RotationPivot as usize] = AiMat4::translation(v);
                chain[Comp::RotationPivotInverse as usize] = AiMat4::translation([-v[0], -v[1], -v[2]]);
            }
        }
        if let Some(v) = g("RotationOffset")? {
            if sq(v) > eps {
                bits |= b(Comp::RotationOffset);
                chain[Comp::RotationOffset as usize] = AiMat4::translation(v);
            }
        }
        if let Some(v) = g("ScalingOffset")? {
            if sq(v) > eps {
                bits |= b(Comp::ScalingOffset);
                chain[Comp::ScalingOffset as usize] = AiMat4::translation(v);
            }
        }
        if let Some(v) = g("ScalingPivot")? {
            if sq(v) > eps {
                bits |= b(Comp::ScalingPivot) | b(Comp::ScalingPivotInverse);
                chain[Comp::ScalingPivot as usize] = AiMat4::translation(v);
                chain[Comp::ScalingPivotInverse as usize] = AiMat4::translation([-v[0], -v[1], -v[2]]);
            }
        }
        if let Some(v) = g("Lcl Translation")? {
            if sq(v) > eps {
                bits |= b(Comp::Translation);
                chain[Comp::Translation as usize] = AiMat4::translation(v);
            }
        }
        if let Some(v) = g("Lcl Scaling")? {
            if sq(sub1(v)) > eps {
                bits |= b(Comp::Scaling);
                chain[Comp::Scaling as usize] = AiMat4::scaling(v);
            }
        }
        if let Some(v) = g("Lcl Rotation")? {
            if sq(v) > eps {
                bits |= b(Comp::Rotation);
                chain[Comp::Rotation as usize] = rotation_matrix(rot, v);
            }
        }
        if let Some(v) = g("GeometricScaling")? {
            if sq(sub1(v)) > eps {
                bits |= b(Comp::GeometricScaling);
                chain[Comp::GeometricScaling as usize] = AiMat4::scaling(v);
                let mut inv = v;
                let mut can = true;
                for i in 0..3 {
                    if v[i].abs() > eps {
                        inv[i] = 1.0 / v[i];
                    } else {
                        can = false;
                        break;
                    }
                }
                if can {
                    bits |= b(Comp::GeometricScalingInverse);
                    chain[Comp::GeometricScalingInverse as usize] = AiMat4::scaling(inv);
                }
            }
        }
        if let Some(v) = g("GeometricRotation")? {
            if sq(v) > eps {
                bits |= b(Comp::GeometricRotation) | b(Comp::GeometricRotationInverse);
                chain[Comp::GeometricRotation as usize] = rotation_matrix(rot, v);
                chain[Comp::GeometricRotationInverse as usize] = rotation_matrix(rot, v).inverse();
            }
        }
        if let Some(v) = g("GeometricTranslation")? {
            if sq(v) > eps {
                bits |= b(Comp::GeometricTranslation) | b(Comp::GeometricTranslationInverse);
                chain[Comp::GeometricTranslation as usize] = AiMat4::translation(v);
                chain[Comp::GeometricTranslationInverse as usize] = AiMat4::translation([-v[0], -v[1], -v[2]]);
            }
        }
        let simple = b(Comp::Translation) + b(Comp::Scaling) + b(Comp::Rotation);
        let complex = ((1u32 << COMP_MAX) - 1) - simple;
        let mut out = Vec::new();
        let mut post = Vec::new();
        if bits & complex != 0 && self.doc.settings.preserve_pivots {
            let anim_bits = self.node_anim_chain_bits.get(name).copied().unwrap_or(0);
            for (i, &c) in ALL_COMPS.iter().enumerate() {
                let bit = 1u32 << i;
                if bits & bit == 0 && anim_bits & bit == 0 {
                    continue;
                }
                if c == Comp::PostRotation {
                    chain[i] = chain[i].inverse();
                }
                let nd = (chain_node_name(name, c), chain[i]);
                if matches!(
                    c,
                    Comp::GeometricScalingInverse | Comp::GeometricRotationInverse | Comp::GeometricTranslationInverse
                ) {
                    post.push(nd);
                } else {
                    out.push(nd);
                }
            }
            return Ok((out, post, true));
        }
        let mut t = AiMat4::IDENTITY;
        for m in chain.iter().skip(Comp::Translation as usize) {
            t = t.mul(m);
        }
        out.push((name.to_string(), t));
        Ok((out, post, false))
    }

    fn convert_model(&mut self, model: &Model, node: usize, abs: &AiMat4) -> Result<(), ImportError> {
        let mut meshes = Vec::new();
        for geo in model.geometry.clone() {
            let ObjectKind::Geometry(g) = &geo.kind else { continue };
            if g.mesh.is_some() {
                meshes.extend(self.convert_mesh(&geo, g, model, node, abs)?);
            }
        }
        if !meshes.is_empty() {
            self.out.nodes[node].meshes = meshes;
        }
        Ok(())
    }

    fn convert_mesh(&mut self, geo_obj: &Rc<Object>, geo: &Geometry, model: &Model, node: usize, abs: &AiMat4) -> Result<Vec<u32>, ImportError> {
        if let Some(v) = self.meshes_converted.get(&geo_obj.id) {
            return Ok(v.clone());
        }
        let mesh = geo.mesh.as_ref().unwrap();
        if mesh.vertices.is_empty() || mesh.faces.is_empty() {
            return Ok(vec![]);
        }
        let mind = &mesh.materials;
        if self.doc.settings.read_materials && !mind.is_empty() {
            let base = mind[0];
            if mind.iter().any(|&i| i != base) {
                let mut had = std::collections::BTreeSet::new();
                let mut out = Vec::new();
                for &i in mind {
                    if had.insert(i) {
                        out.push(self.convert_mesh_multi(geo_obj, geo, model, i, node, abs)?);
                    }
                }
                return Ok(out);
            }
        }
        Ok(vec![self.convert_mesh_single(geo_obj, geo, model, node, abs)?])
    }

    fn setup_empty_mesh(&mut self, geo_obj: &Rc<Object>, node: usize) -> AiMesh {
        let idx = self.out.meshes.len() as u32;
        self.meshes_converted.entry(geo_obj.id).or_default().push(idx);
        let mut name = geo_obj.name.clone();
        if let Some(s) = name.strip_prefix("Geometry::") {
            name = s.to_string();
        }
        AiMesh { name: if name.is_empty() { self.out.nodes[node].name.clone() } else { name }, ..Default::default() }
    }

    fn binormals(mesh: &MeshGeometry) -> Option<Vec<[f32; 3]>> {
        if !mesh.binormals.is_empty() {
            return Some(mesh.binormals.clone());
        }
        if mesh.normals.is_empty() {
            return None;
        }
        Some(
            mesh.normals
                .iter()
                .zip(mesh.tangents.iter())
                .map(|(a, b)| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]])
                .collect(),
        )
    }

    fn ptype(n: u32) -> u32 {
        match n {
            1 => PT_POINT,
            2 => PT_LINE,
            3 => PT_TRIANGLE,
            _ => PT_POLYGON,
        }
    }

    fn convert_mesh_single(&mut self, geo_obj: &Rc<Object>, geo: &Geometry, model: &Model, node: usize, abs: &AiMat4) -> Result<u32, ImportError> {
        let mesh = geo.mesh.as_ref().unwrap();
        let mut out = self.setup_empty_mesh(geo_obj, node);
        let mesh_index = self.out.meshes.len() as u32;
        out.vertices = mesh.vertices.clone();
        let mut cursor = 0u32;
        for &pc in &mesh.faces {
            out.primitive_types |= Self::ptype(pc);
            out.faces.push((cursor..cursor + pc).collect());
            cursor += pc;
        }
        if !mesh.normals.is_empty() {
            out.normals = Some(mesh.normals.clone());
        }
        if !mesh.tangents.is_empty() {
            if let Some(b) = Self::binormals(mesh) {
                out.tangents = Some(mesh.tangents.clone());
                out.bitangents = Some(b);
            }
        }
        for i in 0..MAX_UV {
            if mesh.uvs[i].is_empty() {
                break;
            }
            out.uvs[i] = Some(mesh.uvs[i].iter().map(|v| [v[0], v[1], 0.0]).collect());
            out.num_uv_components[i] = 2;
        }
        for i in 0..MAX_COLORS {
            if mesh.colors[i].is_empty() {
                break;
            }
            out.colors[i] = Some(mesh.colors[i].clone());
        }
        // The mesh slot must exist before materials look meshes up.
        self.out.meshes.push(AiMesh::default());
        if !self.doc.settings.read_materials || mesh.materials.is_empty() {
            out.material_index = self.default_material();
        } else {
            out.material_index = self.convert_material_for_mesh(model, mesh, mesh.materials[0])?;
        }
        if self.doc.settings.read_weights {
            if let Some(skin) = &geo.skin {
                self.convert_weights(&mut out, mesh, skin, None, abs)?;
            }
        }
        self.out.meshes[mesh_index as usize] = out;
        Ok(mesh_index)
    }

    fn convert_mesh_multi(
        &mut self,
        geo_obj: &Rc<Object>,
        geo: &Geometry,
        model: &Model,
        index: i32,
        node: usize,
        abs: &AiMat4,
    ) -> Result<u32, ImportError> {
        let mesh = geo.mesh.as_ref().unwrap();
        let mut out = self.setup_empty_mesh(geo_obj, node);
        let mesh_index = self.out.meshes.len() as u32;
        let process_weights = self.doc.settings.read_weights && geo.skin.is_some();
        let mut count_vertices = 0u32;
        for (m, &f) in mesh.materials.iter().zip(mesh.faces.iter()) {
            if *m == index {
                count_vertices += f;
            }
        }
        let mut reverse = if process_weights { vec![0u32; count_vertices as usize] } else { Vec::new() };
        let binorm = if mesh.tangents.is_empty() { None } else { Self::binormals(mesh) };
        let has_n = !mesh.normals.is_empty();
        let has_t = binorm.is_some();
        let num_uvs = (0..MAX_UV).take_while(|&i| !mesh.uvs[i].is_empty()).count();
        let num_vcs = (0..MAX_COLORS).take_while(|&i| !mesh.colors[i].is_empty()).count();
        let (mut ns, mut ts, mut bs) = (Vec::new(), Vec::new(), Vec::new());
        let mut uvs: Vec<Vec<[f32; 3]>> = vec![Vec::new(); num_uvs];
        let mut cols: Vec<Vec<[f32; 4]>> = vec![Vec::new(); num_vcs];
        let (mut cursor, mut in_cursor) = (0u32, 0u32);
        for (m, &pc) in mesh.materials.iter().zip(mesh.faces.iter()) {
            if *m != index {
                in_cursor += pc;
                continue;
            }
            out.primitive_types |= Self::ptype(pc);
            let mut f = Vec::with_capacity(pc as usize);
            for _ in 0..pc {
                f.push(cursor);
                if process_weights {
                    reverse[cursor as usize] = in_cursor;
                }
                let ic = in_cursor as usize;
                out.vertices.push(mesh.vertices[ic]);
                if has_n {
                    ns.push(mesh.normals[ic]);
                }
                if let Some(b) = &binorm {
                    ts.push(mesh.tangents[ic]);
                    bs.push(b[ic]);
                }
                for j in 0..num_uvs {
                    let v = mesh.uvs[j][ic];
                    uvs[j].push([v[0], v[1], 0.0]);
                }
                for j in 0..num_vcs {
                    cols[j].push(mesh.colors[j][ic]);
                }
                cursor += 1;
                in_cursor += 1;
            }
            out.faces.push(f);
        }
        if has_n {
            out.normals = Some(ns);
        }
        if has_t {
            out.tangents = Some(ts);
            out.bitangents = Some(bs);
        }
        for (j, u) in uvs.into_iter().enumerate() {
            out.uvs[j] = Some(u);
            out.num_uv_components[j] = 2;
        }
        for (j, c) in cols.into_iter().enumerate() {
            out.colors[j] = Some(c);
        }
        self.out.meshes.push(AiMesh::default());
        out.material_index = self.convert_material_for_mesh(model, mesh, index)?;
        if process_weights {
            let skin = geo.skin.clone().unwrap();
            self.convert_weights(&mut out, mesh, &skin, Some((index, &reverse)), abs)?;
        }
        self.out.meshes[mesh_index as usize] = out;
        Ok(mesh_index)
    }

    fn convert_weights(
        &mut self,
        out: &mut AiMesh,
        geo: &MeshGeometry,
        skin: &Rc<Object>,
        mat: Option<(i32, &Vec<u32>)>,
        _abs: &AiMat4,
    ) -> Result<(), ImportError> {
        let ObjectKind::Skin(sk) = &skin.kind else { return Ok(()) };
        let mut bones: Vec<AiBone> = Vec::new();
        let mut bone_map: HashMap<String, usize> = HashMap::new();
        for cobj in &sk.clusters {
            let ObjectKind::Cluster(cluster) = &cobj.kind else { continue };
            let mut out_indices: Vec<u32> = Vec::new();
            let mut index_out: Vec<Option<usize>> = Vec::new();
            let mut count_out: Vec<usize> = Vec::new();
            for &index in &cluster.indices {
                let Some(out_idx) = geo.to_output_vertex_index(index as u32) else {
                    return Err(ImportError("FBX: cluster index out of range".into()));
                };
                index_out.push(None);
                count_out.push(0);
                for &oi in out_idx {
                    let ok = match mat {
                        None => true,
                        Some((m, _)) => geo.materials[geo.face_for_vertex_index(oi)] == m,
                    };
                    if ok {
                        if index_out.last().unwrap().is_none() {
                            *index_out.last_mut().unwrap() = Some(out_indices.len());
                        }
                        match mat {
                            None => out_indices.push(oi),
                            Some((_, rev)) => {
                                let pos = rev.partition_point(|&x| x < oi);
                                out_indices.push(pos as u32);
                            }
                        }
                        *count_out.last_mut().unwrap() += 1;
                    }
                }
            }
            // ConvertCluster
            let ObjectKind::Model(target) = &cluster.node.kind else { continue };
            let deformer_name = target.name.clone();
            if let Some(&bi) = bone_map.get(&deformer_name) {
                let b = bones[bi].clone();
                bones.push(b);
                continue;
            }
            let mut bone = AiBone { name: fix_node_name(&deformer_name), offset: cluster.transform, weights: Vec::new() };
            for (i, io) in index_out.iter().enumerate() {
                let Some(ii) = io else { continue };
                for j in 0..count_out[i] {
                    bone.weights.push((out_indices[ii + j], cluster.weights[i]));
                }
            }
            bone_map.insert(deformer_name, bones.len());
            bones.push(bone);
        }
        out.bones = bones;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Materials

    fn default_material(&mut self) -> u32 {
        if let Some(d) = self.default_material {
            return d;
        }
        let mut m = AiMaterial::default();
        m.add_floats("$clr.diffuse", 0, 0, &[0.8, 0.8, 0.8]);
        m.add_string("?mat.name", 0, 0, "DefaultMaterial");
        self.out.materials.push(m);
        let i = (self.out.materials.len() - 1) as u32;
        self.default_material = Some(i);
        i
    }

    fn convert_material_for_mesh(&mut self, model: &Model, geo: &MeshGeometry, index: i32) -> Result<u32, ImportError> {
        if index < 0 || index as usize >= model.materials.len() {
            return Ok(self.default_material());
        }
        let mat = model.materials[index as usize].clone();
        if let Some(&i) = self.materials_converted.get(&mat.id) {
            return Ok(i);
        }
        let i = self.convert_material(&mat, geo)?;
        self.materials_converted.insert(mat.id, i);
        Ok(i)
    }

    fn convert_material(&mut self, obj: &Rc<Object>, geo: &MeshGeometry) -> Result<u32, ImportError> {
        let ObjectKind::Material(mat) = &obj.kind else { return Ok(self.default_material()) };
        let idx = self.out.materials.len() as u32;
        self.materials_converted.insert(obj.id, idx);
        self.out.materials.push(AiMaterial::default());
        let mut out = AiMaterial::default();
        let mut name = mat.name.clone();
        if let Some(s) = name.strip_prefix("Material::") {
            name = s.to_string();
        }
        if !name.is_empty() {
            out.add_string("?mat.name", 0, 0, &name);
        }
        if mat.shading == "phong" {
            out.add_ints("$mat.shadingm", 0, 0, &[3]);
        }
        let p = &mat.props;
        // SetShadingPropertiesCommon
        let color_factored = |c: &str, f: &str, tmpl: bool| -> Result<Option<[f32; 3]>, ImportError> {
            let Some(mut v) = p.get_ok::<[f32; 3]>(c, tmpl)? else { return Ok(None) };
            if f.is_empty() {
                return Ok(Some(v));
            }
            if let Some(fac) = p.get_ok::<f32>(f, tmpl)? {
                v = [v[0] * fac, v[1] * fac, v[2] * fac];
            }
            Ok(Some(v))
        };
        if let Some(v) = color_factored("DiffuseColor", "DiffuseFactor", true)? {
            out.add_floats("$clr.diffuse", 0, 0, &v);
        }
        if let Some(v) = color_factored("EmissiveColor", "EmissiveFactor", true)? {
            out.add_floats("$clr.emissive", 0, 0, &v);
        } else if let Some(v) = p.get_ok::<[f32; 3]>("Maya|emissive", false)? {
            out.add_floats("$clr.emissive", 0, 0, &v);
        }
        if let Some(v) = color_factored("AmbientColor", "AmbientFactor", true)? {
            out.add_floats("$clr.ambient", 0, 0, &v);
        }
        if let Some(v) = p.get_ok::<[f32; 3]>("SpecularColor", true)? {
            out.add_floats("$clr.specular", 0, 0, &v);
        }
        if let Some(v) = p.get_ok::<f32>("SpecularFactor", true)? {
            out.add_floats("$mat.shinpercent", 0, 0, &[v]);
        }
        if let Some(v) = p.get_ok::<f32>("ShininessExponent", false)? {
            out.add_floats("$mat.shininess", 0, 0, &[v]);
            let rough = 1.0 - (v.sqrt() / 10.0);
            out.add_floats("$mat.roughnessFactor", 0, 0, &[rough]);
        }
        let mut calc_opacity = 1.0f32;
        if let Some(t) = color_factored("TransparentColor", "TransparencyFactor", false)? {
            out.add_floats("$clr.transparent", 0, 0, &t);
            calc_opacity = 1.0 - ((t[0] + t[1] + t[2]) / 3.0);
        }
        if let Some(v) = p.get_ok::<f32>("TransparencyFactor", false)? {
            out.add_floats("$mat.transparencyfactor", 0, 0, &[v]);
        }
        if let Some(v) = p.get_ok::<f32>("Opacity", false)? {
            out.add_floats("$mat.opacity", 0, 0, &[v]);
        } else if calc_opacity as f64 != 1.0 {
            out.add_floats("$mat.opacity", 0, 0, &[calc_opacity]);
        }
        if let Some(v) = p.get_ok::<[f32; 3]>("ReflectionColor", true)? {
            out.add_floats("$clr.reflective", 0, 0, &v);
        }
        if let Some(v) = p.get_ok::<f32>("ReflectionFactor", true)? {
            out.add_floats("$mat.reflectivity", 0, 0, &[v]);
        }
        if let Some(v) = p.get_ok::<f32>("BumpFactor", false)? {
            out.add_floats("$mat.bumpscaling", 0, 0, &[v]);
        }
        if let Some(v) = p.get_ok::<f32>("DisplacementFactor", false)? {
            out.add_floats("$mat.displacementscaling", 0, 0, &[v]);
        }
        if let Some(v) = p.get_ok::<[f32; 3]>("Maya|base_color", false)? {
            out.add_floats("$clr.base", 0, 0, &v);
        }
        for (k, key) in [
            ("Maya|use_color_map", "$mat.useColorMap"),
            ("Maya|use_metallic_map", "$mat.useMetallicMap"),
            ("Maya|metallic", "$mat.metallicFactor"),
            ("Maya|use_roughness_map", "$mat.useRoughnessMap"),
            ("Maya|roughness", "$mat.roughnessFactor"),
            ("Maya|use_emissive_map", "$mat.useEmissiveMap"),
            ("Maya|emissive_intensity", "$mat.emissiveIntensity"),
            ("Maya|use_ao_map", "$mat.useAOMap"),
        ] {
            if let Some(v) = p.get_ok::<f32>(k, false)? {
                out.add_floats(key, 0, 0, &[v]);
            }
        }
        // SetShadingPropertiesRaw
        for (k, v) in p.unparsed() {
            let name = format!("$raw.{k}");
            match v {
                Prop::Vec3(a) => out.add_floats(&name, 0, 0, &a),
                Prop::Color4(a) => out.add_floats(&name, 0, 0, &a),
                Prop::Float(f) => out.add_floats(&name, 0, 0, &[f]),
                Prop::Int(i) => out.add_ints(&name, 0, 0, &[i]),
                Prop::Bool(b) => out.add_ints(&name, 0, 0, &[b as i32]),
                Prop::Str(s) => out.add_string(&name, 0, 0, &s),
                _ => {}
            }
        }
        for (k, tex) in &mat.textures {
            let ObjectKind::Texture(t) = &tex.kind else { continue };
            let name = format!("$raw.{k}");
            let mut path = t.relative_filename.clone();
            if let Some(media) = &t.media {
                if let ObjectKind::Video(v) = &media.kind {
                    if v.content_length > 0 {
                        let i = self.video_index(media.id);
                        path = format!("*{i}");
                    }
                }
            }
            out.add_string(&format!("{name}|file"), 18, 0, &path);
            out.add_floats(&format!("{name}|uvtrafo"), 18, 0, &uv_trafo(t));
            let uv = self.uv_index(t, geo)?;
            out.add_ints(&format!("{name}|uvwsrc"), 18, 0, &[uv]);
        }
        // SetTextureProperties
        let names: [(&str, u32); 42] = [
            ("DiffuseColor", 1),
            ("AmbientColor", 3),
            ("EmissiveColor", 4),
            ("SpecularColor", 2),
            ("SpecularFactor", 2),
            ("TransparentColor", 8),
            ("ReflectionColor", 11),
            ("DisplacementColor", 9),
            ("NormalMap", 6),
            ("Bump", 5),
            ("ShininessExponent", 7),
            ("TransparencyFactor", 8),
            ("EmissiveFactor", 4),
            ("ReflectionFactor", 15),
            ("Maya|DiffuseTexture", 1),
            ("Maya|NormalTexture", 6),
            ("Maya|SpecularTexture", 2),
            ("Maya|FalloffTexture", 8),
            ("Maya|ReflectionMapTexture", 11),
            ("Maya|baseColor", 12),
            ("Maya|normalCamera", 13),
            ("Maya|emissionColor", 14),
            ("Maya|metalness", 15),
            ("Maya|diffuseRoughness", 16),
            ("Maya|TEX_color_map", 12),
            ("Maya|TEX_normal_map", 13),
            ("Maya|TEX_emissive_map", 14),
            ("Maya|TEX_metallic_map", 15),
            ("Maya|TEX_roughness_map", 16),
            ("Maya|TEX_ao_map", 17),
            ("3dsMax|Parameters|base_color_map", 12),
            ("3dsMax|Parameters|bump_map", 13),
            ("3dsMax|Parameters|emission_map", 14),
            ("3dsMax|Parameters|metalness_map", 15),
            ("3dsMax|Parameters|roughness_map", 16),
            ("3dsMax|main|base_color_map", 12),
            ("3dsMax|main|norm_map", 13),
            ("3dsMax|main|emit_color_map", 14),
            ("3dsMax|main|ao_map", 17),
            ("3dsMax|main|opacity_map", 8),
            ("3dsMax|main|metalness_map", 15),
            ("3dsMax|main|specular_map", 2),
        ];
        for (prop, target) in names {
            if let Some(tex) = mat.textures.get(prop) {
                if let ObjectKind::Texture(t) = &tex.kind {
                    out.add_string("$tex.file", target, 0, &t.relative_filename);
                    out.add_floats("$tex.uvtrafo", target, 0, &uv_trafo(t));
                    let uv = self.uv_index(t, geo)?;
                    out.add_ints("$tex.uvwsrc", target, 0, &[uv]);
                }
            }
        }
        // (layered textures: none in the shipped files)
        self.out.materials[idx as usize] = out;
        Ok(idx)
    }

    fn video_index(&mut self, id: u64) -> u32 {
        if let Some(&i) = self.textures_converted.get(&id) {
            return i;
        }
        let i = self.num_textures;
        self.num_textures += 1;
        self.textures_converted.insert(id, i);
        i
    }

    fn uv_index(&self, t: &Texture, geo: &MeshGeometry) -> Result<i32, ImportError> {
        let Some(uvset) = t.props.get_ok::<String>("UVSet", false)? else { return Ok(0) };
        if uvset == "default" || uvset.is_empty() {
            return Ok(0);
        }
        for i in 0..MAX_UV {
            if geo.uvs[i].is_empty() {
                break;
            }
            if geo.uv_names[i] == uvset {
                return Ok(i as i32);
            }
        }
        Ok(0)
    }

    // -----------------------------------------------------------------------
    // Animations

    fn convert_animations(&mut self) -> Result<(), ImportError> {
        let g = &self.doc.globals;
        let mut tm = g.get_or::<i32>("TimeMode", 0)?;
        if !(0..15).contains(&tm) {
            tm = 0;
        }
        let custom = g.get_or::<f32>("CustomFrameRate", -1.0)?;
        self.anim_fps = match tm {
            0 => 1.0,
            1 => 120.0,
            2 => 100.0,
            3 => 60.0,
            4 => 50.0,
            5 => 48.0,
            6 | 7 => 30.0,
            8 | 9 => 29.9700262,
            10 => 25.0,
            11 => 24.0,
            12 => 1000.0,
            13 => 23.976,
            _ => custom as f64,
        };
        if !self.doc.settings.read_animations {
            return Ok(());
        }
        for st in self.doc.animation_stacks()? {
            self.convert_stack(&st)?;
        }
        Ok(())
    }

    fn convert_stack(&mut self, obj: &Rc<Object>) -> Result<(), ImportError> {
        let ObjectKind::AnimationStack(st) = &obj.kind else { return Ok(()) };
        if st.layers.is_empty() {
            return Ok(());
        }
        let mut name = st.name.clone();
        if let Some(s) = name.strip_prefix("AnimationStack::") {
            name = s.to_string();
        } else if let Some(s) = name.strip_prefix("AnimStack::") {
            name = s.to_string();
        }
        let whitelist = ["Lcl Scaling", "Lcl Rotation", "Lcl Translation", "DeformPercent"];
        let mut node_map: BTreeMap<String, Vec<Rc<Object>>> = BTreeMap::new();
        for layer in &st.layers {
            let ObjectKind::AnimationLayer(l) = &layer.kind else { continue };
            for n in self.doc.layer_nodes(l, &whitelist)? {
                let ObjectKind::AnimationCurveNode(cn) = &n.kind else { continue };
                if let Some(t) = &cn.target {
                    if let Some(m) = t.as_model() {
                        node_map.entry(fix_node_name(&m.name)).or_default().push(n.clone());
                    }
                }
            }
        }
        let mut node_anims = Vec::new();
        let (mut min_time, mut max_time) = (1e10f64, -1e10f64);
        let mut start = st.props.get_or::<i64>("LocalStart", 0)?;
        let mut stop = st.props.get_or::<i64>("LocalStop", 0)?;
        let has_ss = start != 0 || stop != 0;
        if !has_ss {
            start = -9223372036854775807i64 + 20000;
            stop = 9223372036854775807i64 - 20000;
        }
        for (k, v) in &node_map {
            self.generate_node_animations(&mut node_anims, k, v, start, stop, &mut max_time, &mut min_time)?;
        }
        if node_anims.is_empty() {
            return Ok(());
        }
        let start_fps = if has_ss { start as f64 / FBX_TIME * self.anim_fps } else { min_time };
        let stop_fps = if has_ss { stop as f64 / FBX_TIME * self.anim_fps } else { max_time };
        for ch in node_anims.iter_mut() {
            for k in ch.position_keys.iter_mut() {
                k.time -= start_fps;
            }
            for k in ch.rotation_keys.iter_mut() {
                k.time -= start_fps;
            }
            for k in ch.scaling_keys.iter_mut() {
                k.time -= start_fps;
            }
        }
        self.out.animations.push(AiAnimation {
            name,
            duration: stop_fps - start_fps,
            ticks_per_second: self.anim_fps,
            channels: node_anims,
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn generate_node_animations(
        &mut self,
        out: &mut Vec<AiNodeAnim>,
        fixed_name: &str,
        curves: &[Rc<Object>],
        start: i64,
        stop: i64,
        max_time: &mut f64,
        min_time: &mut f64,
    ) -> Result<(), ImportError> {
        let mut prop_map: BTreeMap<String, Vec<Rc<Object>>> = BTreeMap::new();
        let mut curve_node: Option<Rc<Object>> = None;
        for n in curves {
            let ObjectKind::AnimationCurveNode(cn) = &n.kind else { continue };
            if cn.prop.is_empty() {
                continue;
            }
            curve_node = Some(n.clone());
            if self.doc.curves(cn)?.is_empty() {
                continue;
            }
            prop_map.entry(cn.prop.clone()).or_default().push(n.clone());
        }
        let Some(cnode) = curve_node else { return Ok(()) };
        let ObjectKind::AnimationCurveNode(cn) = &cnode.kind else { return Ok(()) };
        let Some(target_obj) = cn.target.clone() else { return Ok(()) };
        let Some(target) = target_obj.as_model() else { return Ok(()) };
        let mut chain: [Option<&Vec<Rc<Object>>>; COMP_MAX] = [None; COMP_MAX];
        let mut has_any = false;
        let mut has_complex = false;
        for (i, &c) in ALL_COMPS.iter().enumerate() {
            if c == Comp::RotationPivotInverse || c == Comp::ScalingPivotInverse {
                continue;
            }
            chain[i] = prop_map.get(comp_property(c));
            if let Some(nodes) = chain[i] {
                if self.doc.settings.optimize_empty_animation_curves && self.is_redundant(target, c, nodes)? {
                    // assimp `continue`s here but keeps chain[i] set.
                    continue;
                }
                has_any = true;
                if c != Comp::Rotation && c != Comp::Scaling && c != Comp::Translation {
                    has_complex = true;
                }
            }
        }
        if !has_any {
            return Ok(());
        }
        if !has_complex && !self.needs_complex_chain(target)? {
            let nd = self.simple_node_anim(fixed_name, target, &chain, start, stop, max_time, min_time)?;
            if !(nd.position_keys.is_empty() && nd.rotation_keys.is_empty() && nd.scaling_keys.is_empty()) {
                out.push(nd);
            }
            return Ok(());
        }
        let mut flags = 0u32;
        for (i, &c) in ALL_COMPS.iter().enumerate() {
            let Some(nodes) = chain[i] else { continue };
            let bit = 1u32 << i;
            flags |= bit;
            let cname = chain_node_name(fixed_name, c);
            let na = match c {
                Comp::Rotation | Comp::PreRotation | Comp::PostRotation | Comp::GeometricRotation => {
                    let mut na = AiNodeAnim { node_name: cname, ..Default::default() };
                    let inputs = self.rotation_keyframe_list(nodes, start, stop)?;
                    let keys = key_time_list(&inputs);
                    na.rotation_keys = self.interpolate_quat(&keys, &inputs, [0.0; 3], max_time, min_time, rotation_order(target)?);
                    na.scaling_keys = vec![AiVectorKey { time: 0.0, value: [1.0; 3] }];
                    na.position_keys = vec![AiVectorKey { time: 0.0, value: [0.0; 3] }];
                    na
                }
                Comp::Scaling | Comp::GeometricScaling => {
                    let mut na = AiNodeAnim { node_name: cname, ..Default::default() };
                    let inputs = self.keyframe_list(nodes, start, stop)?;
                    let keys = key_time_list(&inputs);
                    na.scaling_keys = self.interpolate_vec(&keys, &inputs, [1.0; 3], max_time, min_time);
                    na.rotation_keys = vec![AiQuatKey { time: 0.0, value: AiQuat::default() }];
                    na.position_keys = vec![AiVectorKey { time: 0.0, value: [0.0; 3] }];
                    na
                }
                _ => {
                    let na = self.translation_anim(&cname, nodes, start, stop, max_time, min_time, false)?;
                    if c == Comp::RotationPivot || c == Comp::ScalingPivot {
                        let inv_c = if c == Comp::RotationPivot { Comp::RotationPivotInverse } else { Comp::ScalingPivotInverse };
                        let inv = self.translation_anim(&chain_node_name(fixed_name, inv_c), nodes, start, stop, max_time, min_time, true)?;
                        if !(inv.position_keys.is_empty() && inv.rotation_keys.is_empty() && inv.scaling_keys.is_empty()) {
                            out.push(inv);
                        }
                        flags |= bit << (Comp::RotationPivotInverse as usize - i);
                    }
                    na
                }
            };
            if !(na.position_keys.is_empty() && na.rotation_keys.is_empty() && na.scaling_keys.is_empty()) {
                out.push(na);
            }
        }
        self.node_anim_chain_bits.insert(fixed_name.to_string(), flags);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn translation_anim(
        &mut self,
        name: &str,
        nodes: &[Rc<Object>],
        start: i64,
        stop: i64,
        max_time: &mut f64,
        min_time: &mut f64,
        inverse: bool,
    ) -> Result<AiNodeAnim, ImportError> {
        let mut na = AiNodeAnim { node_name: name.to_string(), ..Default::default() };
        let inputs = self.keyframe_list(nodes, start, stop)?;
        let keys = key_time_list(&inputs);
        na.position_keys = self.interpolate_vec(&keys, &inputs, [0.0; 3], max_time, min_time);
        if inverse {
            for k in na.position_keys.iter_mut() {
                k.value = [k.value[0] * -1.0, k.value[1] * -1.0, k.value[2] * -1.0];
            }
        }
        na.scaling_keys = vec![AiVectorKey { time: 0.0, value: [1.0; 3] }];
        na.rotation_keys = vec![AiQuatKey { time: 0.0, value: AiQuat::default() }];
        Ok(na)
    }

    fn is_redundant(&self, target: &Model, c: Comp, nodes: &[Rc<Object>]) -> Result<bool, ImportError> {
        if nodes.len() > 1 {
            return Ok(false);
        }
        let ObjectKind::AnimationCurveNode(cn) = &nodes[0].kind else { return Ok(false) };
        let sub = self.doc.curves(cn)?;
        let get = |k: &str| -> Option<Vec<f32>> {
            sub.get(k).and_then(|o| if let ObjectKind::AnimationCurve(c) = &o.kind { Some(c.values.clone()) } else { None })
        };
        let (Some(vx), Some(vy), Some(vz)) = (get("d|X"), get("d|Y"), get("d|Z")) else { return Ok(false) };
        if vx.len() != 1 || vy.len() != 1 || vz.len() != 1 {
            return Ok(false);
        }
        let def = if c == Comp::Scaling { [1.0; 3] } else { [0.0; 3] };
        let sv = target.props.get_or::<[f32; 3]>(comp_property(c), def)?;
        let d = [vx[0] - sv[0], vy[0] - sv[1], vz[0] - sv[2]];
        Ok(sq(d) < f32::EPSILON)
    }

    fn needs_complex_chain(&self, model: &Model) -> Result<bool, ImportError> {
        for &c in ALL_COMPS.iter() {
            if matches!(c, Comp::Rotation | Comp::Scaling | Comp::Translation | Comp::PreRotation | Comp::PostRotation) {
                continue;
            }
            let scale = c == Comp::GeometricScaling || c == Comp::Scaling;
            if let Some(v) = model.props.get_ok::<[f32; 3]>(comp_property(c), false)? {
                if scale {
                    if sq(sub1(v)) > f32::EPSILON {
                        return Ok(true);
                    }
                } else if sq(v) > f32::EPSILON {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn keyframe_list(&self, nodes: &[Rc<Object>], start: i64, stop: i64) -> Result<Vec<KeyFrameList>, ImportError> {
        let (adj_start, adj_stop) = (start - 10000, stop + 10000);
        let mut inputs = Vec::new();
        for n in nodes {
            let ObjectKind::AnimationCurveNode(cn) = &n.kind else { continue };
            for (k, c) in self.doc.curves(cn)? {
                let mapto = match k.as_str() {
                    "d|X" => 0,
                    "d|Y" => 1,
                    "d|Z" => 2,
                    _ => continue,
                };
                let ObjectKind::AnimationCurve(curve) = &c.kind else { continue };
                let (mut ks, mut vs) = (Vec::new(), Vec::new());
                for (t, v) in curve.keys.iter().zip(curve.values.iter()) {
                    if *t >= adj_start && *t <= adj_stop {
                        ks.push(*t);
                        vs.push(*v);
                    }
                }
                inputs.push((ks, vs, mapto));
            }
        }
        Ok(inputs)
    }

    fn rotation_keyframe_list(&self, nodes: &[Rc<Object>], start: i64, stop: i64) -> Result<Vec<KeyFrameList>, ImportError> {
        let (adj_start, adj_stop) = (start - 10000, stop + 10000);
        let mut inputs = Vec::new();
        for n in nodes {
            let ObjectKind::AnimationCurveNode(cn) = &n.kind else { continue };
            for (k, c) in self.doc.curves(cn)? {
                let mapto = match k.as_str() {
                    "d|X" => 0,
                    "d|Y" => 1,
                    "d|Z" => 2,
                    _ => continue,
                };
                let ObjectKind::AnimationCurve(curve) = &c.kind else { continue };
                let count = curve.keys.len();
                let (mut ks, mut vs) = (Vec::new(), Vec::new());
                let mut tp = curve.keys[0];
                let mut vp = curve.values[0];
                ks.push(tp);
                vs.push(vp);
                if count > 1 {
                    let mut tc = curve.keys[1];
                    let mut vc = curve.values[1];
                    for n in 1..count {
                        while (vc - vp).abs() >= 180.0 {
                            let step = ((tc - tp) as f64 / (vc - vp).abs() as f64 * 179.0f32 as f64).floor();
                            let tnew = tp + step as i64;
                            let vnew = vp + (vc - vp) * (step / (tc - tp) as f64) as f32;
                            if tnew >= adj_start && tnew <= adj_stop {
                                ks.push(tnew);
                                vs.push(vnew);
                            } else {
                                break;
                            }
                            tp = tnew;
                            vp = vnew;
                        }
                        if tc >= adj_start && tc <= adj_stop {
                            ks.push(tc);
                            vs.push(vc);
                        }
                        if n + 1 < count {
                            tp = tc;
                            vp = vc;
                            tc = curve.keys[n + 1];
                            vc = curve.values[n + 1];
                        }
                    }
                }
                inputs.push((ks, vs, mapto));
            }
        }
        Ok(inputs)
    }

    fn interpolate_vec(
        &self,
        keys: &[i64],
        inputs: &[KeyFrameList],
        def: [f32; 3],
        max_time: &mut f64,
        min_time: &mut f64,
    ) -> Vec<AiVectorKey> {
        let mut next = vec![0usize; inputs.len()];
        let mut out = Vec::with_capacity(keys.len());
        for &time in keys {
            let mut result = def;
            for (i, (ks, vs, mapto)) in inputs.iter().enumerate() {
                let ksize = ks.len();
                if ksize == 0 {
                    continue;
                }
                if ksize > next[i] && ks[next[i]] == time {
                    next[i] += 1;
                }
                let id0 = if next[i] > 0 { next[i] - 1 } else { 0 };
                let id1 = if next[i] == ksize { ksize - 1 } else { next[i] };
                let (va, vb) = (vs[id0], vs[id1]);
                let (ta, tb) = (ks[id0], ks[id1]);
                let factor = if tb == ta { 0.0f32 } else { (time - ta) as f32 / (tb - ta) as f32 };
                result[*mapto] = va + (vb - va) * factor;
            }
            let t = time as f64 / FBX_TIME * self.anim_fps;
            *min_time = min_time.min(t);
            *max_time = max_time.max(t);
            out.push(AiVectorKey { time: t, value: result });
        }
        out
    }

    fn interpolate_quat(
        &self,
        keys: &[i64],
        inputs: &[KeyFrameList],
        def: [f32; 3],
        max_time: &mut f64,
        min_time: &mut f64,
        order: i32,
    ) -> Vec<AiQuatKey> {
        let temp = self.interpolate_vec(keys, inputs, def, max_time, min_time);
        let mut lastq = AiQuat::default();
        temp.iter()
            .map(|k| {
                let mut q = AiQuat::from_matrix(&m3(&rotation_matrix(order, k.value)));
                if q.x * lastq.x + q.y * lastq.y + q.z * lastq.z + q.w * lastq.w < 0.0 {
                    q = AiQuat { w: -q.w, x: -q.x, y: -q.y, z: -q.z };
                }
                lastq = q;
                AiQuatKey { time: k.time, value: q }
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn simple_node_anim(
        &self,
        name: &str,
        target: &Model,
        chain: &[Option<&Vec<Rc<Object>>>; COMP_MAX],
        start: i64,
        stop: i64,
        max_time: &mut f64,
        min_time: &mut f64,
    ) -> Result<AiNodeAnim, ImportError> {
        let props = &target.props;
        let mut lists: Vec<Vec<KeyFrameList>> = vec![Vec::new(); COMP_MAX];
        let mut keytimes: Vec<i64> = Vec::new();
        for (i, &c) in ALL_COMPS.iter().enumerate() {
            let Some(nodes) = chain[i] else { continue };
            lists[i] = if matches!(c, Comp::Rotation | Comp::PreRotation | Comp::PostRotation | Comp::GeometricRotation) {
                self.rotation_keyframe_list(nodes, start, stop)?
            } else {
                self.keyframe_list(nodes, start, stop)?
            };
            for l in &lists[i] {
                keytimes.extend_from_slice(&l.0);
            }
            keytimes.sort();
            keytimes.dedup();
        }
        let rot_order = rotation_order(target)?;
        let def_t = props.get_or::<[f32; 3]>("Lcl Translation", [0.0; 3])?;
        let def_r = props.get_or::<[f32; 3]>("Lcl Rotation", [0.0; 3])?;
        let def_s = props.get_or::<[f32; 3]>("Lcl Scaling", [1.0; 3])?;
        let def_q = euler_to_quat(def_r, rot_order);
        let conv = |t: i64| t as f64 / FBX_TIME * self.anim_fps;
        let tr = Comp::Translation as usize;
        let ro = Comp::Rotation as usize;
        let sc = Comp::Scaling as usize;
        let mut outt = if !lists[tr].is_empty() {
            self.interpolate_vec(&keytimes, &lists[tr], def_t, max_time, min_time)
        } else {
            keytimes.iter().map(|&t| AiVectorKey { time: conv(t), value: def_t }).collect()
        };
        let mut outr = if !lists[ro].is_empty() {
            self.interpolate_quat(&keytimes, &lists[ro], def_r, max_time, min_time, rot_order)
        } else {
            keytimes.iter().map(|&t| AiQuatKey { time: conv(t), value: def_q }).collect()
        };
        let mut outs = if !lists[sc].is_empty() {
            self.interpolate_vec(&keytimes, &lists[sc], def_s, max_time, min_time)
        } else {
            keytimes.iter().map(|&t| AiVectorKey { time: conv(t), value: def_s }).collect()
        };
        if let Some(pre) = props.get_ok::<[f32; 3]>("PreRotation", false)? {
            if sq(pre) > 1e-6 {
                let pq = euler_to_quat(pre, 0);
                for k in outr.iter_mut() {
                    k.value = pq.mul(&k.value);
                }
            }
        }
        if let Some(post) = props.get_ok::<[f32; 3]>("PostRotation", false)? {
            if sq(post) > 1e-6 {
                let pq = euler_to_quat(post, 0);
                for k in outr.iter_mut() {
                    k.value = k.value.mul(&pq);
                }
            }
        }
        for i in 0..keytimes.len() {
            let mut m = AiMat4::translation(outt[i].value);
            m = m.mul(&AiMat4::from_3x3(&outr[i].value.get_matrix()));
            m = m.mul(&AiMat4::scaling(outs[i].value));
            let (s, r, t) = m.decompose();
            outs[i].value = s;
            outr[i].value = r;
            outt[i].value = t;
        }
        Ok(AiNodeAnim { node_name: name.to_string(), position_keys: outt, rotation_keys: outr, scaling_keys: outs })
    }
}

fn uv_trafo(t: &Texture) -> [f32; 5] {
    [t.uv_trans[0], t.uv_trans[1], t.uv_scaling[0], t.uv_scaling[1], t.uv_rotation]
}

fn key_time_list(inputs: &[KeyFrameList]) -> Vec<i64> {
    let mut keys = Vec::new();
    let mut next = vec![0usize; inputs.len()];
    loop {
        let mut min_tick = i64::MAX;
        for (i, (ks, _, _)) in inputs.iter().enumerate() {
            if ks.len() > next[i] && ks[next[i]] < min_tick {
                min_tick = ks[next[i]];
            }
        }
        if min_tick == i64::MAX {
            break;
        }
        keys.push(min_tick);
        for (i, (ks, _, _)) in inputs.iter().enumerate() {
            while ks.len() > next[i] && ks[next[i]] == min_tick {
                next[i] += 1;
            }
        }
    }
    keys
}
