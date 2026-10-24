//! The FBX DOM (`FBXDocument`, `FBXProperties`, `FBXModel`,
//! `FBXMeshGeometry`, `FBXDeformer`, `FBXMaterial`, `FBXAnimation`):
//! objects are built lazily from the element tree and linked through the
//! connection table, as assimp does.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use super::parser::*;
use crate::import::aimath::AiMat4;
use crate::import::scene::{MAX_COLORS, MAX_UV};
use crate::import::ImportError;

// ---------------------------------------------------------------------------
// Properties

#[derive(Clone, Debug, PartialEq)]
pub enum Prop {
    Str(String),
    Bool(bool),
    Int(i32),
    ULong(u64),
    KTime(i64),
    Vec3([f32; 3]),
    Float(f32),
    Color4([f32; 4]),
}

/// `ReadTypedProperty`.
fn read_typed_property(el: &Element) -> Result<Option<Prop>, ImportError> {
    let tok = &el.tokens;
    if tok.len() < 2 {
        return Ok(None);
    }
    let s = parse_string(&tok[1])?;
    let need = |n: usize| -> Result<(), ImportError> {
        if tok.len() < n {
            return err(format!("Not enough tokens for property of type {s}"));
        }
        Ok(())
    };
    Ok(Some(match s.as_str() {
        "KString" => {
            need(5)?;
            Prop::Str(parse_string(&tok[4])?)
        }
        "bool" | "Bool" => {
            need(5)?;
            Prop::Bool(parse_int(&tok[4])? != 0)
        }
        "int" | "Int" | "enum" | "Enum" | "Integer" => {
            need(5)?;
            Prop::Int(parse_int(&tok[4])?)
        }
        "ULongLong" => {
            need(5)?;
            Prop::ULong(parse_id(&tok[4])?)
        }
        "KTime" => {
            need(5)?;
            Prop::KTime(parse_int64(&tok[4])?)
        }
        "Vector3D" | "ColorRGB" | "Vector" | "Color" | "Lcl Translation" | "Lcl Rotation"
        | "Lcl Scaling" => {
            need(7)?;
            Prop::Vec3([
                parse_float(&tok[4])?,
                parse_float(&tok[5])?,
                parse_float(&tok[6])?,
            ])
        }
        "double" | "Number" | "float" | "Float" | "FieldOfView" | "UnitScaleFactor" => {
            need(5)?;
            Prop::Float(parse_float(&tok[4])?)
        }
        "ColorAndAlpha" => {
            need(8)?;
            Prop::Color4([
                parse_float(&tok[4])?,
                parse_float(&tok[5])?,
                parse_float(&tok[6])?,
                parse_float(&tok[7])?,
            ])
        }
        _ => return Ok(None),
    }))
}

/// `PropertyTable`: lazily parsed `P` entries plus an optional template.
/// Which entries were read matters: `GetUnparsedProperties` (the `$raw.`
/// material keys) lists only the ones nobody asked for.
#[derive(Debug, Default)]
pub struct PropertyTable {
    lazy: BTreeMap<String, Option<Prop>>,
    lazy_err: BTreeMap<String, String>,
    parsed: RefCell<std::collections::BTreeSet<String>>,
    pub template: Option<Rc<PropertyTable>>,
}

impl PropertyTable {
    pub fn from_element(
        el: &Element,
        template: Option<Rc<PropertyTable>>,
    ) -> Result<PropertyTable, ImportError> {
        let sc = required_scope(el)?;
        let mut t = PropertyTable {
            template,
            ..Default::default()
        };
        for (k, e) in sc.iter() {
            if k != "P" {
                continue;
            }
            // PeekPropertyName
            if e.tokens.len() < 4 {
                continue;
            }
            let name = parse_string(&e.tokens[0])?;
            if name.is_empty() || t.lazy.contains_key(&name) || t.lazy_err.contains_key(&name) {
                continue;
            }
            match read_typed_property(e) {
                Ok(p) => {
                    t.lazy.insert(name, p);
                }
                Err(er) => {
                    t.lazy_err.insert(name, er.0);
                }
            }
        }
        Ok(t)
    }

    /// `PropertyTable::Get` (falls back to the template).
    pub fn get(&self, name: &str) -> Result<Option<Prop>, ImportError> {
        if let Some(e) = self.lazy_err.get(name) {
            return Err(ImportError(e.clone()));
        }
        if let Some(p) = self.lazy.get(name) {
            self.parsed.borrow_mut().insert(name.to_string());
            return Ok(p.clone());
        }
        match &self.template {
            Some(t) => t.get(name),
            None => Ok(None),
        }
    }

    pub fn get_or<T: FromProp>(&self, name: &str, default: T) -> Result<T, ImportError> {
        Ok(self
            .get(name)?
            .and_then(|p| T::from_prop(&p))
            .unwrap_or(default))
    }

    /// `PropertyGet<T>(props, name, ok, useTemplate)`.
    pub fn get_ok<T: FromProp>(
        &self,
        name: &str,
        use_template: bool,
    ) -> Result<Option<T>, ImportError> {
        let mut p = self.get(name)?;
        if p.is_none() {
            if !use_template {
                return Ok(None);
            }
            match &self.template {
                Some(t) => p = t.get(name)?,
                None => return Ok(None),
            }
        }
        Ok(p.and_then(|p| T::from_prop(&p)))
    }

    /// `GetUnparsedProperties`: entries not read yet that parse to a known
    /// type, sorted by name.
    pub fn unparsed(&self) -> Vec<(String, Prop)> {
        let parsed = self.parsed.borrow();
        self.lazy
            .iter()
            .filter(|(k, _)| !parsed.contains(*k))
            .filter_map(|(k, v)| v.clone().map(|v| (k.clone(), v)))
            .collect()
    }
}

pub trait FromProp: Sized {
    fn from_prop(p: &Prop) -> Option<Self>;
}

impl FromProp for f32 {
    fn from_prop(p: &Prop) -> Option<Self> {
        if let Prop::Float(v) = p {
            Some(*v)
        } else {
            None
        }
    }
}

impl FromProp for i32 {
    fn from_prop(p: &Prop) -> Option<Self> {
        if let Prop::Int(v) = p {
            Some(*v)
        } else {
            None
        }
    }
}

impl FromProp for i64 {
    fn from_prop(p: &Prop) -> Option<Self> {
        if let Prop::KTime(v) = p {
            Some(*v)
        } else {
            None
        }
    }
}

impl FromProp for [f32; 3] {
    fn from_prop(p: &Prop) -> Option<Self> {
        if let Prop::Vec3(v) = p {
            Some(*v)
        } else {
            None
        }
    }
}

impl FromProp for String {
    fn from_prop(p: &Prop) -> Option<Self> {
        if let Prop::Str(v) = p {
            Some(v.clone())
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Objects

#[derive(Debug)]
pub struct Model {
    pub id: u64,
    pub name: String,
    pub props: Rc<PropertyTable>,
    pub materials: Vec<Rc<Object>>,
    pub geometry: Vec<Rc<Object>>,
    pub attributes: Vec<Rc<Object>>,
}

#[derive(Debug, Default)]
pub struct MeshGeometry {
    pub vertices: Vec<[f32; 3]>,
    pub faces: Vec<u32>,
    pub mapping_offsets: Vec<u32>,
    pub mapping_counts: Vec<u32>,
    pub mappings: Vec<u32>,
    pub normals: Vec<[f32; 3]>,
    pub tangents: Vec<[f32; 3]>,
    pub binormals: Vec<[f32; 3]>,
    pub uvs: [Vec<[f32; 2]>; MAX_UV],
    pub uv_names: [String; MAX_UV],
    pub colors: [Vec<[f32; 4]>; MAX_COLORS],
    pub materials: Vec<i32>,
    pub faces_start: RefCell<Vec<u32>>,
}

impl MeshGeometry {
    /// `ToOutputVertexIndex`.
    pub fn to_output_vertex_index(&self, index: u32) -> Option<&[u32]> {
        let i = index as usize;
        if i >= self.mapping_counts.len() {
            return None;
        }
        let o = self.mapping_offsets[i] as usize;
        Some(&self.mappings[o..o + self.mapping_counts[i] as usize])
    }

    /// `FaceForVertexIndex`.
    pub fn face_for_vertex_index(&self, index: u32) -> usize {
        let mut fs = self.faces_start.borrow_mut();
        if fs.is_empty() {
            let mut acc = 0u32;
            fs.push(0);
            for &f in &self.faces {
                acc += f;
                fs.push(acc);
            }
            fs.pop();
        }
        let ub = fs.partition_point(|&s| s <= index);
        ub - 1
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Geometry {
    pub id: u64,
    pub name: String,
    pub skin: Option<Rc<Object>>,
    pub mesh: Option<MeshGeometry>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Cluster {
    pub indices: Vec<i32>,
    pub weights: Vec<f32>,
    pub transform: AiMat4,
    pub transform_link: AiMat4,
    pub node: Rc<Object>,
}

#[derive(Debug)]
pub struct Skin {
    pub clusters: Vec<Rc<Object>>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Material {
    pub name: String,
    pub shading: String,
    pub props: Rc<PropertyTable>,
    /// `std::map<string, Texture*>` keyed by the connection property.
    pub textures: BTreeMap<String, Rc<Object>>,
    pub layered: BTreeMap<String, Vec<Option<Rc<Object>>>>,
}

#[derive(Debug)]
pub struct Texture {
    pub relative_filename: String,
    pub uv_trans: [f32; 2],
    pub uv_scaling: [f32; 2],
    pub uv_rotation: f32,
    pub props: Rc<PropertyTable>,
    pub media: Option<Rc<Object>>,
}

#[derive(Debug)]
pub struct Video {
    pub content_length: u64,
}

#[derive(Debug)]
pub struct AnimationCurve {
    pub keys: Vec<i64>,
    pub values: Vec<f32>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct AnimationCurveNode {
    pub id: u64,
    pub name: String,
    pub target: Option<Rc<Object>>,
    pub prop: String,
    pub curves: RefCell<Option<BTreeMap<String, Rc<Object>>>>,
}

#[derive(Debug)]
pub struct AnimationLayer {
    pub id: u64,
}

#[derive(Debug)]
pub struct AnimationStack {
    pub name: String,
    pub props: Rc<PropertyTable>,
    pub layers: Vec<Rc<Object>>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum ObjectKind {
    Model(Model),
    Geometry(Geometry),
    NodeAttribute { class: String },
    Cluster(Cluster),
    Skin(Skin),
    OtherDeformer,
    Material(Material),
    Texture(Texture),
    LayeredTexture,
    Video(Video),
    AnimationStack(AnimationStack),
    AnimationLayer(AnimationLayer),
    AnimationCurve(AnimationCurve),
    AnimationCurveNode(AnimationCurveNode),
}

#[derive(Debug)]
pub struct Object {
    pub id: u64,
    pub name: String,
    pub kind: ObjectKind,
}

impl Object {
    pub fn as_model(&self) -> Option<&Model> {
        if let ObjectKind::Model(m) = &self.kind {
            Some(m)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
pub struct Connection {
    pub order: u64,
    pub src: u64,
    pub dest: u64,
    pub prop: String,
}

pub struct ImportSettings {
    pub read_all_layers: bool,
    pub read_materials: bool,
    pub read_textures: bool,
    pub read_animations: bool,
    pub read_weights: bool,
    pub preserve_pivots: bool,
    pub optimize_empty_animation_curves: bool,
}

impl Default for ImportSettings {
    fn default() -> Self {
        ImportSettings {
            read_all_layers: true,
            read_materials: true,
            read_textures: true,
            read_animations: true,
            read_weights: true,
            preserve_pivots: true,
            optimize_empty_animation_curves: true,
        }
    }
}

struct LazyObject<'a, 'd> {
    element: &'d Element<'a>,
    key: String,
    state: Cell<u8>, // 0 idle, 1 being constructed, 2 failed
    object: RefCell<Option<Rc<Object>>>,
}

pub struct Document<'a, 'd> {
    pub settings: ImportSettings,
    pub fbx_version: i32,
    pub creator: String,
    pub globals: Rc<PropertyTable>,
    templates: HashMap<String, Rc<PropertyTable>>,
    objects: BTreeMap<u64, LazyObject<'a, 'd>>,
    animation_stacks: Vec<u64>,
    src_conns: BTreeMap<u64, Vec<Connection>>,
    dest_conns: BTreeMap<u64, Vec<Connection>>,
}

const SUPPORTED_MIN: i32 = 7100;

impl<'a, 'd> Document<'a, 'd> {
    pub fn new(root: &'d Scope<'a>, settings: ImportSettings) -> Result<Self, ImportError> {
        let mut doc = Document {
            settings,
            fbx_version: 0,
            creator: String::new(),
            globals: Rc::new(PropertyTable::default()),
            templates: HashMap::new(),
            objects: BTreeMap::new(),
            animation_stacks: Vec::new(),
            src_conns: BTreeMap::new(),
            dest_conns: BTreeMap::new(),
        };
        // ReadHeader
        let ehead = root
            .get("FBXHeaderExtension")
            .filter(|e| e.compound.is_some());
        let Some(ehead) = ehead else {
            return err("no FBXHeaderExtension dictionary found");
        };
        let shead = required_scope(ehead)?;
        doc.fbx_version = parse_int(required_token(required_element(shead, "FBXVersion")?, 0)?)?;
        if doc.fbx_version < SUPPORTED_MIN {
            return err("unsupported, old format version");
        }
        if let Some(c) = shead.get("Creator") {
            doc.creator = parse_string(required_token(c, 0)?)?;
        }
        // ReadPropertyTemplates
        if let Some(edefs) = root.get("Definitions").and_then(|e| e.compound.as_deref()) {
            for el in edefs.collection("ObjectType") {
                let Some(cur) = el.compound.as_deref() else {
                    continue;
                };
                let Some(t0) = el.tokens.first() else {
                    continue;
                };
                let oname = parse_string(t0)?;
                for inner in cur.collection("PropertyTemplate") {
                    let Some(isc) = inner.compound.as_deref() else {
                        continue;
                    };
                    let Some(ct) = inner.tokens.first() else {
                        continue;
                    };
                    let pname = parse_string(ct)?;
                    if let Some(p70) = isc.get("Properties70") {
                        let t = PropertyTable::from_element(p70, None)?;
                        doc.templates.insert(format!("{oname}.{pname}"), Rc::new(t));
                    }
                }
            }
        }
        // ReadGlobalSettings
        match root.get("GlobalSettings").filter(|e| e.compound.is_some()) {
            Some(e) => {
                doc.globals = doc.property_table("", e, required_scope(e)?)?;
            }
            None => doc.globals = Rc::new(PropertyTable::default()),
        }
        // ReadObjects
        let Some(eobjects) = root.get("Objects").filter(|e| e.compound.is_some()) else {
            return err("no Objects dictionary found");
        };
        doc.objects.insert(
            0,
            LazyObject {
                element: eobjects,
                key: "Objects".into(),
                state: Cell::new(0),
                object: RefCell::new(None),
            },
        );
        for (k, el) in required_scope(eobjects)?.iter() {
            let Some(t0) = el.tokens.first() else {
                return err("expected ID after object key");
            };
            let id = parse_id(t0)?;
            if id == 0 {
                return err("encountered object with implicitly defined id 0");
            }
            doc.objects.insert(
                id,
                LazyObject {
                    element: el,
                    key: k.clone(),
                    state: Cell::new(0),
                    object: RefCell::new(None),
                },
            );
            if k == "AnimationStack" {
                doc.animation_stacks.push(id);
            }
        }
        // ReadConnections
        let Some(econns) = root.get("Connections").filter(|e| e.compound.is_some()) else {
            return err("no Connections dictionary found");
        };
        let mut order = 0u64;
        for el in required_scope(econns)?.collection("C") {
            let ty = parse_string(required_token(el, 0)?)?;
            if ty == "PP" {
                continue;
            }
            let src = parse_id(required_token(el, 1)?)?;
            let dest = parse_id(required_token(el, 2)?)?;
            let prop = if ty == "OP" {
                parse_string(required_token(el, 3)?)?
            } else {
                String::new()
            };
            if !doc.objects.contains_key(&src) || !doc.objects.contains_key(&dest) {
                continue;
            }
            let c = Connection {
                order,
                src,
                dest,
                prop,
            };
            order += 1;
            doc.src_conns.entry(src).or_default().push(c.clone());
            doc.dest_conns.entry(dest).or_default().push(c);
        }
        Ok(doc)
    }

    /// `GetPropertyTable`.
    pub fn property_table(
        &self,
        template: &str,
        _el: &Element,
        sc: &Scope,
    ) -> Result<Rc<PropertyTable>, ImportError> {
        let tmpl = if template.is_empty() {
            None
        } else {
            self.templates.get(template).cloned()
        };
        match sc.get("Properties70").filter(|e| e.compound.is_some()) {
            None => Ok(tmpl.unwrap_or_else(|| Rc::new(PropertyTable::default()))),
            Some(p70) => Ok(Rc::new(PropertyTable::from_element(p70, tmpl)?)),
        }
    }

    pub fn object_key(&self, id: u64) -> Option<&str> {
        self.objects.get(&id).map(|o| o.key.as_str())
    }

    /// Connections where `id` is the source (`is_src`) or destination,
    /// optionally filtered by the other end's object key, in file order.
    pub fn connections(&self, id: u64, is_src: bool, classes: Option<&[&str]>) -> Vec<Connection> {
        let map = if is_src {
            &self.src_conns
        } else {
            &self.dest_conns
        };
        let mut v: Vec<Connection> = map
            .get(&id)
            .map(|v| {
                v.iter()
                    .filter(|c| match classes {
                        None => true,
                        Some(cl) => {
                            let other = if is_src { c.dest } else { c.src };
                            let key = self.object_key(other).unwrap_or("");
                            cl.contains(&key)
                        }
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        v.sort_by_key(|c| c.order);
        v
    }

    pub fn by_dest(&self, id: u64, classes: &[&str]) -> Vec<Connection> {
        self.connections(id, false, Some(classes))
    }

    pub fn by_dest_all(&self, id: u64) -> Vec<Connection> {
        self.connections(id, false, None)
    }

    pub fn by_src(&self, id: u64, classes: &[&str]) -> Vec<Connection> {
        self.connections(id, true, Some(classes))
    }

    pub fn animation_stacks(&self) -> Result<Vec<Rc<Object>>, ImportError> {
        let mut out = Vec::new();
        for &id in &self.animation_stacks {
            if let Some(o) = self.get(id)? {
                if matches!(o.kind, ObjectKind::AnimationStack(_)) {
                    out.push(o);
                }
            }
        }
        Ok(out)
    }

    /// `LazyObject::Get`: construction errors make the object null (non
    /// strict mode); objects already being built return null.
    pub fn get(&self, id: u64) -> Result<Option<Rc<Object>>, ImportError> {
        let Some(lo) = self.objects.get(&id) else {
            return Ok(None);
        };
        if lo.state.get() != 0 {
            return Ok(None);
        }
        if let Some(o) = lo.object.borrow().as_ref() {
            return Ok(Some(o.clone()));
        }
        lo.state.set(1);
        let r = self.construct(id, lo);
        match r {
            Ok(Some(o)) => {
                lo.state.set(0);
                *lo.object.borrow_mut() = Some(o.clone());
                Ok(Some(o))
            }
            Ok(None) => {
                lo.state.set(0);
                Ok(None)
            }
            Err(_) => {
                lo.state.set(2);
                Ok(None)
            }
        }
    }

    fn construct(
        &self,
        id: u64,
        lo: &LazyObject<'a, 'd>,
    ) -> Result<Option<Rc<Object>>, ImportError> {
        let el = lo.element;
        let tokens = &el.tokens;
        if tokens.len() < 3 {
            return err("expected at least 3 tokens: id, name and class tag");
        }
        let mut name = parse_string(&tokens[1])?;
        if tokens[1].binary {
            let b = name.as_bytes().to_vec();
            let mut i = 0;
            while i < b.len() {
                if b[i] == 0 && b.get(i + 1) == Some(&1) {
                    name = format!(
                        "{}::{}",
                        bytes_to_string(&b[i + 2..]),
                        bytes_to_string(&b[..i])
                    );
                    break;
                }
                i += 1;
            }
        }
        let classtag = parse_string(&tokens[2])?;
        let key = lo.key.as_str();
        let kind = match key {
            "Geometry" => {
                if classtag == "Mesh" {
                    ObjectKind::Geometry(self.read_geometry(id, el, &name, true)?)
                } else if classtag == "Shape" || classtag == "Line" {
                    ObjectKind::Geometry(self.read_geometry(id, el, &name, false)?)
                } else {
                    return Ok(None);
                }
            }
            "NodeAttribute" => {
                if ["Camera", "CameraSwitcher", "Light", "Null", "LimbNode"]
                    .contains(&classtag.as_str())
                {
                    ObjectKind::NodeAttribute {
                        class: classtag.clone(),
                    }
                } else {
                    return Ok(None);
                }
            }
            "Deformer" => match classtag.as_str() {
                "Cluster" => ObjectKind::Cluster(self.read_cluster(id, el)?),
                "Skin" => ObjectKind::Skin(self.read_skin(id)?),
                "BlendShape" | "BlendShapeChannel" => ObjectKind::OtherDeformer,
                _ => return Ok(None),
            },
            "Model" => {
                if classtag == "IKEffector" || classtag == "FKEffector" {
                    return Ok(None);
                }
                ObjectKind::Model(self.read_model(id, el, &name)?)
            }
            "Material" => ObjectKind::Material(self.read_material(id, el, &name)?),
            "Texture" => ObjectKind::Texture(self.read_texture(id, el)?),
            "LayeredTexture" => ObjectKind::LayeredTexture,
            "Video" => ObjectKind::Video(self.read_video(el)?),
            "AnimationStack" => {
                let sc = required_scope(el)?;
                let props = self.property_table("AnimationStack.FbxAnimStack", el, sc)?;
                let mut layers = Vec::new();
                for c in self.by_dest(id, &["AnimationLayer"]) {
                    if !c.prop.is_empty() {
                        continue;
                    }
                    if let Some(o) = self.get(c.src)? {
                        if matches!(o.kind, ObjectKind::AnimationLayer(_)) {
                            layers.push(o);
                        }
                    }
                }
                ObjectKind::AnimationStack(AnimationStack {
                    name: name.clone(),
                    props,
                    layers,
                })
            }
            "AnimationLayer" => ObjectKind::AnimationLayer(AnimationLayer { id }),
            "AnimationCurve" => {
                let sc = required_scope(el)?;
                let keys = read_int64s(required_element(sc, "KeyTime")?)?;
                let values = read_floats(required_element(sc, "KeyValueFloat")?)?;
                if keys.len() != values.len() {
                    return err(
                        "the number of key times does not match the number of keyframe values",
                    );
                }
                if keys.windows(2).any(|w| w[0] >= w[1]) {
                    return err("the keyframes are not in ascending order");
                }
                ObjectKind::AnimationCurve(AnimationCurve { keys, values })
            }
            "AnimationCurveNode" => {
                let mut target = None;
                let mut prop = String::new();
                for c in self.by_src(id, &["Model", "NodeAttribute", "Deformer"]) {
                    if c.prop.is_empty() {
                        continue;
                    }
                    let Some(ob) = self.get(c.dest)? else {
                        continue;
                    };
                    target = Some(ob);
                    prop = c.prop.clone();
                    break;
                }
                ObjectKind::AnimationCurveNode(AnimationCurveNode {
                    id,
                    name: name.clone(),
                    target,
                    prop,
                    curves: RefCell::new(None),
                })
            }
            _ => return Ok(None),
        };
        Ok(Some(Rc::new(Object { id, name, kind })))
    }

    fn read_model(&self, id: u64, el: &Element, name: &str) -> Result<Model, ImportError> {
        let sc = required_scope(el)?;
        let props = self.property_table("Model.FbxNode", el, sc)?;
        let mut m = Model {
            id,
            name: name.to_string(),
            props,
            materials: vec![],
            geometry: vec![],
            attributes: vec![],
        };
        for c in self.by_dest(id, &["Geometry", "Material", "NodeAttribute"]) {
            if !c.prop.is_empty() {
                continue;
            }
            let Some(ob) = self.get(c.src)? else { continue };
            match ob.kind {
                ObjectKind::Material(_) => m.materials.push(ob),
                ObjectKind::Geometry(_) => m.geometry.push(ob),
                ObjectKind::NodeAttribute { .. } => m.attributes.push(ob),
                _ => {}
            }
        }
        Ok(m)
    }

    fn read_geometry(
        &self,
        id: u64,
        el: &Element,
        name: &str,
        is_mesh: bool,
    ) -> Result<Geometry, ImportError> {
        let mut skin = None;
        for c in self.by_dest(id, &["Deformer"]) {
            if !c.prop.is_empty() {
                continue;
            }
            if let Some(o) = self.get(c.src)? {
                if matches!(o.kind, ObjectKind::Skin(_)) {
                    skin = Some(o);
                }
            }
        }
        let mesh = if is_mesh {
            Some(self.read_mesh(el)?)
        } else {
            None
        };
        Ok(Geometry {
            id,
            name: name.to_string(),
            skin,
            mesh,
        })
    }

    fn read_mesh(&self, el: &Element) -> Result<MeshGeometry, ImportError> {
        let Some(sc) = el.compound.as_deref() else {
            return err("failed to read Geometry object (class: Mesh), no data scope found");
        };
        let verts_el = required_element(sc, "Vertices")?;
        let pvi_el = required_element(sc, "PolygonVertexIndex")?;
        let tv = read_float_tuples(verts_el, 3)?;
        let temp_verts: Vec<[f32; 3]> = tv.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        let temp_faces = read_ints(pvi_el)?;
        let mut m = MeshGeometry::default();
        let vc = temp_verts.len();
        m.mapping_offsets = vec![0; vc];
        m.mapping_counts = vec![0; vc];
        m.mappings = vec![0; temp_faces.len()];
        let mut count = 0u32;
        for &index in &temp_faces {
            let absi = if index < 0 {
                (-(index as i64) - 1) as usize
            } else {
                index as usize
            };
            if absi >= vc {
                return err("polygon vertex index out of range");
            }
            m.vertices.push(temp_verts[absi]);
            count += 1;
            m.mapping_counts[absi] += 1;
            if index < 0 {
                m.faces.push(count);
                count = 0;
            }
        }
        let mut cursor = 0u32;
        for i in 0..vc {
            m.mapping_offsets[i] = cursor;
            cursor += m.mapping_counts[i];
            m.mapping_counts[i] = 0;
        }
        cursor = 0;
        for &index in &temp_faces {
            let absi = if index < 0 {
                (-(index as i64) - 1) as usize
            } else {
                index as usize
            };
            m.mappings[(m.mapping_offsets[absi] + m.mapping_counts[absi]) as usize] = cursor;
            m.mapping_counts[absi] += 1;
            cursor += 1;
        }
        for layer in sc.collection("Layer") {
            let index = parse_int(
                layer
                    .tokens
                    .first()
                    .ok_or_else(|| ImportError("FBX: layer index".into()))?,
            )?;
            if self.settings.read_all_layers || index == 0 {
                let lsc = required_scope(layer)?;
                for le in lsc.collection("LayerElement") {
                    let les = required_scope(le)?;
                    let ty = parse_string(required_token(required_element(les, "Type")?, 0)?)?;
                    let typed_index =
                        parse_int(required_token(required_element(les, "TypedIndex")?, 0)?)?;
                    let mut found = false;
                    for cand in sc.collection(&ty) {
                        let ci = parse_int(required_token(cand, 0)?)?;
                        if ci == typed_index {
                            read_vertex_data(&mut m, &ty, typed_index, required_scope(cand)?)?;
                            found = true;
                            break;
                        }
                    }
                    let _ = found;
                }
            }
        }
        Ok(m)
    }

    fn read_cluster(&self, id: u64, el: &Element) -> Result<Cluster, ImportError> {
        let sc = required_scope(el)?;
        let indexes = sc.get("Indexes");
        let weights = sc.get("Weights");
        let transform = read_matrix(required_element(sc, "Transform")?)?;
        let transform_link = read_matrix(required_element(sc, "TransformLink")?)?;
        if indexes.is_some() != weights.is_some() {
            return err("either Indexes or Weights are missing from Cluster");
        }
        let (mut ind, mut w) = (vec![], vec![]);
        if let (Some(i), Some(ww)) = (indexes, weights) {
            ind = read_ints(i)?;
            w = read_floats(ww)?;
        }
        if ind.len() != w.len() {
            return err("sizes of index and weight array don't match up");
        }
        let mut node = None;
        for c in self.by_dest(id, &["Model"]) {
            if !c.prop.is_empty() {
                continue;
            }
            if let Some(o) = self.get(c.src)? {
                if o.as_model().is_some() {
                    node = Some(o);
                    break;
                }
            }
        }
        let Some(node) = node else {
            return err("failed to read target Node for Cluster");
        };
        Ok(Cluster {
            indices: ind,
            weights: w,
            transform,
            transform_link,
            node,
        })
    }

    fn read_skin(&self, id: u64) -> Result<Skin, ImportError> {
        let mut clusters = Vec::new();
        for c in self.by_dest(id, &["Deformer"]) {
            if !c.prop.is_empty() {
                continue;
            }
            if let Some(o) = self.get(c.src)? {
                if matches!(o.kind, ObjectKind::Cluster(_)) {
                    clusters.push(o);
                }
            }
        }
        Ok(Skin { clusters })
    }

    fn read_material(&self, id: u64, el: &Element, name: &str) -> Result<Material, ImportError> {
        let sc = required_scope(el)?;
        let mut shading = match sc.get("ShadingModel") {
            Some(s) => parse_string(required_token(s, 0)?)?,
            None => "phong".into(),
        };
        shading = shading.to_ascii_lowercase();
        let tmpl = match shading.as_str() {
            "phong" => "Material.FbxSurfacePhong",
            "lambert" => "Material.FbxSurfaceLambert",
            _ => "",
        };
        let props = self.property_table(tmpl, el, sc)?;
        let mut textures = BTreeMap::new();
        let mut layered = BTreeMap::new();
        for c in self.by_dest_all(id) {
            if c.prop.is_empty() {
                continue;
            }
            let Some(ob) = self.get(c.src)? else { continue };
            match ob.kind {
                ObjectKind::Texture(_) => {
                    textures.insert(c.prop.clone(), ob);
                }
                ObjectKind::LayeredTexture => {
                    let mut texs = Vec::new();
                    for lc in self.by_dest_all(ob.id) {
                        let t = self
                            .get(lc.src)?
                            .filter(|o| matches!(o.kind, ObjectKind::Texture(_)));
                        texs.push(t);
                    }
                    layered.insert(c.prop.clone(), texs);
                }
                _ => {}
            }
        }
        Ok(Material {
            name: name.to_string(),
            shading,
            props,
            textures,
            layered,
        })
    }

    fn read_texture(&self, id: u64, el: &Element) -> Result<Texture, ImportError> {
        let sc = required_scope(el)?;
        let mut t = Texture {
            relative_filename: String::new(),
            uv_trans: [0.0, 0.0],
            uv_scaling: [1.0, 1.0],
            uv_rotation: 0.0,
            props: Rc::new(PropertyTable::default()),
            media: None,
        };
        if let Some(e) = sc.get("Type") {
            parse_string(required_token(e, 0)?)?;
        }
        if let Some(e) = sc.get("FileName") {
            parse_string(required_token(e, 0)?)?;
        }
        if let Some(e) = sc.get("RelativeFilename") {
            t.relative_filename = parse_string(required_token(e, 0)?)?;
        }
        if let Some(e) = sc.get("ModelUVTranslation") {
            t.uv_trans = [
                parse_float(required_token(e, 0)?)?,
                parse_float(required_token(e, 1)?)?,
            ];
        }
        if let Some(e) = sc.get("ModelUVScaling") {
            t.uv_scaling = [
                parse_float(required_token(e, 0)?)?,
                parse_float(required_token(e, 1)?)?,
            ];
        }
        t.props = self.property_table("Texture.FbxFileTexture", el, sc)?;
        if let Some(s) = t.props.get_ok::<[f32; 3]>("Scaling", false)? {
            t.uv_scaling = [s[0], s[1]];
        }
        if let Some(s) = t.props.get_ok::<[f32; 3]>("Translation", false)? {
            t.uv_trans = [s[0], s[1]];
        }
        if let Some(s) = t.props.get_ok::<[f32; 3]>("Rotation", false)? {
            t.uv_rotation = s[2];
        }
        if self.settings.read_textures {
            for c in self.by_dest_all(id) {
                if let Some(o) = self.get(c.src)? {
                    if matches!(o.kind, ObjectKind::Video(_)) {
                        t.media = Some(o);
                    }
                }
            }
        }
        Ok(t)
    }

    fn read_video(&self, el: &Element) -> Result<Video, ImportError> {
        let sc = required_scope(el)?;
        let mut content_length = 0u64;
        if let Some(c) = sc.get("Content") {
            if let Some(tok) = c.tokens.first() {
                if tok.binary {
                    if tok.bytes.len() >= 5 && tok.bytes[0] == b'R' {
                        content_length =
                            u32::from_le_bytes(tok.bytes[1..5].try_into().unwrap()) as u64;
                    }
                } else if tok.bytes.first() == Some(&b'"') {
                    // base64 text: decoded size
                    let mut total = 0u64;
                    for t in &c.tokens {
                        let n = t.bytes.len().saturating_sub(2);
                        let body = &t.bytes[1..1 + n];
                        let pad = body.iter().rev().take_while(|&&b| b == b'=').count();
                        if n % 4 == 0 && n > 0 {
                            total += (n / 4 * 3 - pad) as u64;
                        }
                    }
                    content_length = total;
                }
            }
        }
        Ok(Video { content_length })
    }

    /// `AnimationCurveNode::Curves`.
    pub fn curves(
        &self,
        node: &AnimationCurveNode,
    ) -> Result<BTreeMap<String, Rc<Object>>, ImportError> {
        if let Some(c) = node.curves.borrow().as_ref() {
            if !c.is_empty() {
                return Ok(c.clone());
            }
        }
        let mut curves = BTreeMap::new();
        for c in self.by_dest(node.id, &["AnimationCurve"]) {
            if c.prop.is_empty() {
                continue;
            }
            let Some(o) = self.get(c.src)? else { continue };
            if matches!(o.kind, ObjectKind::AnimationCurve(_)) {
                curves.insert(c.prop.clone(), o);
            }
        }
        *node.curves.borrow_mut() = Some(curves.clone());
        Ok(curves)
    }

    /// `AnimationLayer::Nodes(whitelist)`.
    pub fn layer_nodes(
        &self,
        layer: &AnimationLayer,
        whitelist: &[&str],
    ) -> Result<Vec<Rc<Object>>, ImportError> {
        let mut out = Vec::new();
        for c in self.by_dest(layer.id, &["AnimationCurveNode"]) {
            if !c.prop.is_empty() {
                continue;
            }
            let Some(o) = self.get(c.src)? else { continue };
            if let ObjectKind::AnimationCurveNode(n) = &o.kind {
                if whitelist.contains(&n.prop.as_str()) {
                    out.push(o.clone());
                }
            }
        }
        Ok(out)
    }
}

fn read_vertex_data(
    m: &mut MeshGeometry,
    ty: &str,
    index: i32,
    source: &Scope,
) -> Result<(), ImportError> {
    let mit = parse_string(required_token(
        required_element(source, "MappingInformationType")?,
        0,
    )?)?;
    let rit = parse_string(required_token(
        required_element(source, "ReferenceInformationType")?,
        0,
    )?)?;
    match ty {
        "LayerElementUV" => {
            if index as usize >= MAX_UV || index < 0 {
                return Ok(());
            }
            let i = index as usize;
            m.uv_names[i] = match source.get("Name") {
                Some(n) => parse_string(required_token(n, 0)?)?,
                None => String::new(),
            };
            let mut out = Vec::new();
            resolve(m, &mut out, source, &mit, &rit, "UV", "UVIndex", 2, |c| {
                [c[0], c[1]]
            })?;
            m.uvs[i] = out;
        }
        "LayerElementMaterial" => {
            if !m.materials.is_empty() {
                return Ok(());
            }
            let face_count = m.faces.len();
            if face_count == 0 {
                return Ok(());
            }
            let mut mats = read_ints(required_element(source, "Materials")?)?;
            if mit == "AllSame" {
                if mats.is_empty() {
                    return Ok(());
                } else if mats.len() > 1 {
                    // assimp clears the list and then reads index 0 of the
                    // resized (zero-filled) array.
                    mats.clear();
                }
                let v = *mats.first().unwrap_or(&0);
                mats = vec![v; m.vertices.len()];
            } else if mit == "ByPolygon" && rit == "IndexToDirect" {
                mats.resize(face_count, 0);
            } else {
                return Ok(());
            }
            if mats.iter().all(|&n| n < 0) {
                return Ok(());
            }
            m.materials = mats;
        }
        "LayerElementNormal" => {
            if !m.normals.is_empty() {
                return Ok(());
            }
            let mut out = Vec::new();
            resolve(
                m,
                &mut out,
                source,
                &mit,
                &rit,
                "Normals",
                "NormalsIndex",
                3,
                |c| [c[0], c[1], c[2]],
            )?;
            m.normals = out;
        }
        "LayerElementTangent" => {
            if !m.tangents.is_empty() {
                return Ok(());
            }
            let (s, si) = if source.count("Tangents") > 0 {
                ("Tangents", "TangentsIndex")
            } else {
                ("Tangent", "TangentIndex")
            };
            let mut out = Vec::new();
            resolve(m, &mut out, source, &mit, &rit, s, si, 3, |c| {
                [c[0], c[1], c[2]]
            })?;
            m.tangents = out;
        }
        "LayerElementBinormal" => {
            if !m.binormals.is_empty() {
                return Ok(());
            }
            let (s, si) = if source.count("Binormals") > 0 {
                ("Binormals", "BinormalsIndex")
            } else {
                ("Binormal", "BinormalIndex")
            };
            let mut out = Vec::new();
            resolve(m, &mut out, source, &mit, &rit, s, si, 3, |c| {
                [c[0], c[1], c[2]]
            })?;
            m.binormals = out;
        }
        "LayerElementColor" => {
            if index as usize >= MAX_COLORS || index < 0 {
                return Ok(());
            }
            let mut out = Vec::new();
            resolve(
                m,
                &mut out,
                source,
                &mit,
                &rit,
                "Colors",
                "ColorIndex",
                4,
                |c| [c[0], c[1], c[2], c[3]],
            )?;
            m.colors[index as usize] = out;
        }
        _ => {}
    }
    Ok(())
}

/// `ResolveVertexDataArray`.
#[allow(clippy::too_many_arguments)]
fn resolve<T: Copy + Default>(
    m: &MeshGeometry,
    out: &mut Vec<T>,
    source: &Scope,
    mit: &str,
    rit: &str,
    data_name: &str,
    index_name: &str,
    n: usize,
    conv: impl Fn(&[f32]) -> T,
) -> Result<(), ImportError> {
    let mut is_direct = rit == "Direct";
    let mut is_index = rit == "IndexToDirect";
    let has_data = source.get(data_name).is_some();
    let has_index = source.get(index_name).is_some();
    if is_index && !has_index {
        is_direct = true;
        is_index = false;
    }
    let vertex_count = m.vertices.len();
    let read = |name: &str| -> Result<Vec<T>, ImportError> {
        let v = read_float_tuples(required_element(source, name)?, n)?;
        Ok(v.chunks_exact(n).map(&conv).collect())
    };
    if mit == "ByVertice" && is_direct {
        if !has_data {
            return Ok(());
        }
        let temp = read(data_name)?;
        if temp.len() != m.mapping_offsets.len() {
            return Ok(());
        }
        out.resize(vertex_count, T::default());
        for (i, d) in temp.iter().enumerate() {
            let s = m.mapping_offsets[i] as usize;
            let e = s + m.mapping_counts[i] as usize;
            for j in s..e {
                out[m.mappings[j] as usize] = *d;
            }
        }
    } else if mit == "ByVertice" && is_index {
        if !has_data || !has_index {
            return Ok(());
        }
        let temp = read(data_name)?;
        let idx = read_ints(required_element(source, index_name)?)?;
        if idx.len() != vertex_count {
            return Ok(());
        }
        out.resize(vertex_count, T::default());
        for (i, &ui) in idx.iter().enumerate() {
            // assimp indexes the mapping arrays by position here (a known
            // upstream quirk); guard against running past them.
            if i >= m.mapping_offsets.len() {
                break;
            }
            let s = m.mapping_offsets[i] as usize;
            let e = s + m.mapping_counts[i] as usize;
            for j in s..e {
                if ui as usize >= temp.len() || ui < 0 {
                    return err("index out of range");
                }
                out[m.mappings[j] as usize] = temp[ui as usize];
            }
        }
    } else if mit == "ByPolygonVertex" && is_direct {
        if !has_data {
            return Ok(());
        }
        let temp = read(data_name)?;
        if temp.len() != vertex_count {
            return Ok(());
        }
        *out = temp;
    } else if mit == "ByPolygonVertex" && is_index {
        if !has_data || !has_index {
            return Ok(());
        }
        let temp = read(data_name)?;
        let mut idx = read_ints(required_element(source, index_name)?)?;
        if idx.len() > vertex_count {
            idx.truncate(vertex_count);
        }
        if idx.len() != vertex_count {
            return Ok(());
        }
        out.resize(vertex_count, T::default());
        for (next, &i) in idx.iter().enumerate() {
            if i == -1 {
                out[next] = T::default();
                continue;
            }
            if i < 0 || i as usize >= temp.len() {
                return err("index out of range");
            }
            out[next] = temp[i as usize];
        }
    }
    Ok(())
}
