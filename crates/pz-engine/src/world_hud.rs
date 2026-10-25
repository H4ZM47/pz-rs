//! The HUD hosts the world renderer feeds: `hud::picking::PickWorld` over
//! pz-world-render's [`WorldPick`] (the squares and the chunk renderer's
//! recorded `ObjectRenderInfo`s) and `hud::host::HudCamera` over the
//! frame's [`SharedCameraFrame`].

use pz_iso_render::cell::Layer;
use pz_kahlua::Value;
use pz_lua_api::gizmo::{GizmoCamera, GizmoFrame, GizmoPrimitive};
use pz_lua_api::hud::host::{with_host, HudCamera, HudGizmoCamera};
use pz_lua_api::hud::picking::{
    PickKind, PickObject, PickPlayer, PickWorld, RenderInfo, RenderLayer, SpriteFlags,
};
use pz_render::vbo::VboRun;
use pz_render::DrawCmd;
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

impl HudGizmoCamera for WorldHudCamera {
    /// `IsoCamera.frameState.camCharacterX/Y/Z` and `zoom`, and
    /// `IsoCamera.cameras[0]`'s right-click, target, current and deferred
    /// offsets.
    fn gizmo_camera(&self) -> GizmoCamera {
        let f = self.0.borrow();
        let (cam_character, frame_zoom) = f.frame_camera();
        let o = f.offsets[0];
        GizmoCamera {
            cam_character,
            frame_zoom,
            right_click: o.right_click,
            t_off: f.t_offset(0),
            off: f.offset(0),
            defered: o.defered,
        }
    }
}

/// Installs `frame` as the HUD's camera and gizmo camera
/// (`HudHost::camera`, `HudHost::gizmo_camera`), leaving the other HUD
/// inputs as they are.
pub fn install_hud_camera(frame: SharedCameraFrame) {
    with_host(|h| {
        h.camera = Box::new(WorldHudCamera(frame.clone()));
        h.gizmo_camera = Box::new(WorldHudCamera(frame));
    });
}

/// `Gizmos.SceneDrawer.render` (`Scene.render`) for a scene's frame: each
/// primitive's `VBORenderer` runs under `projection * model-view`.
pub fn gizmo_draws(frame: &GizmoFrame) -> Vec<DrawCmd> {
    let mut out = Vec::new();
    let mvp = |mv: &pz_joml::Matrix4f| frame.projection.mul(mv).m;
    let mut push =
        |runs: Vec<VboRun>| out.extend(runs.into_iter().map(|r| DrawCmd::VboRun(Box::new(r))));
    for p in &frame.primitives {
        match p {
            GizmoPrimitive::Axis {
                model_view,
                length,
                flip,
            } => {
                push(vec![pz_render::vbo::debug_draw_axis(
                    mvp(model_view),
                    *length,
                    *flip,
                )]);
            }
            GizmoPrimitive::Cylinder {
                model_view,
                base,
                top,
                height,
                slices,
                stacks,
                color,
            } => {
                push(pz_render::vbo::cylinder_fill(
                    mvp(model_view),
                    *base,
                    *top,
                    *height,
                    *slices,
                    *stacks,
                    *color,
                ));
            }
            GizmoPrimitive::Torus {
                model_view,
                r,
                c,
                sides,
                rings,
                color,
                camera_ray,
            } => {
                push(pz_render::vbo::torus(
                    mvp(model_view),
                    f64::from(*r),
                    f64::from(*c),
                    *sides,
                    *rings,
                    *color,
                    camera_ray.direction,
                ));
            }
        }
    }
    out
}

/// `Gizmos.getInstance().render(IsoCamera.frameState.playerIndex)` in
/// `IsoWorld.render`, right after the cell: the frame player's gizmo, if
/// one is visible, drawn into the current (offscreen) target.
pub fn render_gizmos(player: usize, out: &mut Vec<DrawCmd>) {
    if let Some(frame) = pz_lua_api::gizmo::render(player) {
        out.extend(gizmo_draws(&frame));
    }
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

    #[test]
    fn gizmo_camera_reads_frame_state_and_camera_0() {
        let frame = SharedCameraFrame::default();
        install_hud_camera(frame.clone());
        let p0 = Camera::centred(100.0, 200.0, 0.0, 1280, 720, 1.5);
        let p1 = Camera::centred(50.0, 60.0, 1.0, 640, 360, 0.75);
        {
            let mut f = frame.borrow_mut();
            f.cameras[0] = Some(p0);
            f.cameras[1] = Some(p1);
            f.offsets[0] = pz_world_render::PlayerCameraOffsets {
                right_click: (3.0, -2.0),
                t_off: None,
                defered: (0.25, -0.5),
            };
            f.frame_player_index = 1;
        }
        let g = with_host(|h| h.gizmo_camera.gizmo_camera());
        // frameState is the frame player's; the offsets are cameras[0]'s.
        assert_eq!((g.cam_character, g.frame_zoom), ([50.0, 60.0, 1.0], 0.75));
        let off0 = (p0.off_x as f32, p0.off_y as f32);
        assert_eq!((g.off, g.t_off), (off0, off0));
        assert_eq!((g.right_click, g.defered), ((3.0, -2.0), (0.25, -0.5)));
        frame.borrow_mut().offsets[0].t_off = Some((7.0, 8.0));
        assert_eq!(
            with_host(|h| h.gizmo_camera.gizmo_camera()).t_off,
            (7.0, 8.0)
        );
    }

    fn px(p: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * w + x) * 4) as usize;
        [p[i], p[i + 1], p[i + 2]]
    }

    /// A synthetic `GizmoFrame` (orthographic projection, the translate
    /// gizmo's axis lines and an arrow, the rotate gizmo's z ring) drawn
    /// through `gizmo_draws` on the GPU.
    #[test]
    fn gizmo_frame_draws_axis_arrow_and_ring() {
        use pz_joml::Matrix4f;
        let Ok(mut g) = pz_gpu::Gpu::new_headless(64, 64) else {
            eprintln!("skipping: no GPU");
            return;
        };
        let projection = Matrix4f::set_ortho(-1.0, 1.0, -1.0, 1.0, -1.0, 1.0);
        // The origin on a pixel centre: (32.5, 31.5) from the top left.
        let mv = Matrix4f::translation(1.0 / 64.0, 1.0 / 64.0, 0.0);
        let frame = GizmoFrame {
            bounds: (0, 0, 64, 64),
            projection,
            primitives: vec![
                GizmoPrimitive::Torus {
                    model_view: mv,
                    r: 0.02,
                    c: 0.4,
                    sides: 8,
                    rings: 32,
                    color: [0.0, 0.0, 1.0],
                    camera_ray: pz_lua_api::gizmo::math::Ray {
                        origin: [0.0; 3],
                        direction: [0.0, 0.0, -1.0],
                        t: 0.0,
                    },
                },
                GizmoPrimitive::Axis {
                    model_view: mv,
                    length: 0.5,
                    flip: [false; 3],
                },
                // A green arrow shaft along x (z rotated onto +x).
                GizmoPrimitive::Cylinder {
                    model_view: mv
                        .mul(&Matrix4f::rotation(1.5707964, 0.0, 1.0, 0.0))
                        .translate(0.0, 0.0, -0.75),
                    base: 0.1,
                    top: 0.1,
                    height: 0.5,
                    slices: 8,
                    stacks: 1,
                    color: [0.0, 1.0, 0.0, 1.0],
                },
            ],
        };
        let mut dl = pz_render::DrawList::default();
        dl.cmds.push(DrawCmd::GlEnable(pz_render::gl::BLEND));
        dl.cmds.push(DrawCmd::BlendFunc(
            pz_render::gl::SRC_ALPHA,
            pz_render::gl::ONE_MINUS_SRC_ALPHA,
        ));
        dl.cmds.extend(gizmo_draws(&frame));
        let params = pz_gpu::FrameParams {
            clear: Some([0.0, 0.0, 0.0, 1.0]),
            ..pz_gpu::FrameParams::ui(64, 64)
        };
        let mut tm = pz_render::TextureManager::new();
        g.render_frame(&[(params, &dl)], &mut tm);
        let p = g.read_pixels();
        // Red x axis along row 31 from column 33, green y axis up column 32.
        assert_eq!(px(&p, 64, 40, 31), [255, 0, 0]);
        assert_eq!(px(&p, 64, 32, 24), [0, 255, 0]);
        // The ring (radius 2 * 0.4 = 0.8, 25.6 pixels) in blue.
        assert_eq!(px(&p, 64, 32, 6), [0, 0, 255]);
        assert_eq!(px(&p, 64, 6, 31), [0, 0, 255]);
        // The shaft: x from -0.75 to -0.25 (columns 8..24), 0.1 either side.
        assert_eq!(px(&p, 64, 16, 31), [0, 255, 0]);
        assert_eq!(px(&p, 64, 16, 28), [0, 255, 0]);
        assert_eq!(px(&p, 64, 16, 20), [0, 0, 0]);
    }
}
