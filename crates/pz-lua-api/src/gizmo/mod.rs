//! `zombie.gizmo`: the translate and rotate gizmos of the extended
//! placement UI (`ISExtendedPlacementUI`), drawn over the world for a
//! player. The hit tests, mouse tracking and Lua callbacks live here; the
//! drawing is a list of [`GizmoPrimitive`]s for the render thread.
//!
//! In the original, `Gizmo`'s parent and origin are `final null` and
//! nothing sets its child or the scene's selected attachment, so those
//! transforms are identity and the line-to-origin and double-axis paths
//! never run; they are left out.

// The original's rounded angle constants (0.5235988, 1.5707964, ...).
#![allow(clippy::approx_constant)]

pub mod math;

use crate::hud::host::with_host;
use math::{closest_distance_between_lines, closest_distance_line_circle, Circle, Ray};
use pz_joml::vec3::{self, V};
use pz_joml::{Matrix4f, Quaternionf};
use pz_kahlua::{TableRef, Thread, Value};
use pz_ui::with_ui;
use std::cell::RefCell;
use std::rc::Rc;

/// `Core.scale`.
const CORE_SCALE: f32 = (std::f64::consts::SQRT_2 / 2.0 / 10.0) as f32 / 1.5;
/// `(float)Math.PI / 180`.
const DEG2RAD: f32 = std::f32::consts::PI / 180.0;

/// `zombie.gizmo.Axis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    None,
    X,
    Y,
    Z,
    XY,
    XZ,
    YZ,
}

/// `zombie.gizmo.TransformMode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformMode {
    Global,
    Local,
}

/// The camera values `Scene.calcMatrices` reads (`IsoCamera.frameState`
/// and `IsoCamera.cameras[0]`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GizmoCamera {
    /// `frameState.camCharacterX/Y/Z`.
    pub cam_character: [f32; 3],
    /// `frameState.zoom`.
    pub frame_zoom: f32,
    /// `cameras[0].rightClickX/Y`.
    pub right_click: (f32, f32),
    /// `cameras[0].getTOffX/Y()`.
    pub t_off: (f32, f32),
    /// `cameras[0].getOffX/Y()`.
    pub off: (f32, f32),
    /// `cameras[0].deferedX/Y`.
    pub defered: (f32, f32),
}

impl Default for GizmoCamera {
    fn default() -> Self {
        GizmoCamera {
            cam_character: [0.0; 3],
            frame_zoom: 1.0,
            right_click: (0.0, 0.0),
            t_off: (0.0, 0.0),
            off: (0.0, 0.0),
            defered: (0.0, 0.0),
        }
    }
}

fn tile_scale() -> i32 {
    crate::bind_core::shared_core().borrow().tile_scale
}

/// `IsoCamera.getScreenLeft/Top/Width/Height(player)`.
fn screen_rect(player: usize) -> (i32, i32, i32, i32) {
    let (sw, sh) = {
        let c = crate::bind_core::shared_core();
        let c = c.borrow();
        (c.get_screen_width(), c.get_screen_height())
    };
    let n = with_host(|h| h.players.num_players());
    let left = if player == 1 || player == 3 {
        sw / 2
    } else {
        0
    };
    let top = if player == 2 || player == 3 {
        sh / 2
    } else {
        0
    };
    (
        left,
        top,
        if n > 1 { sw / 2 } else { sw },
        if n > 2 { sh / 2 } else { sh },
    )
}

/// `StateData` (one copy: the main and render states coincide here).
#[derive(Clone, Debug)]
pub struct StateData {
    pub zoom_f: f32,
    pub projection: Matrix4f,
    pub model_view: Matrix4f,
    pub gizmo_translate: V,
    pub gizmo_rotate: V,
    pub gizmo_world_pos: V,
    pub gizmo_transform: Matrix4f,
    pub gizmo_axis: Axis,
    pub hide: [bool; 3],
    pub camera_ray: Ray,
    has_gizmo: bool,
}

impl Default for StateData {
    fn default() -> Self {
        StateData {
            zoom_f: 0.0,
            projection: Matrix4f::identity(),
            model_view: Matrix4f::identity(),
            gizmo_translate: [0.0; 3],
            gizmo_rotate: [0.0; 3],
            gizmo_world_pos: [0.0; 3],
            gizmo_transform: Matrix4f::identity(),
            gizmo_axis: Axis::None,
            hide: [false; 3],
            camera_ray: Ray::default(),
            has_gizmo: false,
        }
    }
}

/// What a gizmo draws, for the render thread. Matrices are the full
/// model-view at that point (the scene's `projection` applies on top).
#[derive(Clone, Debug, PartialEq)]
pub enum GizmoPrimitive {
    /// `Model.debugDrawAxis`: red, green and blue lines from the origin
    /// (`flip` negates an axis), one pixel wide, no depth test.
    Axis {
        model_view: Matrix4f,
        length: f32,
        flip: [bool; 3],
    },
    /// `VBORenderer.addCylinder_Fill(base, top, height, slices, stacks, r, g, b, a)`.
    Cylinder {
        model_view: Matrix4f,
        base: f32,
        top: f32,
        height: f32,
        slices: i32,
        stacks: i32,
        color: [f32; 4],
    },
    /// `VBORenderer.addTorus(r, c, sides, rings, r, g, b, cameraRay)` with
    /// depth testing off; the ray is in the torus' space.
    Torus {
        model_view: Matrix4f,
        r: f32,
        c: f32,
        sides: i32,
        rings: i32,
        color: [f32; 3],
        camera_ray: Ray,
    },
}

/// A scene's frame for the render thread.
#[derive(Clone, Debug, PartialEq)]
pub struct GizmoFrame {
    pub bounds: (i32, i32, i32, i32),
    pub projection: Matrix4f,
    pub primitives: Vec<GizmoPrimitive>,
}

/// `Gizmo` (with `TranslateGizmo` / `RotateGizmo`'s state).
pub struct Gizmo {
    pub kind: GizmoKind,
    pub visible: bool,
    pub transform_mode: TransformMode,
    pub rotate: V,
    pub world_pos: Rc<crate::bind_misc_classes::Vector3f>,
    pub rotation_obj: Rc<crate::bind_misc_classes::Vector3f>,
    pub scale: f32,
    pub axis_visible: [bool; 3],
    pub reverse_z_axis: bool,
    pub table: Option<TableRef>,
    track_axis: Axis,
    start_xfrm: Matrix4f,
    start_inv_xfrm: Matrix4f,
    start_pos: V,
    current_pos: V,
    start_translate: V,
    // Rotate.
    track_circle: Circle,
    start_rotate: V,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoKind {
    Translate,
    Rotate,
}

pub type GizmoRef = Rc<RefCell<Gizmo>>;

impl Gizmo {
    fn new(kind: GizmoKind) -> Self {
        Gizmo {
            kind,
            visible: false,
            transform_mode: if kind == GizmoKind::Rotate {
                TransformMode::Local
            } else {
                TransformMode::Global
            },
            rotate: [0.0; 3],
            world_pos: Rc::default(),
            rotation_obj: Rc::default(),
            scale: 1.0,
            axis_visible: [true; 3],
            reverse_z_axis: kind == GizmoKind::Translate,
            table: None,
            track_axis: Axis::None,
            start_xfrm: Matrix4f::identity(),
            start_inv_xfrm: Matrix4f::identity(),
            start_pos: [0.0; 3],
            current_pos: [0.0; 3],
            start_translate: [0.0; 3],
            track_circle: Circle::default(),
            start_rotate: [0.0; 3],
        }
    }

    pub fn world_position(&self) -> V {
        self.world_pos.get()
    }

    /// `setRotation(rx, ry, rz)`: stored as (rx, rz, ry).
    pub fn set_rotation(&mut self, rx: f32, ry: f32, rz: f32) {
        self.rotate = [rx, rz, ry];
        self.rotation_obj.set(self.rotate);
    }

    pub fn rotation(&self) -> V {
        self.rotation_obj.get()
    }
}

/// `zombie.gizmo.Scene`.
#[derive(Default)]
pub struct Scene {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    pub state: StateData,
    pub gizmo: Option<GizmoRef>,
    pub mouse_down: bool,
}


/// A Lua call the gizmo makes (`onTranslateGizmo` / `onRotateGizmo`),
/// run after the scene is released.
pub struct Callback {
    pub table: TableRef,
    pub name: &'static str,
    pub value: V,
}

impl Scene {
    pub fn set_bounds(&mut self, x: i32, y: i32, w: i32, h: i32) {
        (self.x, self.y, self.width, self.height) = (x, y, w, h);
    }

    /// `calcMatrices`.
    fn calc_matrices(&self) -> (Matrix4f, Matrix4f) {
        let cam = with_host(|h| h.gizmo_camera.gizmo_camera());
        let [cx, cy, cz] = cam.cam_character;
        let (mut x, mut y, z) = (cx, cy, cz);
        let ts = tile_scale() as f32;
        // PlayerCamera.XToIso / YToIso of (-tox - rcx, -toy - rcy) at floor 0.
        let sx = ((-cam.t_off.0 - cam.right_click.0) as i32) as f32;
        let sy = ((-cam.t_off.1 - cam.right_click.1) as i32) as f32;
        let (px, py) = (sx + cam.off.0, sy + cam.off.1);
        x -= (px + 2.0 * py) / (64.0 * ts) + 3.0 * 0.0;
        y -= (px - 2.0 * py) / (-64.0 * ts) + 3.0 * 0.0;
        x += cam.defered.0;
        y += cam.defered.1;
        let zoom = with_host(|h| h.camera.zoom(0));
        let (_, _, sw0, sh0) = screen_rect(0);
        let w = (sw0 as f32 * zoom / 1920.0) as f64;
        let h = (sh0 as f32 * zoom / 1920.0) as f64;
        let projection = Matrix4f::set_ortho(
            -(w as f32) / 2.0,
            w as f32 / 2.0,
            -(h as f32) / 2.0,
            h as f32 / 2.0,
            -1.0,
            1.0,
        );
        let s = CORE_SCALE * ts / 2.0;
        let dif_x = (cx - x) as f64;
        let dif_y = (cy - y) as f64;
        let mv = Matrix4f::identity()
            .scale(s, s, s)
            .rotate(0.5235988, 1.0, 0.0, 0.0)
            .rotate(2.3561945, 0.0, 1.0, 0.0)
            .translate(-(dif_x as f32), (cz - z) * 2.44949, -(dif_y as f32))
            .scale(-1.0, 1.0, 1.0)
            .translate(0.0, -0.71999997, 0.0);
        (projection, mv)
    }

    /// `setModelViewProjection` + `setGizmoTransforms`.
    fn prepare(&mut self) {
        let (p, mv) = self.calc_matrices();
        self.state.projection = p;
        self.state.model_view = mv;
        self.state.zoom_f = with_host(|h| h.gizmo_camera.gizmo_camera()).frame_zoom;
        let cam = with_host(|h| h.gizmo_camera.gizmo_camera()).cam_character;
        self.state.has_gizmo = self.gizmo.is_some();
        if let Some(g) = &self.gizmo {
            let g = g.borrow();
            let wp = g.world_position();
            let t = [
                wp[0] - cam[0],
                (wp[2] - cam[2]) * 3.0 * 0.8164967,
                -(wp[1] - cam[1]),
            ];
            self.state.gizmo_translate = t;
            let r = g.rotation();
            self.state.gizmo_rotate = r;
            self.state.gizmo_world_pos = wp;
            self.state.gizmo_transform = Matrix4f::translation(t[0], t[1], t[2]).rotate_xyz(
                r[0] * DEG2RAD,
                r[1] * DEG2RAD,
                r[2] * DEG2RAD,
            );
        }
    }

    fn viewport(&self) -> [i32; 4] {
        [0, 0, self.width, self.height]
    }

    /// `getCameraRay(uiX, uiY)` with the current matrices.
    pub fn camera_ray(&self, ui_x: f32, ui_y: f32) -> Ray {
        let inv = self.state.projection.mul(&self.state.model_view).invert();
        let start = inv.unproject_inv(ui_x, ui_y, 0.0, self.viewport());
        let end = inv.unproject_inv(ui_x, ui_y, 1.0, self.viewport());
        Ray {
            origin: start,
            direction: vec3::normalize(vec3::sub(end, start)),
            t: 0.0,
        }
    }

    /// `sceneToUIX/Y`.
    pub fn scene_to_ui(&self, p: V) -> (f32, f32) {
        let m = self.state.projection.mul(&self.state.model_view);
        let w = m.project(p[0], p[1], p[2], self.viewport());
        (w[0], self.height as f32 - w[1])
    }

    /// `renderMain()`: the matrices and the mouse's camera ray.
    pub fn render_main(&mut self) {
        self.prepare();
        let (mx, my) = with_ui(|m| (m.mouse.x, m.mouse.y));
        let (mx, my) = ((mx - self.x) as f32, (my - self.y) as f32);
        self.state.camera_ray = self.camera_ray(mx, self.height as f32 - my);
    }

    /// `hitTest(mouseX, mouseY)`: starts, continues or stops a drag on the
    /// gizmo; true while dragging.
    pub fn hit_test(&mut self, mouse_x: i32, mouse_y: i32, calls: &mut Vec<Callback>) -> bool {
        self.prepare();
        let Some(g) = self.gizmo.clone().filter(|g| g.borrow().visible) else {
            return self.mouse_down;
        };
        let (mx, my) = ((mouse_x - self.x) as f32, (mouse_y - self.y) as f32);
        let (down, was) = with_ui(|m| {
            (
                m.mouse.is_button_down(0),
                m.mouse.prev_down.first().copied().unwrap_or(false),
            )
        });
        let mut g = g.borrow_mut();
        if down {
            if self.mouse_down {
                self.update_tracking(&mut g, mx, my, calls);
            } else if !was {
                self.state.gizmo_axis = self.gizmo_hit_test(&g, mx, my);
                if self.state.gizmo_axis != Axis::None {
                    self.mouse_down = true;
                    let a = self.state.gizmo_axis;
                    self.start_tracking(&mut g, mx, my, a);
                }
            }
        } else {
            if self.mouse_down {
                self.mouse_down = false;
                g.track_axis = Axis::None;
            }
            self.state.gizmo_axis = self.gizmo_hit_test(&g, mx, my);
        }
        self.mouse_down
    }

    fn gizmo_xfrm(&self, g: &Gizmo) -> Matrix4f {
        let mut m = self.state.gizmo_transform;
        if g.transform_mode == TransformMode::Global {
            m = m.set_rotation_xyz(0.0, 0.0, 0.0);
        }
        m
    }

    fn gizmo_hit_test(&mut self, g: &Gizmo, ui_x: f32, ui_y: f32) -> Axis {
        if !g.visible {
            return Axis::None;
        }
        match g.kind {
            GizmoKind::Translate => self.translate_hit_test(g, ui_x, ui_y),
            GizmoKind::Rotate => self.rotate_hit_test(g, ui_x, ui_y),
        }
    }

    fn translate_hit_test(&mut self, g: &Gizmo, ui_x: f32, ui_y: f32) -> Axis {
        let xfrm = self.gizmo_xfrm(g);
        let ui_y = self.height as f32 - ui_y;
        let mut cam = self.camera_ray(ui_x, ui_y);
        let origin = xfrm.transform_position([0.0; 3]);
        let s = g.scale;
        let (length, thickness, offset) = (0.5 * s, 0.05 * s, 0.3 * s);
        let flip_z = if g.transform_mode == TransformMode::Global && g.reverse_z_axis {
            -1.0
        } else {
            1.0
        };
        let mut test = |i: usize, dir: V, cam: &mut Ray| {
            let mut axis = Ray {
                origin,
                direction: vec3::normalize(xfrm.transform_direction(dir)),
                t: 0.0,
            };
            let mut dist = closest_distance_between_lines(&mut axis, cam);
            let mut t = axis.t;
            if !g.axis_visible[i] || t < offset || t >= offset + length {
                t = f32::MAX;
                dist = f32::MAX;
            }
            let dot = vec3::dot(axis.direction, cam.direction);
            self.state.hide[i] = !g.axis_visible[i] || dot.abs() > 0.9;
            (dist, t)
        };
        let (dx, xt) = test(0, [1.0, 0.0, 0.0], &mut cam);
        let (dy, yt) = test(1, [0.0, 1.0, 0.0], &mut cam);
        let (dz, zt) = test(2, [0.0, 0.0, flip_z], &mut cam);
        let inside = |t: f32| t >= offset && t < offset + length;
        if inside(xt) && dx < dy && dx < dz {
            return if dx <= thickness / 2.0 {
                Axis::X
            } else {
                Axis::None
            };
        }
        if inside(yt) && dy < dx && dy < dz {
            return if dy <= thickness / 2.0 {
                Axis::Y
            } else {
                Axis::None
            };
        }
        if inside(zt) && dz < dx && dz < dy {
            return if dz <= thickness / 2.0 {
                Axis::Z
            } else {
                Axis::None
            };
        }
        Axis::None
    }

    /// `Gizmo.getPointOnAxis` for a single axis.
    fn point_on_axis(&self, g: &Gizmo, ui_x: f32, ui_y: f32, a: Axis, xfrm: &Matrix4f) -> V {
        let ui_y = self.height as f32 - ui_y;
        let mut cam = self.camera_ray(ui_x, ui_y);
        let dir = match a {
            Axis::X => [1.0, 0.0, 0.0],
            Axis::Y => [0.0, 1.0, 0.0],
            Axis::Z => [
                0.0,
                0.0,
                if g.transform_mode == TransformMode::Global && g.reverse_z_axis {
                    -1.0
                } else {
                    1.0
                },
            ],
            // The double-axis planes are never enabled in the original.
            _ => return [0.0; 3],
        };
        let mut axis = Ray {
            origin: xfrm.transform_position([0.0; 3]),
            direction: vec3::normalize(xfrm.transform_direction(dir)),
            t: 0.0,
        };
        closest_distance_between_lines(&mut axis, &mut cam);
        vec3::add(vec3::mul(axis.direction, axis.t), axis.origin)
    }

    fn start_tracking(&mut self, g: &mut Gizmo, ui_x: f32, ui_y: f32, a: Axis) {
        match g.kind {
            GizmoKind::Translate => {
                g.start_xfrm = self.gizmo_xfrm(g);
                g.start_inv_xfrm = g.start_xfrm.invert();
                g.track_axis = a;
                let x = g.start_xfrm;
                g.start_pos = self.point_on_axis(g, ui_x, ui_y, a, &x);
                g.start_translate = self.state.gizmo_world_pos;
            }
            GizmoKind::Rotate => {
                g.start_rotate = self.state.gizmo_rotate;
                g.start_xfrm = self.gizmo_xfrm(g);
                g.start_inv_xfrm = self.state.gizmo_transform.invert();
                g.track_axis = a;
                let x = g.start_xfrm;
                let (_, p) = self.rotate_point_on_axis(g, ui_x, ui_y, &x);
                g.start_pos = p;
            }
        }
    }

    fn update_tracking(&mut self, g: &mut Gizmo, ui_x: f32, ui_y: f32, calls: &mut Vec<Callback>) {
        match g.kind {
            GizmoKind::Translate => {
                let x = g.start_xfrm;
                let pos = self.point_on_axis(g, ui_x, ui_y, g.track_axis, &x);
                if vec3::bit_eq(g.current_pos, pos) {
                    return;
                }
                g.current_pos = pos;
                let delta = vec3::sub(g.current_pos, g.start_pos);
                if let Some(t) = &g.table {
                    let st = g.start_translate;
                    let v = [
                        st[0] + delta[0],
                        st[1] - delta[2],
                        st[2] + delta[1] / 2.44949,
                    ];
                    calls.push(Callback {
                        table: t.clone(),
                        name: "onTranslateGizmo",
                        value: v,
                    });
                }
            }
            GizmoKind::Rotate => {
                let x = g.start_xfrm;
                let (_, pos) = self.rotate_point_on_axis(g, ui_x, ui_y, &x);
                if vec3::bit_eq(g.current_pos, pos) {
                    return;
                }
                g.current_pos = pos;
                let mut angle = calculate_rotation(g.start_pos, g.current_pos, &g.track_circle);
                if crate::bind_input::is_key_down_raw(29) {
                    angle = if angle > 0.0 {
                        ((angle / 5.0) as i32 * 5) as f32
                    } else {
                        (java_round(-angle / 5.0) * -5) as f32
                    };
                }
                g.track_circle.orientation = match g.track_axis {
                    Axis::X => [1.0, 0.0, 0.0],
                    Axis::Y => [0.0, 1.0, 0.0],
                    Axis::Z => [0.0, 0.0, 1.0],
                    _ => g.track_circle.orientation,
                };
                let mut orientation = g.track_circle.orientation;
                // The original passes the mouse y unflipped here.
                let cam = self.camera_ray(ui_x, ui_y);
                let o2 = vec3::normalize(g.start_xfrm.transform_direction(orientation));
                if vec3::dot(cam.direction, o2) > 0.0 {
                    angle *= -1.0;
                }
                if g.transform_mode == TransformMode::Global {
                    orientation = g.start_inv_xfrm.transform_direction(orientation);
                }
                let q = Quaternionf::from_axis_angle_deg(orientation, angle);
                let delta = vec3::mul(q.euler_angles_xyz(), 57.295776);
                if let Some(t) = &g.table {
                    let sr = g.start_rotate;
                    let q0 = Quaternionf::rotation_xyz(
                        sr[0] * DEG2RAD,
                        sr[1] * DEG2RAD,
                        sr[2] * DEG2RAD,
                    );
                    let q1 = Quaternionf::rotation_xyz(
                        delta[0] * DEG2RAD,
                        delta[1] * DEG2RAD,
                        delta[2] * DEG2RAD,
                    );
                    let rot = vec3::mul(q0.mul(&q1).euler_angles_xyz(), 57.295776);
                    calls.push(Callback {
                        table: t.clone(),
                        name: "onRotateGizmo",
                        value: [rot[0], rot[2], rot[1]],
                    });
                }
            }
        }
    }

    /// `RotateGizmo.getPointOnAxis`: the closest point on the gizmo's
    /// screen-space circle (radius 0.5 in UI units, as in the original).
    fn rotate_point_on_axis(
        &self,
        g: &mut Gizmo,
        ui_x: f32,
        ui_y: f32,
        xfrm: &Matrix4f,
    ) -> (f32, V) {
        let c = xfrm.get_translation();
        let (cx, cy) = self.scene_to_ui(c);
        g.track_circle = Circle {
            center: [cx, cy, 0.0],
            orientation: [0.0, 0.0, 1.0],
            radius: 0.5,
        };
        let ray = Ray {
            origin: [ui_x, ui_y, 0.0],
            direction: [0.0, 0.0, -1.0],
            t: 0.0,
        };
        closest_distance_line_circle(&ray, &g.track_circle)
    }

    fn rotate_hit_test(&mut self, g: &Gizmo, ui_x: f32, ui_y: f32) -> Axis {
        let ui_y = self.height as f32 - ui_y;
        let cam = self.camera_ray(ui_x, ui_y);
        let mut xfrm = self.state.gizmo_transform;
        let s = xfrm.get_scale();
        xfrm = xfrm.scale(1.0 / s[0], 1.0 / s[1], 1.0 / s[2]);
        if g.transform_mode == TransformMode::Global {
            xfrm = xfrm.set_rotation_xyz(0.0, 0.0, 0.0);
        }
        let radius = 0.5 * g.scale;
        let center = xfrm.transform_project([0.0; 3]);
        let ax = vec3::normalize(xfrm.transform_direction([1.0, 0.0, 0.0]));
        let ay = vec3::normalize(xfrm.transform_direction([0.0, 1.0, 0.0]));
        let az = vec3::normalize(xfrm.transform_direction([0.0, 0.0, 1.0]));
        let dx = self.hit_test_circle(&cam, &circle_segments(center, radius, ay, az));
        let dy = self.hit_test_circle(&cam, &circle_segments(center, radius, ax, az));
        let dz = self.hit_test_circle(&cam, &circle_segments(center, radius, ax, ay));
        if dx < dy && dx < dz {
            return if dx <= 8.0 { Axis::X } else { Axis::None };
        }
        if dy < dx && dy < dz {
            return if dy <= 8.0 { Axis::Y } else { Axis::None };
        }
        if dz < dx && dz < dy {
            return if dz <= 8.0 { Axis::Z } else { Axis::None };
        }
        Axis::None
    }

    /// `hitTestCircle`: the screen distance from the camera ray's origin
    /// to the projected circle.
    fn hit_test_circle(&self, cam: &Ray, pts: &[V]) -> f32 {
        let (camx, camy) = self.scene_to_ui(cam.origin);
        let mut closest = f32::MAX;
        let mut p1 = pts[0];
        for &p2 in &pts[1..] {
            let (x1, y1) = self.scene_to_ui(p1);
            let (x2, y2) = self.scene_to_ui(p2);
            let len_sq = ((x2 - x1) as f64).powf(2.0) + ((y2 - y1) as f64).powf(2.0);
            if len_sq < 0.001 {
                p1 = p2;
                continue;
            }
            let u = ((camx - x1) * (x2 - x1) + (camy - y1) * (y2 - y1)) as f64 / len_sq;
            let (mut xu, mut yu) = (
                x1 as f64 + u * (x2 - x1) as f64,
                y1 as f64 + u * (y2 - y1) as f64,
            );
            if u <= 0.0 {
                (xu, yu) = (x1 as f64, y1 as f64);
            } else if u >= 1.0 {
                (xu, yu) = (x2 as f64, y2 as f64);
            }
            // IsoUtils.DistanceTo2D.
            let (ddx, ddy) = (camx - xu as f32, camy - yu as f32);
            let d = ((ddx * ddx + ddy * ddy) as f64).sqrt() as f32;
            if d < closest {
                closest = d;
            }
            p1 = p2;
        }
        closest
    }

    /// `render()` (the render-thread half): the gizmo's primitives.
    pub fn render(&self) -> Vec<GizmoPrimitive> {
        let Some(g) = self.gizmo.as_ref() else {
            return Vec::new();
        };
        let g = g.borrow();
        if !g.visible {
            return Vec::new();
        }
        let st = &self.state;
        let mut m = st.gizmo_transform;
        let s = m.get_scale();
        m = m.scale(1.0 / s[0], 1.0 / s[1], 1.0 / s[2]);
        if g.transform_mode == TransformMode::Global {
            m = m.set_rotation_xyz(0.0, 0.0, 0.0);
        }
        let mut out = Vec::new();
        match g.kind {
            GizmoKind::Translate => {
                let loaded = st.model_view.mul(&m);
                let flip_z = g.transform_mode == TransformMode::Global && g.reverse_z_axis;
                out.push(GizmoPrimitive::Axis {
                    model_view: loaded,
                    length: 0.5,
                    flip: [false, false, flip_z],
                });
                let s2 = g.scale;
                let (thickness, length, offset) = (0.05 * s2, 0.5 * s2, 0.3 * s2);
                let hl = |a: Axis, b: Axis, c: Axis| {
                    [a, b, c]
                        .iter()
                        .any(|x| st.gizmo_axis == *x || g.track_axis == *x)
                };
                let arrow = |local: Matrix4f, color: [f32; 4], out: &mut Vec<GizmoPrimitive>| {
                    out.push(GizmoPrimitive::Cylinder {
                        model_view: loaded.mul(&local),
                        base: thickness / 2.0,
                        top: thickness / 2.0,
                        height: length,
                        slices: 8,
                        stacks: 1,
                        color,
                    });
                    let tip = local.translate(0.0, 0.0, length);
                    out.push(GizmoPrimitive::Cylinder {
                        model_view: loaded.mul(&tip),
                        base: thickness / 2.0 * 2.0,
                        top: 0.0,
                        height: 0.1 * s2,
                        slices: 8,
                        stacks: 1,
                        color,
                    });
                };
                if !st.hide[0] {
                    let r = if hl(Axis::X, Axis::XY, Axis::XZ) {
                        1.0
                    } else {
                        0.5
                    };
                    arrow(
                        Matrix4f::rotation(1.5707964, 0.0, 1.0, 0.0).translate(0.0, 0.0, offset),
                        [r, 0.0, 0.0, 1.0],
                        &mut out,
                    );
                }
                if !st.hide[1] {
                    let gc = if hl(Axis::Y, Axis::XY, Axis::YZ) {
                        1.0
                    } else {
                        0.5
                    };
                    arrow(
                        Matrix4f::rotation(-1.5707964, 1.0, 0.0, 0.0).translate(0.0, 0.0, offset),
                        [0.0, gc, 0.0, 1.0],
                        &mut out,
                    );
                }
                if !st.hide[2] {
                    let b = if hl(Axis::Z, Axis::XZ, Axis::YZ) {
                        1.0
                    } else {
                        0.5
                    };
                    arrow(
                        Matrix4f::translation(0.0, 0.0, -offset).rotate_y(std::f32::consts::PI),
                        [0.0, 0.0, b, 1.0],
                        &mut out,
                    );
                }
            }
            GizmoKind::Rotate => {
                let radius = 0.5 * g.scale;
                let axis = if g.track_axis == Axis::None {
                    st.gizmo_axis
                } else {
                    g.track_axis
                };
                let torus = |am: Matrix4f, color: [f32; 3], out: &mut Vec<GizmoPrimitive>| {
                    let inv = am.invert();
                    let ray = Ray {
                        origin: inv.transform_position(st.camera_ray.origin),
                        direction: inv.transform_direction(st.camera_ray.direction),
                        t: st.camera_ray.t,
                    };
                    out.push(GizmoPrimitive::Torus {
                        model_view: st.model_view.mul(&am),
                        r: 0.01 * g.scale,
                        c: radius / 2.0,
                        sides: 8,
                        rings: 32,
                        color,
                        camera_ray: ray,
                    });
                };
                if g.track_axis == Axis::None || g.track_axis == Axis::X {
                    torus(
                        m.rotate_y(1.5707964),
                        [if axis == Axis::X { 1.0 } else { 0.5 }, 0.0, 0.0],
                        &mut out,
                    );
                }
                if g.track_axis == Axis::None || g.track_axis == Axis::Y {
                    torus(
                        m.rotate_x(1.5707964),
                        [0.0, if axis == Axis::Y { 1.0 } else { 0.5 }, 0.0],
                        &mut out,
                    );
                }
                if g.track_axis == Axis::None || g.track_axis == Axis::Z {
                    torus(
                        m,
                        [0.0, 0.0, if axis == Axis::Z { 1.0 } else { 0.5 }],
                        &mut out,
                    );
                }
                out.push(GizmoPrimitive::Axis {
                    model_view: st.model_view.mul(&m),
                    length: 0.5,
                    flip: [false; 3],
                });
            }
        }
        out
    }
}

/// `Math.round(float)`.
fn java_round(v: f32) -> i32 {
    (v + 0.5).floor() as i32
}

/// `RotateGizmo.getCircleSegments`: 33 points round the circle.
fn circle_segments(center: V, radius: f32, o1: V, o2: V) -> Vec<V> {
    (0..=32)
        .map(|i| {
            let t = if i == 0 {
                0.0
            } else {
                i as f64 * 360.0 / 32.0 * 0.01745329238474369
            };
            let p = vec3::mul(
                vec3::add(vec3::mul(o1, t.cos() as f32), vec3::mul(o2, t.sin() as f32)),
                radius,
            );
            vec3::add(center, p)
        })
        .collect()
}

/// `RotateGizmo.calculateRotation`: the signed angle (degrees) between two
/// points on the circle.
fn calculate_rotation(pp: V, pc: V, c: &Circle) -> f32 {
    if vec3::bit_eq(pp, pc) {
        return 0.0;
    }
    let vp = vec3::normalize(vec3::sub(pp, c.center));
    let vc = vec3::normalize(vec3::sub(pc, c.center));
    let angle = (vec3::dot(vc, vp) as f64).acos() as f32;
    let cross = vec3::cross(vp, vc);
    let sign = vec3::dot(cross, c.orientation).signum();
    let sign = if vec3::dot(cross, c.orientation) == 0.0 {
        0.0
    } else {
        sign
    };
    sign * angle * 57.295776
}

/// `Gizmos.PlayerData`.
#[derive(Default)]
struct PlayerData {
    gizmo: Option<GizmoRef>,
    scene: Option<Rc<RefCell<Scene>>>,
    rotate: Option<GizmoRef>,
    translate: Option<GizmoRef>,
}

impl PlayerData {
    fn scene(&mut self) -> Rc<RefCell<Scene>> {
        self.scene.get_or_insert_with(Rc::default).clone()
    }
}

/// `Gizmos.getInstance()`.
#[derive(Default)]
pub struct Gizmos {
    players: [PlayerData; 4],
}

thread_local! {
    static GIZMO_STATE: RefCell<Gizmos> = RefCell::default();
}

fn with_gizmos<R>(f: impl FnOnce(&mut Gizmos) -> R) -> R {
    GIZMO_STATE.with(|g| f(&mut g.borrow_mut()))
}

/// `getGizmo(player)`.
pub fn get_gizmo(player: usize) -> Option<GizmoRef> {
    with_gizmos(|g| g.players[player].gizmo.clone())
}

/// `setGizmo(player, gizmo)`.
pub fn set_gizmo(player: usize, gizmo: Option<GizmoRef>) {
    with_gizmos(|g| g.players[player].gizmo = gizmo);
}

/// `getRotateGizmo(player)` (made once per player).
pub fn rotate_gizmo(player: usize) -> GizmoRef {
    with_gizmos(|g| {
        let p = &mut g.players[player];
        let _ = p.scene();
        p.rotate
            .get_or_insert_with(|| Rc::new(RefCell::new(Gizmo::new(GizmoKind::Rotate))))
            .clone()
    })
}

/// `getTranslateGizmo(player)`.
pub fn translate_gizmo(player: usize) -> GizmoRef {
    with_gizmos(|g| {
        let p = &mut g.players[player];
        let _ = p.scene();
        p.translate
            .get_or_insert_with(|| Rc::new(RefCell::new(Gizmo::new(GizmoKind::Translate))))
            .clone()
    })
}

fn scene(player: usize) -> Rc<RefCell<Scene>> {
    with_gizmos(|g| g.players[player].scene())
}

/// `isTrackingMouse()` (player 0's scene).
pub fn is_tracking_mouse() -> bool {
    scene(0).borrow().mouse_down
}

/// `hitTest(mouseX, mouseY)` (player 0's scene), from `UIManager`'s mouse
/// handling: true consumes the press. Runs the Lua callbacks.
pub fn hit_test(t: &mut Thread, mouse_x: i32, mouse_y: i32) -> bool {
    let mut calls = Vec::new();
    let r = scene(0).borrow_mut().hit_test(mouse_x, mouse_y, &mut calls);
    for c in calls {
        let f = c.table.get_str(c.name);
        let v = crate::bind_misc_classes::vector3f_value(c.value);
        // LuaManager.caller.pcall: errors are reported, not raised.
        if let Err((e, _)) = t.pcall(&f, &[Value::Table(c.table.clone()), v]) {
            eprintln!("Gizmos {}: {e:?}", c.name);
        }
    }
    r
}

/// `render(player)`: the scene for the player's view, if its gizmo is
/// visible (`IsoWorld.render` calls this for the frame's player).
pub fn render(player: usize) -> Option<GizmoFrame> {
    let g = get_gizmo(player).filter(|g| g.borrow().visible)?;
    let sc = scene(player);
    let mut s = sc.borrow_mut();
    s.gizmo = Some(g);
    let (l, t, w, h) = screen_rect(player);
    s.set_bounds(l, t, w, h);
    s.render_main();
    Some(GizmoFrame {
        bounds: (l, t, w, h),
        projection: s.state.projection,
        primitives: s.render(),
    })
}

// ---- Lua ------------------------------------------------------------------------

use crate::expose::{
    bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, with_this, JavaEnum,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall};

pub const GIZMOS: &str = "zombie.gizmo.Gizmos";
pub const TRANSLATE_GIZMO: &str = "zombie.gizmo.TranslateGizmo";
pub const ROTATE_GIZMO: &str = "zombie.gizmo.RotateGizmo";
pub const TRANSFORM_MODE: &str = "zombie.gizmo.TransformMode";

struct GizmosObject;
impl LuaObject for GizmosObject {
    fn class_name(&self) -> &str {
        GIZMOS
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A gizmo as Lua holds it.
pub struct GizmoObject(pub GizmoRef);
impl LuaObject for GizmoObject {
    fn class_name(&self) -> &str {
        match self.0.borrow().kind {
            GizmoKind::Translate => TRANSLATE_GIZMO,
            GizmoKind::Rotate => ROTATE_GIZMO,
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

thread_local! {
    static INSTANCE: Value = Value::Object(Rc::new(GizmosObject));
    static OBJECTS: RefCell<Vec<(std::rc::Weak<RefCell<Gizmo>>, Value)>> = RefCell::default();
}

fn gizmo_value(g: Option<GizmoRef>) -> Value {
    let Some(g) = g else { return Value::Nil };
    OBJECTS.with(|o| {
        let mut o = o.borrow_mut();
        if let Some((_, v)) = o
            .iter()
            .find(|(w, _)| w.upgrade().is_some_and(|x| Rc::ptr_eq(&x, &g)))
        {
            return v.clone();
        }
        let v = Value::Object(Rc::new(GizmoObject(g.clone())));
        o.push((Rc::downgrade(&g), v.clone()));
        v
    })
}

fn this(c: &NativeCall<'_>) -> LuaResult<GizmoRef> {
    with_this::<GizmoObject, _>(c, |o| o.0.clone())
}

fn player(c: &NativeCall<'_>, i: usize) -> LuaResult<usize> {
    let p = int_arg(c, i);
    usize::try_from(p)
        .ok()
        .filter(|p| *p < 4)
        .ok_or_else(|| LuaError::Runtime {
            message: format!("Index {p} out of bounds for length 4"),
            class: "java.lang.ArrayIndexOutOfBoundsException",
        })
}

/// Register `Gizmos`, the two gizmo classes and `TransformMode`.
pub fn register(thread: &mut Thread) {
    with_ui(|m| m.gizmo_hit_test_hook = Some(Rc::new(hit_test)));
    expose_enum(thread, TRANSFORM_MODE, &["Global", "Local"]);
    let b = expose_class(thread, GIZMOS, None);
    b.static_fn("getInstance", |c| Ok(c.push(INSTANCE.with(Clone::clone))));
    b.method("getGizmo", |c| {
        let p = player(c, 1)?;
        Ok(c.push(gizmo_value(get_gizmo(p))))
    });
    b.method("setGizmo", |c| {
        let p = player(c, 1)?;
        let g = match c.arg(2) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<GizmoObject>()
                .map(|g| g.0.clone()),
            _ => None,
        };
        set_gizmo(p, g);
        Ok(0)
    });
    b.method("getRotateGizmo", |c| {
        let p = player(c, 1)?;
        Ok(c.push(gizmo_value(Some(rotate_gizmo(p)))))
    });
    b.method("getTranslateGizmo", |c| {
        let p = player(c, 1)?;
        Ok(c.push(gizmo_value(Some(translate_gizmo(p)))))
    });
    b.method("isTrackingMouse", |c| Ok(c.push(is_tracking_mouse())));
    b.method("hitTest", |c| {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let r = hit_test(c.thread, x, y);
        Ok(c.push(r))
    });
    // The drawing is the render thread's (`render(player)` hands it over).
    b.method("render", |_| Ok(0));
    for class in [TRANSLATE_GIZMO, ROTATE_GIZMO] {
        let b = expose_class(thread, class, None);
        b.method("isVisible", |c| Ok(c.push(this(c)?.borrow().visible)));
        b.method("setVisible", |c| {
            this(c)?.borrow_mut().visible = bool_arg(c, 1);
            Ok(0)
        });
        b.method("setWorldPosition", |c| {
            let v = [
                num_arg(c, 1) as f32,
                num_arg(c, 2) as f32,
                num_arg(c, 3) as f32,
            ];
            this(c)?.borrow().world_pos.set(v);
            Ok(0)
        });
        b.method("getWorldPosition", |c| {
            let v = this(c)?.borrow().world_pos.clone();
            Ok(c.push(Value::Object(v)))
        });
        b.method("setRotation", |c| {
            let (x, y, z) = (
                num_arg(c, 1) as f32,
                num_arg(c, 2) as f32,
                num_arg(c, 3) as f32,
            );
            this(c)?.borrow_mut().set_rotation(x, y, z);
            Ok(0)
        });
        b.method("getRotation", |c| {
            let v = this(c)?.borrow().rotation_obj.clone();
            Ok(c.push(Value::Object(v)))
        });
        b.method("getScale", |c| Ok(c.push(this(c)?.borrow().scale as f64)));
        b.method("setTransformMode", |c| {
            let m = match c.arg(1) {
                Value::Object(o) => o
                    .as_any()
                    .downcast_ref::<JavaEnum>()
                    .filter(|e| e.class == TRANSFORM_MODE)
                    .map(|e| {
                        if e.name == "Local" {
                            TransformMode::Local
                        } else {
                            TransformMode::Global
                        }
                    }),
                _ => None,
            };
            let m = m.ok_or(LuaError::Runtime {
                message: String::new(),
                class: "java.lang.NullPointerException",
            })?;
            this(c)?.borrow_mut().transform_mode = m;
            Ok(0)
        });
        b.method("getTransformMode", |c| {
            let m = this(c)?.borrow().transform_mode;
            Ok(c.push(enum_value(
                TRANSFORM_MODE,
                if m == TransformMode::Local {
                    "Local"
                } else {
                    "Global"
                },
            )))
        });
        b.method("setTable", |c| {
            let t = match c.arg(1) {
                Value::Table(t) => Some(t),
                _ => None,
            };
            this(c)?.borrow_mut().table = t;
            Ok(0)
        });
        b.method("getTable", |c| {
            let t = this(c)?.borrow().table.clone();
            Ok(c.push(t.map_or(Value::Nil, Value::Table)))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::host::{install, HudGizmoCamera, HudHost};
    use crate::hud::test_util::*;
    use pz_ui::tooltip::java_float_to_string as jf;

    struct Cam;
    impl HudGizmoCamera for Cam {
        fn gizmo_camera(&self) -> GizmoCamera {
            GizmoCamera {
                cam_character: [100.5, 200.25, 0.0],
                frame_zoom: 1.0,
                right_click: (3.0, -2.0),
                t_off: (1231.0, -575.0),
                off: (1237.0, -569.0),
                defered: (0.25, -0.5),
            }
        }
    }

    fn m(a: &Matrix4f) -> String {
        let mut v = Vec::new();
        for c in 0..4 {
            for r in 0..4 {
                v.push(jf(a.m[c][r]));
            }
        }
        v.join(" ")
    }
    fn v(a: V) -> String {
        format!("{} {} {}", jf(a[0]), jf(a[1]), jf(a[2]))
    }
    fn axis(a: Axis) -> &'static str {
        match a {
            Axis::None => ".",
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
            Axis::XY => "XY",
            Axis::XZ => "XZ",
            Axis::YZ => "YZ",
        }
    }
    fn mouse(x: i32, y: i32, down: bool, prev: bool) {
        with_ui(|m| {
            (m.mouse.x, m.mouse.y) = (x, y);
            m.mouse.down[0] = down;
            m.mouse.prev_down[0] = prev;
        });
    }
    fn hit(t: &mut Thread, x: i32, y: i32) -> bool {
        hit_test(t, x, y)
    }
    fn state(got: &mut Vec<String>, l: &str) {
        let sc = scene(0);
        let s = &sc.borrow().state;
        got.push(format!("{l} proj {}", m(&s.projection)));
        got.push(format!("{l} mv {}", m(&s.model_view)));
        got.push(format!(
            "{l} xfrm {} t {} r {}",
            m(&s.gizmo_transform),
            v(s.gizmo_translate),
            v(s.gizmo_rotate)
        ));
        got.push(format!(
            "{l} ray {} {}",
            v(s.camera_ray.origin),
            v(s.camera_ray.direction)
        ));
    }
    fn grid(t: &mut Thread, got: &mut Vec<String>, l: &str, cx: i32, cy: i32) {
        for dy in (-90..=90).step_by(6) {
            let mut row = format!("{l} row {dy} ");
            for dx in (-90..=90).step_by(6) {
                mouse(cx + dx, cy + dy, false, false);
                hit(t, cx + dx, cy + dy);
                row.push_str(axis(scene(0).borrow().state.gizmo_axis));
            }
            got.push(row);
        }
        let h = scene(0).borrow().state.hide;
        got.push(format!("{l} hide {} {} {}", h[0], h[1], h[2]));
    }
    fn find(t: &mut Thread, cx: i32, cy: i32, want: &str) -> Option<(i32, i32)> {
        for dy in (-90..=90).step_by(6) {
            for dx in (-90..=90).step_by(6) {
                mouse(cx + dx, cy + dy, false, false);
                hit(t, cx + dx, cy + dy);
                if axis(scene(0).borrow().state.gizmo_axis) == want {
                    return Some((cx + dx, cy + dy));
                }
            }
        }
        None
    }
    fn drag(t: &mut Thread, got: &mut Vec<String>, l: &str, g: &GizmoRef, at: Option<(i32, i32)>) {
        let Some((x, y)) = at else {
            got.push(format!("{l} none"));
            return;
        };
        mouse(x, y, true, false);
        let r = hit(t, x, y);
        got.push(format!("{l} start {r} {}", is_tracking_mouse()));
        for (dx, dy) in [(5, 0), (5, 0), (12, -7), (-20, 15)] {
            mouse(x + dx, y + dy, true, true);
            hit(t, x + dx, y + dy);
            let g = g.borrow();
            if g.kind == GizmoKind::Translate {
                got.push(format!(
                    "{l} move {dx} {dy} start {} cur {} st {}",
                    v(g.start_pos),
                    v(g.current_pos),
                    v(g.start_translate)
                ));
            } else {
                got.push(format!(
                    "{l} move {dx} {dy} start {} cur {} circle {} {} sr {}",
                    v(g.start_pos),
                    v(g.current_pos),
                    v(g.track_circle.center),
                    v(g.track_circle.orientation),
                    v(g.start_rotate)
                ));
            }
        }
        mouse(x, y, false, true);
        let r = hit(t, x, y);
        got.push(format!("{l} end {r} {}", is_tracking_mouse()));
    }

    /// Parity with the original `Gizmos` (oracle `gizmo.txt`): the scene's
    /// matrices and camera ray, the translate gizmo's hit test over a grid
    /// round it (global, then local) and drags on each axis, the rotate
    /// gizmo's (local, then global), JOML's quaternion maths.
    #[test]
    fn matches_gizmo_oracle() {
        let Some(text) = oracle("gizmo.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install(HudHost {
            gizmo_camera: Box::new(Cam),
            ..HudHost::default()
        });
        let mut t = thread();
        crate::bind_core::shared_core()
            .borrow_mut()
            .set_screen_size(1280, 720);
        crate::bind_core::shared_core().borrow_mut().tile_scale = 2;
        GIZMO_STATE.with(|g| *g.borrow_mut() = Gizmos::default());
        let mut got = vec!["camera 1237.0 -569.0 1231.0 -575.0".to_string()];
        let tg = translate_gizmo(0);
        tg.borrow().world_pos.set([101.2, 199.7, 0.0]);
        tg.borrow_mut().set_rotation(10.0, 20.0, 30.0);
        tg.borrow_mut().visible = true;
        set_gizmo(0, Some(tg.clone()));
        mouse(700, 300, false, false);
        render(0).unwrap();
        state(&mut got, "translate");
        let center = |got: &mut Vec<String>| {
            let sc = scene(0);
            let s = sc.borrow();
            let (x, y) = s.scene_to_ui(s.state.gizmo_translate);
            got.push(format!("center {} {}", jf(x), jf(y)));
            (x as i32, y as i32)
        };
        let (cx, cy) = center(&mut got);
        grid(&mut t, &mut got, "tg", cx, cy);
        for ax in ["X", "Y", "Z"] {
            let at = find(&mut t, cx, cy, ax);
            drag(&mut t, &mut got, &format!("tg drag {ax}"), &tg, at);
        }
        tg.borrow_mut().transform_mode = TransformMode::Local;
        grid(&mut t, &mut got, "tl", cx, cy);
        for ax in ["X", "Y", "Z"] {
            let at = find(&mut t, cx, cy, ax);
            drag(&mut t, &mut got, &format!("tl drag {ax}"), &tg, at);
        }
        let rg = rotate_gizmo(0);
        rg.borrow().world_pos.set([99.8, 201.1, 1.0]);
        rg.borrow_mut().set_rotation(-15.0, 40.0, 5.0);
        rg.borrow_mut().visible = true;
        set_gizmo(0, Some(rg.clone()));
        mouse(600, 400, false, false);
        render(0).unwrap();
        state(&mut got, "rotate");
        let (cx, cy) = center(&mut got);
        grid(&mut t, &mut got, "rl", cx, cy);
        for ax in ["X", "Y", "Z"] {
            let at = find(&mut t, cx, cy, ax);
            drag(&mut t, &mut got, &format!("rl drag {ax}"), &rg, at);
        }
        rg.borrow_mut().transform_mode = TransformMode::Global;
        grid(&mut t, &mut got, "rg", cx, cy);
        for q in [
            [1.0f32, 0.0, 0.0, 30.0],
            [0.0, 1.0, 0.0, -45.5],
            [0.3, 0.4, 0.866, 12.0],
        ] {
            let a1 = Quaternionf::from_axis_angle_deg([q[0], q[1], q[2]], q[3]);
            let e = a1.euler_angles_xyz();
            let q0 = Quaternionf::rotation_xyz(0.2, -0.7, 1.1)
                .mul(&Quaternionf::rotation_xyz(e[0], e[1], e[2]));
            got.push(format!(
                "quat {} {} {} {} euler {} mul {}",
                jf(a1.x),
                jf(a1.y),
                jf(a1.z),
                jf(a1.w),
                v(e),
                v(q0.euler_angles_xyz())
            ));
        }
        set_gizmo(0, None);
        let want: Vec<&str> = text
            .lines()
            .filter(|l| !l.starts_with("fake ") && !l.starts_with("tex "))
            .collect();
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "line {i}");
        }
        assert_eq!(got.len(), want.len());
    }

    /// A drag calls the table's `onTranslateGizmo` with the new world
    /// position; `UIManager`'s left button goes through the hook; the
    /// render hands over the primitives.
    #[test]
    fn callbacks_hook_and_primitives() {
        install(HudHost {
            gizmo_camera: Box::new(Cam),
            ..HudHost::default()
        });
        let mut t = thread();
        register(&mut t);
        crate::bind_misc_classes::register(&mut t, None);
        crate::bind_core::shared_core()
            .borrow_mut()
            .set_screen_size(1280, 720);
        GIZMO_STATE.with(|g| *g.borrow_mut() = Gizmos::default());
        run(
            &mut t,
            "ui = { onTranslateGizmo = function(self, v) moved = { v:x(), v:y(), v:z() } end }
             local g = Gizmos.getInstance():getTranslateGizmo(0)
             g:setWorldPosition(101.2, 199.7, 0); g:setRotation(10, 20, 30); g:setVisible(true); g:setTable(ui)
             Gizmos.getInstance():setGizmo(0, g)
             mode = g:getTransformMode() == TransformMode.Global",
        );
        assert!(eval(&mut t, "mode").truthy());
        let frame = render(0).unwrap();
        assert!(matches!(
            frame.primitives[0],
            GizmoPrimitive::Axis {
                flip: [false, false, true],
                ..
            }
        ));
        assert_eq!(
            frame
                .primitives
                .iter()
                .filter(|p| matches!(p, GizmoPrimitive::Cylinder { .. }))
                .count(),
            6
        );
        let (cx, cy) = {
            let sc = scene(0);
            let s = sc.borrow();
            let (x, y) = s.scene_to_ui(s.state.gizmo_translate);
            (x as i32, y as i32)
        };
        let at = find(&mut t, cx, cy, "X").unwrap();
        mouse(at.0, at.1, true, false);
        let hook = with_ui(|m| m.gizmo_hit_test_hook.clone()).unwrap();
        assert!(hook(&mut t, at.0, at.1));
        mouse(at.0 + 10, at.1, true, true);
        hook(&mut t, at.0 + 10, at.1);
        let moved = eval(&mut t, "moved[1]").as_num().unwrap();
        assert!(moved > 101.2, "{moved}");
        assert_eq!(eval(&mut t, "moved[2]").as_num(), Some(199.7f32 as f64));
    }
}
