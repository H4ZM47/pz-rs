//! An in-memory `aiScene`: what assimp hands jassimp after an importer
//! and the post-processing steps ran. Nodes live in an arena so steps can
//! walk parents and children freely.

use super::aimath::{AiMat4, AiQuat};

pub const MAX_UV: usize = 8;
pub const MAX_COLORS: usize = 8;

pub const PT_POINT: u32 = 0x1;
pub const PT_LINE: u32 = 0x2;
pub const PT_TRIANGLE: u32 = 0x4;
pub const PT_POLYGON: u32 = 0x8;
pub const PT_NGON_ENCODING: u32 = 0x10;

#[derive(Clone, Debug, Default)]
pub struct AiNode {
    pub name: String,
    pub transform: AiMat4,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub meshes: Vec<u32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiBone {
    pub name: String,
    /// (vertex id, weight)
    pub weights: Vec<(u32, f32)>,
    pub offset: AiMat4,
}

#[derive(Clone, Debug, Default)]
pub struct AiMesh {
    pub name: String,
    pub vertices: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub tangents: Option<Vec<[f32; 3]>>,
    pub bitangents: Option<Vec<[f32; 3]>>,
    pub colors: [Option<Vec<[f32; 4]>>; MAX_COLORS],
    pub uvs: [Option<Vec<[f32; 3]>>; MAX_UV],
    pub num_uv_components: [u32; MAX_UV],
    pub faces: Vec<Vec<u32>>,
    pub bones: Vec<AiBone>,
    pub material_index: u32,
    pub primitive_types: u32,
}

impl AiMesh {
    pub fn num_vertices(&self) -> usize {
        self.vertices.len()
    }
    pub fn has_normals(&self) -> bool {
        self.normals.is_some() && !self.vertices.is_empty()
    }
    pub fn has_tangents(&self) -> bool {
        self.tangents.is_some() && self.bitangents.is_some() && !self.vertices.is_empty()
    }
    pub fn has_uv(&self, i: usize) -> bool {
        i < MAX_UV && self.uvs[i].is_some() && !self.vertices.is_empty()
    }
    pub fn has_colors(&self, i: usize) -> bool {
        i < MAX_COLORS && self.colors[i].is_some() && !self.vertices.is_empty()
    }
    pub fn num_uv_channels(&self) -> usize {
        let mut n = 0;
        while n < MAX_UV && self.has_uv(n) {
            n += 1;
        }
        n
    }
    pub fn num_color_channels(&self) -> usize {
        let mut n = 0;
        while n < MAX_COLORS && self.has_colors(n) {
            n += 1;
        }
        n
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiVectorKey {
    pub time: f64,
    pub value: [f32; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiQuatKey {
    pub time: f64,
    pub value: AiQuat,
}

#[derive(Clone, Debug, Default)]
pub struct AiNodeAnim {
    pub node_name: String,
    pub position_keys: Vec<AiVectorKey>,
    pub rotation_keys: Vec<AiQuatKey>,
    pub scaling_keys: Vec<AiVectorKey>,
}

#[derive(Clone, Debug, Default)]
pub struct AiAnimation {
    pub name: String,
    pub duration: f64,
    pub ticks_per_second: f64,
    pub channels: Vec<AiNodeAnim>,
}

/// One `aiMaterialProperty`: enough to reproduce the steps that compare
/// materials (`RemoveRedundantMaterials`).
#[derive(Clone, Debug, PartialEq)]
pub struct AiMaterialProperty {
    pub key: String,
    pub semantic: u32,
    pub index: u32,
    pub ty: u32,
    pub data: Vec<u8>,
}

pub const PTI_FLOAT: u32 = 0x1;
pub const PTI_DOUBLE: u32 = 0x2;
pub const PTI_STRING: u32 = 0x3;
pub const PTI_INTEGER: u32 = 0x4;
pub const PTI_BUFFER: u32 = 0x5;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiMaterial {
    pub properties: Vec<AiMaterialProperty>,
}

impl AiMaterial {
    /// `aiMaterial::AddBinaryProperty`: replaces a property with the same
    /// key/semantic/index in place, else appends.
    pub fn add(&mut self, key: &str, semantic: u32, index: u32, ty: u32, data: Vec<u8>) {
        let p = AiMaterialProperty {
            key: key.to_string(),
            semantic,
            index,
            ty,
            data,
        };
        if let Some(e) = self
            .properties
            .iter_mut()
            .find(|e| e.key == key && e.semantic == semantic && e.index == index)
        {
            *e = p;
        } else {
            self.properties.push(p);
        }
    }

    pub fn add_floats(&mut self, key: &str, semantic: u32, index: u32, v: &[f32]) {
        let data = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.add(key, semantic, index, PTI_FLOAT, data);
    }

    pub fn add_ints(&mut self, key: &str, semantic: u32, index: u32, v: &[i32]) {
        let data = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.add(key, semantic, index, PTI_INTEGER, data);
    }

    /// `AddProperty(const aiString*)`: u32 length, bytes, NUL.
    pub fn add_string(&mut self, key: &str, semantic: u32, index: u32, s: &str) {
        let mut data = (s.len() as u32).to_le_bytes().to_vec();
        data.extend_from_slice(s.as_bytes());
        data.push(0);
        self.add(key, semantic, index, PTI_STRING, data);
    }

    pub fn get_string(&self, key: &str, semantic: u32, index: u32) -> Option<String> {
        let p = self.properties.iter().find(|e| {
            e.key == key && e.semantic == semantic && e.index == index && e.ty == PTI_STRING
        })?;
        if p.data.len() < 4 {
            return None;
        }
        let n = u32::from_le_bytes(p.data[0..4].try_into().ok()?) as usize;
        Some(String::from_utf8_lossy(&p.data[4..(4 + n).min(p.data.len())]).into_owned())
    }
}

#[derive(Clone, Debug, Default)]
pub struct AiScene {
    pub nodes: Vec<AiNode>,
    pub root: usize,
    pub meshes: Vec<AiMesh>,
    pub materials: Vec<AiMaterial>,
    pub animations: Vec<AiAnimation>,
}

impl AiScene {
    pub fn add_node(&mut self, name: &str, transform: AiMat4, parent: Option<usize>) -> usize {
        let id = self.nodes.len();
        self.nodes.push(AiNode {
            name: name.to_string(),
            transform,
            parent,
            children: vec![],
            meshes: vec![],
        });
        if let Some(p) = parent {
            self.nodes[p].children.push(id);
        }
        id
    }

    /// `aiNode::FindNode` from the root: the node itself, then children
    /// depth first.
    pub fn find_node(&self, name: &str) -> Option<usize> {
        self.find_node_from(self.root, name)
    }

    pub fn find_node_from(&self, start: usize, name: &str) -> Option<usize> {
        if self.nodes[start].name == name {
            return Some(start);
        }
        for &c in &self.nodes[start].children {
            if let Some(f) = self.find_node_from(c, name) {
                return Some(f);
            }
        }
        None
    }

    /// Nodes reachable from the root in pre-order.
    pub fn preorder(&self) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![self.root];
        while let Some(n) = stack.pop() {
            out.push(n);
            for &c in self.nodes[n].children.iter().rev() {
                stack.push(c);
            }
        }
        out
    }
}
