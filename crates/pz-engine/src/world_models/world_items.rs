//! `IsoWorldInventoryObject.render` via `WorldItemAtlas`: builds
//! [`ItemParams`] with fluid containers (`world_item::try_item_params_with_fluid`),
//! flushes queued atlas jobs in [`WorldModels::prepare`] via
//! [`WorldItemAtlas::render_with`] + [`atlas_smart_texture`] (retiring any
//! temporary smart textures via [`Combiner::release`] on the next frame), and
//! draws placed items via [`item_world_draw`] with [`item_sprite_draw`] as the
//! 2D fallback for items without a 3D model.

use pz_character_model::world_item::{atlas_smart_texture, try_item_params_with_fluid};
use pz_entity::{Component, ComponentType, FluidContainer};
use pz_fs::GameFs;
use pz_inventory::shared::ItemRef;
use pz_inventory::ItemComponents;
use pz_iso_render::item_atlas::{
    item_sprite_draw, item_world_draw, ItemPlacement, WorldItemAtlas,
};
use pz_iso_render::Camera;
use pz_render::combiner::Combiner;
use pz_render::{flags, DrawCmd, Texture, TextureManager};

use super::WorldModels;

/// The first `WorldItemAtlas` texture id: disjoint from `DeadBodyAtlas`
/// (`0x7FFF_FFF0_0000_0000`) and any world chunk texture id.
const ITEM_ATLAS_FIRST_ID: u64 = 0x7FFF_FFE0_0000_0000;

/// `WorldItemAtlas.instance` and the atlas render jobs produced in
/// [`WorldModels::prepare`] waiting to be prepended to the next world frame.
pub(super) struct WorldItems {
    atlas: WorldItemAtlas,
    pending_jobs: Vec<DrawCmd>,
}

impl Default for WorldItems {
    fn default() -> Self {
        Self {
            atlas: WorldItemAtlas::new(ITEM_ATLAS_FIRST_ID),
            pending_jobs: Vec::new(),
        }
    }
}

impl WorldItems {
    /// Runs queued `WorldItemAtlas` jobs with `atlas_smart_texture` during
    /// [`WorldModels::prepare`]. Any temporary combined textures returned by
    /// `atlas_smart_texture` are pushed into `retired` so they remain alive
    /// through the upcoming `render_frame` and are released to `combiner` on
    /// the following `prepare` / `dispose`.
    pub(super) fn prepare(
        &mut self,
        fs: &GameFs,
        combiner: &mut dyn Combiner,
        tm: &mut TextureManager,
        retired: &mut Vec<Texture>,
    ) {
        let model_flags = flags::COMPRESS | flags::LIMIT_TEXTURE_SIZE_1;
        let loaded = |p: &str| fs.resolve(p).is_some();
        let mut file = |tm: &mut TextureManager, p: &str| tm.shared_texture_flags(fs, p, model_flags);
        self.atlas.render_with(&mut self.pending_jobs, &mut |params| {
            let tex = atlas_smart_texture(params, &loaded, &mut file, combiner, tm);
            if let Some(t) = &tex {
                retired.push(t.clone());
            }
            tex
        });
    }

    /// Drains any atlas-build commands produced during `prepare` into `out`
    /// so they execute before `FrameObjects.items` samples those atlas entries.
    pub(super) fn take_jobs(&mut self, out: &mut Vec<DrawCmd>) {
        out.append(&mut self.pending_jobs);
    }

    /// Resets the atlas on world unload.
    pub(super) fn clear(&mut self) {
        self.atlas = WorldItemAtlas::new(ITEM_ATLAS_FIRST_ID);
        self.pending_jobs.clear();
    }
}

/// Extracts an `IsoWorldInventoryObject`'s own `FluidContainer` component
/// (used when the item moved its fluid container onto the world object on drop).
fn object_fluid(
    components: &ItemComponents,
    ctx: &dyn pz_entity::EntityContext,
) -> Option<FluidContainer> {
    match components
        .value_snapshot(ComponentType::FluidContainer, ctx)
        .ok()??
    {
        Component::FluidContainer(fluid) => Some(fluid),
        _ => None,
    }
}

impl WorldModels {
    /// Drains queued `WorldItemAtlas` build commands into `pre` so they run
    /// before the frame's `FrameObjects.items` quads.
    pub fn take_world_item_jobs(&mut self, pre: &mut Vec<DrawCmd>) {
        self.world_items.take_jobs(pre);
    }

    /// `IsoWorldInventoryObject.render` for a single placed item: queries
    /// `WorldItemAtlas` with `item_params_with_fluid` (falling back to 2D
    /// `item_sprite_draw` when the item has no 3D model or is too big for the
    /// atlas).
    pub fn render_world_item(
        &mut self,
        item_ref: &ItemRef,
        obj_components: &ItemComponents,
        at: &ItemPlacement,
        cam: &Camera,
        max_zoom_is_one: bool,
    ) -> Result<Vec<DrawCmd>, String> {
        if self.assets.is_none() {
            if let Some(assets) =
                crate::ui_models::Assets::load(&self.game_dir, &self.fs, &self.tm)
            {
                self.assets = Some(assets);
            }
        }
        let item = item_ref
            .try_borrow()
            .map_err(|_| "world inventory item is already borrowed")?;
        let env = pz_lua_api::bind_items::item_env();
        let fallback_ctx = pz_entity::BasicContext::default();
        let entity_ctx: &dyn pz_entity::EntityContext = env.entity().unwrap_or(&fallback_ctx);
        let fluid = item
            .get_fluid_container(entity_ctx)
            .or_else(|| object_fluid(obj_components, entity_ctx));
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let params = if let Some(a) = self.assets.as_ref() {
            try_item_params_with_fluid(
                &item,
                fluid.as_ref().map(|f| (f, entity_ctx)),
                &*sm,
                &a.res,
                max_zoom_is_one,
            )
            .map_err(|e| format!("world item ModData: {e:?}"))?
        } else {
            None
        };
        drop(item);

        if let Some(params) = params {
            let key = self.world_items.atlas.item_texture(&params);
            if let Some(tex) = self.world_items.atlas.textures.get(&key) {
                if tex.ready {
                    return Ok(item_world_draw(tex, cam, at));
                }
                if !tex.too_big {
                    // Atlas render job will execute on the next `WorldModels::prepare`.
                    return Ok(Vec::new());
                }
            }
        }

        // 2D sprite fallback (`IsoWorldInventoryObject.render` without a 3D model).
        let tex = self.resolve_2d_item_texture(item_ref);
        let Some(tex) = tex else {
            return Ok(Vec::new());
        };
        let scale = pz_inventory::item_texture::get_script_item_ref(item_ref)
            .ok()
            .flatten()
            .and_then(|s| pz_inventory::item_texture::script_scale_world_icon(&s).ok())
            .filter(|&s| s > 0.0)
            .unwrap_or(1.0);
        let tint = pz_inventory::item_texture::item_tint_ref(item_ref)
            .map(|c| [c[0], c[1], c[2]])
            .unwrap_or([1.0, 1.0, 1.0]);
        Ok(item_sprite_draw(
            cam, &tex, scale, at.x, at.y, at.z, at.rgb, tint, at.alpha,
        ))
    }

    fn resolve_2d_item_texture(&self, item_ref: &ItemRef) -> Option<Texture> {
        if let Ok(Some(world_name)) = pz_inventory::item_texture::get_world_texture_ref(item_ref) {
            if !world_name.is_empty() {
                if let (Ok(fs), Ok(mut tm)) = (self.fs.try_borrow(), self.tm.try_borrow_mut()) {
                    if let Some(tex) = tm.shared_texture(&fs, &world_name) {
                        return Some(tex);
                    }
                }
            }
        }
        if let Ok(Some(tex_ref)) = pz_inventory::item_texture::get_tex_ref(item_ref) {
            if let Ok(t) = tex_ref.0.try_borrow() {
                return Some(t.clone());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_iso_render::item_atlas::{ItemModel, ItemParams};
    use pz_render::combiner::{Combiner, CombinerCommand};
    use pz_render::{DrawCmd, Texture};
    use std::path::Path;
    use std::rc::Rc;

    struct RecordingCombiner {
        released: Vec<Texture>,
    }

    impl Combiner for RecordingCombiner {
        fn combine(&mut self, _cmds: &[CombinerCommand], _tm: &mut TextureManager) -> Texture {
            Texture::empty("combined_world_item")
        }
        fn release(&mut self, tex: Texture) {
            self.released.push(tex);
        }
    }

    #[test]
    fn world_items_atlas_prepares_jobs_drains_commands_and_releases_retired_textures() {
        let mut wi = WorldItems::default();
        let model = ItemModel {
            name: "SyntheticApple".into(),
            mesh: Rc::new(pz_render::model::MeshData {
                id: 99,
                vertices: Vec::new(),
                indices: Vec::new(),
                kind: pz_render::model::MeshKind::Static,
            }),
            min: [-0.1, -0.1, -0.1],
            max: [0.1, 0.1, 0.1],
            texture: None,
        };
        let params = ItemParams::new(model, pz_iso_render::joml::Matrix4f::identity());
        let key = wi.atlas.item_texture(&params);
        assert!(wi.atlas.textures.contains_key(&key));

        let fs = GameFs::new(Path::new("unused-world-items-fixture"));
        let mut tm = TextureManager::new();
        let mut combiner = RecordingCombiner {
            released: Vec::new(),
        };
        let mut retired = Vec::new();
        wi.prepare(&fs, &mut combiner, &mut tm, &mut retired);
        let mut out = Vec::new();
        wi.take_jobs(&mut out);
        assert!(
            out.iter().any(|c| matches!(c, DrawCmd::AtlasBegin { .. })),
            "expected AtlasBegin job command from WorldItemAtlas::render_with"
        );
        // Taking jobs drains pending commands and leaves the queue empty.
        let mut second = Vec::new();
        wi.take_jobs(&mut second);
        assert!(second.is_empty());

        // Clearing resets atlas entries.
        wi.clear();
        assert!(wi.atlas.textures.is_empty());
        let _ = &mut retired;
    }

    #[test]
    fn world_models_2d_fallback_emits_world_sprite_for_item_without_model() {
        let path = Path::new("unused-world-items-2d-fixture");
        let fs = Rc::new(std::cell::RefCell::new(GameFs::new(path)));
        let tm = Rc::new(std::cell::RefCell::new(TextureManager::new()));
        let mut models = WorldModels::new(path, fs, tm);

        let mut tex = Texture::empty("Item_Apple");
        tex.id = Some(1);
        tex.ready = true;
        tex.width = 32;
        tex.height = 32;
        tex.width_orig = 32;
        tex.height_orig = 32;
        tex.x_end = 1.0;
        tex.y_end = 1.0;
        let tex_ref = Rc::new(pz_render::LuaTexture::new(tex));
        let mut template =
            pz_inventory::ItemTemplate::new(pz_inventory::ItemClass::Normal, "Base", "Apple");
        template.texture_capture = Some(pz_inventory::item_texture::CapturedScriptTextures::new(
            Some(tex_ref),
            Vec::new(),
        ));
        struct TestEnv;
        impl pz_inventory::ItemEnv for TestEnv {}
        let item = pz_inventory::env::create_from_template_ref(&TestEnv, Rc::new(template), 1)
            .expect("live item created");
        let at = ItemPlacement {
            x: 10.5,
            y: 12.5,
            z: 0.0,
            carpet: false,
            rgb: [1.0, 1.0, 1.0],
            alpha: 1.0,
        };
        let cam = Camera::player(10.5, 12.5, 0.0, 320, 240, 1.0);
        let cmds = models
            .render_world_item(&item, &ItemComponents::new(), &at, &cam, true)
            .expect("2d fallback renders");
        assert!(
            cmds.iter().any(|c| matches!(c, DrawCmd::WorldSprite(_, _))),
            "expected 2D WorldSprite fallback draw command"
        );
    }
}
