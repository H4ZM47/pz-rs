//! `WeatherFxMask`: keeps the weather effects out of the player's building.
//!
//! While the player is inside (`PlayerFxMask.hasMaskToDraw`), the squares
//! of the building on the player's level that are on screen are found by
//! rasterising the view (`scanForTiles`, `RasterizeBounds`,
//! `zombie.worldMap.Rasterize`) and recorded with flags
//! (`addMaskLocation`): floors to cover, walls and the objects and
//! characters just outside them that stand in front. Each frame
//! (`renderFxMask`) the mask is drawn into one screen-sized texture
//! (`fboMask`): floor and wall stand-in sprites blended in, walls of the
//! squares outside the building cut back out (`glBlendFunc(0, 0)` with a
//! reverse-subtract equation). Every weather layer is then drawn into a
//! second texture (`fboParticles`), the mask subtracted from it, and the
//! result blended over the world. Outside, the layers are drawn straight
//! into the world (`IsoWeatherFX.render`).
//!
//! Frame order, as the original calls it: [`WeatherFxMask::init_mask`]
//! from `IsoWorld.render`; [`WeatherFxMask::check_visible_squares`] (then
//! `force_mask_update` and `init_mask` when it says so) and
//! [`WeatherFxMask::set_diamond_iter_done`] from `FBORenderCell` when the
//! grid stacks were recalculated; then [`render_weather_fx`] from
//! `IsoWorld.renderWeatherFX`, inside the offscreen world pass.

use std::collections::HashMap;
use std::rc::Rc;

use pz_render::iso::{fastfloor, x_to_iso, x_to_screen, y_to_iso, y_to_screen, TILE_SCALE};
use pz_render::{gl, DrawCmd, DrawList, Rgba, Texture};

use crate::weather_fx::{IsoWeatherFx, WeatherFxView};
use crate::world::{BuildingId, RenderObject, RenderSquare, SpriteInfo, TileType};

pub const BIT_FLOOR: i32 = 0;
pub const BIT_WALLN: i32 = 1;
pub const BIT_WALLW: i32 = 2;
pub const BIT_IS_CUT: i32 = 4;
pub const BIT_CHARS: i32 = 8;
pub const BIT_OBJECTS: i32 = 16;
pub const BIT_WALL_SE: i32 = 32;
pub const BIT_DOOR: i32 = 64;

/// An `IsoWorldRegion` id.
pub type RegionId = u32;

/// What the mask reads from the world.
pub trait FxMaskWorld {
    /// `chunkMap.getGridSquare(x, y, z)`.
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare>;
    /// `gs.has(IsoFlagType.exterior)`.
    fn is_exterior(&self, sq: &RenderSquare) -> bool {
        sq.has(|f| f.exterior)
    }
    /// `gs.getIsoWorldRegion()`, or `IsoRegions.getIsoWorldRegion(x, y,
    /// z)` where there is no square.
    fn world_region(&self, _x: i32, _y: i32, _z: i32) -> Option<RegionId> {
        None
    }
    /// `IWorldRegion.isFogMask()`.
    fn region_is_fog_mask(&self, _r: RegionId) -> bool {
        false
    }
    /// `IWorldRegion.getNeighbors()`.
    fn region_neighbors(&self, _r: RegionId) -> Vec<RegionId> {
        Vec::new()
    }
    /// `IsoGridSquare.renderCharacters(maxZ, false, false)` into the mask:
    /// the square's characters drawn in white. The renderer has no
    /// characters, so nothing by default.
    fn render_characters(&self, _sq: &RenderSquare, _max_z: i32, _out: &mut Vec<DrawCmd>) {}
}

/// The player `PlayerFxMask.initMask` reads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FxMaskPlayer {
    /// `player.getSquare()`'s position.
    pub square: Option<[i32; 3]>,
    /// `player.getBuilding()`.
    pub building: Option<BuildingId>,
    /// `player.getZ()`.
    pub z: f32,
    /// `player.getMasterRegion()`.
    pub master_region: Option<RegionId>,
}

/// One `WeatherFxMask` location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FxMask {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub flags: i32,
    /// Whether a square was given (`gs != null`); the square itself is
    /// looked up again when drawn.
    pub has_square: bool,
    pub enabled: bool,
}

/// `RasterizeBounds`: the view's corners on a level, in squares, and the
/// screen corners of the squares under them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RasterizeBounds {
    pub x: [f32; 4],
    pub y: [f32; 4],
    pub cx: [i32; 4],
    pub cy: [i32; 4],
}

impl RasterizeBounds {
    /// `calculate(playerIndex, z)` for a camera offset and offscreen size.
    pub fn calculate(off_x: f32, off_y: f32, ow: i32, oh: i32, z: i32) -> Self {
        let (w, h) = (ow as f32, oh as f32);
        let zf = z as f32;
        let iso = |sx: f32, sy: f32| {
            (
                x_to_iso(sx + off_x, sy + off_y, zf),
                y_to_iso(sx + off_x, sy + off_y, zf),
            )
        };
        let corners = [iso(0.0, 0.0), iso(w, 0.0), iso(w, h), iso(0.0, h)];
        let mut b = RasterizeBounds::default();
        for (i, (x, y)) in corners.into_iter().enumerate() {
            b.x[i] = x;
            b.y[i] = y;
        }
        let d = [(-0.5, 0.5), (0.5, -0.5), (1.5, 0.5), (0.5, 1.5)];
        for (i, (ddx, ddy)) in d.into_iter().enumerate() {
            let sx = fastfloor(b.x[i]) as f32 + ddx;
            let sy = fastfloor(b.y[i]) as f32 + ddy;
            b.cx[i] = x_to_screen(sx, sy) as i32;
            b.cy[i] = y_to_screen(sx, sy, -666.0, -666) as i32;
        }
        for i in [2, 3] {
            b.x[i] += 3.0;
            b.y[i] += 3.0;
        }
        b
    }

    /// `equals`: the screen corners match.
    pub fn same_screen(&self, o: &RasterizeBounds) -> bool {
        self.cx == o.cx && self.cy == o.cy
    }
}

/// `zombie.worldMap.Rasterize.Edge`.
#[derive(Clone, Copy, Debug)]
struct Edge {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    dx: f32,
    dy: f32,
}

impl Edge {
    fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        let (x0, y0, x1, y1) = if y0 > y1 {
            (x1, y1, x0, y0)
        } else {
            (x0, y0, x1, y1)
        };
        Edge {
            x0,
            y0,
            x1,
            y1,
            dx: x1 - x0,
            dy: y1 - y0,
        }
    }
}

fn scan_span(mut e0: Edge, mut e1: Edge, min: i32, max: i32, s: &mut impl FnMut(i32, i32)) {
    let y0 = f64::from(min).max(f64::from(e1.y0).floor()) as i32;
    let y1 = f64::from(max).min(f64::from(e1.y1).ceil()) as i32;
    if e0.x0 == e1.x0 && e0.y0 == e1.y0 {
        if e0.x0 + e1.dy / e0.dy * e0.dx < e1.x1 {
            std::mem::swap(&mut e0, &mut e1);
        }
    } else if e0.x1 - e1.dy / e0.dy * e0.dx < e1.x0 {
        std::mem::swap(&mut e0, &mut e1);
    }
    let m0 = f64::from(e0.dx) / f64::from(e0.dy);
    let m1 = f64::from(e1.dx) / f64::from(e1.dy);
    let d0 = if e0.dx > 0.0 { 1.0 } else { 0.0 };
    let d1 = if e1.dx < 0.0 { 1.0 } else { 0.0 };
    for y in y0..y1 {
        let yf = f64::from(y);
        let x0 = m0 * 0f64.max(f64::from(e0.dy).min(yf + d0 - f64::from(e0.y0))) + f64::from(e0.x0);
        let x1 = m1 * 0f64.max(f64::from(e1.dy).min(yf + d1 - f64::from(e1.y0))) + f64::from(e1.x0);
        for x in (x1.floor() as i32)..(x0.ceil() as i32) {
            s(x, y);
        }
    }
}

/// `Rasterize.scanTriangle`: every square the triangle touches, row by row.
#[allow(clippy::too_many_arguments)]
pub fn scan_triangle(
    xa: f32,
    ya: f32,
    xb: f32,
    yb: f32,
    xc: f32,
    yc: f32,
    min: i32,
    max: i32,
    mut s: impl FnMut(i32, i32),
) {
    let mut ab = Edge::new(xa, ya, xb, yb);
    let mut bc = Edge::new(xb, yb, xc, yc);
    let mut ca = Edge::new(xc, yc, xa, ya);
    if ab.dy > ca.dy {
        std::mem::swap(&mut ab, &mut ca);
    }
    if bc.dy > ca.dy {
        std::mem::swap(&mut bc, &mut ca);
    }
    if ab.dy > 0.0 {
        scan_span(ca, ab, min, max, &mut s);
    }
    if bc.dy > 0.0 {
        scan_span(ca, bc, min, max, &mut s);
    }
}

/// `WeatherFxMask.PlayerFxMask` with the static state the original keeps
/// for one player (`rasterizeBounds[player]`, `maskingEnabled`).
#[derive(Clone, Debug)]
pub struct WeatherFxMask {
    pub masks: Vec<FxMask>,
    map: HashMap<(i32, i32), usize>,
    /// `plrSquare`: the square the mask was built for.
    pub plr_square: Option<[i32; 3]>,
    pub requires_update: bool,
    pub has_mask_to_draw: bool,
    pub player_z: i32,
    pub player: FxMaskPlayer,
    pub cur_region: Option<RegionId>,
    pub cur_connected_regions: Vec<RegionId>,
    pub diamond_iter_done: bool,
    is_first_square: bool,
    first_square: Option<[i32; 3]>,
    pub rasterize_bounds: Option<RasterizeBounds>,
    /// `maskingEnabled` (and the FBOs being available).
    pub masking_enabled: bool,
}

impl Default for WeatherFxMask {
    fn default() -> Self {
        WeatherFxMask {
            masks: Vec::new(),
            map: HashMap::new(),
            plr_square: None,
            requires_update: false,
            has_mask_to_draw: true,
            player_z: 0,
            player: FxMaskPlayer::default(),
            cur_region: None,
            cur_connected_regions: Vec::new(),
            diamond_iter_done: false,
            is_first_square: true,
            first_square: None,
            rasterize_bounds: None,
            masking_enabled: true,
        }
    }
}

impl WeatherFxMask {
    /// `forceMaskUpdate`.
    pub fn force_mask_update(&mut self) {
        self.plr_square = None;
    }

    /// `setDiamondIterDone`.
    pub fn set_diamond_iter_done(&mut self) {
        self.diamond_iter_done = true;
    }

    /// `initMask()` for the frame's player; `has_fx` is
    /// `IsoWeatherFX.instance != null`.
    pub fn init_mask(&mut self, world: &dyn FxMaskWorld, player: FxMaskPlayer, has_fx: bool) {
        self.player = player;
        self.player_z = fastfloor(player.z);
        self.diamond_iter_done = false;
        self.requires_update = false;
        if self.is_first_square || self.plr_square.is_none() || self.plr_square != player.square {
            self.plr_square = player.square;
            self.masks.clear();
            self.map.clear();
            self.requires_update = true;
            if self.first_square.is_none() {
                self.first_square = self.plr_square;
            }
            if self.first_square.is_some() && self.first_square != self.plr_square {
                self.is_first_square = false;
            }
        }
        self.cur_region = player.master_region;
        self.cur_connected_regions.clear();
        match self.cur_region {
            Some(r) if world.region_is_fog_mask(r) => {
                let mut todo = vec![r];
                while !todo.is_empty() {
                    let cur = todo.remove(0);
                    self.cur_connected_regions.push(cur);
                    for n in world.region_neighbors(cur) {
                        if todo.contains(&n)
                            || self.cur_connected_regions.contains(&n)
                            || !world.region_is_fog_mask(n)
                        {
                            continue;
                        }
                        todo.push(n);
                    }
                }
            }
            _ => self.cur_region = None,
        }
        if !has_fx {
            self.has_mask_to_draw = false;
            return;
        }
        let sq = player.square.and_then(|[x, y, z]| world.square(x, y, z));
        self.has_mask_to_draw = sq
            .is_some_and(|s| s.cut.building.is_some() || !world.is_exterior(s))
            || self.cur_region.is_some_and(|r| world.region_is_fog_mask(r));
    }

    /// `checkVisibleSquares(playerIndex, z)`: whether the view moved to
    /// other squares since the last scan.
    pub fn check_visible_squares(&self, view: &WeatherFxView, z: i32) -> bool {
        if !self.has_mask_to_draw {
            return false;
        }
        let Some(rb) = &self.rasterize_bounds else {
            return true;
        };
        !RasterizeBounds::calculate(
            view.off_x,
            view.off_y,
            view.offscreen_width,
            view.offscreen_height,
            z,
        )
        .same_screen(rb)
    }

    fn get_mask(&self, x: i32, y: i32) -> Option<usize> {
        self.map.get(&(x, y)).copied()
    }

    /// `addMask(x, y, z, gs, flags, enabled)`.
    pub fn add_mask(
        &mut self,
        x: i32,
        y: i32,
        z: i32,
        has_square: bool,
        flags: i32,
        enabled: bool,
    ) {
        if !self.has_mask_to_draw || !self.requires_update {
            return;
        }
        match self.get_mask(x, y) {
            None => {
                self.masks.push(FxMask {
                    x,
                    y,
                    z,
                    flags,
                    has_square,
                    enabled,
                });
                self.map.insert((x, y), self.masks.len() - 1);
            }
            Some(i) => {
                if self.masks[i].flags != flags {
                    self.masks[i].flags |= flags;
                }
                let a = self.masks[i];
                if !a.enabled && enabled {
                    self.masks.push(FxMask {
                        x,
                        y,
                        z,
                        flags: a.flags,
                        has_square,
                        enabled,
                    });
                    self.map.insert((x, y), self.masks.len() - 1);
                } else {
                    let m = &mut self.masks[i];
                    m.enabled = m.enabled || enabled;
                    if enabled && has_square && !m.has_square {
                        m.has_square = true;
                    }
                }
            }
        }
    }

    fn in_fog_region(&self, world: &dyn FxMaskWorld, r: Option<RegionId>) -> bool {
        self.cur_region.is_some()
            && r.is_some_and(|r| {
                world.region_is_fog_mask(r)
                    && (Some(r) == self.cur_region || self.cur_connected_regions.contains(&r))
            })
    }

    /// `isInPlayerBuilding(gs, x, y, z)`.
    fn is_in_player_building(
        &self,
        world: &dyn FxMaskWorld,
        gs: Option<&RenderSquare>,
        x: i32,
        y: i32,
        z: i32,
    ) -> bool {
        match gs {
            Some(sq) if sq.has(|f| f.solidfloor) => {
                if sq.cut.building.is_some() && sq.cut.building == self.player.building {
                    return true;
                }
                if sq.cut.building.is_none() {
                    return self.in_fog_region(world, world.world_region(x, y, z));
                }
                false
            }
            _ => {
                if self.is_interior_location(world, x, y, z) {
                    return true;
                }
                match gs {
                    Some(sq) if sq.cut.building.is_none() => {
                        self.in_fog_region(world, world.world_region(x, y, z))
                    }
                    None if self.cur_region.is_some() => {
                        self.in_fog_region(world, world.world_region(x, y, z))
                    }
                    _ => false,
                }
            }
        }
    }

    /// `isInteriorLocation(x, y, maxZ)`.
    fn is_interior_location(&self, world: &dyn FxMaskWorld, x: i32, y: i32, max_z: i32) -> bool {
        for z in (0..=max_z).rev() {
            let Some(sq) = world.square(x, y, z) else {
                continue;
            };
            if sq.cut.building.is_some() && sq.cut.building == self.player.building {
                return true;
            }
            if world.is_exterior(sq) {
                return false;
            }
        }
        false
    }

    /// `addMaskLocation(gs, x, y, z)`.
    pub fn add_mask_location(&mut self, world: &dyn FxMaskWorld, x: i32, y: i32, z: i32) {
        if !self.requires_update || !self.has_mask_to_draw || self.player_z != z {
            return;
        }
        let gs = world.square(x, y, z);
        let inside = |dx: i32, dy: i32| {
            self.is_in_player_building(world, world.square(x + dx, y + dy, z), x + dx, y + dy, z)
        };
        if self.is_in_player_building(world, gs, x, y, z) {
            let connect_n = !inside(0, -1);
            let connect_w = !inside(-1, 0);
            let connect_nw = !inside(-1, -1);
            let mut walls = 0;
            if connect_n {
                walls |= BIT_WALLN;
            }
            if connect_w {
                walls |= BIT_WALLW;
            }
            if connect_nw {
                walls |= BIT_WALL_SE;
            }
            let mut added = false;
            let is_stairs = gs.is_some_and(|s| {
                s.objects
                    .iter()
                    .any(|o| o.sprite.as_ref().is_some_and(|sp| sp.stairs.is_some()))
            });
            let has = gs.is_some();
            if let Some(sq) = gs {
                if connect_n || connect_w || connect_nw {
                    if connect_n && !sq.has(|f| f.wall_n) && !sq.has(|f| f.wall_nw) {
                        self.add_mask(x - 1, y, z, false, 8, false);
                        self.add_mask(x, y, z, has, 24, true);
                        self.add_mask(x + 1, y, z, false, 24, false);
                        self.add_mask(x + 2, y, z, false, 8, false);
                        self.add_mask(x, y + 1, z, false, 8, false);
                        self.add_mask(x + 1, y + 1, z, false, 24, false);
                        self.add_mask(x + 2, y + 1, z, false, 24, false);
                        self.add_mask(x + 2, y + 2, z, false, 16, false);
                        self.add_mask(x + 3, y + 2, z, false, 16, false);
                        added = true;
                    }
                    if connect_w && !sq.has(|f| f.wall_w) && !sq.has(|f| f.wall_nw) {
                        self.add_mask(x, y - 1, z, false, 8, false);
                        self.add_mask(x, y, z, has, 24, true);
                        self.add_mask(x, y + 1, z, false, 24, false);
                        self.add_mask(x, y + 2, z, false, 8, false);
                        self.add_mask(x + 1, y, z, false, 8, false);
                        self.add_mask(x + 1, y + 1, z, false, 24, false);
                        self.add_mask(x + 1, y + 2, z, false, 24, false);
                        self.add_mask(x + 2, y + 2, z, false, 16, false);
                        self.add_mask(x + 2, y + 3, z, false, 16, false);
                        added = true;
                    }
                    if connect_nw {
                        self.add_mask(x, y, z, has, if is_stairs { 24 } else { walls }, true);
                        added = true;
                    }
                }
            }
            if !added {
                self.add_mask(x, y, z, has, if is_stairs { 24 } else { walls }, true);
            }
        } else {
            let connect_n = inside(0, -1);
            let connect_w = inside(-1, 0);
            if connect_n || connect_w {
                let mut flags = BIT_IS_CUT;
                if connect_n {
                    flags |= BIT_WALLN;
                }
                if connect_w {
                    flags |= BIT_WALLW;
                }
                if gs.is_some_and(has_door) {
                    flags |= BIT_DOOR;
                }
                self.add_mask(x, y, z, gs.is_some(), flags, true);
            } else if inside(-1, -1) {
                self.add_mask(x, y, z, gs.is_some(), BIT_IS_CUT, true);
            }
        }
    }

    /// `scanForTiles(player)`: every square of the view on the player's
    /// level through [`WeatherFxMask::add_mask_location`].
    pub fn scan_for_tiles(&mut self, world: &dyn FxMaskWorld, view: &WeatherFxView) {
        if self.diamond_iter_done {
            return;
        }
        let z = fastfloor(self.player.z);
        let rb = RasterizeBounds::calculate(
            view.off_x,
            view.off_y,
            view.offscreen_width,
            view.offscreen_height,
            z,
        );
        self.rasterize_bounds = Some(rb);
        let mut squares = Vec::new();
        scan_triangle(
            rb.x[0],
            rb.y[0],
            rb.x[1],
            rb.y[1],
            rb.x[3],
            rb.y[3],
            0,
            100_000,
            |x, y| squares.push((x, y)),
        );
        scan_triangle(
            rb.x[1],
            rb.y[1],
            rb.x[2],
            rb.y[2],
            rb.x[3],
            rb.y[3],
            0,
            100_000,
            |x, y| squares.push((x, y)),
        );
        for (x, y) in squares {
            self.add_mask_location(world, x, y, z);
        }
    }
}

/// `getHasDoor(gs)`: an open doorway in a cut wall the player can see.
fn has_door(sq: &RenderSquare) -> bool {
    let cut = sq
        .objects
        .iter()
        .any(|o| o.sprite.as_ref().is_some_and(|s| s.cut_n || s.cut_w));
    cut && sq.has(|f| f.door_wall_n || f.door_wall_w)
        && !sq.has(|f| f.door_n)
        && !sq.has(|f| f.door_w)
        && sq.cut.can_see
}

/// The stand-in sprites the mask is drawn with (`WeatherFxMask.init`):
/// `floors_interior_tilesandwood_01_16`, `walls_interior_house_01_21`
/// (north), `_20` (west), `_22` (north-west), `_23` (south-east), and
/// `media/textures/weather/fogwhite.png`.
#[derive(Clone, Debug, Default)]
pub struct FxMaskSprites {
    pub floor: Option<Rc<SpriteInfo>>,
    pub wall_n: Option<Rc<SpriteInfo>>,
    pub wall_w: Option<Rc<SpriteInfo>>,
    pub wall_nw: Option<Rc<SpriteInfo>>,
    pub wall_se: Option<Rc<SpriteInfo>>,
    pub white: Option<Texture>,
}

impl FxMaskSprites {
    /// The sprite names, in field order.
    pub const SPRITES: [&'static str; 5] = [
        "floors_interior_tilesandwood_01_16",
        "walls_interior_house_01_21",
        "walls_interior_house_01_20",
        "walls_interior_house_01_22",
        "walls_interior_house_01_23",
    ];
    pub const WHITE: &'static str = "media/textures/weather/fogwhite.png";
}

/// The frame values `renderFxMask` reads beyond the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FxMaskFrame {
    /// `IsoCamera.frameState.camCharacterZ`.
    pub cam_character_z: f32,
    /// `PlayerCamera.fixJigglyModelsX/Y` (times the zoom when drawn).
    pub fix_jiggly: [f32; 2],
    /// `RenderSettings.getPlayerSettings(player).isExterior()` (always
    /// true in build 42).
    pub is_exterior: bool,
    /// `getMaskClearColorForPlayer` (always transparent black in build 42).
    pub mask_clear_color: Rgba,
    /// `IsoCamera.getScreenLeft/Top(player)`.
    pub screen_left: i32,
    pub screen_top: i32,
}

impl Default for FxMaskFrame {
    fn default() -> Self {
        FxMaskFrame {
            cam_character_z: 0.0,
            fix_jiggly: [0.0; 2],
            is_exterior: true,
            mask_clear_color: Rgba(0.0, 0.0, 0.0, 0.0),
            screen_left: 0,
            screen_top: 0,
        }
    }
}

/// `IsoSprite.render(null|obj, x, y, z, ...)` while the mask renders: the
/// old placement (no chunk texture), the quad snapped outwards to whole
/// pixels, no depth.
#[allow(clippy::too_many_arguments)]
fn mask_sprite(
    sprite: &SpriteInfo,
    x: f32,
    y: f32,
    z: f32,
    offset_x: f32,
    offset_y: f32,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    let Some(tex) = sprite.texture.as_ref() else {
        return;
    };
    let mut sx = x_to_screen(x, y) - offset_x - view.off_x + sprite.soff_x as f32;
    let mut sy = y_to_screen(x, y, z, 0) - offset_y - view.off_y + sprite.soff_y as f32;
    sx += f.fix_jiggly[0] * view.zoom;
    sy += f.fix_jiggly[1] * view.zoom;
    let scale = if TILE_SCALE == 2 && tex.width_orig() == 64 && tex.height_orig() == 128 {
        2.0
    } else {
        1.0
    };
    let (mut w, mut h) = (tex.width as f32, tex.height as f32);
    if scale != 1.0 {
        sx += tex.offset_x * (scale - 1.0);
        w *= scale;
        sy += tex.offset_y * (scale - 1.0);
        h *= scale;
    }
    // Texture.render adds the trim offset; SpriteRenderer.render snaps
    // while the mask renders.
    let (px, py) = (sx + tex.offset_x, sy + tex.offset_y);
    let (x1, y1) = (px.floor(), py.floor());
    let (x2, y2) = ((px + w).ceil(), (py + h).ceil());
    let mut dl = DrawList::default();
    dl.render(
        Some(tex),
        x1,
        y1,
        x2 - x1,
        y2 - y1,
        Rgba(1.0, 1.0, 1.0, 1.0),
    );
    out.extend(dl.cmds);
}

fn offset_x() -> f32 {
    (32 * TILE_SCALE) as f32
}

fn offset_y() -> f32 {
    (96 * TILE_SCALE) as f32
}

/// `renderMaskWall`/`renderMaskWallNoCuts`: the wall stand-in for the
/// sides given.
fn mask_wall(
    s: &FxMaskSprites,
    m: &FxMask,
    n: bool,
    w: bool,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    let sprite = match (n, w) {
        (true, true) => &s.wall_nw,
        (true, false) => &s.wall_n,
        (false, true) => &s.wall_w,
        _ => &s.wall_se,
    };
    if let Some(sp) = sprite {
        mask_sprite(
            sp,
            m.x as f32,
            m.y as f32,
            m.z as f32,
            offset_x(),
            offset_y(),
            view,
            f,
            out,
        );
    }
}

/// `IsoObject.renderFxMask`: the object's sprite and overlay in white.
fn object_fx_mask(
    obj: &RenderObject,
    sq: &RenderSquare,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    let oy = obj.offset_y + obj.render_y_offset * TILE_SCALE as f32;
    let (x, y, z) = (sq.x as f32, sq.y as f32, sq.z as f32);
    if let Some(sp) = &obj.sprite {
        mask_sprite(sp, x, y, z, obj.offset_x, oy, view, f, out);
    }
    if let Some(o) = &obj.overlay {
        mask_sprite(&o.sprite, x, y, z, obj.offset_x, oy, view, f, out);
    }
}

/// `IsoGridSquare.RenderMinusFloorFxMask(maxZ, false, false)`: the
/// square's objects other than floors and south/east wall attachments.
fn render_minus_floor_fx_mask(
    sq: &RenderSquare,
    max_z: i32,
    cam_z: f32,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    for obj in &sq.objects {
        let Some(sp) = &obj.sprite else { continue };
        let mut doit = !(sp.flags.solidfloor || sp.render_layer == 1);
        if sq.z >= max_z && !sp.always_draw {
            doit = false;
        }
        if matches!(
            sp.tile_type,
            TileType::WestRoofB | TileType::WestRoofM | TileType::WestRoofT
        ) && sq.z == max_z - 1
            && sq.z == fastfloor(cam_z)
        {
            doit = false;
        }
        if sq.is_sprite_on_south_or_east_wall(obj) {
            doit = false;
        }
        if doit {
            // Walls cut away near the player go through DoCutawayShaderSprite
            // in the original; here they are drawn whole.
            object_fx_mask(obj, sq, view, f, out);
        }
    }
}

/// `IsoGridSquare.RenderOpenDoorOnly()`: objects attached to the north or
/// west wall.
fn render_open_door_only(
    sq: &RenderSquare,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    for obj in &sq.objects {
        if obj
            .sprite
            .as_ref()
            .is_some_and(|s| s.flags.attached_n || s.flags.attached_w)
        {
            object_fx_mask(obj, sq, view, f, out);
        }
    }
}

/// The world projection `glDoEndFrameFx` returns to.
fn world_ortho(view: &WeatherFxView) -> DrawCmd {
    let (w, h) = (view.screen_width as u32, view.screen_height as u32);
    DrawCmd::Ortho {
        left: 0.0,
        right: view.offscreen_width as f32,
        bottom: view.offscreen_height as f32,
        top: 0.0,
        width: w,
        height: h,
    }
}

/// `glBuffer(4|6)` and `glDoStartFrameFx(ow, oh)`: weather texture `index`
/// bound and cleared, an unzoomed projection over the offscreen size.
fn begin_fx(index: usize, view: &WeatherFxView, out: &mut Vec<DrawCmd>) {
    let (w, h) = (view.screen_width as u32, view.screen_height as u32);
    out.push(DrawCmd::WeatherFxBegin {
        index,
        width: w,
        height: h,
    });
    out.push(DrawCmd::GlEnable(gl::BLEND));
    out.push(DrawCmd::DepthFunc(gl::ALWAYS));
    out.push(DrawCmd::AlphaFunc(gl::GREATER, 0.0));
    out.push(world_ortho(view));
}

/// `drawFxMask(player)`.
pub fn draw_fx_mask(
    mask: &WeatherFxMask,
    world: &dyn FxMaskWorld,
    sprites: &FxMaskSprites,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    begin_fx(0, view, out);
    // IsoCell.drawStencilMask leaves the alpha test on, passing everything;
    // its stencil marks nothing the mask draws read.
    out.push(DrawCmd::GlEnable(gl::ALPHA_TEST));
    out.push(DrawCmd::AlphaFunc(gl::ALWAYS, 0.0));
    out.push(DrawCmd::DepthMask(false));
    out.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
    for m in mask.masks.iter().filter(|m| m.enabled) {
        let gs = if m.has_square {
            world.square(m.x, m.y, m.z)
        } else {
            None
        };
        let (wn, ww) = (m.flags & BIT_WALLN != 0, m.flags & BIT_WALLW != 0);
        if m.flags & BIT_IS_CUT != 0 {
            out.push(DrawCmd::BlendFunc(gl::ZERO, gl::ZERO));
            out.push(DrawCmd::BlendEquation(gl::FUNC_REVERSE_SUBTRACT));
            out.push(DrawCmd::GlEnable(gl::ALPHA_TEST));
            out.push(DrawCmd::AlphaFunc(gl::GREATER, 0.02));
            if gs.is_some() {
                mask_wall(sprites, m, wn, ww, view, f, out);
            }
            out.push(DrawCmd::BlendEquation(gl::FUNC_ADD));
            if let (true, Some(sq)) = (m.flags & BIT_DOOR != 0, gs) {
                out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
                render_open_door_only(sq, view, f, out);
            }
            continue;
        }
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
        if let Some(fl) = &sprites.floor {
            mask_sprite(
                fl,
                m.x as f32,
                m.y as f32,
                m.z as f32,
                offset_x(),
                offset_y(),
                view,
                f,
                out,
            );
        }
        let objects = m.flags & BIT_OBJECTS != 0;
        let chars = m.flags & BIT_CHARS != 0;
        if !objects && gs.is_some() {
            if wn || ww {
                mask_wall(sprites, m, wn, ww, view, f, out);
            } else if m.flags & BIT_WALL_SE != 0 {
                mask_wall(sprites, m, false, false, view, f, out);
            }
        }
        if let (true, Some(sq)) = (objects, gs) {
            render_minus_floor_fx_mask(sq, m.z + 1, f.cam_character_z, view, f, out);
        }
        if let (true, Some(sq)) = (chars, gs) {
            world.render_characters(sq, m.z + 1, out);
            out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
        }
    }
    out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    out.push(DrawCmd::WeatherFxEnd);
    out.push(world_ortho(view));
}

/// `drawFxLayered(player, doClouds, doFog, doPrecip)`: one layer (or,
/// with none, the indoor darkening) into `fboParticles`, the mask taken
/// out, the result over the world.
#[allow(clippy::too_many_arguments)]
pub fn draw_fx_layered(
    fx: &mut IsoWeatherFx,
    mask: &WeatherFxMask,
    sprites: &FxMaskSprites,
    clouds: bool,
    fog: bool,
    precip: bool,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    let (ow, oh) = (view.offscreen_width as f32, view.offscreen_height as f32);
    let (sx, sy, sw, sh) = (
        f.screen_left,
        f.screen_top,
        view.screen_width,
        view.screen_height,
    );
    out.push(DrawCmd::DepthMask(false));
    out.push(DrawCmd::GlDisable(gl::DEPTH_TEST));
    begin_fx(1, view, out);
    out.push(DrawCmd::BlendFuncSeparate(
        gl::ONE,
        gl::ONE_MINUS_SRC_ALPHA,
        gl::ONE,
        gl::ONE_MINUS_SRC_ALPHA,
    ));
    if !(clouds || fog || precip) {
        let c = f.mask_clear_color;
        let mut dl = DrawList::default();
        dl.renderi(
            sprites.white.as_ref(),
            0,
            0,
            view.offscreen_width,
            view.offscreen_height,
            c,
        );
        out.extend(dl.cmds);
    } else {
        fx.render_layered(clouds, fog, precip, view, out);
        // The particle drawer pushes and pops the GL state around its
        // draw: the alpha test the mask pass left on comes back.
        out.push(DrawCmd::GlEnable(gl::ALPHA_TEST));
    }
    let fbo_w = sw.max(1) as f32;
    let fbo_h = sh.max(1) as f32;
    if mask.masking_enabled {
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
        out.push(DrawCmd::BlendEquation(gl::FUNC_REVERSE_SUBTRACT));
        // Texture.rendershader2(0, 0, ow, oh, sx, sy, sw, sh).
        let t = pz_render::weather_fx_texture(0, sw as u32, sh as u32);
        let (u0, u1) = (sx as f32 / fbo_w, (sx + sw) as f32 / fbo_w);
        let (v_top, v_bottom) = ((sy + sh) as f32 / fbo_h, sy as f32 / fbo_h);
        let mut dl = DrawList::default();
        dl.render_uv(
            Some(&t),
            0.0,
            0.0,
            ow,
            oh,
            Rgba(1.0, 1.0, 1.0, 1.0),
            [u0, u1, u1, u0],
            [v_top, v_top, v_bottom, v_bottom],
        );
        out.extend(dl.cmds);
        out.push(DrawCmd::BlendEquation(gl::FUNC_ADD));
    }
    out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    out.push(DrawCmd::WeatherFxEnd);
    out.push(world_ortho(view));
    let t = pz_render::weather_fx_texture(1, sw as u32, sh as u32);
    let sx1 = sx as f32 / fbo_w;
    let ey1 = sy as f32 / fbo_h;
    let ex1 = (sx + sw) as f32 / fbo_w;
    let sy1 = (sy + sh) as f32 / fbo_h;
    out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
    let mut dl = DrawList::default();
    dl.render_uv(
        Some(&t),
        0.0,
        0.0,
        ow,
        oh,
        Rgba(1.0, 1.0, 1.0, 1.0),
        [sx1, ex1, ex1, sx1],
        [sy1, sy1, ey1, ey1],
    );
    out.extend(dl.cmds);
    out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
}

/// `IsoWorld.renderWeatherFX`: the drawers' `startFrame`, then
/// `WeatherFxMask.renderFxMask`. Recorded into the offscreen world pass,
/// whose projection it restores.
#[allow(clippy::too_many_arguments)]
pub fn render_weather_fx(
    fx: &mut IsoWeatherFx,
    mask: &mut WeatherFxMask,
    world: &dyn FxMaskWorld,
    sprites: &FxMaskSprites,
    view: &WeatherFxView,
    f: &FxMaskFrame,
    out: &mut Vec<DrawCmd>,
) {
    fx.start_frame();
    if f.cam_character_z < 0.0 {
        return;
    }
    if !mask.masking_enabled || !mask.has_mask_to_draw {
        out.push(DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA));
        fx.render(view, out);
        return;
    }
    mask.scan_for_tiles(world, view);
    draw_fx_mask(mask, world, sprites, view, f, out);
    if !f.is_exterior {
        draw_fx_layered(fx, mask, sprites, false, false, false, view, f, out);
    }
    if fx.has_clouds_to_render() {
        draw_fx_layered(fx, mask, sprites, true, false, false, view, f, out);
    }
    if fx.has_fog_to_render() && view.fog_quality == 2 {
        draw_fx_layered(fx, mask, sprites, false, true, false, view, f, out);
    }
    if view.render_precipitation == 1 && fx.has_precipitation_to_render() {
        draw_fx_layered(fx, mask, sprites, false, false, true, view, f, out);
    }
    // The original leaves the mask's alpha test on for whatever follows;
    // the world pass here expects it off.
    out.push(DrawCmd::GlDisable(gl::ALPHA_TEST));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{ObjectKind, SpriteFlags};

    /// A 5x5 room at (10..15, 10..15) on level 0 with walls on its north
    /// and west sides, in an exterior field.
    struct Room(HashMap<(i32, i32, i32), RenderSquare>);

    impl Room {
        fn new() -> Self {
            let mut m = HashMap::new();
            let floor = |f: SpriteFlags| {
                Rc::new(SpriteInfo {
                    flags: SpriteFlags {
                        solidfloor: true,
                        ..f
                    },
                    ..Default::default()
                })
            };
            for x in 5..20 {
                for y in 5..20 {
                    let mut sq = RenderSquare::new(x, y, 0);
                    let inside = (10..15).contains(&x) && (10..15).contains(&y);
                    let f = SpriteFlags {
                        exterior: !inside,
                        wall_n: inside && y == 10,
                        wall_w: inside && x == 10,
                        ..Default::default()
                    };
                    sq.objects
                        .push(RenderObject::new(ObjectKind::Plain, Some(floor(f))));
                    if inside {
                        sq.cut.building = Some(BuildingId(1));
                    }
                    m.insert((x, y, 0), sq);
                }
            }
            Room(m)
        }
    }

    impl FxMaskWorld for Room {
        fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
            self.0.get(&(x, y, z))
        }
    }

    fn inside_player() -> FxMaskPlayer {
        FxMaskPlayer {
            square: Some([12, 12, 0]),
            building: Some(BuildingId(1)),
            z: 0.0,
            master_region: None,
        }
    }

    fn flags(m: &WeatherFxMask, x: i32, y: i32) -> Option<(i32, bool)> {
        m.get_mask(x, y)
            .map(|i| (m.masks[i].flags, m.masks[i].enabled))
    }

    #[test]
    fn mask_marks_the_building_and_the_walls_around_it() {
        let w = Room::new();
        let mut m = WeatherFxMask::default();
        m.init_mask(&w, inside_player(), true);
        assert!(m.has_mask_to_draw && m.requires_update);
        for (x, y) in [
            (12, 12),
            (10, 12),
            (12, 10),
            (10, 10),
            (15, 12),
            (12, 15),
            (15, 15),
            (16, 16),
        ] {
            m.add_mask_location(&w, x, y, 0);
        }
        // Inside, away from the walls: floor only.
        assert_eq!(flags(&m, 12, 12), Some((0, true)));
        // The west edge has its wall: west and north-west connections.
        assert_eq!(flags(&m, 10, 12), Some((BIT_WALLW | BIT_WALL_SE, true)));
        assert_eq!(flags(&m, 12, 10), Some((BIT_WALLN | BIT_WALL_SE, true)));
        assert_eq!(
            flags(&m, 10, 10),
            Some((BIT_WALLN | BIT_WALLW | BIT_WALL_SE, true))
        );
        // Outside next to the east and south sides: cut walls.
        assert_eq!(flags(&m, 15, 12), Some((BIT_IS_CUT | BIT_WALLW, true)));
        assert_eq!(flags(&m, 12, 15), Some((BIT_IS_CUT | BIT_WALLN, true)));
        assert_eq!(flags(&m, 15, 15), Some((BIT_IS_CUT, true)));
        assert_eq!(flags(&m, 16, 16), None);
        // Not again until the player moves (or the view does).
        let n = m.masks.len();
        m.init_mask(&w, inside_player(), true);
        m.init_mask(
            &w,
            FxMaskPlayer {
                square: Some([13, 12, 0]),
                ..inside_player()
            },
            true,
        );
        assert!(m.requires_update && m.masks.is_empty());
        m.init_mask(
            &w,
            FxMaskPlayer {
                square: Some([13, 12, 0]),
                ..inside_player()
            },
            true,
        );
        assert!(!m.requires_update);
        m.add_mask_location(&w, 12, 12, 0);
        assert!(m.masks.is_empty() && n > 0);
    }

    #[test]
    fn open_edges_mask_the_squares_beyond() {
        // No wall on the room's north side: the squares in front of the
        // opening get object and character masks, disabled.
        let mut w = Room::new();
        for x in 10..15 {
            let sq = w.0.get_mut(&(x, 10, 0)).unwrap();
            let s = Rc::new(SpriteInfo {
                flags: SpriteFlags {
                    solidfloor: true,
                    wall_w: x == 10,
                    ..Default::default()
                },
                ..Default::default()
            });
            sq.objects[0] = RenderObject::new(ObjectKind::Plain, Some(s));
        }
        let mut m = WeatherFxMask::default();
        m.init_mask(&w, inside_player(), true);
        m.add_mask_location(&w, 12, 10, 0);
        assert_eq!(
            flags(&m, 12, 10),
            Some((BIT_CHARS | BIT_OBJECTS | BIT_WALLN | BIT_WALL_SE, true))
        );
        assert_eq!(flags(&m, 11, 10), Some((BIT_CHARS, false)));
        assert_eq!(flags(&m, 15, 12), Some((BIT_OBJECTS, false)));
        // A later enabled add of a disabled location replaces it.
        m.add_mask_location(&w, 13, 10, 0);
        assert_eq!(flags(&m, 13, 10).map(|f| f.1), Some(true));
    }

    #[test]
    fn no_mask_outdoors() {
        let w = Room::new();
        let mut m = WeatherFxMask::default();
        m.init_mask(
            &w,
            FxMaskPlayer {
                square: Some([7, 7, 0]),
                ..Default::default()
            },
            true,
        );
        assert!(!m.has_mask_to_draw);
        m.add_mask_location(&w, 12, 12, 0);
        assert!(m.masks.is_empty());
        m.init_mask(&w, inside_player(), false);
        assert!(!m.has_mask_to_draw);
    }

    #[test]
    fn scan_covers_the_view_on_the_player_level() {
        let w = Room::new();
        let mut m = WeatherFxMask::default();
        m.init_mask(&w, inside_player(), true);
        let cam = crate::Camera::centred(12.5, 12.5, 0.0, 640, 360, 1.0);
        let view = WeatherFxView::new(cam.off_x as f32, cam.off_y as f32, 640, 360, 1.0);
        assert!(m.check_visible_squares(&view, 0));
        m.scan_for_tiles(&w, &view);
        assert!(!m.check_visible_squares(&view, 0));
        // The whole room is on screen.
        for x in 10..15 {
            for y in 10..15 {
                assert!(flags(&m, x, y).is_some(), "{x},{y}");
            }
        }
        // Skipped once the grid stacks were rebuilt this frame.
        m.force_mask_update();
        m.init_mask(&w, inside_player(), true);
        m.set_diamond_iter_done();
        m.scan_for_tiles(&w, &view);
        assert!(m.masks.is_empty());
    }

    #[test]
    fn render_takes_the_masked_path_indoors() {
        let w = Room::new();
        let rand = pz_core::random::Rand::new(3);
        let mut fx = IsoWeatherFx::new(&rand, &Default::default());
        let c = crate::weather_fx::WeatherFxClimate {
            precipitation_intensity: 1.0,
            precipitation_is_snow: false,
            ..Default::default()
        };
        fx.update_fx(&c, &Default::default());
        for _ in 0..50 {
            fx.update(&rand, &Default::default());
        }
        let view = WeatherFxView::new(0.0, 0.0, 640, 360, 1.0);
        let mut m = WeatherFxMask::default();
        m.init_mask(&w, inside_player(), true);
        let mut out = Vec::new();
        render_weather_fx(
            &mut fx,
            &mut m,
            &w,
            &FxMaskSprites::default(),
            &view,
            &FxMaskFrame::default(),
            &mut out,
        );
        let begins: Vec<usize> = out
            .iter()
            .filter_map(|c| {
                if let DrawCmd::WeatherFxBegin { index, .. } = c {
                    Some(*index)
                } else {
                    None
                }
            })
            .collect();
        // The mask, then precipitation (no clouds or fog).
        assert_eq!(begins, [0, 1]);
        assert!(out
            .iter()
            .any(|c| matches!(c, DrawCmd::BlendEquation(gl::FUNC_REVERSE_SUBTRACT))));
        // Outdoors the layers go straight to the world.
        let mut m = WeatherFxMask::default();
        m.init_mask(
            &w,
            FxMaskPlayer {
                square: Some([7, 7, 0]),
                ..Default::default()
            },
            true,
        );
        let mut out = Vec::new();
        render_weather_fx(
            &mut fx,
            &mut m,
            &w,
            &FxMaskSprites::default(),
            &view,
            &FxMaskFrame::default(),
            &mut out,
        );
        assert!(!out
            .iter()
            .any(|c| matches!(c, DrawCmd::WeatherFxBegin { .. })));
    }
}
