//! `zombie.iso.WorldMarkers`: per-player homing arrows that point at a
//! target from the screen edge, direction arrows (at the target when it is
//! on screen, else on the screen border) and pulsing grid-square markers
//! (drawn in the world by the FBO chunk renderer from [`fbo_markers`]).

use super::host::with_host;
use pz_anim::math::pzmath;
use pz_kahlua::Thread;
use pz_render::{Rgba, Texture};
use pz_ui::with_ui;
use std::cell::RefCell;
use std::rc::Rc;

pub const WORLD_MARKERS: &str = "zombie.iso.WorldMarkers";
pub const DIRECTION_ARROW: &str = "zombie.iso.WorldMarkers$DirectionArrow";
pub const PLAYER_HOMING_POINT: &str = "zombie.iso.WorldMarkers$PlayerHomingPoint";
pub const GRID_SQUARE_MARKER: &str = "zombie.iso.WorldMarkers$GridSquareMarker";

fn highlight(name: &str) -> Option<Texture> {
    pz_render::get_shared_texture(&format!("media/textures/highlights/{name}.png"))
}

fn tile_scale() -> i32 {
    crate::bind_core::shared_core().borrow().tile_scale
}

fn multiplier() -> f32 {
    with_host(|h| h.game_time.get_multiplier())
}

fn is_server() -> bool {
    with_host(|h| h.network.is_server())
}

/// `PlayerCamera.XToScreenExact(x, y, z, 0)` / `YToScreenExact`.
fn to_screen_exact(player: usize, x: f32, y: f32, z: f32) -> (f32, f32) {
    let ts = tile_scale() as f32;
    let (ox, oy) = with_host(|h| h.camera.camera_offset(player));
    let sx = x * (32.0 * ts) - y * (32.0 * ts);
    let sy = y * (16.0 * ts) + x * (16.0 * ts) + (0.0 - z) * (96.0 * ts);
    (sx - ox, sy - oy)
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
    let w = if n > 1 { sw / 2 } else { sw };
    let h = if n > 2 { sh / 2 } else { sh };
    (left, top, w, h)
}

/// `PZMath.radToDeg`.
fn rad_to_deg(r: f32) -> f32 {
    57.295776 * r
}

fn get_distance(dx: i32, dy: i32, sx: i32, sy: i32) -> i32 {
    (((dx - sx) as f64).powf(2.0) + ((dy - sy) as f64).powf(2.0)).sqrt() as i32
}

fn get_angle(px: i32, py: i32, tx: i32, ty: i32) -> f32 {
    let mut a = ((ty - py) as f64).atan2((tx - px) as f64).to_degrees() as f32;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

fn angle_degrees(mut a: f32) -> f32 {
    if a < 0.0 {
        a += 360.0;
    }
    if a > 360.0 {
        a -= 360.0;
    }
    a
}

#[derive(Clone, Copy, Default, Debug, PartialEq)]
struct Point {
    x: f32,
    y: f32,
}

type Line = (Point, Point);

/// `intersectLineSegments(l1, l2, out)`.
fn intersect(l1: Line, l2: Line) -> Option<Point> {
    let (x1, y1, x2, y2) = (l1.0.x, l1.0.y, l1.1.x, l1.1.y);
    let (x3, y3, x4, y4) = (l2.0.x, l2.0.y, l2.1.x, l2.1.y);
    let d = (y4 - y3) * (x2 - x1) - (x4 - x3) * (y2 - y1);
    if d == 0.0 {
        return None;
    }
    let (yd, xd) = (y1 - y3, x1 - x3);
    let ua = ((x4 - x3) * yd - (y4 - y3) * xd) / d;
    if !(0.0..=1.0).contains(&ua) {
        return None;
    }
    let ub = ((x2 - x1) * yd - (y2 - y1) * xd) / d;
    if !(0.0..=1.0).contains(&ub) {
        return None;
    }
    Some(Point {
        x: x1 + (x2 - x1) * ua,
        y: y1 + (y2 - y1) * ua,
    })
}

/// `WorldMarkers.PlayerScreen`: the player's view with a 100 px inner
/// margin.
#[derive(Default, Clone, Copy)]
struct PlayerScreen {
    center_x: f32,
    center_y: f32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    inner_x: f32,
    inner_y: f32,
    inner_x2: f32,
    inner_y2: f32,
}

impl PlayerScreen {
    fn update(&mut self, player: usize) {
        let (_, _, w, h) = screen_rect(player);
        self.x = 0.0;
        self.y = 0.0;
        self.width = w as f32;
        self.height = h as f32;
        self.center_x = self.x + self.width / 2.0;
        self.center_y = self.y + self.height / 2.0;
        self.inner_x = self.x + 100.0;
        self.inner_y = self.y + 100.0;
        self.inner_x2 = self.inner_x + (self.width - 200.0);
        self.inner_y2 = self.inner_y + (self.height - 200.0);
    }
    fn borders(&self) -> [Line; 4] {
        let p = |x, y| Point { x, y };
        [
            (
                p(self.inner_x, self.inner_y),
                p(self.inner_x2, self.inner_y),
            ),
            (
                p(self.inner_x2, self.inner_y),
                p(self.inner_x2, self.inner_y2),
            ),
            (
                p(self.inner_x, self.inner_y2),
                p(self.inner_x2, self.inner_y2),
            ),
            (
                p(self.inner_x, self.inner_y),
                p(self.inner_x, self.inner_y2),
            ),
        ]
    }
    fn clamp_x(&self, x: f32) -> f32 {
        pzmath::clamp(x, self.inner_x, self.inner_x2)
    }
    fn clamp_y(&self, y: f32) -> f32 {
        pzmath::clamp(y, self.inner_y, self.inner_y2)
    }
    fn is_on_screen(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
    fn is_within_inner(&self, x: f32, y: f32) -> bool {
        x >= self.inner_x && x < self.inner_x2 && y >= self.inner_y && y < self.inner_y2
    }
}

/// `WorldMarkers.DirectionArrow`.
pub struct DirectionArrow {
    pub id: i32,
    pub active: bool,
    pub is_removed: bool,
    is_draw_on_world: bool,
    render_texture: Option<Texture>,
    pub texture: Option<Texture>,
    pub tex_stairs_up: Option<Texture>,
    pub tex_stairs_down: Option<Texture>,
    pub tex_down: Option<Texture>,
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub render_width: f32,
    pub render_height: f32,
    angle: f32,
    last_was_within_view: bool,
    render_screen_x: f32,
    render_screen_y: f32,
    render_with_angle: bool,
    render_size_mod: f32,
}

impl DirectionArrow {
    fn new(id: i32) -> Self {
        DirectionArrow {
            id,
            active: true,
            is_removed: false,
            is_draw_on_world: false,
            render_texture: None,
            texture: None,
            tex_stairs_up: None,
            tex_stairs_down: None,
            tex_down: None,
            x: 0,
            y: 0,
            z: 0,
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
            render_width: 32.0,
            render_height: 32.0,
            angle: 0.0,
            last_was_within_view: true,
            render_screen_x: 0.0,
            render_screen_y: 0.0,
            render_with_angle: true,
            render_size_mod: 1.0,
        }
    }
    /// `setTexture(name)` (null: `dir_arrow_up`).
    pub fn set_texture(&mut self, name: Option<&str>) {
        self.texture = highlight(name.unwrap_or("dir_arrow_up"));
    }
    pub fn set_tex_down(&mut self, name: &str) {
        self.tex_down = highlight(name);
    }
    pub fn set_tex_stairs_down(&mut self, name: &str) {
        self.tex_stairs_down = highlight(name);
    }
    pub fn set_tex_stairs_up(&mut self, name: &str) {
        self.tex_stairs_up = highlight(name);
    }
    /// `setZ(z)`: clamped to -32..31.
    pub fn set_z(&mut self, z: i32) {
        self.z = z.clamp(-32, 31);
    }
}

/// `WorldMarkers.PlayerHomingPoint`.
pub struct PlayerHomingPoint {
    pub id: i32,
    pub texture: Option<Texture>,
    pub x: i32,
    pub y: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    angle: f32,
    pub target_angle: f32,
    pub custom_target_angle: bool,
    pub angle_lerp_val: f32,
    pub movement_lerp_val: f32,
    dist: i32,
    targ_render_x: f32,
    targ_render_y: f32,
    render_x: f32,
    render_y: f32,
    pub render_offset_x: f32,
    pub render_offset_y: f32,
    pub render_width: f32,
    pub render_height: f32,
    render_size_mod: f32,
    target_screen_x: f32,
    target_screen_y: f32,
    target_on_screen: bool,
    pub stick_to_char_dist: f32,
    pub active: bool,
    pub home_on_target_in_view: bool,
    pub home_on_target_dist: i32,
    pub home_on_offset_x: f32,
    pub home_on_offset_y: f32,
    pub is_removed: bool,
}

impl PlayerHomingPoint {
    /// `new PlayerHomingPoint(plrIndex)`: starts at the view's centre
    /// (`renderX/Y` keep the field initializers' zero).
    fn new(id: i32, player: usize) -> Self {
        let (l, t, w, h) = screen_rect(player);
        PlayerHomingPoint {
            id,
            texture: None,
            x: 0,
            y: 0,
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
            angle: 0.0,
            target_angle: 0.0,
            custom_target_angle: false,
            angle_lerp_val: 0.25,
            movement_lerp_val: 0.25,
            dist: 0,
            targ_render_x: l as f32 + w as f32 / 2.0,
            targ_render_y: t as f32 + h as f32 / 2.0,
            render_x: 0.0,
            render_y: 0.0,
            render_offset_x: 0.0,
            render_offset_y: 50.0,
            render_width: 32.0,
            render_height: 32.0,
            render_size_mod: 1.0,
            target_screen_x: 0.0,
            target_screen_y: 0.0,
            target_on_screen: false,
            stick_to_char_dist: 130.0,
            active: false,
            home_on_target_in_view: true,
            home_on_target_dist: 20,
            home_on_offset_x: 0.0,
            home_on_offset_y: 0.0,
            is_removed: false,
        }
    }
    /// `setTexture(name)` (null: `arrow_triangle`).
    pub fn set_texture(&mut self, name: Option<&str>) {
        self.texture = highlight(name.unwrap_or("arrow_triangle"));
    }
    pub fn set_table_surface(&mut self) {
        self.home_on_offset_y = -30.0 * tile_scale() as f32;
    }
    pub fn set_high_counter(&mut self) {
        self.home_on_offset_y = -50.0 * tile_scale() as f32;
    }
    pub fn set_y_offset_scaled(&mut self, o: f32) {
        self.home_on_offset_y = o * tile_scale() as f32;
    }
    pub fn set_x_offset_scaled(&mut self, o: f32) {
        self.home_on_offset_x = o * tile_scale() as f32;
    }
}

/// `WorldMarkers.GridSquareMarker`.
pub struct GridSquareMarker {
    pub id: i32,
    pub texture_name: String,
    pub overlay_texture_name: Option<String>,
    /// The `IsoSpriteInstance` scale of the marker and its overlay.
    pub sprite_scale: f32,
    pub has_overlay: bool,
    pub orig_x: f32,
    pub orig_y: f32,
    pub orig_z: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    scale_ratio: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub size: f32,
    pub do_blink: bool,
    pub do_alpha: bool,
    pub scale_circle_texture: bool,
    pub fade_speed: f32,
    pub alpha: f32,
    pub alpha_max: f32,
    pub alpha_min: f32,
    alpha_inc: bool,
    pub active: bool,
    pub is_removed: bool,
}

impl GridSquareMarker {
    fn new(id: i32) -> Self {
        GridSquareMarker {
            id,
            texture_name: String::new(),
            overlay_texture_name: None,
            sprite_scale: 1.0,
            has_overlay: false,
            orig_x: 0.0,
            orig_y: 0.0,
            orig_z: 0.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
            scale_ratio: 0.0,
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
            size: 0.0,
            do_blink: false,
            do_alpha: false,
            scale_circle_texture: false,
            fade_speed: 0.006,
            alpha: 0.0,
            alpha_max: 1.0,
            alpha_min: 0.3,
            alpha_inc: true,
            active: true,
            is_removed: false,
        }
    }

    fn scaled(&self, size: f32) -> f32 {
        size * if self.scale_circle_texture { 1.5 } else { 1.0 }
    }

    /// `init(texid, overlay, x, y, z, size)`: `Err` is the original's NPE
    /// on a missing texture.
    pub fn init(
        &mut self,
        texid: Option<&str>,
        overlay: Option<&str>,
        x: i32,
        y: i32,
        z: i32,
        size: f32,
    ) -> Result<(), MarkerError> {
        let texid = texid.unwrap_or("circle_center");
        self.texture_name = texid.to_string();
        self.overlay_texture_name = overlay.map(str::to_string);
        let tex = highlight(texid).ok_or(MarkerError::Null)?;
        let w = tex.width as i32 as f32;
        let tile_w = 64.0 * tile_scale() as f32;
        self.scale_ratio = 1.0 / (w / tile_w);
        self.has_overlay = overlay.is_some();
        self.set_pos_and_size(x, y, z, size);
        Ok(())
    }

    pub fn set_pos_and_size(&mut self, x: i32, y: i32, z: i32, size: f32) {
        let s = self.scaled(size);
        self.sprite_scale = self.scale_ratio * s;
        self.size = size;
        (self.orig_x, self.orig_y, self.orig_z) = (x as f32, y as f32, z as f32);
        self.x = x as f32 - (s - 0.5);
        self.y = y as f32 + 0.5;
        self.z = z as f32;
    }

    pub fn set_pos(&mut self, x: i32, y: i32, z: i32) {
        let s = self.scaled(self.size);
        (self.orig_x, self.orig_y, self.orig_z) = (x as f32, y as f32, z as f32);
        self.x = x as f32 - (s - 0.5);
        self.y = y as f32 + 0.5;
        self.z = z as f32;
    }

    pub fn set_size(&mut self, size: f32) {
        let s = self.scaled(size);
        self.sprite_scale = self.scale_ratio * s;
        self.size = size;
        self.x = self.orig_x - (s - 0.5);
        self.y = self.orig_y + 0.5;
        self.z = self.orig_z;
    }

    pub fn set_scale_circle_texture(&mut self, b: bool) {
        self.scale_circle_texture = b;
        let s = self.scaled(self.size);
        self.sprite_scale = self.scale_ratio * s;
        self.x = self.orig_x - (s - 0.5);
    }
}

pub type ArrowRef = Rc<RefCell<DirectionArrow>>;
pub type PointRef = Rc<RefCell<PlayerHomingPoint>>;
pub type MarkerRef = Rc<RefCell<GridSquareMarker>>;

/// `WorldMarkers.instance`.
#[derive(Default)]
pub struct WorldMarkers {
    next_grid_square_marker_id: i32,
    next_homing_point_id: i32,
    grid_square_markers: Vec<MarkerRef>,
    /// `null` until `init()`.
    homing_points: Option<[Vec<PointRef>; 4]>,
    direction_arrows: Option<[Vec<ArrowRef>; 4]>,
    screen: PlayerScreen,
}

/// The original's `NullPointerException` / `IndexOutOfBoundsException`.
#[derive(Debug, PartialEq)]
pub enum MarkerError {
    Null,
    IndexOutOfBounds(usize, usize),
}

thread_local! {
    static INSTANCE: RefCell<WorldMarkers> = RefCell::default();
}

/// Runs `f` on `WorldMarkers.instance`. Not re-entrant.
pub fn with_markers<R>(f: impl FnOnce(&mut WorldMarkers) -> R) -> R {
    INSTANCE.with(|w| f(&mut w.borrow_mut()))
}

/// A `FBORenderWorldMarkers` highlight: the square the marker covers and
/// its textures, before the on-screen (chunk map) test.
#[derive(Clone, Debug, PartialEq)]
pub struct FboMarker {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub z: f32,
    pub color: Rgba,
    pub texture1: Option<Texture>,
    pub texture2: Option<Texture>,
}

impl WorldMarkers {
    /// `init()`: the per-player lists (not on a server).
    pub fn init(&mut self) {
        if is_server() {
            return;
        }
        self.homing_points = Some(Default::default());
        self.direction_arrows = Some(Default::default());
    }

    /// `reset()`.
    pub fn reset(&mut self) -> Result<(), MarkerError> {
        let (h, d) = (
            self.homing_points.as_mut().ok_or(MarkerError::Null)?,
            self.direction_arrows.as_mut().ok_or(MarkerError::Null)?,
        );
        h.iter_mut().for_each(Vec::clear);
        d.iter_mut().for_each(Vec::clear);
        self.grid_square_markers.clear();
        Ok(())
    }

    fn points(&mut self) -> Result<&mut [Vec<PointRef>; 4], MarkerError> {
        self.homing_points.as_mut().ok_or(MarkerError::Null)
    }

    fn arrows(&mut self) -> Result<&mut [Vec<ArrowRef>; 4], MarkerError> {
        self.direction_arrows.as_mut().ok_or(MarkerError::Null)
    }

    /// `getHomingPoint(id)`: the original walks each list from the end
    /// *upwards*, so it only finds the last point and throws past it.
    pub fn get_homing_point(&mut self, id: i32) -> Result<Option<PointRef>, MarkerError> {
        for list in self.points()?.iter() {
            if list.is_empty() {
                continue;
            }
            let j = list.len() - 1;
            if list[j].borrow().id == id {
                return Ok(Some(list[j].clone()));
            }
            return Err(MarkerError::IndexOutOfBounds(j + 1, list.len()));
        }
        Ok(None)
    }

    /// `addPlayerHomingPoint(player, x, y, texname, r, g, b, a,
    /// homeOnTarget, homeOnDist)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_player_homing_point(
        &mut self,
        player: usize,
        x: i32,
        y: i32,
        tex: Option<&str>,
        c: Rgba,
        home_on_target: bool,
        home_on_dist: i32,
    ) -> Result<Option<PointRef>, MarkerError> {
        if is_server() {
            return Ok(None);
        }
        let id = self.next_homing_point_id;
        self.next_homing_point_id += 1;
        let mut p = PlayerHomingPoint::new(id, player);
        p.active = true;
        p.set_texture(tex);
        (p.x, p.y) = (x, y);
        (p.r, p.g, p.b, p.a) = (c.0, c.1, c.2, c.3);
        p.home_on_target_in_view = home_on_target;
        p.home_on_target_dist = home_on_dist;
        let p = Rc::new(RefCell::new(p));
        self.points()?[player].push(p.clone());
        Ok(Some(p))
    }

    /// `removeHomingPoint(id)`.
    pub fn remove_homing_point(&mut self, id: i32) -> Result<bool, MarkerError> {
        for list in self.points()?.iter_mut() {
            if let Some(j) = list.iter().rposition(|p| p.borrow().id == id) {
                list[j].borrow_mut().is_removed = true;
                list.remove(j);
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `removePlayerHomingPoint(player, id)`.
    pub fn remove_player_homing_point(
        &mut self,
        player: usize,
        id: i32,
    ) -> Result<bool, MarkerError> {
        let list = &mut self.points()?[player];
        if let Some(j) = list.iter().rposition(|p| p.borrow().id == id) {
            list[j].borrow_mut().is_removed = true;
            list.remove(j);
            return Ok(true);
        }
        Ok(false)
    }

    /// `removeAllHomingPoints(player)` (the points are not marked removed).
    pub fn remove_all_homing_points(&mut self, player: usize) -> Result<(), MarkerError> {
        self.points()?[player].clear();
        Ok(())
    }

    /// `getDirectionArrow(id)`.
    pub fn get_direction_arrow(&mut self, id: i32) -> Result<Option<ArrowRef>, MarkerError> {
        for list in self.arrows()?.iter() {
            if let Some(a) = list.iter().rev().find(|a| a.borrow().id == id) {
                return Ok(Some(a.clone()));
            }
        }
        Ok(None)
    }

    /// `addDirectionArrow(player, x, y, z, texname, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_direction_arrow(
        &mut self,
        player: usize,
        x: i32,
        y: i32,
        z: i32,
        tex: Option<&str>,
        c: Rgba,
    ) -> Result<Option<ArrowRef>, MarkerError> {
        if is_server() {
            return Ok(None);
        }
        // Arrows share the homing points' id counter.
        let id = self.next_homing_point_id;
        self.next_homing_point_id += 1;
        let mut a = DirectionArrow::new(id);
        a.active = true;
        a.set_texture(tex);
        a.set_tex_down("dir_arrow_down");
        a.set_tex_stairs_up("dir_arrow_stairs_up");
        a.set_tex_stairs_down("dir_arrow_stairs_down");
        (a.x, a.y) = (x, y);
        a.set_z(z);
        (a.r, a.g, a.b, a.a) = (c.0, c.1, c.2, c.3);
        let a = Rc::new(RefCell::new(a));
        self.arrows()?[player].push(a.clone());
        Ok(Some(a))
    }

    pub fn remove_direction_arrow(&mut self, id: i32) -> Result<bool, MarkerError> {
        for list in self.arrows()?.iter_mut() {
            if let Some(j) = list.iter().rposition(|a| a.borrow().id == id) {
                list[j].borrow_mut().is_removed = true;
                list.remove(j);
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn remove_player_direction_arrow(
        &mut self,
        player: usize,
        id: i32,
    ) -> Result<bool, MarkerError> {
        let list = &mut self.arrows()?[player];
        if let Some(j) = list.iter().rposition(|a| a.borrow().id == id) {
            list[j].borrow_mut().is_removed = true;
            list.remove(j);
            return Ok(true);
        }
        Ok(false)
    }

    pub fn remove_all_direction_arrows(&mut self, player: usize) -> Result<(), MarkerError> {
        self.arrows()?[player].clear();
        Ok(())
    }

    /// `update()`: marker fades, then the current player's homing points
    /// and arrows.
    pub fn update(&mut self) -> Result<(), MarkerError> {
        if is_server() {
            return Ok(());
        }
        self.update_grid_square_markers();
        self.update_homing_points()?;
        self.update_direction_arrows()
    }

    fn update_direction_arrows(&mut self) -> Result<(), MarkerError> {
        let pi = with_host(|h| h.camera.frame_player_index());
        let mut screen = self.screen;
        let lists = self.arrows()?;
        for (i, list) in lists.iter_mut().enumerate() {
            if i != pi || list.is_empty() {
                continue;
            }
            list.retain(|a| !a.borrow().is_removed);
            screen.update(i);
            for a in list.iter() {
                let mut a = a.borrow_mut();
                if !a.active {
                    continue;
                }
                let Some((px, py, pz)) = with_host(|h| h.players.player_square(i)) else {
                    continue;
                };
                let zoom = with_host(|h| h.camera.zoom(i));
                let dist = get_distance(px, py, a.x, a.y);
                let mut accurate = false;
                let mut within = false;
                let (mut tx, mut ty) = (0.0f32, 0.0f32);
                if dist < 300 {
                    accurate = true;
                    let (sx, sy) = to_screen_exact(i, a.x as f32, a.y as f32, pz as f32);
                    tx = sx / zoom;
                    ty = sy / zoom;
                    if screen.is_within_inner(tx, ty) {
                        within = true;
                    }
                }
                if !within {
                    a.render_with_angle = true;
                    a.is_draw_on_world = false;
                    a.render_texture = a.texture.clone();
                    a.render_size_mod = 1.0;
                    let (cx, cy) = (screen.center_x, screen.center_y);
                    let angle = if !accurate {
                        let an = get_angle(a.x, a.y, px, py);
                        angle_degrees(angle_degrees(180.0 - an) + 45.0)
                    } else {
                        let an = get_angle(cx as i32, cy as i32, tx as i32, ty as i32);
                        angle_degrees(angle_degrees(180.0 - an) - 90.0)
                    };
                    if angle != a.angle {
                        if !a.last_was_within_view {
                            let l = pzmath::lerp_angle(
                                pzmath::deg_to_rad(a.angle),
                                pzmath::deg_to_rad(angle),
                                0.25 * multiplier(),
                            );
                            a.angle = rad_to_deg(l);
                        } else {
                            a.angle = angle;
                        }
                    }
                    let rad = (a.angle as f64).to_radians();
                    let end_x = cx + 32000.0 * rad.sin() as f32;
                    let end_y = cy + 32000.0 * rad.cos() as f32;
                    a.render_screen_x = cx;
                    a.render_screen_y = cy;
                    let line = (Point { x: cx, y: cy }, Point { x: end_x, y: end_y });
                    for b in screen.borders() {
                        if let Some(p) = intersect(line, b) {
                            a.render_screen_x = p.x;
                            a.render_screen_y = p.y;
                            break;
                        }
                    }
                    a.last_was_within_view = false;
                    continue;
                }
                a.render_with_angle = false;
                a.is_draw_on_world = false;
                a.render_size_mod = 1.0;
                if zoom > 1.0 {
                    a.render_size_mod /= zoom;
                }
                a.render_screen_x = tx;
                a.render_screen_y = ty;
                // The original falls back to texStairsUp when going down.
                a.render_texture = if pz == a.z {
                    a.tex_down.clone().or_else(|| a.texture.clone())
                } else if a.z > pz {
                    a.tex_stairs_up.clone().or_else(|| a.texture.clone())
                } else if a.tex_stairs_down.is_some() {
                    a.tex_stairs_up.clone()
                } else {
                    a.texture.clone()
                };
                a.last_was_within_view = true;
            }
        }
        self.screen = screen;
        Ok(())
    }

    fn update_homing_points(&mut self) -> Result<(), MarkerError> {
        let pi = with_host(|h| h.camera.frame_player_index());
        let mut screen = self.screen;
        let lists = self.points()?;
        for (i, list) in lists.iter_mut().enumerate() {
            if i != pi || list.is_empty() {
                continue;
            }
            list.retain(|p| !p.borrow().is_removed);
            screen.update(i);
            for p in list.iter() {
                let mut p = p.borrow_mut();
                if !p.active {
                    continue;
                }
                let Some((px, py, _)) = with_host(|h| h.players.player_square(i)) else {
                    continue;
                };
                let zoom = with_host(|h| h.camera.zoom(i));
                let m = multiplier();
                p.render_size_mod = 1.0;
                if zoom > 1.0 {
                    p.render_size_mod /= zoom;
                }
                p.dist = get_distance(px, py, p.x, p.y);
                p.target_on_screen = false;
                if (p.dist as f32) < 200.0 {
                    let (sx, sy) = to_screen_exact(i, p.x as f32, p.y as f32, 0.0);
                    p.target_screen_x = sx / zoom;
                    p.target_screen_y = sy / zoom;
                    p.target_screen_x += p.home_on_offset_x / zoom;
                    p.target_screen_y += p.home_on_offset_y / zoom;
                    p.target_on_screen = screen.is_on_screen(p.target_screen_x, p.target_screen_y);
                }
                let base_x = screen.center_x + p.render_offset_x / zoom;
                let base_y = screen.center_y + p.render_offset_y / zoom;
                if !p.custom_target_angle {
                    p.target_angle = if !p.target_on_screen {
                        let an = get_angle(p.x, p.y, px, py);
                        angle_degrees(angle_degrees(180.0 - an) + 45.0)
                    } else {
                        let an = get_angle(
                            base_x as i32,
                            base_y as i32,
                            p.target_screen_x as i32,
                            p.target_screen_y as i32,
                        );
                        angle_degrees(angle_degrees(180.0 - an) - 90.0)
                    };
                }
                if p.target_angle != p.angle {
                    let l = pzmath::lerp_angle(
                        pzmath::deg_to_rad(p.angle),
                        pzmath::deg_to_rad(p.target_angle),
                        p.angle_lerp_val * m,
                    );
                    p.angle = rad_to_deg(l);
                }
                let off = p.stick_to_char_dist / zoom;
                let rad = (p.angle as f64).to_radians();
                p.targ_render_x = base_x + off * rad.sin() as f32;
                p.targ_render_y = base_y + off * rad.cos() as f32;
                let multi = p.movement_lerp_val;
                if p.target_on_screen {
                    let d_targ = get_distance(
                        p.targ_render_x as i32,
                        p.targ_render_y as i32,
                        p.target_screen_x as i32,
                        p.target_screen_y as i32,
                    ) as f32;
                    let mut d_base = get_distance(
                        base_x as i32,
                        base_y as i32,
                        p.target_screen_x as i32,
                        p.target_screen_y as i32,
                    ) as f32;
                    if d_base < d_targ
                        || p.home_on_target_in_view && p.dist <= p.home_on_target_dist
                    {
                        d_base *= 0.75;
                        let tr = (p.target_angle as f64).to_radians();
                        p.targ_render_x = base_x + d_base * tr.sin() as f32;
                        p.targ_render_y = base_y + d_base * tr.cos() as f32;
                    }
                }
                p.targ_render_x = screen.clamp_x(p.targ_render_x);
                p.targ_render_y = screen.clamp_y(p.targ_render_y);
                if p.targ_render_x != p.render_x {
                    p.render_x = pzmath::lerp(p.render_x, p.targ_render_x, multi * m);
                }
                if p.targ_render_y != p.render_y {
                    p.render_y = pzmath::lerp(p.render_y, p.targ_render_y, multi * m);
                }
            }
        }
        self.screen = screen;
        Ok(())
    }

    /// The pulse: up to `alphaMax`, down to `alphaMin` (then back from
    /// 0.3), on player 0's frame only.
    fn update_grid_square_markers(&mut self) {
        if with_host(|h| h.camera.frame_player_index()) != 0 || self.grid_square_markers.is_empty()
        {
            return;
        }
        self.grid_square_markers.retain(|m| !m.borrow().is_removed);
        let mult = multiplier();
        for m in &self.grid_square_markers {
            let mut m = m.borrow_mut();
            if m.alpha_inc {
                m.alpha += mult * m.fade_speed;
                if m.alpha > m.alpha_max {
                    m.alpha_inc = false;
                    m.alpha = m.alpha_max;
                }
            } else {
                m.alpha -= mult * m.fade_speed;
                if m.alpha < m.alpha_min {
                    m.alpha_inc = true;
                    m.alpha = 0.3;
                }
            }
        }
    }

    pub fn remove_grid_square_marker(&mut self, id: i32) -> bool {
        if let Some(j) = self
            .grid_square_markers
            .iter()
            .rposition(|m| m.borrow().id == id)
        {
            self.grid_square_markers[j].borrow_mut().is_removed = true;
            self.grid_square_markers.remove(j);
            return true;
        }
        false
    }

    pub fn get_grid_square_marker(&self, id: i32) -> Option<MarkerRef> {
        self.grid_square_markers
            .iter()
            .find(|m| m.borrow().id == id)
            .cloned()
    }

    /// `addGridSquareMarker(texid, overlay, square, r, g, b, doAlpha, size,
    /// fadeSpeed, fadeMin, fadeMax)`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_grid_square_marker(
        &mut self,
        tex: Option<&str>,
        overlay: Option<&str>,
        sq: (i32, i32, i32),
        c: (f32, f32, f32),
        do_alpha: bool,
        size: f32,
        fade: (f32, f32, f32),
    ) -> Result<Option<MarkerRef>, MarkerError> {
        if is_server() {
            return Ok(None);
        }
        let id = self.next_grid_square_marker_id;
        self.next_grid_square_marker_id += 1;
        let mut m = GridSquareMarker::new(id);
        m.init(tex, overlay, sq.0, sq.1, sq.2, size)?;
        (m.r, m.g, m.b, m.a) = (c.0, c.1, c.2, 1.0);
        m.do_alpha = do_alpha;
        m.fade_speed = fade.0;
        m.alpha = 0.0;
        m.alpha_min = fade.1;
        m.alpha_max = fade.2;
        let m = Rc::new(RefCell::new(m));
        self.grid_square_markers.push(m.clone());
        Ok(Some(m))
    }

    /// `render()`: `update()`, then the homing points and the screen-space
    /// direction arrows of the frame's player.
    pub fn render(&mut self) -> Result<(), MarkerError> {
        self.update()?;
        self.render_homing_point()?;
        self.render_direction_arrow(false)
    }

    pub fn render_homing_point(&mut self) -> Result<(), MarkerError> {
        if is_server() {
            return Ok(());
        }
        let pi = with_host(|h| h.camera.frame_player_index());
        let fog = with_host(|h| h.climate.fog_intensity());
        for (i, list) in self.points()?.iter().enumerate() {
            if i != pi {
                continue;
            }
            for p in list {
                let p = p.borrow();
                let Some(tex) = p.texture.as_ref().filter(|_| p.active) else {
                    continue;
                };
                let mut angle = 180.0 - p.angle;
                if angle < 0.0 {
                    angle += 360.0;
                }
                let mut alpha = p.a;
                if fog > 0.0 && alpha < 1.0 {
                    alpha += (1.0 - alpha) * fog * 2.0;
                    alpha = pzmath::clamp(alpha, 0.0, 1.0);
                }
                draw_texture_angle(
                    tex,
                    p.render_width,
                    p.render_height,
                    p.render_x as f64,
                    p.render_y as f64,
                    angle as f64,
                    Rgba(p.r, p.g, p.b, alpha),
                    p.render_size_mod,
                );
            }
        }
        Ok(())
    }

    pub fn render_direction_arrow(&mut self, world_draw: bool) -> Result<(), MarkerError> {
        if is_server() {
            return Ok(());
        }
        let pi = with_host(|h| h.camera.frame_player_index());
        for (i, list) in self.arrows()?.iter().enumerate() {
            if i != pi {
                continue;
            }
            for a in list {
                let a = a.borrow();
                let Some(tex) = a
                    .render_texture
                    .as_ref()
                    .filter(|_| a.active && a.is_draw_on_world == world_draw)
                else {
                    continue;
                };
                let mut angle = 0.0f32;
                if a.render_with_angle {
                    angle = 180.0 - a.angle;
                    if angle < 0.0 {
                        angle += 360.0;
                    }
                }
                draw_texture_angle(
                    tex,
                    a.render_width,
                    a.render_height,
                    a.render_screen_x as f64,
                    a.render_screen_y as f64,
                    angle as f64,
                    Rgba(a.r, a.g, a.b, a.a),
                    a.render_size_mod,
                );
            }
        }
        Ok(())
    }

    /// `FBORenderWorldMarkers.render(z, gridSquareMarkers)`'s highlights
    /// for level `z` (the world renderer drops those off the chunk map and
    /// draws the rest).
    pub fn fbo_markers(&self, z: i32) -> Vec<FboMarker> {
        let tex =
            |n: &str| pz_render::get_shared_texture(&format!("media/textures/worldMap/{n}.png"));
        self.grid_square_markers
            .iter()
            .filter_map(|m| {
                let m = m.borrow();
                if !m.active || m.z != z as f32 {
                    return None;
                }
                let (cx, cy) = (m.orig_x + 0.5, m.orig_y + 0.5);
                let r = m.size * 0.69;
                let mut f = FboMarker {
                    x1: cx - r,
                    y1: cy - r,
                    x2: cx + r,
                    y2: cy + r,
                    z: m.orig_z,
                    color: Rgba(m.r, m.g, m.b, m.alpha),
                    texture1: None,
                    texture2: None,
                };
                match m.texture_name.as_str() {
                    "circle_center" | "circle_highlight_2" => f.texture1 = tex("circle_center"),
                    "circle_highlight" => {
                        f.texture1 = tex("circle_center");
                        f.texture2 = tex("circle_only_highlight");
                    }
                    _ => {}
                }
                if matches!(
                    m.overlay_texture_name.as_deref(),
                    Some("circle_only_highlight" | "circle_only_highlight_2")
                ) {
                    f.texture2 = tex("circle_only_highlight");
                }
                Some(f)
            })
            .collect()
    }

    pub fn grid_square_markers(&self) -> &[MarkerRef] {
        &self.grid_square_markers
    }
}

/// `DrawTextureAngle`: the texture rotated about its centre.
#[allow(clippy::too_many_arguments)]
fn draw_texture_angle(
    tex: &Texture,
    w: f32,
    h: f32,
    cx: f64,
    cy: f64,
    angle: f64,
    c: Rgba,
    size: f32,
) {
    let dx = w * size / 2.0;
    let dy = h * size / 2.0;
    let rad = (180.0 + angle).to_radians();
    let (xc, xs) = (rad.cos() * dx as f64, rad.sin() * dx as f64);
    let (yc, ys) = (rad.cos() * dy as f64, rad.sin() * dy as f64);
    let pts = [
        [xc - ys, yc + xs],
        [-xc - ys, yc - xs],
        [-xc + ys, -yc - xs],
        [xc + ys, -yc + xs],
    ];
    let pts = pts.map(|p| [(p[0] + cx) as f32, (p[1] + cy) as f32]);
    with_ui(|m| m.sprite.render4(Some(tex), pts, c));
}

/// `WorldMarkers.instance.render()` for the frame's player
/// (`IngameState.renderframeui`).
pub fn render(_t: &mut Thread) {
    if let Err(e) = with_markers(WorldMarkers::render) {
        eprintln!("WorldMarkers.render: {e:?}");
    }
}

// ---- Lua ------------------------------------------------------------------------

use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg, with_this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};
use std::collections::HashMap;
use std::rc::Weak;

/// `WorldMarkers.instance` as Lua sees it.
pub struct WorldMarkersObject;

impl LuaObject for WorldMarkersObject {
    fn class_name(&self) -> &str {
        WORLD_MARKERS
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

macro_rules! lua_wrapper {
    ($name:ident, $inner:ty, $class:expr) => {
        pub struct $name(pub Rc<RefCell<$inner>>);
        impl LuaObject for $name {
            fn class_name(&self) -> &str {
                $class
            }
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
    };
}
lua_wrapper!(ArrowObject, DirectionArrow, DIRECTION_ARROW);
lua_wrapper!(PointObject, PlayerHomingPoint, PLAYER_HOMING_POINT);
lua_wrapper!(MarkerObject, GridSquareMarker, GRID_SQUARE_MARKER);

thread_local! {
    /// One Lua object per marker, so Lua's `==` holds.
    static OBJECTS: RefCell<HashMap<usize, Weak<dyn LuaObject>>> = RefCell::default();
    static INSTANCE_VALUE: Value = Value::Object(Rc::new(WorldMarkersObject));
}

fn object<T: 'static>(r: &Rc<RefCell<T>>, make: impl FnOnce() -> Rc<dyn LuaObject>) -> Value {
    let key = Rc::as_ptr(r) as *const () as usize;
    OBJECTS.with(|o| {
        let mut o = o.borrow_mut();
        if let Some(v) = o.get(&key).and_then(Weak::upgrade) {
            return Value::Object(v);
        }
        let v = make();
        o.insert(key, Rc::downgrade(&v));
        Value::Object(v)
    })
}

fn arrow_value(a: Option<ArrowRef>) -> Value {
    a.map_or(Value::Nil, |a| {
        object(&a, || Rc::new(ArrowObject(a.clone())))
    })
}
fn point_value(p: Option<PointRef>) -> Value {
    p.map_or(Value::Nil, |p| {
        object(&p, || Rc::new(PointObject(p.clone())))
    })
}
fn marker_value(m: Option<MarkerRef>) -> Value {
    m.map_or(Value::Nil, |m| {
        object(&m, || Rc::new(MarkerObject(m.clone())))
    })
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

fn lua_err(e: MarkerError) -> LuaError {
    match e {
        MarkerError::Null => npe(),
        MarkerError::IndexOutOfBounds(i, n) => LuaError::Runtime {
            message: format!("Index {i} out of bounds for length {n}"),
            class: "java.lang.IndexOutOfBoundsException",
        },
    }
}

fn player_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<usize> {
    let v = c.arg(i);
    with_host(|h| h.players.player_index_of(&v)).ok_or_else(npe)
}

fn f(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

/// An id or the marker object of type `T` (its id via `id`).
fn id_arg<T: 'static, W: 'static>(
    c: &NativeCall<'_>,
    i: usize,
    inner: impl Fn(&W) -> &Rc<RefCell<T>>,
    id: impl Fn(&T) -> i32,
) -> LuaResult<i32> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<W>()
            .map(|w| id(&inner(w).borrow()))
            .ok_or_else(npe),
        Value::Nil => Err(npe()),
        _ => Ok(int_arg(c, i)),
    }
}

fn arrow(c: &NativeCall<'_>) -> LuaResult<ArrowRef> {
    with_this::<ArrowObject, _>(c, |o| o.0.clone())
}
fn point(c: &NativeCall<'_>) -> LuaResult<PointRef> {
    with_this::<PointObject, _>(c, |o| o.0.clone())
}
fn marker(c: &NativeCall<'_>) -> LuaResult<MarkerRef> {
    with_this::<MarkerObject, _>(c, |o| o.0.clone())
}

/// Register `getWorldMarkers()` and the four classes.
pub fn register(thread: &mut Thread) {
    global(thread, "getWorldMarkers", |c| {
        Ok(c.push(INSTANCE_VALUE.with(Clone::clone)))
    });
    let b = expose_class(thread, WORLD_MARKERS, None);
    let w = |r: Result<Value, MarkerError>| r.map_err(lua_err);
    b.method("init", |_| {
        with_markers(WorldMarkers::init);
        Ok(0)
    });
    b.method("reset", |_| {
        with_markers(WorldMarkers::reset).map_err(lua_err)?;
        Ok(0)
    });
    b.method("update", |_| {
        with_markers(WorldMarkers::update).map_err(lua_err)?;
        Ok(0)
    });
    b.method("render", |_| {
        with_markers(WorldMarkers::render).map_err(lua_err)?;
        Ok(0)
    });
    b.method("renderHomingPoint", |_| {
        with_markers(WorldMarkers::render_homing_point).map_err(lua_err)?;
        Ok(0)
    });
    b.method("renderDirectionArrow", |c| {
        let wd = bool_arg(c, 1);
        with_markers(|m| m.render_direction_arrow(wd)).map_err(lua_err)?;
        Ok(0)
    });
    b.method("debugRender", |_| Ok(0));
    // The FBO chunk renderer draws the grid-square markers (`fbo_markers`).
    b.method("renderGridSquareMarkers", |_| Ok(0));
    b.method("getHomingPoint", move |c| {
        let id = int_arg(c, 1);
        let v = w(with_markers(|m| m.get_homing_point(id)).map(point_value))?;
        Ok(c.push(v))
    });
    b.method("addPlayerHomingPoint", move |c| {
        let pl = player_arg(c, 1)?;
        let (x, y) = (int_arg(c, 2), int_arg(c, 3));
        let r = match c.nargs() - 1 {
            3 => with_markers(|m| {
                m.add_player_homing_point(
                    pl,
                    x,
                    y,
                    Some("arrow_triangle"),
                    Rgba(1.0, 1.0, 1.0, 1.0),
                    true,
                    20,
                )
            }),
            7 => {
                let col = Rgba(f(c, 4), f(c, 5), f(c, 6), f(c, 7));
                with_markers(|m| {
                    m.add_player_homing_point(pl, x, y, Some("arrow_triangle"), col, true, 20)
                })
            }
            _ => {
                let tex = str_arg(c, 4);
                let col = Rgba(f(c, 5), f(c, 6), f(c, 7), f(c, 8));
                let (home, dist) = (bool_arg(c, 9), int_arg(c, 10));
                with_markers(|m| {
                    m.add_player_homing_point(pl, x, y, tex.as_deref(), col, home, dist)
                })
            }
        };
        let v = w(r.map(point_value))?;
        Ok(c.push(v))
    });
    b.method("removeHomingPoint", |c| {
        let id = id_arg(c, 1, |o: &PointObject| &o.0, |p| p.id)?;
        let r = with_markers(|m| m.remove_homing_point(id)).map_err(lua_err)?;
        Ok(c.push(r))
    });
    b.method("removePlayerHomingPoint", |c| {
        let pl = player_arg(c, 1)?;
        let id = id_arg(c, 2, |o: &PointObject| &o.0, |p| p.id)?;
        let r = with_markers(|m| m.remove_player_homing_point(pl, id)).map_err(lua_err)?;
        Ok(c.push(r))
    });
    b.method("removeAllHomingPoints", |c| {
        let pl = player_arg(c, 1)?;
        with_markers(|m| m.remove_all_homing_points(pl)).map_err(lua_err)?;
        Ok(0)
    });
    b.method("getDirectionArrow", move |c| {
        let id = int_arg(c, 1);
        let v = w(with_markers(|m| m.get_direction_arrow(id)).map(arrow_value))?;
        Ok(c.push(v))
    });
    b.method("addDirectionArrow", move |c| {
        let pl = player_arg(c, 1)?;
        let (x, y, z) = (int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let tex = str_arg(c, 5);
        let col = Rgba(f(c, 6), f(c, 7), f(c, 8), f(c, 9));
        let v = w(
            with_markers(|m| m.add_direction_arrow(pl, x, y, z, tex.as_deref(), col))
                .map(arrow_value),
        )?;
        Ok(c.push(v))
    });
    b.method("removeDirectionArrow", |c| {
        let id = id_arg(c, 1, |o: &ArrowObject| &o.0, |a| a.id)?;
        let r = with_markers(|m| m.remove_direction_arrow(id)).map_err(lua_err)?;
        Ok(c.push(r))
    });
    b.method("removePlayerDirectionArrow", |c| {
        let pl = player_arg(c, 1)?;
        let id = id_arg(c, 2, |o: &ArrowObject| &o.0, |a| a.id)?;
        let r = with_markers(|m| m.remove_player_direction_arrow(pl, id)).map_err(lua_err)?;
        Ok(c.push(r))
    });
    b.method("removeAllDirectionArrows", |c| {
        let pl = player_arg(c, 1)?;
        with_markers(|m| m.remove_all_direction_arrows(pl)).map_err(lua_err)?;
        Ok(0)
    });
    b.method("removeGridSquareMarker", |c| {
        let id = id_arg(c, 1, |o: &MarkerObject| &o.0, |m| m.id)?;
        Ok(c.push(with_markers(|m| m.remove_grid_square_marker(id))))
    });
    b.method("getGridSquareMarker", |c| {
        let id = int_arg(c, 1);
        Ok(c.push(marker_value(with_markers(|m| m.get_grid_square_marker(id)))))
    });
    b.method("addGridSquareMarker", move |c| {
        // (square, r, g, b, doAlpha, size) or (texid, overlay, square, ...).
        let (tex, overlay, at) = if c.nargs() - 1 == 6 {
            (
                Some("circle_center".to_string()),
                Some("circle_only_highlight".to_string()),
                1,
            )
        } else {
            (
                str_arg(c, 1).map(|s| s.to_string()),
                str_arg(c, 2).map(|s| s.to_string()),
                3,
            )
        };
        let sq = c.arg(at);
        let sq = with_host(|h| h.picking.square_coords(&sq)).ok_or_else(npe)?;
        let col = (f(c, at + 1), f(c, at + 2), f(c, at + 3));
        let (do_alpha, size) = (bool_arg(c, at + 4), f(c, at + 5));
        let fade = if c.nargs() > 11 {
            (f(c, at + 6), f(c, at + 7), f(c, at + 8))
        } else {
            (0.006, 0.3, 1.0)
        };
        let v = w(with_markers(|m| {
            m.add_grid_square_marker(
                tex.as_deref(),
                overlay.as_deref(),
                sq,
                col,
                do_alpha,
                size,
                fade,
            )
        })
        .map(marker_value))?;
        Ok(c.push(v))
    });

    macro_rules! props {
        ($b:ident, $get:ident, { $($g:literal => |$v:ident| $ge:expr),* $(,)? }, { $($s:literal => |$t:ident, $c:ident| $se:expr),* $(,)? }) => {
            $( $b.method($g, |c| { let r = $get(c)?; let $v = r.borrow(); let out: Value = $ge.into(); Ok(c.push(out)) }); )*
            $( $b.method($s, |$c| { let r = $get($c)?; let mut $t = r.borrow_mut(); $se; Ok(0) }); )*
        };
    }

    let b = expose_class(thread, DIRECTION_ARROW, None);
    props!(b, arrow, {
        "isRemoved" => |a| a.is_removed,
        "isActive" => |a| a.active,
        "getR" => |a| a.r as f64,
        "getG" => |a| a.g as f64,
        "getB" => |a| a.b as f64,
        "getA" => |a| a.a as f64,
        "getID" => |a| a.id as f64,
        "getX" => |a| a.x as f64,
        "getY" => |a| a.y as f64,
        "getZ" => |a| a.z as f64,
        "getRenderWidth" => |a| a.render_width as f64,
        "getRenderHeight" => |a| a.render_height as f64,
    }, {
        "remove" => |a, c| a.is_removed = true,
        "setActive" => |a, c| a.active = bool_arg(c, 1),
        "setR" => |a, c| a.r = f(c, 1),
        "setG" => |a, c| a.g = f(c, 1),
        "setB" => |a, c| a.b = f(c, 1),
        "setA" => |a, c| a.a = f(c, 1),
        "setRGBA" => |a, c| (a.r, a.g, a.b, a.a) = (f(c, 1), f(c, 2), f(c, 3), f(c, 4)),
        "setX" => |a, c| a.x = int_arg(c, 1),
        "setY" => |a, c| a.y = int_arg(c, 1),
        "setZ" => |a, c| a.set_z(int_arg(c, 1)),
        "setRenderWidth" => |a, c| a.render_width = f(c, 1),
        "setRenderHeight" => |a, c| a.render_height = f(c, 1),
        "setTexture" => |a, c| a.set_texture(str_arg(c, 1).as_deref()),
        "setTexDown" => |a, c| a.set_tex_down(&str_arg(c, 1).ok_or_else(npe)?),
        "setTexStairsDown" => |a, c| a.set_tex_stairs_down(&str_arg(c, 1).ok_or_else(npe)?),
        "setTexStairsUp" => |a, c| a.set_tex_stairs_up(&str_arg(c, 1).ok_or_else(npe)?),
    });

    let b = expose_class(thread, PLAYER_HOMING_POINT, None);
    props!(b, point, {
        "isRemoved" => |p| p.is_removed,
        "isActive" => |p| p.active,
        "getR" => |p| p.r as f64,
        "getG" => |p| p.g as f64,
        "getB" => |p| p.b as f64,
        "getA" => |p| p.a as f64,
        "getHomeOnTargetDist" => |p| p.home_on_target_dist as f64,
        "getID" => |p| p.id as f64,
        "getTargetAngle" => |p| p.target_angle as f64,
        "isCustomTargetAngle" => |p| p.custom_target_angle,
        "getX" => |p| p.x as f64,
        "getY" => |p| p.y as f64,
        "getAngleLerpVal" => |p| p.angle_lerp_val as f64,
        "getMovementLerpVal" => |p| p.movement_lerp_val as f64,
        "isHomeOnTargetInView" => |p| p.home_on_target_in_view,
        "getRenderWidth" => |p| p.render_width as f64,
        "getRenderHeight" => |p| p.render_height as f64,
        "getStickToCharDist" => |p| p.stick_to_char_dist as f64,
        "getRenderOffsetX" => |p| p.render_offset_x as f64,
        "getRenderOffsetY" => |p| p.render_offset_y as f64,
        "getHomeOnOffsetX" => |p| p.home_on_offset_x as f64,
        "getHomeOnOffsetY" => |p| p.home_on_offset_y as f64,
    }, {
        "remove" => |p, c| p.is_removed = true,
        "setActive" => |p, c| p.active = bool_arg(c, 1),
        "setR" => |p, c| p.r = f(c, 1),
        "setG" => |p, c| p.g = f(c, 1),
        "setB" => |p, c| p.b = f(c, 1),
        "setA" => |p, c| p.a = f(c, 1),
        "setHomeOnTargetDist" => |p, c| p.home_on_target_dist = int_arg(c, 1),
        "setTargetAngle" => |p, c| p.target_angle = f(c, 1),
        "setCustomTargetAngle" => |p, c| p.custom_target_angle = bool_arg(c, 1),
        "setX" => |p, c| p.x = int_arg(c, 1),
        "setY" => |p, c| p.y = int_arg(c, 1),
        "setAngleLerpVal" => |p, c| p.angle_lerp_val = f(c, 1),
        "setMovementLerpVal" => |p, c| p.movement_lerp_val = f(c, 1),
        "setHomeOnTargetInView" => |p, c| p.home_on_target_in_view = bool_arg(c, 1),
        "setRenderWidth" => |p, c| p.render_width = f(c, 1),
        "setRenderHeight" => |p, c| p.render_height = f(c, 1),
        "setStickToCharDist" => |p, c| p.stick_to_char_dist = f(c, 1),
        "setRenderOffsetX" => |p, c| p.render_offset_x = f(c, 1),
        "setRenderOffsetY" => |p, c| p.render_offset_y = f(c, 1),
        "setHomeOnOffsetX" => |p, c| p.home_on_offset_x = f(c, 1),
        "setHomeOnOffsetY" => |p, c| p.home_on_offset_y = f(c, 1),
        "setTexture" => |p, c| p.set_texture(str_arg(c, 1).as_deref()),
        "setTableSurface" => |p, c| p.set_table_surface(),
        "setHighCounter" => |p, c| p.set_high_counter(),
        "setYOffsetScaled" => |p, c| p.set_y_offset_scaled(f(c, 1)),
        "setXOffsetScaled" => |p, c| p.set_x_offset_scaled(f(c, 1)),
    });

    let b = expose_class(thread, GRID_SQUARE_MARKER, None);
    props!(b, marker, {
        "getID" => |m| m.id as f64,
        "isRemoved" => |m| m.is_removed,
        "isActive" => |m| m.active,
        "getSize" => |m| m.size as f64,
        "getX" => |m| m.x as f64,
        "getY" => |m| m.y as f64,
        "getZ" => |m| m.z as f64,
        "getR" => |m| m.r as f64,
        "getG" => |m| m.g as f64,
        "getB" => |m| m.b as f64,
        "getA" => |m| m.a as f64,
        "getAlpha" => |m| m.alpha as f64,
        "getAlphaMax" => |m| m.alpha_max as f64,
        "getAlphaMin" => |m| m.alpha_min as f64,
        "isDoAlpha" => |m| m.do_alpha,
        "getFadeSpeed" => |m| m.fade_speed as f64,
        "isDoBlink" => |m| m.do_blink,
        "isScaleCircleTexture" => |m| m.scale_circle_texture,
        "getOriginalX" => |m| m.orig_x as f64,
        "getOriginalY" => |m| m.orig_y as f64,
        "getOriginalZ" => |m| m.orig_z as f64,
        "getTextureName" => |m| Value::str(&m.texture_name),
        "getOverlayTextureName" => |m| m.overlay_texture_name.as_deref().map_or(Value::Nil, Value::str),
    }, {
        "remove" => |m, c| m.is_removed = true,
        "setActive" => |m, c| m.active = bool_arg(c, 1),
        "setR" => |m, c| m.r = f(c, 1),
        "setG" => |m, c| m.g = f(c, 1),
        "setB" => |m, c| m.b = f(c, 1),
        "setA" => |m, c| m.a = f(c, 1),
        "setAlpha" => |m, c| m.alpha = f(c, 1),
        "setAlphaMax" => |m, c| m.alpha_max = f(c, 1),
        "setAlphaMin" => |m, c| m.alpha_min = f(c, 1),
        "setDoAlpha" => |m, c| m.do_alpha = bool_arg(c, 1),
        "setFadeSpeed" => |m, c| m.fade_speed = f(c, 1),
        "setDoBlink" => |m, c| m.do_blink = bool_arg(c, 1),
        "setScaleCircleTexture" => |m, c| m.set_scale_circle_texture(bool_arg(c, 1)),
        "setPosAndSize" => |m, c| m.set_pos_and_size(int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), f(c, 4)),
        "setPos" => |m, c| m.set_pos(int_arg(c, 1), int_arg(c, 2), int_arg(c, 3)),
        "setSize" => |m, c| m.set_size(f(c, 1)),
        "init" => |m, c| {
            let (t, o) = (str_arg(c, 1), str_arg(c, 2));
            m.init(t.as_deref(), o.as_deref(), int_arg(c, 3), int_arg(c, 4), int_arg(c, 5), f(c, 6)).map_err(|_| npe())?
        },
    });
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudCamera, HudClimate, HudGameTime, HudHost, HudPlayers};
    use super::super::test_util::*;
    use super::*;
    use pz_ui::tooltip::java_float_to_string as jf;
    use pz_ui::UiDrawCmd;
    use std::cell::Cell;

    thread_local! {
        static SQUARE: Cell<(i32, i32, i32)> = const { Cell::new((100, 200, 0)) };
        static FOG: Cell<f32> = const { Cell::new(0.0) };
        static MULT: Cell<f32> = const { Cell::new(1.0) };
        static OFF: Cell<(f32, f32)> = const { Cell::new((0.0, 0.0)) };
    }
    struct P;
    impl HudPlayers for P {
        fn num_players(&self) -> i32 {
            1
        }
        fn player_square(&self, i: usize) -> Option<(i32, i32, i32)> {
            (i == 0).then(|| SQUARE.with(Cell::get))
        }
        fn player_index_of(&self, v: &Value) -> Option<usize> {
            matches!(v, Value::Num(_)).then_some(0)
        }
    }
    struct Cam;
    impl HudCamera for Cam {
        fn camera_offset(&self, _p: usize) -> (f32, f32) {
            OFF.with(Cell::get)
        }
    }
    struct Clim;
    impl HudClimate for Clim {
        fn fog_intensity(&self) -> f32 {
            FOG.with(Cell::get)
        }
    }
    struct Time;
    impl HudGameTime for Time {
        fn get_multiplier(&self) -> f32 {
            MULT.with(Cell::get)
        }
    }

    fn host() -> HudHost {
        HudHost {
            players: Box::new(P),
            camera: Box::new(Cam),
            climate: Box::new(Clim),
            game_time: Box::new(Time),
            ..HudHost::default()
        }
    }

    fn b(v: bool) -> &'static str {
        if v {
            "true"
        } else {
            "false"
        }
    }
    fn pt(l: &str, p: &PointRef) -> String {
        let p = p.borrow();
        format!(
            "{l} id {} render {} {} targ {} {} angle {} target {} dist {} onscreen {} tscreen {} {}",
            p.id,
            jf(p.render_x),
            jf(p.render_y),
            jf(p.targ_render_x),
            jf(p.targ_render_y),
            jf(p.angle),
            jf(p.target_angle),
            p.dist,
            b(p.target_on_screen),
            jf(p.target_screen_x),
            jf(p.target_screen_y)
        )
    }
    fn ar(l: &str, a: &ArrowRef) -> String {
        let a = a.borrow();
        format!(
            "{l} id {} screen {} {} angle {} withAngle {} within {} size {} tex {}",
            a.id,
            jf(a.render_screen_x),
            jf(a.render_screen_y),
            jf(a.angle),
            b(a.render_with_angle),
            b(a.last_was_within_view),
            jf(a.render_size_mod),
            a.render_texture
                .as_ref()
                .map_or("null".to_string(), |t| t.name.to_string())
        )
    }
    fn mk(l: &str, m: &MarkerRef) -> String {
        let m = m.borrow();
        format!(
            "{l} id {} pos {} {} {} orig {} {} size {} alpha {} tex {} {}",
            m.id,
            jf(m.x),
            jf(m.y),
            jf(m.z),
            jf(m.orig_x),
            jf(m.orig_y),
            jf(m.size),
            jf(m.alpha),
            m.texture_name,
            m.overlay_texture_name.as_deref().unwrap_or("null")
        )
    }
    fn flush(got: &mut Vec<String>, l: &str) {
        got.push(format!("case {l}"));
        let v: Vec<String> = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Quad4 {
                        texture,
                        points,
                        color,
                    } => {
                        let mut s = format!(
                            "quad {}",
                            texture
                                .as_ref()
                                .map_or("null".to_string(), |t| t.name.to_string())
                        );
                        for p in points {
                            s.push_str(&format!(" {} {}", jf(p[0]), jf(p[1])));
                        }
                        Some(format!(
                            "{s} {} {} {} {}",
                            jf(color.0),
                            jf(color.1),
                            jf(color.2),
                            jf(color.3)
                        ))
                    }
                    _ => None,
                })
                .collect()
        });
        got.extend(v);
        with_ui(|m| m.sprite.clear());
    }
    /// The oracle's `quad` lines print the corners as doubles; the sprite
    /// takes floats, so compare them as floats.
    fn normalise(l: &str) -> String {
        match l.strip_prefix("quad ") {
            Some(r) => {
                let p: Vec<&str> = r.split(' ').collect();
                let mut s = format!("quad {}", p[0]);
                for v in &p[1..9] {
                    s.push_str(&format!(" {}", jf(v.parse::<f64>().unwrap() as f32)));
                }
                format!("{s} {}", p[9..].join(" "))
            }
            None => l.to_string(),
        }
    }

    /// Parity with the original `WorldMarkers` (oracle `markers.txt`).
    #[test]
    fn matches_markers_oracle() {
        let Some(text) = oracle("markers.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_only(&oracle_textures_ex(&text));
        install(host());
        let _t = thread();
        crate::bind_core::shared_core()
            .borrow_mut()
            .set_screen_size(1280, 720);
        crate::bind_core::shared_core().borrow_mut().tile_scale = 2;
        let mult: f32 = text
            .lines()
            .find_map(|l| l.strip_prefix("multiplier "))
            .unwrap()
            .parse()
            .unwrap();
        MULT.with(|m| m.set(mult));
        let ts = |x: f32, y: f32| (x * 64.0 - y * 64.0, y * 32.0 + x * 32.0);
        let (cx, cy) = ts(100.0, 200.0);
        OFF.with(|o| o.set(((cx - 640.0) as i32 as f32, (cy - 360.0) as i32 as f32)));
        SQUARE.with(|s| s.set((100, 200, 0)));
        FOG.with(|f| f.set(0.0));
        with_ui(|m| m.sprite.clear());
        let mut got = vec![
            format!("multiplier {}", jf(mult)),
            format!(
                "camera {} {}",
                jf(OFF.with(Cell::get).0),
                jf(OFF.with(Cell::get).1)
            ),
        ];
        let res = with_markers(|w| -> Result<(), MarkerError> {
            *w = WorldMarkers::default();
            w.init();
            let white = Rgba(1.0, 1.0, 1.0, 1.0);
            let far = w
                .add_player_homing_point(0, 600, 250, Some("arrow_triangle"), white, true, 20)?
                .unwrap();
            let near = w
                .add_player_homing_point(
                    0,
                    104,
                    203,
                    Some("arrow_triangle"),
                    Rgba(0.5, 0.6, 0.7, 0.5),
                    true,
                    20,
                )?
                .unwrap();
            let on_top = w
                .add_player_homing_point(0, 110, 195, Some("arrow_custom"), white, false, 5)?
                .unwrap();
            on_top.borrow_mut().set_table_surface();
            let a_far = w
                .add_direction_arrow(0, 800, 700, 0, None, Rgba(1.0, 0.0, 0.0, 1.0))?
                .unwrap();
            let a_off = w
                .add_direction_arrow(
                    0,
                    120,
                    170,
                    0,
                    Some("dir_arrow_up"),
                    Rgba(0.0, 1.0, 0.0, 1.0),
                )?
                .unwrap();
            let a_in = w
                .add_direction_arrow(
                    0,
                    101,
                    201,
                    0,
                    Some("dir_arrow_up"),
                    Rgba(0.0, 0.0, 1.0, 1.0),
                )?
                .unwrap();
            let a_up = w
                .add_direction_arrow(
                    0,
                    102,
                    199,
                    2,
                    Some("dir_arrow_up"),
                    Rgba(1.0, 1.0, 0.0, 1.0),
                )?
                .unwrap();
            let a_down = w
                .add_direction_arrow(
                    0,
                    99,
                    201,
                    -1,
                    Some("dir_arrow_up"),
                    Rgba(1.0, 0.0, 1.0, 1.0),
                )?
                .unwrap();
            let m1 = w
                .add_grid_square_marker(
                    Some("circle_center"),
                    Some("circle_only_highlight"),
                    (100, 200, 0),
                    (0.2, 0.3, 0.4),
                    true,
                    1.5,
                    (0.006, 0.3, 1.0),
                )?
                .unwrap();
            let m2 = w
                .add_grid_square_marker(
                    Some("circle_center"),
                    None,
                    (105, 206, 1),
                    (1.0, 1.0, 1.0),
                    false,
                    2.0,
                    (0.1, 0.4, 0.9),
                )?
                .unwrap();
            m2.borrow_mut().set_scale_circle_texture(true);
            got.push(mk("m1", &m1));
            got.push(mk("m2", &m2));
            for f in 0..6 {
                if f == 3 {
                    FOG.with(|x| x.set(0.2));
                    SQUARE.with(|s| s.set((300, 200, 0)));
                }
                if f == 4 {
                    SQUARE.with(|s| s.set((100, 200, 0)));
                    a_off.borrow_mut().set_z(40);
                    near.borrow_mut().custom_target_angle = true;
                    near.borrow_mut().target_angle = 33.0;
                }
                w.render()?;
                got.push(format!("frame {f}"));
                got.push(pt("far", &far));
                got.push(pt("near", &near));
                got.push(pt("onTop", &on_top));
                for (l, a) in [
                    ("aFar", &a_far),
                    ("aOff", &a_off),
                    ("aIn", &a_in),
                    ("aUp", &a_up),
                    ("aDown", &a_down),
                ] {
                    got.push(ar(l, a));
                }
                got.push(mk("m1", &m1));
                got.push(mk("m2", &m2));
                flush(&mut got, &format!("frame {f}"));
            }
            for _ in 0..70 {
                w.update()?;
            }
            got.push(mk("m1 pulsed", &m1));
            got.push(mk("m2 pulsed", &m2));
            m1.borrow_mut().set_size(3.0);
            got.push(mk("m1 size", &m1));
            m1.borrow_mut().set_pos(7, 8, 0);
            got.push(mk("m1 pos", &m1));
            m2.borrow_mut().set_scale_circle_texture(false);
            got.push(mk("m2 unscaled", &m2));
            got.push(format!(
                "get last {}",
                w.get_homing_point(on_top.borrow().id)?.unwrap().borrow().id
            ));
            let Err(MarkerError::IndexOutOfBounds(i, n)) = w.get_homing_point(far.borrow().id)
            else {
                panic!()
            };
            got.push(format!("get first java.lang.IndexOutOfBoundsException: Index {i} out of bounds for length {n}"));
            got.push(format!(
                "get arrow {} {}",
                w.get_direction_arrow(a_off.borrow().id)?
                    .unwrap()
                    .borrow()
                    .id,
                w.get_direction_arrow(999)?
                    .map_or("null".into(), |a| a.borrow().id.to_string())
            ));
            let nid = near.borrow().id;
            let (r1, r2) = (w.remove_homing_point(nid)?, w.remove_homing_point(nid)?);
            let aid = a_in.borrow().id;
            let r3 = w.remove_player_direction_arrow(0, aid)?;
            let m1id = m1.borrow().id;
            let r4 = w.remove_grid_square_marker(m1id);
            let g2 = w
                .get_grid_square_marker(m2.borrow().id)
                .unwrap()
                .borrow()
                .id;
            got.push(format!(
                "remove {} {} {} {} {} {g2}",
                b(r1),
                b(r2),
                b(near.borrow().is_removed),
                b(r3),
                b(r4)
            ));
            a_up.borrow_mut().is_removed = true;
            w.render()?;
            flush(&mut got, "after remove");
            w.reset()?;
            w.render()?;
            flush(&mut got, "after reset");
            Ok(())
        });
        res.unwrap();
        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("fake ") && !l.starts_with("tex "))
            .map(normalise)
            .collect();
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "line {i}");
        }
        assert_eq!(got.len(), want.len());
    }

    /// Before `init()` the per-player lists are null; the FBO highlights;
    /// the Lua API builds and finds the same objects.
    #[test]
    fn init_fbo_and_lua() {
        install_textures(&[("media/textures/highlights/circle_center.png", 128, 128)]);
        install(host());
        let mut t = thread();
        with_markers(|w| {
            *w = WorldMarkers::default();
            assert!(matches!(
                w.add_direction_arrow(0, 1, 2, 0, None, Rgba(1.0, 1.0, 1.0, 1.0)),
                Err(MarkerError::Null)
            ));
            w.init();
            let m = w
                .add_grid_square_marker(
                    Some("circle_highlight"),
                    Some("circle_only_highlight_2"),
                    (10, 20, 1),
                    (0.1, 0.2, 0.3),
                    true,
                    2.0,
                    (0.006, 0.3, 1.0),
                )
                .unwrap()
                .unwrap();
            m.borrow_mut().alpha = 0.5;
            let f = w.fbo_markers(1);
            assert_eq!(f.len(), 1);
            assert_eq!(
                (f[0].x1, f[0].y1, f[0].x2, f[0].y2),
                (10.5 - 1.38, 20.5 - 1.38, 10.5 + 1.38, 20.5 + 1.38)
            );
            assert_eq!(f[0].color, Rgba(0.1, 0.2, 0.3, 0.5));
            assert!(f[0]
                .texture1
                .as_ref()
                .unwrap()
                .name
                .ends_with("worldMap/circle_center.png"));
            assert!(f[0]
                .texture2
                .as_ref()
                .unwrap()
                .name
                .ends_with("worldMap/circle_only_highlight.png"));
            assert!(w.fbo_markers(0).is_empty());
        });
        run(
            &mut t,
            "local w = getWorldMarkers()
             p = w:addPlayerHomingPoint(0, 5, 6); p:setX(7); p:setHomeOnOffsetY(-12)
             same = w:getHomingPoint(p:getID()) == p
             a = w:addDirectionArrow(0, 1, 2, 3, nil, 1, 1, 1, 1); a:setZ(-99)
             az = a:getZ(); removed = w:removePlayerHomingPoint(0, p); gone = p:isRemoved()",
        );
        assert!(eval(&mut t, "same").truthy());
        assert_eq!(eval(&mut t, "az").as_num(), Some(-32.0));
        assert!(eval(&mut t, "removed").truthy());
        assert!(eval(&mut t, "gone").truthy());
    }
}
