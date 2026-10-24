//! DirectX `.x` files (`XFileParser` + `XFileImporter` of assimp 5.3):
//! text and binary encodings (the game ships text only). Produces the
//! scene the importer hands to assimp's post-processing, including the
//! importer's own left-handed conversion and winding flip.

use super::aimath::{AiMat4, AiQuat};
use super::scene::*;
use super::{fast_atoreal, ImportError};
use crate::import::postprocess::{flip_winding_order, make_left_handed};

#[derive(Default)]
struct XFace {
    indices: Vec<u32>,
}

#[derive(Default)]
struct XBone {
    name: String,
    weights: Vec<(u32, f32)>,
    offset: AiMat4,
}

#[derive(Default, Clone)]
struct XTex {
    name: String,
    is_normal_map: bool,
}

#[derive(Default, Clone)]
struct XMaterial {
    name: String,
    is_reference: bool,
    diffuse: [f32; 4],
    specular_exponent: f32,
    specular: [f32; 3],
    emissive: [f32; 3],
    textures: Vec<XTex>,
    scene_index: Option<usize>,
}

#[derive(Default)]
struct XMesh {
    name: String,
    positions: Vec<[f32; 3]>,
    pos_faces: Vec<XFace>,
    normals: Vec<[f32; 3]>,
    norm_faces: Vec<XFace>,
    num_textures: usize,
    tex_coords: [Vec<[f32; 2]>; MAX_UV],
    num_color_sets: usize,
    colors: [Vec<[f32; 4]>; MAX_COLORS],
    face_materials: Vec<u32>,
    materials: Vec<XMaterial>,
    bones: Vec<XBone>,
}

#[derive(Default)]
struct XNode {
    name: String,
    trafo: AiMat4,
    parent: Option<usize>,
    children: Vec<usize>,
    meshes: Vec<usize>,
}

#[derive(Default)]
struct XAnimBone {
    bone_name: String,
    pos_keys: Vec<AiVectorKey>,
    rot_keys: Vec<AiQuatKey>,
    scale_keys: Vec<AiVectorKey>,
    trafo_keys: Vec<(f64, AiMat4)>,
}

#[derive(Default)]
struct XAnimation {
    name: String,
    anims: Vec<XAnimBone>,
}

#[derive(Default)]
struct XScene {
    nodes: Vec<XNode>,
    root: Option<usize>,
    meshes: Vec<XMesh>,
    global_meshes: Vec<usize>,
    global_materials: Vec<XMaterial>,
    anims: Vec<XAnimation>,
    anim_ticks_per_second: u32,
}

struct Parser<'a> {
    b: &'a [u8],
    p: usize,
    end: usize,
    binary: bool,
    bin_float_size: usize,
    bin_num_count: u32,
    line: u32,
    scene: XScene,
}

fn err<T>(msg: impl Into<String>) -> Result<T, ImportError> {
    Err(ImportError(format!("X: {}", msg.into())))
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

impl<'a> Parser<'a> {
    fn at(&self, i: usize) -> u8 {
        self.b.get(i).copied().unwrap_or(0)
    }

    fn read_until_end_of_line(&mut self) {
        if self.binary {
            return;
        }
        while self.p < self.end {
            let c = self.b[self.p];
            if c == b'\n' || c == b'\r' {
                self.p += 1;
                self.line += 1;
                return;
            }
            self.p += 1;
        }
    }

    fn find_next_non_whitespace(&mut self) {
        if self.binary {
            return;
        }
        loop {
            while self.p < self.end && is_space(self.b[self.p]) {
                if self.b[self.p] == b'\n' {
                    self.line += 1;
                }
                self.p += 1;
            }
            if self.p >= self.end {
                return;
            }
            if (self.at(self.p) == b'/' && self.at(self.p + 1) == b'/') || self.at(self.p) == b'#' {
                self.read_until_end_of_line();
            } else {
                break;
            }
        }
    }

    fn read_bin_word(&mut self) -> u32 {
        let v = self.at(self.p) as u32 | (self.at(self.p + 1) as u32) << 8;
        self.p += 2;
        v
    }

    fn read_bin_dword(&mut self) -> u32 {
        let v = self.at(self.p) as u32
            | (self.at(self.p + 1) as u32) << 8
            | (self.at(self.p + 2) as u32) << 16
            | (self.at(self.p + 3) as u32) << 24;
        self.p += 4;
        v
    }

    fn remaining(&self) -> isize {
        self.end as isize - self.p as isize
    }

    fn next_token(&mut self) -> String {
        if self.binary {
            if self.remaining() < 2 {
                return String::new();
            }
            let tok = self.read_bin_word();
            return match tok {
                1 => {
                    if self.remaining() < 4 {
                        return String::new();
                    }
                    let len = self.read_bin_dword() as isize;
                    if len < 0 || self.remaining() < len {
                        return String::new();
                    }
                    let s = String::from_utf8_lossy(&self.b[self.p..self.p + len as usize]).into_owned();
                    self.p += len as usize;
                    s
                }
                2 => {
                    if self.remaining() < 4 {
                        return String::new();
                    }
                    let len = self.read_bin_dword() as isize;
                    if self.remaining() < len {
                        return String::new();
                    }
                    let s = String::from_utf8_lossy(&self.b[self.p..self.p + len as usize]).into_owned();
                    self.p += len as usize + 2;
                    s
                }
                3 => {
                    self.p += 4;
                    "<integer>".into()
                }
                5 => {
                    self.p += 16;
                    "<guid>".into()
                }
                6 => {
                    if self.remaining() < 4 {
                        return String::new();
                    }
                    let len = self.read_bin_dword() as usize;
                    self.p += len * 4;
                    "<int_list>".into()
                }
                7 => {
                    if self.remaining() < 4 {
                        return String::new();
                    }
                    let len = self.read_bin_dword() as usize;
                    self.p += len * self.bin_float_size;
                    "<flt_list>".into()
                }
                0x0a => "{".into(),
                0x0b => "}".into(),
                0x0c => "(".into(),
                0x0d => ")".into(),
                0x0e => "[".into(),
                0x0f => "]".into(),
                0x10 => "<".into(),
                0x11 => ">".into(),
                0x12 => ".".into(),
                0x13 => ",".into(),
                0x14 => ";".into(),
                0x1f => "template".into(),
                0x28 => "WORD".into(),
                0x29 => "DWORD".into(),
                0x2a => "FLOAT".into(),
                0x2b => "DOUBLE".into(),
                0x2c => "CHAR".into(),
                0x2d => "UCHAR".into(),
                0x2e => "SWORD".into(),
                0x2f => "SDWORD".into(),
                0x30 => "void".into(),
                0x31 => "string".into(),
                0x32 => "unicode".into(),
                0x33 => "cstring".into(),
                0x34 => "array".into(),
                _ => String::new(),
            };
        }
        self.find_next_non_whitespace();
        let mut s = Vec::new();
        if self.p >= self.end {
            return String::new();
        }
        while self.p < self.end && !is_space(self.b[self.p]) {
            let c = self.b[self.p];
            if c == b';' || c == b'}' || c == b'{' || c == b',' {
                if s.is_empty() {
                    s.push(c);
                    self.p += 1;
                }
                break;
            }
            s.push(c);
            self.p += 1;
        }
        String::from_utf8_lossy(&s).into_owned()
    }

    fn next_token_as_string(&mut self) -> Result<String, ImportError> {
        if self.binary {
            return Ok(self.next_token());
        }
        self.find_next_non_whitespace();
        if self.p >= self.end {
            return err("Unexpected end of file while parsing string");
        }
        if self.b[self.p] != b'"' {
            return err("Expected quotation mark.");
        }
        self.p += 1;
        let start = self.p;
        while self.p < self.end && self.b[self.p] != b'"' {
            self.p += 1;
        }
        let s = String::from_utf8_lossy(&self.b[start..self.p]).into_owned();
        if self.p + 1 >= self.end {
            return err("Unexpected end of file while parsing string");
        }
        if self.at(self.p + 1) != b';' || self.at(self.p) != b'"' {
            return err("Expected quotation mark and semicolon at the end of a string.");
        }
        self.p += 2;
        Ok(s)
    }

    fn check_for_closing_brace(&mut self) -> Result<(), ImportError> {
        if self.next_token() != "}" {
            return err("Closing brace expected.");
        }
        Ok(())
    }

    fn check_for_semicolon(&mut self) -> Result<(), ImportError> {
        if self.binary {
            return Ok(());
        }
        if self.next_token() != ";" {
            return err("Semicolon expected.");
        }
        Ok(())
    }

    fn check_for_separator(&mut self) -> Result<(), ImportError> {
        if self.binary {
            return Ok(());
        }
        let t = self.next_token();
        if t != "," && t != ";" {
            return err("Separator character (';' or ',') expected.");
        }
        Ok(())
    }

    fn test_for_separator(&mut self) {
        if self.binary {
            return;
        }
        self.find_next_non_whitespace();
        if self.p >= self.end {
            return;
        }
        let c = self.b[self.p];
        if c == b';' || c == b',' {
            self.p += 1;
        }
    }

    fn read_head_of_data_object(&mut self) -> Result<String, ImportError> {
        let t = self.next_token();
        let mut name = String::new();
        if t != "{" {
            name = t;
            if self.next_token() != "{" {
                return err("Opening brace expected.");
            }
        }
        Ok(name)
    }

    fn read_int(&mut self) -> Result<u32, ImportError> {
        if self.binary {
            if self.bin_num_count == 0 && self.remaining() >= 2 {
                let tmp = self.read_bin_word();
                if tmp == 0x06 && self.remaining() >= 4 {
                    self.bin_num_count = self.read_bin_dword();
                } else {
                    self.bin_num_count = 1;
                }
            }
            self.bin_num_count = self.bin_num_count.wrapping_sub(1);
            if self.remaining() >= 4 {
                return Ok(self.read_bin_dword());
            }
            self.p = self.end;
            return Ok(0);
        }
        self.find_next_non_whitespace();
        let mut neg = false;
        if self.at(self.p) == b'-' {
            neg = true;
            self.p += 1;
        }
        if !self.at(self.p).is_ascii_digit() {
            return err("Number expected.");
        }
        let mut n: u32 = 0;
        while self.p < self.end {
            let c = self.b[self.p];
            if !c.is_ascii_digit() {
                break;
            }
            n = n.wrapping_mul(10).wrapping_add((c - b'0') as u32);
            self.p += 1;
        }
        self.check_for_separator()?;
        Ok(if neg { (n as i32).wrapping_neg() as u32 } else { n })
    }

    fn read_float(&mut self) -> Result<f32, ImportError> {
        if self.binary {
            if self.bin_num_count == 0 && self.remaining() >= 2 {
                let tmp = self.read_bin_word();
                if tmp == 0x07 && self.remaining() >= 4 {
                    self.bin_num_count = self.read_bin_dword();
                } else {
                    self.bin_num_count = 1;
                }
            }
            self.bin_num_count = self.bin_num_count.wrapping_sub(1);
            if self.bin_float_size == 8 {
                if self.remaining() >= 8 {
                    let v = f64::from_le_bytes(self.b[self.p..self.p + 8].try_into().unwrap());
                    self.p += 8;
                    return Ok(v as f32);
                }
            } else if self.remaining() >= 4 {
                let v = f32::from_le_bytes(self.b[self.p..self.p + 4].try_into().unwrap());
                self.p += 4;
                return Ok(v);
            }
            self.p = self.end;
            return Ok(0.0);
        }
        self.find_next_non_whitespace();
        let rest = &self.b[self.p..];
        if rest.starts_with(b"-1.#IND00") || rest.starts_with(b"1.#IND00") {
            self.p += 9;
            self.check_for_separator()?;
            return Ok(0.0);
        } else if rest.starts_with(b"1.#QNAN0") {
            self.p += 8;
            self.check_for_separator()?;
            return Ok(0.0);
        }
        let (v, n) = fast_atoreal(rest, true)?;
        self.p += n;
        self.check_for_separator()?;
        Ok(v)
    }

    fn read_vector2(&mut self) -> Result<[f32; 2], ImportError> {
        let v = [self.read_float()?, self.read_float()?];
        self.test_for_separator();
        Ok(v)
    }

    fn read_vector3(&mut self) -> Result<[f32; 3], ImportError> {
        let v = [self.read_float()?, self.read_float()?, self.read_float()?];
        self.test_for_separator();
        Ok(v)
    }

    fn read_rgba(&mut self) -> Result<[f32; 4], ImportError> {
        let v = [self.read_float()?, self.read_float()?, self.read_float()?, self.read_float()?];
        self.test_for_separator();
        Ok(v)
    }

    fn read_rgb(&mut self) -> Result<[f32; 3], ImportError> {
        let v = [self.read_float()?, self.read_float()?, self.read_float()?];
        self.test_for_separator();
        Ok(v)
    }

    /// 16 floats in file order a1 b1 c1 d1 a2 ... (column by column).
    fn read_matrix(&mut self) -> Result<AiMat4, ImportError> {
        let mut m = AiMat4::IDENTITY;
        for col in 0..4 {
            for row in 0..4 {
                m.m[row][col] = self.read_float()?;
            }
        }
        Ok(m)
    }

    fn parse_file(&mut self) -> Result<(), ImportError> {
        loop {
            let name = self.next_token();
            if name.is_empty() {
                break;
            }
            match name.as_str() {
                "template" => self.parse_template()?,
                "Frame" => self.parse_frame(None)?,
                "Mesh" => {
                    let id = self.scene.meshes.len();
                    self.scene.meshes.push(XMesh::default());
                    self.parse_mesh(id)?;
                    self.scene.global_meshes.push(id);
                }
                "AnimTicksPerSecond" => {
                    self.read_head_of_data_object()?;
                    self.scene.anim_ticks_per_second = self.read_int()?;
                    self.check_for_closing_brace()?;
                }
                "AnimationSet" => self.parse_animation_set()?,
                "Material" => {
                    let m = self.parse_material()?;
                    self.scene.global_materials.push(m);
                }
                "}" => {}
                _ => self.parse_unknown()?,
            }
        }
        Ok(())
    }

    fn parse_template(&mut self) -> Result<(), ImportError> {
        self.read_head_of_data_object()?;
        self.next_token();
        loop {
            let s = self.next_token();
            if s == "}" {
                break;
            }
            if s.is_empty() {
                return err("Unexpected end of file reached while parsing template definition");
            }
        }
        Ok(())
    }

    fn parse_frame(&mut self, parent: Option<usize>) -> Result<(), ImportError> {
        let name = self.read_head_of_data_object()?;
        let id = self.scene.nodes.len();
        self.scene.nodes.push(XNode { name: name.clone(), parent, ..Default::default() });
        if let Some(p) = parent {
            self.scene.nodes[p].children.push(id);
        } else if let Some(root) = self.scene.root {
            if self.scene.nodes[root].name != "$dummy_root" {
                let dummy = self.scene.nodes.len();
                self.scene.nodes.push(XNode { name: "$dummy_root".into(), ..Default::default() });
                self.scene.nodes[dummy].children.push(root);
                self.scene.nodes[root].parent = Some(dummy);
                self.scene.root = Some(dummy);
            }
            let r = self.scene.root.unwrap();
            self.scene.nodes[r].children.push(id);
            self.scene.nodes[id].parent = Some(r);
        } else {
            self.scene.root = Some(id);
        }
        loop {
            let obj = self.next_token();
            if obj.is_empty() {
                return err("Unexpected end of file reached while parsing frame");
            }
            match obj.as_str() {
                "}" => break,
                "Frame" => self.parse_frame(Some(id))?,
                "FrameTransformMatrix" => {
                    self.read_head_of_data_object()?;
                    let m = self.read_matrix()?;
                    self.check_for_semicolon()?;
                    self.check_for_closing_brace()?;
                    self.scene.nodes[id].trafo = m;
                }
                "Mesh" => {
                    let mid = self.scene.meshes.len();
                    self.scene.meshes.push(XMesh { name: name.clone(), ..Default::default() });
                    self.scene.nodes[id].meshes.push(mid);
                    self.parse_mesh(mid)?;
                }
                _ => self.parse_unknown()?,
            }
        }
        Ok(())
    }

    fn parse_mesh(&mut self, mid: usize) -> Result<(), ImportError> {
        // The object name is read but not stored; the mesh keeps the
        // frame's name.
        self.read_head_of_data_object()?;
        let num_vertices = self.read_int()?;
        let mut positions = Vec::with_capacity(num_vertices as usize);
        for _ in 0..num_vertices {
            positions.push(self.read_vector3()?);
        }
        let num_faces = self.read_int()?;
        let mut faces = Vec::with_capacity(num_faces as usize);
        for _ in 0..num_faces {
            let n = self.read_int()?;
            let mut f = XFace::default();
            for _ in 0..n {
                let idx = self.read_int()?;
                if idx <= num_vertices {
                    f.indices.push(idx);
                }
            }
            self.test_for_separator();
            faces.push(f);
        }
        self.scene.meshes[mid].positions = positions;
        self.scene.meshes[mid].pos_faces = faces;
        loop {
            let obj = self.next_token();
            match obj.as_str() {
                "" => return err("Unexpected end of file while parsing mesh structure"),
                "}" => break,
                "MeshNormals" => self.parse_mesh_normals(mid)?,
                "MeshTextureCoords" => self.parse_mesh_texcoords(mid)?,
                "MeshVertexColors" => self.parse_mesh_vertex_colors(mid)?,
                "MeshMaterialList" => self.parse_mesh_material_list(mid)?,
                "VertexDuplicationIndices" => self.parse_unknown()?,
                "XSkinMeshHeader" => {
                    self.read_head_of_data_object()?;
                    self.read_int()?;
                    self.read_int()?;
                    self.read_int()?;
                    self.check_for_closing_brace()?;
                }
                "SkinWeights" => self.parse_skin_weights(mid)?,
                "DeclData" => self.parse_decl_data(mid)?,
                _ => self.parse_unknown()?,
            }
        }
        Ok(())
    }

    /// `DeclData` (only in the game's assimp build): vertex elements as
    /// DWORDs. Texture coordinates (usage 5) become UV sets; normals
    /// (usage 3) and tangents (usage 6) are read and dropped.
    fn parse_decl_data(&mut self, mid: usize) -> Result<(), ImportError> {
        self.next_token();
        let n_elements = self.read_int()? as i32;
        let mut usages = Vec::new();
        for _ in 0..n_elements.max(0) {
            self.read_int()?;
            self.read_int()?;
            usages.push(self.read_int()?);
            self.read_int()?;
            self.next_token();
        }
        let npos = self.scene.meshes[mid].positions.len();
        let mut sets = Vec::new();
        for &u in &usages {
            if u == 5 {
                let idx = self.scene.meshes[mid].num_textures;
                if idx + 1 > MAX_UV {
                    return err("Too many sets of texture coordinates");
                }
                sets.push(idx);
                self.scene.meshes[mid].num_textures += 1;
                self.scene.meshes[mid].tex_coords[idx].resize(npos, [0.0, 0.0]);
            }
        }
        self.read_int()?;
        if npos > 0 && n_elements >= 1 {
            for v in 0..npos.max(1) {
                let mut k = 0;
                for &u in &usages {
                    match u {
                        3 | 6 => {
                            for _ in 0..3 {
                                self.read_int()?;
                            }
                        }
                        5 => {
                            let a = f32::from_bits(self.read_int()?);
                            let b = f32::from_bits(self.read_int()?);
                            if let Some(&set) = sets.get(k) {
                                self.scene.meshes[mid].tex_coords[set][v] = [a, b];
                            }
                            k += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        self.next_token();
        Ok(())
    }

    fn parse_skin_weights(&mut self, mid: usize) -> Result<(), ImportError> {
        self.read_head_of_data_object()?;
        let name = self.next_token_as_string()?;
        let n = self.read_int()?;
        if n == 0 {
            // The game's assimp build skips the two empty arrays of a
            // bone without weights (`0;` `;` `;`), which stock assimp
            // rejects.
            self.next_token();
            self.next_token();
        }
        let mut weights = Vec::with_capacity(n as usize);
        for _ in 0..n {
            weights.push((self.read_int()?, 0.0f32));
        }
        for w in weights.iter_mut() {
            w.1 = self.read_float()?;
        }
        let offset = self.read_matrix()?;
        self.check_for_semicolon()?;
        self.check_for_closing_brace()?;
        self.scene.meshes[mid].bones.push(XBone { name, weights, offset });
        Ok(())
    }

    fn parse_mesh_normals(&mut self, mid: usize) -> Result<(), ImportError> {
        self.read_head_of_data_object()?;
        let n = self.read_int()?;
        let mut normals = Vec::with_capacity(n as usize);
        for _ in 0..n {
            normals.push(self.read_vector3()?);
        }
        let nf = self.read_int()?;
        if nf as usize != self.scene.meshes[mid].pos_faces.len() {
            return err("Normal face count does not match vertex face count.");
        }
        let mut faces = Vec::new();
        for _ in 0..nf {
            let ni = self.read_int()?;
            let mut f = XFace::default();
            for _ in 0..ni {
                f.indices.push(self.read_int()?);
            }
            self.test_for_separator();
            faces.push(f);
        }
        self.check_for_closing_brace()?;
        let m = &mut self.scene.meshes[mid];
        m.normals = normals;
        m.norm_faces = faces;
        Ok(())
    }

    fn parse_mesh_texcoords(&mut self, mid: usize) -> Result<(), ImportError> {
        self.read_head_of_data_object()?;
        if self.scene.meshes[mid].num_textures + 1 > MAX_UV {
            return err("Too many sets of texture coordinates");
        }
        let set = self.scene.meshes[mid].num_textures;
        self.scene.meshes[mid].num_textures += 1;
        let n = self.read_int()?;
        if n as usize != self.scene.meshes[mid].positions.len() {
            return err("Texture coord count does not match vertex count");
        }
        let mut coords = Vec::with_capacity(n as usize);
        for _ in 0..n {
            coords.push(self.read_vector2()?);
        }
        self.check_for_closing_brace()?;
        self.scene.meshes[mid].tex_coords[set] = coords;
        Ok(())
    }

    fn parse_mesh_vertex_colors(&mut self, mid: usize) -> Result<(), ImportError> {
        self.read_head_of_data_object()?;
        if self.scene.meshes[mid].num_color_sets + 1 > MAX_COLORS {
            return err("Too many colorsets");
        }
        let set = self.scene.meshes[mid].num_color_sets;
        self.scene.meshes[mid].num_color_sets += 1;
        let n = self.read_int()?;
        let npos = self.scene.meshes[mid].positions.len();
        if n as usize != npos {
            return err("Vertex color count does not match vertex count");
        }
        let mut colors = vec![[0.0, 0.0, 0.0, 1.0]; n as usize];
        for _ in 0..n {
            let index = self.read_int()? as usize;
            if index >= npos {
                return err("Vertex color index out of bounds");
            }
            colors[index] = self.read_rgba()?;
            if !self.binary {
                self.find_next_non_whitespace();
                let c = self.at(self.p);
                if c == b';' || c == b',' {
                    self.p += 1;
                }
            }
        }
        self.check_for_closing_brace()?;
        self.scene.meshes[mid].colors[set] = colors;
        Ok(())
    }

    fn parse_mesh_material_list(&mut self, mid: usize) -> Result<(), ImportError> {
        self.read_head_of_data_object()?;
        self.read_int()?;
        let n = self.read_int()?;
        let nfaces = self.scene.meshes[mid].pos_faces.len();
        if n as usize != nfaces && n != 1 {
            return err("Per-Face material index count does not match face count.");
        }
        for _ in 0..n {
            let v = self.read_int()?;
            self.scene.meshes[mid].face_materials.push(v);
        }
        if !self.binary && self.p < self.end && self.b[self.p] == b';' {
            self.p += 1;
        }
        {
            let m = &mut self.scene.meshes[mid];
            while m.face_materials.len() < m.pos_faces.len() {
                let f = m.face_materials[0];
                m.face_materials.push(f);
            }
        }
        loop {
            let obj = self.next_token();
            match obj.as_str() {
                "" => return err("Unexpected end of file while parsing mesh material list."),
                "}" => break,
                "{" => {
                    let name = self.next_token();
                    self.scene.meshes[mid].materials.push(XMaterial {
                        name,
                        is_reference: true,
                        ..Default::default()
                    });
                    self.check_for_closing_brace()?;
                }
                "Material" => {
                    let m = self.parse_material()?;
                    self.scene.meshes[mid].materials.push(m);
                }
                ";" => {}
                _ => self.parse_unknown()?,
            }
        }
        Ok(())
    }

    fn parse_material(&mut self) -> Result<XMaterial, ImportError> {
        let mut name = self.read_head_of_data_object()?;
        if name.is_empty() {
            name = format!("material{}", self.line);
        }
        let mut m = XMaterial { name, ..Default::default() };
        m.diffuse = self.read_rgba()?;
        m.specular_exponent = self.read_float()?;
        m.specular = self.read_rgb()?;
        m.emissive = self.read_rgb()?;
        loop {
            let obj = self.next_token();
            match obj.as_str() {
                "" => return err("Unexpected end of file while parsing mesh material"),
                "}" => break,
                "TextureFilename" | "TextureFileName" => {
                    let t = self.parse_texture_filename()?;
                    m.textures.push(XTex { name: t, is_normal_map: false });
                }
                "NormalmapFilename" | "NormalmapFileName" => {
                    let t = self.parse_texture_filename()?;
                    m.textures.push(XTex { name: t, is_normal_map: true });
                }
                _ => self.parse_unknown()?,
            }
        }
        Ok(m)
    }

    fn parse_texture_filename(&mut self) -> Result<String, ImportError> {
        self.read_head_of_data_object()?;
        let mut name = self.next_token_as_string()?;
        self.check_for_closing_brace()?;
        while let Some(i) = name.find("\\\\") {
            name.replace_range(i..i + 2, "\\");
        }
        Ok(name)
    }

    fn parse_animation_set(&mut self) -> Result<(), ImportError> {
        let name = self.read_head_of_data_object()?;
        self.scene.anims.push(XAnimation { name, anims: vec![] });
        let ai = self.scene.anims.len() - 1;
        loop {
            let obj = self.next_token();
            match obj.as_str() {
                "" => return err("Unexpected end of file while parsing animation set."),
                "}" => break,
                "Animation" => {
                    let b = self.parse_animation()?;
                    self.scene.anims[ai].anims.push(b);
                }
                _ => self.parse_unknown()?,
            }
        }
        Ok(())
    }

    fn parse_animation(&mut self) -> Result<XAnimBone, ImportError> {
        self.read_head_of_data_object()?;
        let mut b = XAnimBone::default();
        loop {
            let obj = self.next_token();
            match obj.as_str() {
                "" => return err("Unexpected end of file while parsing animation."),
                "}" => break,
                "AnimationKey" => self.parse_animation_key(&mut b)?,
                "AnimationOptions" => self.parse_unknown()?,
                "{" => {
                    b.bone_name = self.next_token();
                    self.check_for_closing_brace()?;
                }
                _ => self.parse_unknown()?,
            }
        }
        Ok(b)
    }

    fn parse_animation_key(&mut self, b: &mut XAnimBone) -> Result<(), ImportError> {
        self.read_head_of_data_object()?;
        let key_type = self.read_int()?;
        let num_keys = self.read_int()?;
        for _ in 0..num_keys {
            let time = self.read_int()? as f64;
            match key_type {
                0 => {
                    if self.read_int()? != 4 {
                        return err("Invalid number of arguments for quaternion key in animation");
                    }
                    let w = self.read_float()?;
                    let x = self.read_float()?;
                    let y = self.read_float()?;
                    let z = self.read_float()?;
                    b.rot_keys.push(AiQuatKey { time, value: AiQuat { w, x, y, z } });
                    self.check_for_semicolon()?;
                }
                1 | 2 => {
                    if self.read_int()? != 3 {
                        return err("Invalid number of arguments for vector key in animation");
                    }
                    let v = self.read_vector3()?;
                    let k = AiVectorKey { time, value: v };
                    if key_type == 2 {
                        b.pos_keys.push(k);
                    } else {
                        b.scale_keys.push(k);
                    }
                }
                3 | 4 => {
                    if self.read_int()? != 16 {
                        return err("Invalid number of arguments for matrix key in animation");
                    }
                    let m = self.read_matrix()?;
                    b.trafo_keys.push((time, m));
                    self.check_for_semicolon()?;
                }
                _ => return err(format!("Unknown key type {} in animation.", key_type)),
            }
            self.check_for_separator()?;
        }
        self.check_for_closing_brace()?;
        Ok(())
    }

    fn parse_unknown(&mut self) -> Result<(), ImportError> {
        loop {
            let t = self.next_token();
            if t.is_empty() {
                return err("Unexpected end of file while parsing unknown segment.");
            }
            if t == "{" {
                break;
            }
        }
        let mut counter = 1;
        while counter > 0 {
            let t = self.next_token();
            if t.is_empty() {
                return err("Unexpected end of file while parsing unknown segment.");
            }
            if t == "{" {
                counter += 1;
            } else if t == "}" {
                counter -= 1;
            }
        }
        Ok(())
    }

    fn filter_hierarchy(&mut self, n: usize) {
        if self.scene.nodes[n].children.len() == 1 && self.scene.nodes[n].meshes.is_empty() {
            let child = self.scene.nodes[n].children[0];
            if self.scene.nodes[child].name.is_empty() && !self.scene.nodes[child].meshes.is_empty() {
                let meshes = std::mem::take(&mut self.scene.nodes[child].meshes);
                self.scene.nodes[n].meshes.extend(meshes);
                let ct = self.scene.nodes[child].trafo;
                self.scene.nodes[n].trafo = self.scene.nodes[n].trafo.mul(&ct);
                self.scene.nodes[n].children.clear();
            }
        }
        let children = self.scene.nodes[n].children.clone();
        for c in children {
            self.filter_hierarchy(c);
        }
    }
}

fn parse(data: &[u8]) -> Result<XScene, ImportError> {
    // XFileImporter reads the file into a buffer with a trailing NUL and
    // strips a UTF-8 BOM.
    let mut buf = data.to_vec();
    if buf.len() < 16 {
        return err("XFile is too small.");
    }
    if buf[0] == 0xEF && buf[1] == 0xBB && buf[2] == 0xBF {
        buf.drain(0..3);
    }
    buf.push(0);
    if &buf[0..4] != b"xof " {
        return err("Header mismatch, file is not an XFile.");
    }
    let fmt = &buf[8..12];
    let (binary, compressed) = match fmt {
        b"txt " => (false, false),
        b"bin " => (true, false),
        b"tzip" => (false, true),
        b"bzip" => (true, true),
        _ => return err("Unsupported x-file format"),
    };
    let fs = |i: usize| (buf[i] as i32 - 48) as u32;
    let float_size = fs(12) * 1000 + fs(13) * 100 + fs(14) * 10 + fs(15);
    if float_size != 32 && float_size != 64 {
        return err(format!("Unknown float size {} specified in x-file header.", float_size));
    }
    if compressed {
        return err("compressed X files are not supported");
    }
    let end = buf.len() - 1;
    let mut p = Parser {
        b: &buf,
        p: 16,
        end,
        binary,
        bin_float_size: (float_size / 8) as usize,
        bin_num_count: 0,
        line: 0,
        scene: XScene::default(),
    };
    p.read_until_end_of_line();
    p.parse_file()?;
    if let Some(r) = p.scene.root {
        p.filter_hierarchy(r);
    }
    Ok(p.scene)
}

const MATKEY_NAME: (&str, u32, u32) = ("?mat.name", 0, 0);

fn tex_key(ty: u32, n: u32) -> (&'static str, u32, u32) {
    ("$tex.file", ty, n)
}

fn convert_materials(out: &mut AiScene, mats: &mut [XMaterial]) {
    for m in mats.iter_mut() {
        if m.is_reference {
            for (b, sm) in out.materials.iter().enumerate() {
                if sm.get_string(MATKEY_NAME.0, 0, 0).as_deref() == Some(m.name.as_str()) {
                    m.scene_index = Some(b);
                    break;
                }
            }
            if m.scene_index.is_none() {
                m.scene_index = Some(0);
            }
            continue;
        }
        let mut mat = AiMaterial::default();
        mat.add_string(MATKEY_NAME.0, 0, 0, &m.name);
        // (int)mSpecularExponent == 0.0f ? Gouraud(3) : Phong(4)
        let shade = if (m.specular_exponent as i32) as f32 == 0.0 { 3 } else { 4 };
        mat.add_ints("$mat.shadingm", 0, 0, &[shade]);
        mat.add_floats("$clr.emissive", 0, 0, &m.emissive);
        mat.add_floats("$clr.diffuse", 0, 0, &m.diffuse);
        mat.add_floats("$clr.specular", 0, 0, &m.specular);
        mat.add_floats("$mat.shininess", 0, 0, &[m.specular_exponent]);
        if m.textures.len() == 1 {
            let t = &m.textures[0];
            if !t.name.is_empty() {
                let (k, ty, n) = if t.is_normal_map { tex_key(6, 0) } else { tex_key(1, 0) };
                mat.add_string(k, ty, n, &t.name);
            }
        } else {
            let (mut ihm, mut inm, mut idm, mut ism, mut iam, mut iem) = (0u32, 0u32, 0u32, 0u32, 0u32, 0u32);
            for t in &m.textures {
                let mut sz = t.name.clone().into_bytes();
                if sz.is_empty() {
                    continue;
                }
                let s = sz.iter().rposition(|&c| c == b'\\' || c == b'/').unwrap_or(0);
                if let Some(e) = sz.iter().rposition(|&c| c == b'.') {
                    sz[e] = 0;
                }
                let lower: Vec<u8> = sz.iter().map(|c| c.to_ascii_lowercase()).collect();
                // std::string::find works on the full string including
                // the embedded NUL.
                let find = |pat: &str| lower[s..].windows(pat.len()).any(|w| w == pat.as_bytes());
                let (ty, idx) = if find("bump") || find("height") {
                    ihm += 1;
                    (5, ihm - 1)
                } else if t.is_normal_map || find("normal") || find("nm") {
                    inm += 1;
                    (6, inm - 1)
                } else if find("spec") || find("glanz") {
                    ism += 1;
                    (2, ism - 1)
                } else if find("ambi") || find("env") {
                    iam += 1;
                    (3, iam - 1)
                } else if find("emissive") || find("self") {
                    iem += 1;
                    (4, iem - 1)
                } else {
                    idm += 1;
                    (1, idm - 1)
                };
                mat.add_string("$tex.file", ty, idx, &t.name);
            }
        }
        m.scene_index = Some(out.materials.len());
        out.materials.push(mat);
    }
}

fn create_meshes(out: &mut AiScene, node: usize, xs: &mut XScene, meshes: &[usize]) {
    if meshes.is_empty() {
        return;
    }
    let mut new_meshes = Vec::new();
    for &mi in meshes {
        let mut mats = std::mem::take(&mut xs.meshes[mi].materials);
        convert_materials(out, &mut mats);
        xs.meshes[mi].materials = mats;
        let src = &xs.meshes[mi];
        let num_materials = src.materials.len().max(1);
        for b in 0..num_materials {
            let mut faces = Vec::new();
            let mut num_vertices = 0usize;
            if !src.face_materials.is_empty() {
                for (c, &fm) in src.face_materials.iter().enumerate() {
                    if fm as usize == b {
                        faces.push(c);
                        num_vertices += src.pos_faces[c].indices.len();
                    }
                }
            } else {
                for c in 0..src.pos_faces.len() {
                    faces.push(c);
                    num_vertices += src.pos_faces[c].indices.len();
                }
            }
            if num_vertices == 0 {
                continue;
            }
            let mut mesh = AiMesh {
                name: src.name.clone(),
                material_index: if !src.face_materials.is_empty() {
                    src.materials[b].scene_index.unwrap_or(0) as u32
                } else {
                    0
                },
                vertices: vec![[0.0; 3]; num_vertices],
                ..Default::default()
            };
            if !src.normals.is_empty() {
                mesh.normals = Some(vec![[0.0; 3]; num_vertices]);
            }
            for c in 0..MAX_UV {
                if !src.tex_coords[c].is_empty() {
                    mesh.uvs[c] = Some(vec![[0.0; 3]; num_vertices]);
                }
            }
            for c in 0..MAX_COLORS {
                if !src.colors[c].is_empty() {
                    mesh.colors[c] = Some(vec![[0.0; 4]; num_vertices]);
                }
            }
            let mut new_index = 0usize;
            let mut org_points = vec![0u32; num_vertices];
            for &f in &faces {
                let pf = &src.pos_faces[f];
                let mut df = Vec::with_capacity(pf.indices.len());
                for (d, &pi) in pf.indices.iter().enumerate() {
                    df.push(new_index as u32);
                    if pi as usize > src.positions.len() {
                        continue;
                    }
                    org_points[new_index] = pi;
                    let pi = pi as usize;
                    mesh.vertices[new_index] = src.positions.get(pi).copied().unwrap_or_default();
                    if let Some(n) = mesh.normals.as_mut() {
                        if let Some(nf) = src.norm_faces.get(f) {
                            if nf.indices.len() > d {
                                n[new_index] = src.normals.get(nf.indices[d] as usize).copied().unwrap_or_default();
                            }
                        }
                    }
                    for e in 0..MAX_UV {
                        if let Some(uv) = mesh.uvs[e].as_mut() {
                            let t = src.tex_coords[e].get(pi).copied().unwrap_or_default();
                            uv[new_index] = [t[0], 1.0 - t[1], 0.0];
                        }
                    }
                    for e in 0..MAX_COLORS {
                        if let Some(col) = mesh.colors[e].as_mut() {
                            col[new_index] = src.colors[e].get(pi).copied().unwrap_or_default();
                        }
                    }
                    new_index += 1;
                }
                mesh.faces.push(df);
            }
            for ob in &src.bones {
                let mut old_weights = vec![0.0f32; src.positions.len()];
                for &(v, w) in &ob.weights {
                    if let Some(o) = old_weights.get_mut(v as usize) {
                        *o = w;
                    }
                }
                let mut nw = Vec::new();
                for (d, &op) in org_points.iter().enumerate() {
                    let w = old_weights.get(op as usize).copied().unwrap_or(0.0);
                    if w > 0.0 {
                        nw.push((d as u32, w));
                    }
                }
                if nw.is_empty() {
                    continue;
                }
                mesh.bones.push(AiBone { name: ob.name.clone(), weights: nw, offset: ob.offset });
            }
            new_meshes.push(mesh);
        }
    }
    for m in new_meshes {
        out.nodes[node].meshes.push(out.meshes.len() as u32);
        out.meshes.push(m);
    }
}

fn create_nodes(out: &mut AiScene, xs: &mut XScene, xn: usize, parent: Option<usize>) -> usize {
    let name = xs.nodes[xn].name.clone();
    let id = out.add_node(&name, xs.nodes[xn].trafo, parent);
    let meshes = xs.nodes[xn].meshes.clone();
    create_meshes(out, id, xs, &meshes);
    let children = xs.nodes[xn].children.clone();
    for c in children {
        create_nodes(out, xs, c, Some(id));
    }
    id
}

/// `aiQuaternion::GetMatrix` as the game's assimp build compiles it inside
/// `CreateAnimations`: clang contracts the products into fused
/// multiply-adds, which changes the last bit of many rotation keys.
fn rot_key_matrix(q: &AiQuat) -> [[f32; 3]; 3] {
    let AiQuat { w, x, y, z } = *q;
    [
        [
            y.mul_add(y, z * z).mul_add(-2.0, 1.0),
            2.0 * y.mul_add(x, -(z * w)),
            2.0 * x.mul_add(z, y * w),
        ],
        [
            2.0 * y.mul_add(x, z * w),
            x.mul_add(x, z * z).mul_add(-2.0, 1.0),
            2.0 * y.mul_add(z, -(x * w)),
        ],
        [
            2.0 * x.mul_add(z, -(y * w)),
            2.0 * y.mul_add(z, x * w),
            x.mul_add(x, y * y).mul_add(-2.0, 1.0),
        ],
    ]
}

fn create_animations(out: &mut AiScene, xs: &XScene) {
    for anim in &xs.anims {
        if anim.anims.is_empty() {
            continue;
        }
        let mut na = AiAnimation {
            name: anim.name.clone(),
            duration: 0.0,
            ticks_per_second: xs.anim_ticks_per_second as f64,
            channels: vec![],
        };
        for bone in &anim.anims {
            let mut nb = AiNodeAnim { node_name: bone.bone_name.clone(), ..Default::default() };
            if !bone.trafo_keys.is_empty() {
                for &(time, t) in &bone.trafo_keys {
                    let m = &t.m;
                    let pos = [m[0][3], m[1][3], m[2][3]];
                    nb.position_keys.push(AiVectorKey { time, value: pos });
                    let sx = super::aimath::vlen([m[0][0], m[1][0], m[2][0]]);
                    let sy = super::aimath::vlen([m[0][1], m[1][1], m[2][1]]);
                    let sz = super::aimath::vlen([m[0][2], m[1][2], m[2][2]]);
                    nb.scaling_keys.push(AiVectorKey { time, value: [sx, sy, sz] });
                    let r = [
                        [m[0][0] / sx, m[0][1] / sy, m[0][2] / sz],
                        [m[1][0] / sx, m[1][1] / sy, m[1][2] / sz],
                        [m[2][0] / sx, m[2][1] / sy, m[2][2] / sz],
                    ];
                    nb.rotation_keys.push(AiQuatKey { time, value: AiQuat::from_matrix(&r) });
                }
                na.duration = na.duration.max(bone.trafo_keys.last().unwrap().0);
            } else {
                nb.position_keys = bone.pos_keys.clone();
                for k in &bone.rot_keys {
                    let r = rot_key_matrix(&k.value);
                    let mut q = AiQuat::from_matrix(&r);
                    q.w *= -1.0;
                    nb.rotation_keys.push(AiQuatKey { time: k.time, value: q });
                }
                nb.scaling_keys = bone.scale_keys.clone();
                if let Some(k) = bone.pos_keys.last() {
                    na.duration = na.duration.max(k.time);
                }
                if let Some(k) = bone.rot_keys.last() {
                    na.duration = na.duration.max(k.time);
                }
                if let Some(k) = bone.scale_keys.last() {
                    na.duration = na.duration.max(k.time);
                }
            }
            na.channels.push(nb);
        }
        out.animations.push(na);
    }
}

/// Imports a `.x` file the way `XFileImporter::InternReadFile` does
/// (before assimp's scene preprocessor and post-processing).
pub fn import(data: &[u8]) -> Result<AiScene, ImportError> {
    let mut xs = parse(data)?;
    let mut out = AiScene::default();
    let mut gm = std::mem::take(&mut xs.global_materials);
    convert_materials(&mut out, &mut gm);
    let mut has_root = false;
    if let Some(r) = xs.root {
        out.root = create_nodes(&mut out, &mut xs, r, None);
        has_root = true;
    }
    create_animations(&mut out, &xs);
    if !xs.global_meshes.is_empty() {
        if !has_root {
            out.root = out.add_node("$dummy_node", AiMat4::IDENTITY, None);
            has_root = true;
        }
        let gms = xs.global_meshes.clone();
        let root = out.root;
        create_meshes(&mut out, root, &mut xs, &gms);
    }
    if !has_root {
        return err("XFile is ill-formatted - no content imported.");
    }
    make_left_handed(&mut out);
    flip_winding_order(&mut out);
    if out.materials.is_empty() {
        let mut mat = AiMaterial::default();
        mat.add_ints("$mat.shadingm", 0, 0, &[3]);
        mat.add_floats("$clr.emissive", 0, 0, &[0.0, 0.0, 0.0]);
        mat.add_floats("$clr.specular", 0, 0, &[0.0, 0.0, 0.0]);
        mat.add_floats("$clr.diffuse", 0, 0, &[0.5, 0.5, 0.5]);
        mat.add_ints("$mat.shininess", 0, 0, &[1]);
        out.materials.push(mat);
    }
    Ok(out)
}
