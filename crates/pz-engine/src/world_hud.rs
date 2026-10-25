//! The HUD hosts the world renderer feeds: `hud::picking::PickWorld` over
//! pz-world-render's [`WorldPick`] (the squares and the chunk renderer's
//! recorded `ObjectRenderInfo`s) and `hud::host::HudCamera` over the
//! frame's [`SharedCameraFrame`].

use pz_iso_render::cell::Layer;
use pz_kahlua::Value;
use pz_lua_api::hud::host::{with_host, HudCamera};
use pz_lua_api::hud::picking::{
    PickKind, PickObject, PickPlayer, PickWorld, RenderInfo, RenderLayer, SpriteFlags,
};
use pz_world::IsoGridSquare;
use pz_world_render::{ObjectPick, PickClass, SharedCameraFrame, WorldPick};

/// The picker's inputs owned by systems other than the world renderer.
pub trait PickHooks {
    /// `IsoPlayer.players[0]`; `None` picks nothing (characters thread).
    fn player0(&self) -> Option<PickPlayer>;
    /// The square's Lua object, for the tooltip.
    fn square_value(&self, _sq: &IsoGridSquare) -> Value {
        Value::Nil
    }
    /// `IsoDoor` / door `IsoThumpable`: `isAdjacentToSquare(player's
    /// current square)` for the object at `index` on `sq`.
    fn adjacent_to_player(&self, _sq: &IsoGridSquare, _index: usize) -> bool {
        false
    }
    /// `ClickObject.isInteractiveEntity` (the entity's `UiConfig`, through
    /// the multi-square master) for the object at `index` on `sq`.
    fn interactive_entity(&self, _sq: &IsoGridSquare, _index: usize) -> bool {
        false
    }
    /// The `IsoDeadBody`s among the square's `getStaticMovingObjects()`,
    /// with their `isMouseOver`.
    fn corpses(&self, _x: i32, _y: i32, _z: i32) -> Vec<PickObject> {
        Vec::new()
    }
    /// `DebugOptions.instance.fboRenderChunk.combinedFbo`.
    fn combined_fbo(&self) -> bool {
        false
    }
}

/// No player: nothing is picked.
pub struct NoPickHooks;

impl PickHooks for NoPickHooks {
    fn player0(&self) -> Option<PickPlayer> {
        None
    }
}

/// [`PickWorld`] for player 0 over the world renderer's state and `hooks`.
pub struct EnginePick<'a, H: PickHooks> {
    pub world: WorldPick<'a>,
    pub hooks: H,
}

fn layer(l: Layer) -> RenderLayer {
    match l {
        Layer::None => RenderLayer::None,
        Layer::WorldInventoryObject => RenderLayer::WorldInventoryObject,
        Layer::TranslucentFloor => RenderLayer::TranslucentFloor,
        Layer::Floor => RenderLayer::Floor,
        Layer::Vegetation => RenderLayer::Vegetation,
        Layer::MinusFloor => RenderLayer::MinusFloor,
        Layer::MinusFloorSE => RenderLayer::MinusFloorSE,
        Layer::Translucent => RenderLayer::Translucent,
        Layer::TranslucentSE => RenderLayer::TranslucentSE,
    }
}

fn kind(c: PickClass) -> PickKind {
    match c {
        PickClass::Object => PickKind::Object,
        PickClass::Door => PickKind::Door,
        PickClass::Thumpable {
            door,
            window,
            square_window_on_edge,
            light_source,
        } => PickKind::Thumpable {
            door,
            window,
            square_window_on_edge,
            light_source,
        },
        PickClass::Window => PickKind::Window,
        PickClass::WindowFrame { has_window } => PickKind::WindowFrame { has_window },
        PickClass::Curtain => PickKind::Curtain,
        PickClass::LightSwitch => PickKind::LightSwitch,
        PickClass::Generator => PickKind::Generator,
        PickClass::WaveSignal => PickKind::WaveSignal,
    }
}

impl<H: PickHooks> EnginePick<'_, H> {
    fn pick_object(&self, sq: &IsoGridSquare, o: ObjectPick) -> PickObject {
        let ri = o.render;
        let [x, y, z] = o.square;
        PickObject {
            // Identity: the square and the object's id there.
            id: (u64::from(x as u32) << 40)
                ^ (u64::from(y as u32) << 16)
                ^ (u64::from(z as u8) << 56)
                ^ u64::from(o.id),
            square: o.square,
            object_index: o.index as i32,
            kind: kind(o.class),
            sprite: o.sprite.map(|s| SpriteFlags {
                cut_w: s.cut_w,
                cut_n: s.cut_n,
                water: s.water,
                bed: s.bed,
                water_piped: s.water_piped,
                solid_floor: s.solid_floor,
                west_roof: s.west_roof,
            }),
            has_container: o.has_container,
            interactive_entity: self.hooks.interactive_entity(sq, o.index),
            adjacent_to_player: self.hooks.adjacent_to_player(sq, o.index),
            offset_x: o.offset_x,
            offset_y: o.offset_y,
            render: RenderInfo {
                layer: layer(ri.layer),
                target_alpha: ri.target_alpha,
                cutaway: ri.cutaway,
                render_x: ri.render_x,
                render_y: ri.render_y,
                render_width: ri.render_width,
                render_height: ri.render_height,
                render_scale_x: ri.render_scale_x,
                render_scale_y: ri.render_scale_y,
            },
            mask: o.mask,
            reroute_mask: None,
            have_special_tooltip: o.have_special_tooltip,
            square_value: self.hooks.square_value(sq),
            room: o.room,
            mouse_over: None,
        }
    }
}

impl<H: PickHooks> PickWorld for EnginePick<'_, H> {
    fn player0(&self) -> Option<PickPlayer> {
        self.hooks.player0()
    }

    fn zoom(&self) -> f32 {
        self.world.zoom()
    }

    fn camera_offset(&self) -> (f32, f32) {
        self.world.camera_offset()
    }

    fn objects(&self, x: i32, y: i32, z: i32) -> Option<Vec<PickObject>> {
        let sq = self.world.square(x, y, z)?;
        let objs = self.world.objects(x, y, z)?;
        Some(objs.into_iter().map(|o| self.pick_object(sq, o)).collect())
    }

    fn corpses(&self, x: i32, y: i32, z: i32) -> Vec<PickObject> {
        self.hooks.corpses(x, y, z)
    }

    fn chunk_render_xy(&self, square: [i32; 3]) -> Option<(f32, f32)> {
        self.world.chunk_render_xy(square)
    }

    fn combined_fbo(&self) -> bool {
        self.hooks.combined_fbo()
    }
}

/// [`HudCamera`] over the frame's iso cameras.
pub struct WorldHudCamera(pub SharedCameraFrame);

impl HudCamera for WorldHudCamera {
    fn off_x(&self) -> f32 {
        self.0.borrow().current_offset().0
    }

    fn off_y(&self) -> f32 {
        self.0.borrow().current_offset().1
    }

    fn zoom(&self, player: usize) -> f32 {
        self.0.borrow().zoom(player)
    }

    fn camera_offset(&self, player: usize) -> (f32, f32) {
        self.0.borrow().offset(player)
    }

    fn frame_player_index(&self) -> usize {
        self.0.borrow().frame_player_index
    }
}

/// Installs `frame` as the HUD's camera (`HudHost::camera`), leaving the
/// other HUD inputs as they are.
pub fn install_hud_camera(frame: SharedCameraFrame) {
    with_host(|h| h.camera = Box::new(WorldHudCamera(frame)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_iso_render::Camera;

    #[test]
    fn hud_camera_reads_the_frame() {
        let frame = SharedCameraFrame::default();
        install_hud_camera(frame.clone());
        assert_eq!(with_host(|h| h.camera.zoom(0)), 1.0);
        assert_eq!(with_host(|h| h.camera.off_x()), 0.0);
        let p0 = Camera::centred(100.0, 200.0, 0.0, 1280, 720, 1.5);
        let p1 = Camera::centred(50.0, 60.0, 1.0, 640, 360, 0.75);
        {
            let mut f = frame.borrow_mut();
            f.cameras[0] = Some(p0);
            f.cameras[1] = Some(p1);
            f.player_index = 1;
            f.frame_player_index = 0;
        }
        assert_eq!(
            with_host(|h| (h.camera.zoom(0), h.camera.zoom(1))),
            (1.5, 0.75)
        );
        // getOffX()/getOffY() follow IsoPlayer.getPlayerIndex().
        assert_eq!(
            with_host(|h| (h.camera.off_x(), h.camera.off_y())),
            (p1.off_x as f32, p1.off_y as f32)
        );
        assert_eq!(
            with_host(|h| h.camera.camera_offset(0)),
            (p0.off_x as f32, p0.off_y as f32)
        );
        assert_eq!(with_host(|h| h.camera.frame_player_index()), 0);
        frame.borrow_mut().frame_player_index = 1;
        assert_eq!(with_host(|h| h.camera.frame_player_index()), 1);
    }
}
