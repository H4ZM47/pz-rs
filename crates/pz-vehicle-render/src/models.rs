//! The vehicle's models (`ModelManager.addVehicle`): the body
//! (`VehicleModelInstance`) and one `VehicleSubModelInstance` per visible
//! part model (`BaseVehicle.ModelInfo` with its render transform and
//! animation player), loaded from the model scripts.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pz_anim::AnimationPlayer;
use pz_iso_render::joml::Matrix4f;
use pz_model::assets::{resolve_model_file, split_mesh_path, texture_path, AssetKind, MediaIndex};
use pz_model::loader::{load_mesh, MeshParams};
use pz_model::{Mat4, SkinningData};
use pz_render::model::{MeshData, VehicleShader};
use pz_render::Texture;
use pz_scripts::objects::models::ModelScript;
use pz_scripts::objects::vehicles::Model as ScriptModel;
use pz_scripts::script_type::ScriptType;
use pz_scripts::ScriptManager;
use pz_vehicles::{BaseVehicle, PartRef};

use crate::instance::{VehicleModelInstance, VehicleSkinTextures, VEHICLE_TEXTURE_FLAGS};

/// A model attachment of a model script (`ModelAttachment`).
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub offset: [f32; 3],
    pub rotate: [f32; 3],
    pub scale: f32,
    pub bone: Option<String>,
}

/// A loaded vehicle model (`Model` with its `ModelScript`'s values).
#[derive(Debug)]
pub struct VehicleModel {
    /// The model script's name.
    pub name: String,
    pub mesh: Rc<MeshData>,
    /// `ModelMesh.transform` (column-major).
    pub mesh_transform: Option<Mat4>,
    /// The skinned mesh's skeleton and clips (`model.tag`).
    pub skinning: Option<Rc<SkinningData>>,
    pub is_static: bool,
    /// The shader after `ModelManager.modifyShaderName`; `None` for a
    /// non-vehicle shader.
    pub shader: Option<(VehicleShader, bool)>,
    /// `Model.tex`.
    pub texture: Option<Texture>,
    /// `ModelScript.scale` (`applyModelScriptScale`).
    pub scale: f32,
    pub invert_x: bool,
    pub attachments: Vec<Attachment>,
    pub bone_weights: Vec<pz_scripts::objects::models::model_script::AnimBoneWeight>,
}

impl VehicleModel {
    pub fn attachment(&self, id: &str) -> Option<&Attachment> {
        self.attachments.iter().find(|a| a.id == id)
    }
}

/// Loads vehicle models by model script name (`ModelManager.getLoadedModel`).
pub trait VehicleModelSource {
    fn model(&self, name: &str) -> Option<Rc<VehicleModel>>;
}

/// `ModelManager.modifyShaderName`: the reflective vehicle shaders fall
/// back to `_noreflect` without `bPerfReflections`.
pub fn modify_shader_name(name: &str, reflections: bool) -> String {
    if matches!(
        name,
        "vehicle" | "vehicle_multiuv" | "vehicle_norandom_multiuv"
    ) && !reflections
    {
        format!("{name}_noreflect")
    } else {
        name.to_string()
    }
}

/// A legacy `.txt` mesh (`ModelLoader.loadTxt`, static only). Its
/// `VertexBufferObject` has the tangent at attribute 2 (never enabled) and
/// the texture coordinates at 3, so in the vehicle layout the first UV
/// slot reads zero and the second holds the coordinates.
fn txt_mesh(data: &[u8], is_static: bool, id: u64) -> Option<pz_model::ModelMesh> {
    if !is_static {
        return None;
    }
    let mt = pz_model::txt::load_txt(data, true, false, None).ok()?;
    let mut v = Vec::with_capacity(mt.vertices.len() / 11 * 10);
    for c in mt.vertices.as_chunks::<11>().0 {
        v.extend_from_slice(&c[0..6]);
        v.extend_from_slice(&[0.0, 0.0, c[9], c[10]]);
    }
    let mesh = MeshData {
        id,
        kind: pz_render::model::MeshKind::StaticUv2,
        vertices: v,
        indices: mt.elements,
    };
    Some(pz_model::ModelMesh {
        mesh: Rc::new(mesh),
        min_xyz: [0.0; 3],
        max_xyz: [0.0; 3],
        transform: None,
        skinning: None,
    })
}

/// [`VehicleModelSource`] over the game's media files and model scripts.
/// Textures come from `load_texture(path, flags)`.
pub struct MediaVehicleModels<'a> {
    pub media: &'a MediaIndex,
    pub scripts: Rc<ScriptManager>,
    /// `Core.getPerfReflectionsOnLoad()`.
    pub reflections: bool,
    pub load_texture: &'a dyn Fn(&str, i32) -> Option<Texture>,
    cache: RefCell<HashMap<String, Option<Rc<VehicleModel>>>>,
    next_id: Cell<u64>,
}

impl<'a> MediaVehicleModels<'a> {
    pub fn new(
        media: &'a MediaIndex,
        scripts: Rc<ScriptManager>,
        reflections: bool,
        load_texture: &'a dyn Fn(&str, i32) -> Option<Texture>,
    ) -> Self {
        // Vehicle mesh ids sit apart from the character resources' ids.
        MediaVehicleModels {
            media,
            scripts,
            reflections,
            load_texture,
            cache: RefCell::new(HashMap::new()),
            next_id: Cell::new(1 << 40),
        }
    }

    fn load(&self, name: &str) -> Option<Rc<VehicleModel>> {
        let ms = self.scripts.get::<ModelScript>(ScriptType::Model, name)?;
        let ms = ms.borrow();
        let mesh_name = ms.mesh_name()?.to_string();
        let (raw, sub) = split_mesh_path(&mesh_name);
        let (path, ty) = resolve_model_file(self.media, raw, AssetKind::Mesh)?;
        let data = std::fs::read(path).ok()?;
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let mm = match ty.import_format() {
            Some(format) => {
                let params = MeshParams {
                    is_static: ms.is_static,
                    mesh_name: sub.map(str::to_string),
                    post_process: ms.post_process.clone(),
                    ..Default::default()
                };
                load_mesh(&data, format, &params)
                    .ok()?
                    .into_model_mesh_uv2(id)?
            }
            None => txt_mesh(&data, ms.is_static, id)?,
        };
        let shader = modify_shader_name(ms.shader_name(), self.reflections);
        let texture = ms.texture_name().and_then(|t| {
            let p = texture_path(t);
            self.media.get(&p)?;
            (self.load_texture)(&p, VEHICLE_TEXTURE_FLAGS)
        });
        let attachments = ms
            .attachments
            .iter()
            .map(|a| {
                let a = a.borrow();
                let (o, r) = (a.offset(), a.rotate());
                Attachment {
                    id: a.id().to_string(),
                    offset: [o.x, o.y, o.z],
                    rotate: [r.x, r.y, r.z],
                    scale: a.scale(),
                    bone: a.bone().map(str::to_string),
                }
            })
            .collect();
        Some(Rc::new(VehicleModel {
            name: name.to_string(),
            mesh: mm.mesh,
            mesh_transform: mm.transform,
            skinning: mm.skinning.map(Rc::new),
            is_static: ms.is_static,
            shader: VehicleShader::from_name(&shader),
            texture,
            scale: ms.scale,
            invert_x: ms.invert_x,
            attachments,
            bone_weights: ms.bone_weights.clone(),
        }))
    }
}

impl VehicleModelSource for MediaVehicleModels<'_> {
    fn model(&self, name: &str) -> Option<Rc<VehicleModel>> {
        if let Some(m) = self.cache.borrow().get(name) {
            return m.clone();
        }
        let m = self.load(name);
        self.cache.borrow_mut().insert(name.to_string(), m.clone());
        m
    }
}

/// A visible part model: `BaseVehicle.ModelInfo` plus its
/// `VehicleSubModelInstance`.
pub struct PartModel {
    pub part: PartRef,
    pub script_model: ScriptModel,
    pub wheel_index: i32,
    pub model: Option<Rc<VehicleModel>>,
    /// `ModelInfo.animPlayer` (skinned models).
    pub anim: Option<AnimationPlayer>,
    /// `ModelInfo.track`.
    pub track: Option<pz_anim::track::TrackId>,
    pub render_transform: Matrix4f,
}

/// A vehicle's model slot: the body instance, its textures and the part
/// models, with the transforms `updateTransform` keeps.
pub struct VehicleModelSet {
    pub body: Option<Rc<VehicleModel>>,
    pub instance: VehicleModelInstance,
    pub skin: VehicleSkinTextures,
    pub parts: Vec<PartModel>,
    /// `BaseVehicle.renderTransform`.
    pub render_transform: Matrix4f,
    /// `BaseVehicle.vehicleTransform`.
    pub vehicle_transform: Matrix4f,
}

/// `getModelScriptNameForPart`: the script model's file, or (for a model
/// without one) the installed item's `VehiclePartModel`, which the caller
/// resolves through `item_part_model(part, model id)`.
pub fn model_script_name_for_part(
    part: &PartRef,
    m: &ScriptModel,
    item_part_model: &dyn Fn(&PartRef, &str) -> Option<String>,
) -> Option<String> {
    match &m.file {
        Some(f) => Some(f.clone()),
        None => {
            if !part.has_item() {
                return None;
            }
            item_part_model(part, m.id.as_deref().unwrap_or(""))
        }
    }
}

impl VehicleModelSet {
    /// `ModelManager.addVehicle`: the body model, its skin textures, and a
    /// sub-model per `vehicle.models` entry.
    pub fn create(
        v: &BaseVehicle,
        source: &dyn VehicleModelSource,
        load_texture: &mut dyn FnMut(&str, i32) -> Option<Texture>,
        item_part_model: &dyn Fn(&PartRef, &str) -> Option<String>,
    ) -> Self {
        let body = v
            .script()
            .and_then(|s| s.borrow().model().and_then(|m| m.file.clone()))
            .and_then(|f| source.model(&f));
        let skin = VehicleSkinTextures::skin_of(v)
            .map(|s| VehicleSkinTextures::load(&s, load_texture))
            .unwrap_or_default();
        let instance = VehicleModelInstance {
            scale: body.as_ref().map_or(1.0, |b| b.scale),
            ..Default::default()
        };
        let mut set = VehicleModelSet {
            body,
            instance,
            skin,
            parts: Vec::new(),
            render_transform: Matrix4f::identity(),
            vehicle_transform: Matrix4f::identity(),
        };
        set.sync(v, source, item_part_model);
        set
    }

    /// Re-reads `vehicle.models` (`setModelVisible`'s `Remove`/`addVehicle`),
    /// keeping existing part models' animation state.
    pub fn sync(
        &mut self,
        v: &BaseVehicle,
        source: &dyn VehicleModelSource,
        item_part_model: &dyn Fn(&PartRef, &str) -> Option<String>,
    ) {
        let infos = v.st().models.clone();
        let mut old = std::mem::take(&mut self.parts);
        for info in infos {
            let Some(part) = info.part.upgrade() else {
                continue;
            };
            if let Some(i) = old
                .iter()
                .position(|p| Rc::ptr_eq(&p.part, &part) && p.script_model.id == info.model_id)
            {
                self.parts.push(old.remove(i));
                continue;
            }
            let Some(sm) = part.script_part().and_then(|sp| {
                sp.models
                    .as_ref()
                    .and_then(|m| m.get(info.model_index).cloned())
            }) else {
                continue;
            };
            let name = model_script_name_for_part(&part, &sm, item_part_model);
            let model = name.and_then(|n| source.model(&n));
            self.parts.push(PartModel {
                part,
                script_model: sm,
                wheel_index: info.wheel_index,
                model,
                anim: None,
                track: None,
                render_transform: Matrix4f::identity(),
            });
        }
    }

    /// The part model of `part` (`getModelInfoForPart`).
    pub fn part_model_index(&self, part: &PartRef) -> Option<usize> {
        self.parts.iter().position(|p| Rc::ptr_eq(&p.part, part))
    }

    /// `ModelInfo.getAnimationPlayer`: the parent part's player when the
    /// parent has a model, else the model's own (skinned models only).
    fn anim_owner(&mut self, i: usize) -> Option<usize> {
        if let Some(parent) = self.parts[i].part.parent() {
            if let Some(pi) = self.part_model_index(&parent) {
                return self.anim_owner(pi);
            }
        }
        let p = &mut self.parts[i];
        let model = p.model.as_ref()?;
        if model.is_static {
            return None;
        }
        let skinning = model.skinning.clone()?;
        if p.anim.is_none() {
            p.anim = Some(AnimationPlayer::new(skinning));
        }
        Some(i)
    }

    /// `BaseVehicle.playPartAnim(part, animId)`.
    pub fn play_part_anim(&mut self, part: &PartRef, anim_id: &str) {
        let Some(anim) = part.script_part().and_then(|sp| {
            sp.anims
                .as_ref()
                .and_then(|a| a.iter().find(|a| a.id.as_deref() == Some(anim_id)).cloned())
        }) else {
            return;
        };
        let Some(clip) = anim.anim.clone().filter(|a| !a.trim().is_empty()) else {
            return;
        };
        let Some(i) = self.part_model_index(part) else {
            return;
        };
        let Some(owner) = self.anim_owner(i) else {
            return;
        };
        let old = self.parts[i].track.take();
        let open_delta = part.st().window.map(|w| w.open_delta);
        let player = self.parts[owner].anim.as_mut().expect("player");
        if let Some(t) = old {
            player.remove_track(t);
        }
        if !player.skinning_data().animation_clips.contains_key(&clip) {
            return;
        }
        let Some(id) = player.play(&clip, anim.loop_, false, -1.0) else {
            return;
        };
        if let Some(t) = player.track_mut(id) {
            t.set_blend_weight(1.0);
            t.set_speed_delta(anim.rate);
            t.is_playing = anim.animate;
            t.reverse = anim.reverse;
            if let Some(d) = open_delta {
                let dur = t.get_duration();
                t.set_current_time_value(dur * d);
            }
        }
        self.parts[i].track = Some(id);
    }

    /// `BaseVehicle.postupdate`'s `updateAnimationPlayer` for each part
    /// model: advance (`1/60 * 0.8 * unmoddedMultiplier`), drop finished
    /// playing tracks, and start the door (`Opened`/`Closed`) or window
    /// (`ClosedToOpen`) pose when no track is set.
    pub fn update_animations(&mut self, unmodded_multiplier: f32) {
        let del = 0.016_666_668f32 * 0.8 * unmodded_multiplier;
        for i in 0..self.parts.len() {
            let Some(owner) = self.anim_owner(i) else {
                continue;
            };
            {
                let player = self.parts[owner].anim.as_mut().expect("player");
                player.update(del, None);
                let done: Vec<_> = player
                    .tracks()
                    .iter()
                    .filter(|t| t.is_playing && t.is_finished())
                    .map(|t| t.id)
                    .collect();
                for t in done {
                    player.remove_track(t);
                }
            }
            if let Some(t) = self.parts[i].track {
                let alive = self.parts[owner]
                    .anim
                    .as_ref()
                    .is_some_and(|p| p.tracks().iter().any(|x| x.id == t));
                if !alive {
                    self.parts[i].track = None;
                }
            }
            let part = self.parts[i].part.clone();
            if let Some(t) = self.parts[i].track {
                if let Some(w) = part.st().window {
                    if let Some(tr) = self.parts[owner].anim.as_mut().and_then(|p| p.track_mut(t)) {
                        let dur = tr.get_duration();
                        tr.set_current_time_value(dur * w.open_delta);
                    }
                }
                continue;
            }
            let door = part.st().door;
            if let Some(d) = door {
                self.play_part_anim(&part, if d.open { "Opened" } else { "Closed" });
            }
            let window = part.st().window;
            if window.is_some() {
                self.play_part_anim(&part, "ClosedToOpen");
            }
        }
    }

    /// The animation player a part model is drawn with.
    pub fn part_player(&self, i: usize) -> Option<&AnimationPlayer> {
        let mut i = i;
        loop {
            match self.parts[i]
                .part
                .parent()
                .and_then(|p| self.part_model_index(&p))
            {
                Some(pi) => i = pi,
                None => return self.parts[i].anim.as_ref(),
            }
        }
    }
}
