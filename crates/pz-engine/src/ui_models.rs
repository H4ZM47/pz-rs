//! The engine's [`UIModelRenderer`]: `UI3DModel` avatars (character
//! creation's preview and friends) built from their `AnimatedModel` state
//! — the human visual and worn items as a [`ModelSlot`] with composited
//! textures, posed by the model's anim set and action group — and drawn
//! with [`ui_model_draws`] through the UI's own camera.
//!
//! The assets load on first use: the media index, the body skeletons with
//! the anim set's clips (from `anims_X/Bob` and `anims_X/Kate`), the
//! hair/beard styles and clothing item files. Texture compositing
//! (`ModelInstanceTextureCreator`) needs the GPU, so a rebuilt model's
//! jobs wait for [`UiModels::prepare`], which the frame calls before the UI
//! renders; until then the model is not ready and draws nothing, as the
//! original skips a model whose texture creator has not rendered.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_anim::AnimationSet;
use pz_character_model::combine::run_jobs;
use pz_character_model::data::{
    ClothingItem, HumanModel, ModelScriptRef, Population, ResolvedItemVisual,
};
use pz_character_model::items::EquippedModels;
use pz_character_model::model_tree::{CharacterModelInput, ModelSlot, TextureSource};
use pz_character_model::resources::{MediaModelResources, TextureLoader};
use pz_character_model::texture_creator::{create_textures, SmartTextureJob, TextureCreatorInput};
use pz_character_model::ui_model::{ui_model_draws, AvatarAnimation, UiModelCamera};
use pz_characters::character::state::action::ActionGroups;
use pz_fs::GameFs;
use pz_lua_api::bind_ui3d::{AnimatedModel, UIModelRenderer, UIModelView};
use pz_model::assets::{resolve_model_file, AssetKind, MediaIndex};
use pz_model::loader::{load_animation, load_mesh, MeshParams};
use pz_model::SkinningData;
use pz_render::image::ImageData;
use pz_render::{flags, DrawCmd, Texture, TextureManager};

/// `GameTime.getTimeDelta()` with `fpsMultiplier` at 100, as
/// `setModelData` and `renderMain` force it for an instant pose.
const DT100: f32 = 100.0 / 60.0;

/// Texture files through the shared texture manager.
struct Files {
    tm: Rc<RefCell<TextureManager>>,
    cache: RefCell<HashMap<PathBuf, Option<Texture>>>,
}

impl TextureLoader for Files {
    fn file(&self, path: &Path) -> Option<Texture> {
        if let Some(t) = self.cache.borrow().get(path) {
            return t.clone();
        }
        let t = std::fs::File::open(path)
            .ok()
            .and_then(|f| ImageData::from_png(std::io::BufReader::new(f), false).ok())
            .map(|img| {
                let (w, h) = (img.width, img.height);
                let name = path.to_string_lossy();
                self.tm
                    .borrow_mut()
                    .texture_from_pixels(&name, w, h, img.data.pixels, 0)
            });
        self.cache
            .borrow_mut()
            .insert(path.to_path_buf(), t.clone());
        t
    }
    fn smart(&self, _job: usize) -> Option<Texture> {
        None
    }
}

/// One avatar.
struct Avatar {
    serial: u64,
    slot: ModelSlot,
    anim: AvatarAnimation,
    /// Texture jobs still to run (`textureCreator` not rendered).
    jobs: Option<Vec<SmartTextureJob>>,
}

/// The loaded assets.
struct Assets {
    game_dir: PathBuf,
    media: &'static MediaIndex,
    res: MediaModelResources<'static>,
    pop: Population,
    actions: ActionGroups,
    sets: HashMap<String, Rc<AnimationSet>>,
    /// Body skeleton with the clips, per sex.
    skeletons: HashMap<bool, Rc<SkinningData>>,
    clothing: HashMap<String, Option<Rc<ClothingItem>>>,
}

/// All `UI3DModel`s' models.
pub struct UiModels {
    game_dir: PathBuf,
    fs: Rc<RefCell<GameFs>>,
    tm: Rc<RefCell<TextureManager>>,
    assets: Option<Assets>,
    avatars: HashMap<u64, Avatar>,
    /// `GameTime.getTimeDelta()` for this frame.
    time_delta: f32,
}

impl UiModels {
    pub fn new(game_dir: &Path, fs: Rc<RefCell<GameFs>>, tm: Rc<RefCell<TextureManager>>) -> Self {
        UiModels {
            game_dir: game_dir.to_path_buf(),
            fs,
            tm,
            assets: None,
            avatars: HashMap::new(),
            time_delta: 1.0 / 60.0,
        }
    }

    fn assets(&mut self) -> Option<&mut Assets> {
        if self.assets.is_none() {
            let media: &'static MediaIndex =
                Box::leak(Box::new(MediaIndex::scan(&self.game_dir).ok()?));
            let files: &'static Files = Box::leak(Box::new(Files {
                tm: self.tm.clone(),
                cache: RefCell::default(),
            }));
            let mut pop = Population::default();
            pop.init(&mut self.fs.borrow_mut());
            self.assets = Some(Assets {
                game_dir: self.game_dir.clone(),
                media,
                res: MediaModelResources::new(media, files),
                pop,
                actions: ActionGroups::from_media(self.game_dir.join("media")),
                sets: HashMap::new(),
                skeletons: HashMap::new(),
                clothing: HashMap::new(),
            });
        }
        self.assets.as_mut()
    }

    /// The frame's `GameTime.getTimeDelta()` (seconds at the reference
    /// rate: `fpsMultiplier / 60`).
    pub fn set_time_delta(&mut self, dt: f32) {
        self.time_delta = dt;
    }

    /// Runs the texture jobs of rebuilt avatars on the combiner (the GPU).
    pub fn prepare(&mut self, combiner: &mut dyn pz_render::combiner::Combiner) {
        let fs = self.fs.clone();
        let tm = self.tm.clone();
        let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
        for a in self.avatars.values_mut() {
            let Some(jobs) = a.jobs.take() else { continue };
            let mut tm = tm.borrow_mut();
            let fs = fs.borrow();
            let mut file =
                |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(&fs, p, model_flags);
            let done = run_jobs(&jobs, &mut file, combiner, &mut tm);
            for inst in &mut a.slot.instances {
                if let TextureSource::Smart(j) = inst.tex {
                    inst.tex = match done.get(j).cloned().flatten() {
                        Some(t) => TextureSource::Combined(t),
                        None => TextureSource::None,
                    };
                }
            }
        }
    }
}

impl Assets {
    /// `AnimationSet.GetAnimationSet(name)`: `media/AnimSets/<name>`.
    fn anim_set(&mut self, name: &str) -> Rc<AnimationSet> {
        let dir = self.game_dir.join("media").join("AnimSets").join(name);
        self.sets
            .entry(name.to_lowercase())
            .or_insert_with(|| Rc::new(AnimationSet::load(name, &dir)))
            .clone()
    }

    /// The body's skeleton (`HumanVisual.getModel()`) with every clip the
    /// anim set's nodes name.
    fn skeleton(&mut self, female: bool, set: &AnimationSet) -> Option<Rc<SkinningData>> {
        if let Some(s) = self.skeletons.get(&female) {
            return Some(s.clone());
        }
        let body = if female {
            "Skinned/FemaleBody"
        } else {
            "Skinned/MaleBody"
        };
        let (path, ty) = resolve_model_file(self.media, body, AssetKind::Mesh)?;
        let scene = load_mesh(
            &std::fs::read(path).ok()?,
            ty.import_format()?,
            &MeshParams::default(),
        )
        .ok()?;
        let mut sd = scene.skeleton.clone()?;
        let mut names: Vec<&str> = Vec::new();
        for st in set.states.values() {
            for n in st.nodes.iter().chain(&st.abstract_nodes) {
                names.push(&n.anim_name);
                names.extend(n.alternate_anims.iter().map(String::as_str));
            }
        }
        names.sort_unstable();
        names.dedup();
        for name in names.into_iter().filter(|n| !n.trim().is_empty()) {
            let found = ["Bob", "Kate"].iter().find_map(|f| {
                resolve_model_file(self.media, &format!("{f}/{name}"), AssetKind::Animation)
            });
            let Some((p, ty)) = found else { continue };
            let (Ok(data), Some(fmt)) = (std::fs::read(&p), ty.import_format()) else {
                continue;
            };
            if let Ok(clips) = load_animation(&data, fmt, Some(&sd)) {
                sd.animation_clips.extend(clips);
            }
        }
        let sd = Rc::new(sd);
        self.skeletons.insert(female, sd.clone());
        Some(sd)
    }

    /// `OutfitManager.getClothingItem(name)` from
    /// `media/clothing/clothingItems/<name>.xml`.
    fn clothing_item(&mut self, name: &str) -> Option<Rc<ClothingItem>> {
        if name.trim().is_empty() {
            return None;
        }
        let path = self
            .media
            .get(&format!("media/clothing/clothingItems/{name}.xml"))
            .cloned();
        self.clothing
            .entry(name.to_string())
            .or_insert_with(|| {
                let xml = std::fs::read_to_string(path?).ok()?;
                ClothingItem::parse(name, "", xml.trim_start_matches('\u{feff}')).map(Rc::new)
            })
            .clone()
    }
}

/// `setModelData`: the avatar for `m`'s current visual.
fn build(a: &mut Assets, m: &mut AnimatedModel) -> Option<Avatar> {
    let v = m.visual.clone()?;
    let set = a.anim_set(&m.anim_set_name);
    let sd = a.skeleton(v.female, &set)?;
    let mut anim = AvatarAnimation::new(sd, set);
    let visuals: Vec<ResolvedItemVisual> = v
        .items
        .iter()
        .map(|iv| {
            let c = a.clothing_item(&iv.clothing_item_name);
            let mut r = ResolvedItemVisual::new(&iv.full_type, c);
            r.visual = iv.clone();
            r
        })
        .collect();
    let mut hv = v.human.clone();
    let styles = a.pop.styles(&[], 0.0);
    let human = HumanModel::resolve(&mut hv, v.female, v.zombie, v.skeleton, &styles, &[]);
    let chars = pz_lua_api::bind_characters::shared_characters();
    let group = chars.borrow_mut().body_locations.get_group("Human").ok()?;
    let group = group.borrow();
    let scripts: HashMap<String, ModelScriptRef> = HashMap::new();
    let player = &anim.player;
    let bone_index = |n: &str| player.skinning_bone_index(n, -1);
    let mut slot = ModelSlot::build(&CharacterModelInput {
        human: &human,
        item_visuals: &visuals,
        equipped: &EquippedModels::default(),
        body_locations: &group,
        hair_styles: a.pop.hair_styles.as_ref(),
        beard_styles: a.pop.beard_styles.as_ref(),
        scripts: &scripts,
        body_model_script: None,
        bone_index: &bone_index,
    });
    let jobs = create_textures(
        &mut slot,
        &TextureCreatorInput {
            human: &human,
            item_visuals: &visuals,
            body_locations: &group,
            decals: &|_| None,
            simple_clothing_textures: false,
        },
    );
    let ag = a.actions.get(&m.anim_set_name);
    anim.reset(&ag, m.state.as_deref(), m.animate, &mut m.vars, DT100);
    Some(Avatar {
        serial: m.model_serial,
        slot,
        anim,
        jobs: Some(jobs.jobs),
    })
}

impl UIModelRenderer for UiModels {
    fn render(&mut self, key: u64, m: &mut AnimatedModel, view: &UIModelView) -> Vec<DrawCmd> {
        let rebuild = self
            .avatars
            .get(&key)
            .is_none_or(|a| a.serial != m.model_serial);
        if rebuild {
            let Some(a) = self.assets() else {
                return Vec::new();
            };
            match build(a, m) {
                Some(av) => {
                    self.avatars.insert(key, av);
                }
                None => {
                    self.avatars.remove(&key);
                    m.events.clear();
                    return Vec::new();
                }
            }
        }
        let dt = self.time_delta;
        let Some(av) = self.avatars.get_mut(&key) else {
            return Vec::new();
        };
        // updateInternal.
        for e in std::mem::take(&mut m.events) {
            av.anim.report_event(&e);
        }
        if m.animate {
            av.anim.update(m.angle, &mut m.vars, dt);
        } else {
            av.anim.update_still(m.angle, &mut m.vars, DT100, 0.0);
        }
        if av.jobs.is_some() {
            return Vec::new();
        }
        let cam = UiModelCamera {
            viewport: view.viewport,
            size_v: view.size_v,
            anim_player_angle: av.anim.player.rendered_angle(),
            isometric: m.isometric,
            flip_y: false,
            offset: m.offset,
        };
        let Some(a) = self.assets.as_ref() else {
            return Vec::new();
        };
        ui_model_draws(&av.slot, &av.anim.player, &cam, view.screen, &a.res)
    }

    fn release(&mut self, key: u64) {
        self.avatars.remove(&key);
    }
}

/// [`UiModels`] shared between the Lua thread's renderer slot and the frame.
pub struct SharedUiModels(pub Rc<RefCell<UiModels>>);

impl UIModelRenderer for SharedUiModels {
    fn render(&mut self, key: u64, m: &mut AnimatedModel, view: &UIModelView) -> Vec<DrawCmd> {
        self.0.borrow_mut().render(key, m, view)
    }
    fn release(&mut self, key: u64) {
        if let Ok(mut u) = self.0.try_borrow_mut() {
            u.release(key);
        }
    }
}

/// Install the UI model renderer; the frame calls `prepare` on the result.
pub fn install(game_dir: &Path, fs: Rc<RefCell<GameFs>>) -> Rc<RefCell<UiModels>> {
    let tm = pz_lua_api::bind_render::textures();
    let u = Rc::new(RefCell::new(UiModels::new(game_dir, fs, tm)));
    pz_lua_api::bind_ui3d::install_renderer(Some(Box::new(SharedUiModels(u.clone()))));
    u
}
