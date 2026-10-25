//! The object under the mouse: `FBORenderObjectPicker.ContextPick` (the
//! picker `IsoObjectPicker.ContextPick` delegates to with
//! `PerformanceSettings.fboRenderChunk`, the default), with
//! `IsoObjectPicker.ClickObject.calculateScore` and its comparator, as
//! [`HudPicking`] reads it (`UIManager.getPicked().tile`).
//!
//! The picker reads what the chunk renderer recorded for player 0
//! (`ObjectRenderInfo`: layer, target alpha, cutaway, the drawn rectangle
//! and scale; the chunk texture's position) and the objects' masks. The
//! render side supplies them through [`PickWorld`]; the traversal
//! (squares per level under the mouse, corpses near it), the ordering,
//! the filters, the mask tests and the score are here.

use super::host::HudPicking;
use pz_kahlua::Value;
use pz_render::iso::{coordmodulof, fastfloor, TILE_SCALE};
use pz_ui::tooltip::TooltipObject;
use std::cmp::Ordering;
use std::rc::Rc;

/// `ObjectRenderLayer`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RenderLayer {
    #[default]
    None,
    Floor,
    TranslucentFloor,
    Vegetation,
    Corpse,
    MinusFloor,
    Translucent,
    WorldInventoryObject,
    MinusFloorSE,
    TranslucentSE,
}

impl RenderLayer {
    fn is_translucent(self) -> bool {
        matches!(
            self,
            RenderLayer::Translucent | RenderLayer::TranslucentFloor | RenderLayer::TranslucentSE
        )
    }

    /// `renderLayerIndex`.
    fn index(self) -> i32 {
        match self {
            RenderLayer::None => 1000,
            RenderLayer::Floor | RenderLayer::TranslucentFloor => 0,
            RenderLayer::Vegetation => 1,
            RenderLayer::Corpse => 2,
            RenderLayer::MinusFloor | RenderLayer::Translucent => 3,
            RenderLayer::WorldInventoryObject => 4,
            RenderLayer::MinusFloorSE | RenderLayer::TranslucentSE => 5,
        }
    }
}

/// `object.getRenderInfo(0)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderInfo {
    pub layer: RenderLayer,
    pub target_alpha: f32,
    pub cutaway: bool,
    pub render_x: f32,
    pub render_y: f32,
    pub render_width: f32,
    pub render_height: f32,
    pub render_scale_x: f32,
    pub render_scale_y: f32,
}

/// What kind of `IsoObject` subclass an object is, for the filters and
/// the score.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickKind {
    #[default]
    Object,
    /// `IsoPlayer` (`local0`: `IsoPlayer.players[0]`).
    Player {
        local0: bool,
    },
    Door,
    /// `IsoThumpable`: `isDoor()`, `isWindow()`, the square's window on
    /// its facing edge (`getWindow(getGridSquareEdgeFacingDirection())`),
    /// `getLightSource() != null`.
    Thumpable {
        door: bool,
        window: bool,
        square_window_on_edge: bool,
        light_source: bool,
    },
    Window,
    WindowFrame {
        has_window: bool,
    },
    Curtain,
    LightSwitch,
    Generator,
    WaveSignal,
    DeadBody,
}

/// The sprite properties the picker tests (`IsoFlagType` and the tile
/// type).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpriteFlags {
    pub cut_w: bool,
    pub cut_n: bool,
    pub water: bool,
    pub bed: bool,
    pub water_piped: bool,
    pub solid_floor: bool,
    /// `getTileType()` is `WestRoofB`, `WestRoofM` or `WestRoofT`.
    pub west_roof: bool,
}

/// `isMaskClicked(x, y, flip)`.
pub type MaskFn = Rc<dyn Fn(i32, i32, bool) -> bool>;

/// An object on a square, as the picker sees it.
#[derive(Clone, Default)]
pub struct PickObject {
    /// Identity (`==` in the original).
    pub id: u64,
    /// `object.square` x, y, z.
    pub square: [i32; 3],
    /// `getObjectIndex()`.
    pub object_index: i32,
    pub kind: PickKind,
    /// `sprite != null` and its properties.
    pub sprite: Option<SpriteFlags>,
    /// `object.container != null` (`getContainer()`).
    pub has_container: bool,
    /// `ClickObject.isInteractiveEntity` (its `UiConfig`, through the
    /// multi-square master).
    pub interactive_entity: bool,
    /// `IsoDoor` / door `IsoThumpable`: `isAdjacentToSquare(playerSq)`.
    pub adjacent_to_player: bool,
    /// `offsetX` / `offsetY`.
    pub offset_x: f32,
    pub offset_y: f32,
    pub render: RenderInfo,
    pub mask: Option<MaskFn>,
    /// `rerouteMask`: the object picked instead.
    pub reroute_mask: Option<Rc<PickObject>>,
    /// `haveSpecialTooltip()`.
    pub have_special_tooltip: bool,
    /// The square's Lua object.
    pub square_value: Value,
    /// The square's room (`getRoom()`, `None` outside).
    pub room: Option<i64>,
    /// `IsoDeadBody.isMouseOver(screenX, screenY)`.
    pub mouse_over: Option<Rc<dyn Fn(i32, i32) -> bool>>,
}

impl PickObject {
    fn mask_clicked(&self, x: i32, y: i32, flip: bool) -> bool {
        self.mask.as_ref().is_some_and(|m| m(x, y, flip))
    }

    fn is_door(&self) -> bool {
        matches!(
            self.kind,
            PickKind::Door | PickKind::Thumpable { door: true, .. }
        )
    }
}

/// `IsoPlayer.players[0]` (also `IsoPlayer.getInstance()`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PickPlayer {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getZi()`.
    pub zi: i32,
    /// `getVectorFromDirection` before normalizing (`getForwardIsoDirection`).
    pub forward: (f32, f32),
    /// `getCurrentSquare()` exists, and its room.
    pub has_square: bool,
    pub room: Option<i64>,
}

/// The world and render state the picker reads (owner: world/render
/// thread).
pub trait PickWorld {
    /// `IsoPlayer.players[0]`; `None` picks nothing.
    fn player0(&self) -> Option<PickPlayer>;
    /// `Core.getZoom(0)`.
    fn zoom(&self) -> f32;
    /// `IsoCamera.getOffX(0)` / `getOffY(0)`.
    fn camera_offset(&self) -> (f32, f32);
    /// `square.getObjects()` of the square at `x, y, z`, in order; `None`
    /// without a square.
    fn objects(&self, x: i32, y: i32, z: i32) -> Option<Vec<PickObject>>;
    /// The corpses among the square's `getStaticMovingObjects()`.
    fn corpses(&self, _x: i32, _y: i32, _z: i32) -> Vec<PickObject> {
        Vec::new()
    }
    /// The chunk texture holding the square's level for player 0
    /// (`getRenderLevels(0).getFBOForLevel(z, zoom)`): its `renderX`,
    /// `renderY`; `None` when there is none.
    fn chunk_render_xy(&self, square: [i32; 3]) -> Option<(f32, f32)>;
    /// `DebugOptions.instance.fboRenderChunk.combinedFbo`.
    fn combined_fbo(&self) -> bool {
        false
    }
}

/// `IsoObjectPicker.ClickObject`.
#[derive(Clone)]
pub struct ClickObject {
    pub tile: Rc<PickObject>,
    pub square: [i32; 3],
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub width: i32,
    pub height: i32,
    pub lx: i32,
    pub ly: i32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub flip: bool,
    pub score: i32,
}

const LEFT_SIDE_XY: [i32; 14] = [0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3];
const RIGHT_SIDE_XY: [i32; 14] = [0, 0, 1, 0, 1, 1, 2, 1, 2, 2, 3, 2, 3, 3];

/// `IsoUtils.XToIso(0, sx, sy, z)` / `YToIso`.
fn to_iso(w: &dyn PickWorld, sx: f32, sy: f32, z: f32) -> (f32, f32) {
    let (ox, oy) = w.camera_offset();
    let px = sx + ox;
    let py = sy + oy;
    let x = (px + 2.0 * py) / (64.0 * TILE_SCALE as f32) + 3.0 * z;
    let y = (px - 2.0 * py) / (-64.0 * TILE_SCALE as f32) + 3.0 * z;
    (x, y)
}

fn objects_on_square(w: &dyn PickWorld, x: i32, y: i32, z: i32, out: &mut Vec<Rc<PickObject>>) {
    let Some(objs) = w.objects(x, y, z) else {
        return;
    };
    for o in objs {
        let ri = o.render;
        if ri.layer == RenderLayer::None || ri.target_alpha == 0.0 {
            if !o.sprite.is_some_and(|s| s.water) {
                continue;
            }
            out.push(Rc::new(o));
            continue;
        }
        if ri.render_width <= 0.0 || ri.render_height <= 0.0 {
            continue;
        }
        out.push(Rc::new(o));
    }
}

fn corpses_near(
    w: &dyn PickWorld,
    wx: i32,
    wy: i32,
    z: i32,
    sx: i32,
    sy: i32,
    out: &mut Vec<Rc<PickObject>>,
) {
    for dy in -1..=1 {
        for dx in -1..=1 {
            for c in w.corpses(wx + dx, wy + dy, z) {
                if c.kind == PickKind::DeadBody && c.mouse_over.as_ref().is_some_and(|m| m(sx, sy))
                {
                    out.push(Rc::new(c));
                }
            }
        }
    }
}

/// `getObjectsAt(screenX, screenY)` (zoomed screen coordinates).
fn objects_at(w: &dyn PickWorld, p: &PickPlayer, sx: i32, sy: i32) -> Vec<Rc<PickObject>> {
    let mut out = Vec::new();
    let levels: Vec<i32> = if p.zi < 0 {
        (-32..0).collect()
    } else {
        (0..=31).collect()
    };
    for z in levels {
        let (wx, wy) = to_iso(w, sx as f32, sy as f32, z as f32);
        let right = if p.zi < 0 {
            wx % 1.0 > wy % 1.0
        } else {
            coordmodulof(wx, 1) > coordmodulof(wy, 1)
        };
        let dxy = if right { &RIGHT_SIDE_XY } else { &LEFT_SIDE_XY };
        let (bx, by) = if p.zi < 0 {
            (wx as i32, wy as i32)
        } else {
            (fastfloor(wx), fastfloor(wy))
        };
        for i in (0..dxy.len()).step_by(2) {
            objects_on_square(w, bx + dxy[i], by + dxy[i + 1], z, &mut out);
        }
        corpses_near(w, fastfloor(wx), fastfloor(wy), z, sx, sy, &mut out);
    }
    out
}

/// `shouldIgnoreCutawayWall`.
fn ignore_cutaway_wall(o: &PickObject) -> bool {
    !matches!(
        o.kind,
        PickKind::WindowFrame { has_window: false }
            | PickKind::Thumpable {
                window: true,
                square_window_on_edge: false,
                ..
            }
            | PickKind::Door
    )
}

/// `handleWaterShader`: a water object outside the chunk layers is picked
/// by its square's full tile rectangle.
fn water_click(w: &dyn PickWorld, o: &Rc<PickObject>) -> Option<ClickObject> {
    if o.render.layer != RenderLayer::None || !o.sprite.is_some_and(|s| s.water) {
        return None;
    }
    let [x, y, z] = o.square;
    let (ox, oy) = w.camera_offset();
    let sx = pz_render::iso::x_to_screen(x as f32, y as f32);
    let sy = pz_render::iso::y_to_screen(x as f32, y as f32, z as f32, 0);
    Some(ClickObject {
        tile: o.clone(),
        square: o.square,
        x: (sx - ox - o.offset_x) as i32,
        y: (sy - oy - o.offset_y) as i32,
        z: 0,
        width: 64 * TILE_SCALE,
        height: 128 * TILE_SCALE,
        lx: 0,
        ly: 0,
        scale_x: 1.0,
        scale_y: 1.0,
        flip: false,
        score: 0,
    })
}

/// `getClickObjects(screenX, screenY)`: the objects under the point with
/// their drawn rectangles, by level, render layer and square.
pub fn click_objects(w: &dyn PickWorld, screen_x: i32, screen_y: i32) -> Vec<ClickObject> {
    let Some(p) = w.player0() else {
        return Vec::new();
    };
    let zoom = w.zoom();
    let x = screen_x as f32 * zoom;
    let y = screen_y as f32 * zoom;
    let mut out = Vec::new();
    for o in objects_at(w, &p, x as i32, y as i32) {
        let Some((cx, cy)) = w.chunk_render_xy(o.square) else {
            continue;
        };
        if let Some(c) = water_click(w, &o) {
            out.push(c);
            continue;
        }
        let ri = o.render;
        if ri.cutaway && ignore_cutaway_wall(&o) {
            continue;
        }
        let (rx, ry) = if ri.layer.is_translucent() {
            (ri.render_x as i32, ri.render_y as i32)
        } else if w.combined_fbo() {
            ((cx + ri.render_x) as i32, (cy + ri.render_y) as i32)
        } else {
            (
                (cx * zoom + ri.render_x) as i32,
                (cy * zoom + ri.render_y) as i32,
            )
        };
        out.push(ClickObject {
            square: o.square,
            x: rx,
            y: ry,
            z: o.square[2],
            width: ri.render_width as i32,
            height: ri.render_height as i32,
            lx: 0,
            ly: 0,
            // `(int)renderInfo.renderScaleX` stored in a float.
            scale_x: ri.render_scale_x as i32 as f32,
            scale_y: ri.render_scale_y as i32 as f32,
            flip: false,
            score: 0,
            tile: o,
        });
    }
    out.sort_by(|a, b| {
        (a.square[2] - b.square[2])
            .cmp(&0)
            .then((a.tile.render.layer.index() - b.tile.render.layer.index()).cmp(&0))
            .then(
                ((a.square[0] + a.square[1] * 100_000) - (b.square[0] + b.square[1] * 100_000))
                    .cmp(&0),
            )
    });
    out
}

/// `shouldIgnoreWallLikeObject`.
fn ignore_wall_like(o: &PickObject, s: &SpriteFlags, target_alpha: f32) -> bool {
    if !s.cut_w && !s.cut_n {
        return false;
    }
    if matches!(
        o.kind,
        PickKind::Window | PickKind::Door | PickKind::Thumpable { door: true, .. }
    ) {
        return false;
    }
    target_alpha < 1.0
}

/// `ClickObject.calculateScore()` against `IsoPlayer.getInstance()`.
fn score(c: &ClickObject, p: &PickPlayer) -> i32 {
    let mut score = 1.0f32;
    let [sx, sy, sz] = c.square;
    let mut tx = sx as f32 + 0.5 - p.x;
    let mut ty = sy as f32 + 0.5 - p.y;
    normalize(&mut tx, &mut ty);
    let (mut fx, mut fy) = p.forward;
    normalize(&mut fx, &mut fy);
    let angle = fx * tx + fy * ty;
    score += (angle * 4.0).abs();
    let o = &*c.tile;
    let thumpable = matches!(o.kind, PickKind::Thumpable { .. });
    if o.is_door() {
        score += 6.0;
        if o.adjacent_to_player {
            score += 1.0;
        }
        if p.z > sz as f32 {
            score -= 1000.0;
        }
    } else if o.kind == PickKind::Window {
        score += 4.0;
        if p.z > sz as f32 {
            score -= 1000.0;
        }
    } else {
        if p.has_square && o.room == p.room {
            score += 1.0;
        } else {
            score -= 100_000.0;
        }
        if p.z > sz as f32 {
            score -= 1000.0;
        }
        let faded_thumpable = thumpable
            && o.render.target_alpha < 0.99
            && !o.interactive_entity
            && (o.render.target_alpha < 0.5 || !o.has_container);
        if matches!(o.kind, PickKind::Player { .. }) || faded_thumpable {
            score -= 100_000.0;
        }
        // `sprite.properties`: a picked object without a sprite throws
        // here in the original; only sprite objects reach it.
        let s = o.sprite.unwrap_or_default();
        if o.kind == PickKind::Curtain {
            score += 3.0;
        } else if o.kind == PickKind::LightSwitch {
            score += 20.0;
        } else if s.bed {
            score += 2.0;
        } else if o.has_container {
            score += 10.0;
        } else if o.kind == PickKind::Generator {
            score += 11.0;
        } else if o.kind == PickKind::WaveSignal {
            score += 20.0;
        } else if matches!(
            o.kind,
            PickKind::Thumpable {
                light_source: true,
                ..
            }
        ) || s.water_piped
        {
            score += 3.0;
        } else if s.solid_floor || s.west_roof {
            score -= 100.0;
        } else if s.cut_w || s.cut_n {
            score -= 2.0;
        } else if o.interactive_entity {
            score += 2.0;
        }
    }
    let dist = (p.x - (sx as f32 + 0.5)).abs() + (p.y - (sy as f32 + 0.5)).abs();
    score -= dist / 2.0;
    score as i32
}

/// `Vector2.normalize()`.
fn normalize(x: &mut f32, y: &mut f32) {
    let len_sq = *x * *x + *y * *y;
    if (1.0 - len_sq).abs() < 1.0e-5 {
        return;
    }
    if len_sq == 0.0 {
        *x = 0.0;
        *y = 0.0;
        return;
    }
    let len = len_sq.sqrt();
    *x /= len;
    *y /= len;
}

/// `IsoObjectPicker.comp`.
fn compare(a: &ClickObject, b: &ClickObject) -> Ordering {
    match a.score.cmp(&b.score) {
        Ordering::Equal => {}
        o => return o,
    }
    if a.square == b.square {
        return (a.tile.object_index - b.tile.object_index).cmp(&0);
    }
    Ordering::Equal
}

/// `FBORenderObjectPicker.ContextPick(screenX, screenY)`: the best scored
/// object whose drawn mask is under the point. (The negated comparisons
/// are the original's, false for NaN.)
#[allow(clippy::neg_cmp_op_on_partial_ord)]
pub fn context_pick(w: &dyn PickWorld, screen_x: i32, screen_y: i32) -> Option<ClickObject> {
    let p = w.player0()?;
    let zoom = w.zoom();
    let x = screen_x as f32 * zoom;
    let y = screen_y as f32 * zoom;
    let mut choices = Vec::new();
    for mut c in click_objects(w, screen_x, screen_y).into_iter().rev() {
        let o = c.tile.clone();
        if matches!(o.kind, PickKind::Player { local0: true }) {
            continue;
        }
        if let Some(s) = &o.sprite {
            let ta = o.render.target_alpha;
            if ta == 0.0 || ignore_wall_like(&o, s, ta) {
                continue;
            }
        }
        if !(x > c.x as f32)
            || !(y > c.y as f32)
            || !(x <= (c.x + c.width) as f32)
            || !(y <= (c.y + c.height) as f32)
            || matches!(o.kind, PickKind::Player { .. })
        {
            continue;
        }
        let (mx, my) = if c.scale_x != 1.0 || c.scale_y != 1.0 {
            let x1 = c.x as f32 + (x - c.x as f32) / c.scale_x;
            let y1 = c.y as f32 + (y - c.y as f32) / c.scale_y;
            ((x1 - c.x as f32) as i32, (y1 - c.y as f32) as i32)
        } else {
            ((x - c.x as f32) as i32, (y - c.y as f32) as i32)
        };
        if !o.mask_clicked(mx, my, c.flip) {
            continue;
        }
        if let Some(r) = &o.reroute_mask {
            c.tile = r.clone();
        }
        c.lx = fastfloor(x) - c.x;
        c.ly = fastfloor(y) - c.y;
        choices.push(c);
    }
    if choices.is_empty() {
        return None;
    }
    for c in &mut choices {
        c.score = score(c, &p);
    }
    choices.sort_by(compare);
    choices.pop()
}

/// [`HudPicking`] over a [`PickWorld`]: `ContextPick` at the mouse.
pub struct WorldPicking<W: PickWorld> {
    pub world: W,
}

impl<W: PickWorld> HudPicking for WorldPicking<W> {
    fn picked_tile(&self) -> Option<TooltipObject> {
        let (mx, my) = pz_ui::with_ui(|m| (m.mouse.x, m.mouse.y));
        let c = context_pick(&self.world, mx, my)?;
        Some(TooltipObject {
            id: c.tile.id,
            have_special_tooltip: c.tile.have_special_tooltip,
            square: c.tile.square_value.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// One level-0 tile world: objects with 64x128 rectangles drawn at
    /// their square's screen position, opaque everywhere.
    struct World {
        squares: HashMap<[i32; 3], Vec<PickObject>>,
        player: PickPlayer,
    }

    fn obj(id: u64, sq: [i32; 3], index: i32, kind: PickKind, layer: RenderLayer) -> PickObject {
        let sx = pz_render::iso::x_to_screen(sq[0] as f32, sq[1] as f32);
        let sy = pz_render::iso::y_to_screen(sq[0] as f32, sq[1] as f32, sq[2] as f32, 0);
        PickObject {
            id,
            square: sq,
            object_index: index,
            kind,
            sprite: Some(SpriteFlags::default()),
            render: RenderInfo {
                layer,
                target_alpha: 1.0,
                cutaway: false,
                render_x: sx - 64.0,
                render_y: sy - 192.0,
                render_width: 128.0,
                render_height: 256.0,
                render_scale_x: 1.0,
                render_scale_y: 1.0,
            },
            mask: Some(Rc::new(|_, _, _| true)),
            ..Default::default()
        }
    }

    impl PickWorld for World {
        fn player0(&self) -> Option<PickPlayer> {
            Some(self.player)
        }
        fn zoom(&self) -> f32 {
            1.0
        }
        fn camera_offset(&self) -> (f32, f32) {
            (0.0, 0.0)
        }
        fn objects(&self, x: i32, y: i32, z: i32) -> Option<Vec<PickObject>> {
            self.squares.get(&[x, y, z]).cloned()
        }
        fn chunk_render_xy(&self, _square: [i32; 3]) -> Option<(f32, f32)> {
            Some((0.0, 0.0))
        }
    }

    fn world(objs: Vec<PickObject>) -> World {
        let mut squares: HashMap<[i32; 3], Vec<PickObject>> = HashMap::new();
        for o in objs {
            squares.entry(o.square).or_default().push(o);
        }
        World {
            squares,
            player: PickPlayer {
                x: 10.5,
                y: 12.5,
                z: 0.0,
                zi: 0,
                forward: (0.0, 1.0),
                has_square: true,
                room: None,
            },
        }
    }

    fn point_over(sq: [i32; 3]) -> (i32, i32) {
        let sx = pz_render::iso::x_to_screen(sq[0] as f32 + 0.5, sq[1] as f32 + 0.5);
        let sy = pz_render::iso::y_to_screen(sq[0] as f32 + 0.5, sq[1] as f32 + 0.5, 0.0, 0);
        (sx as i32, sy as i32)
    }

    #[test]
    fn container_beats_floor_and_walls() {
        let sq = [10, 14, 0];
        let floor = obj(1, sq, 0, PickKind::Object, RenderLayer::Floor);
        let mut floor = floor;
        floor.sprite = Some(SpriteFlags {
            solid_floor: true,
            ..Default::default()
        });
        let mut crate_ = obj(2, sq, 1, PickKind::Object, RenderLayer::MinusFloor);
        crate_.has_container = true;
        let w = world(vec![floor, crate_]);
        let (x, y) = point_over(sq);
        let c = context_pick(&w, x, y).expect("picked");
        assert_eq!(c.tile.id, 2);
        // The floor alone is still picked (the only choice).
        let w = world(vec![obj(1, sq, 0, PickKind::Object, RenderLayer::Floor)]);
        assert_eq!(context_pick(&w, x, y).unwrap().tile.id, 1);
    }

    #[test]
    fn players_masks_and_fades_are_skipped() {
        let sq = [11, 13, 0];
        let (x, y) = point_over(sq);
        let w = world(vec![obj(
            1,
            sq,
            0,
            PickKind::Player { local0: false },
            RenderLayer::MinusFloor,
        )]);
        assert!(context_pick(&w, x, y).is_none());
        let mut o = obj(2, sq, 0, PickKind::Object, RenderLayer::MinusFloor);
        o.mask = Some(Rc::new(|_, _, _| false));
        assert!(context_pick(&world(vec![o]), x, y).is_none());
        let mut wall = obj(3, sq, 0, PickKind::Object, RenderLayer::MinusFloor);
        wall.sprite = Some(SpriteFlags {
            cut_w: true,
            ..Default::default()
        });
        wall.render.target_alpha = 0.5;
        assert!(context_pick(&world(vec![wall.clone()]), x, y).is_none());
        // A faded door still picks.
        wall.kind = PickKind::Door;
        assert_eq!(context_pick(&world(vec![wall]), x, y).unwrap().tile.id, 3);
    }

    #[test]
    fn ties_go_to_the_higher_object_index_and_reroutes_apply() {
        let sq = [10, 13, 0];
        let (x, y) = point_over(sq);
        let a = obj(1, sq, 0, PickKind::Object, RenderLayer::MinusFloor);
        let mut b = obj(2, sq, 1, PickKind::Object, RenderLayer::MinusFloor);
        b.reroute_mask = Some(Rc::new(obj(
            9,
            sq,
            2,
            PickKind::Object,
            RenderLayer::MinusFloor,
        )));
        let c = context_pick(&world(vec![a, b]), x, y).unwrap();
        assert_eq!(c.tile.id, 9);
        assert_eq!(
            (c.lx, c.ly),
            (fastfloor(x as f32) - c.x, fastfloor(y as f32) - c.y)
        );
    }

    #[test]
    fn score_terms() {
        let p = PickPlayer {
            x: 10.5,
            y: 10.5,
            z: 0.0,
            zi: 0,
            forward: (1.0, 1.0),
            has_square: true,
            room: Some(4),
        };
        let mk = |kind, room| {
            let mut o = obj(1, [12, 12, 0], 0, kind, RenderLayer::MinusFloor);
            o.room = room;
            ClickObject {
                tile: Rc::new(o),
                square: [12, 12, 0],
                x: 0,
                y: 0,
                z: 0,
                width: 0,
                height: 0,
                lx: 0,
                ly: 0,
                scale_x: 1.0,
                scale_y: 1.0,
                flip: false,
                score: 0,
            }
        };
        // Facing the square: |dot| = 1 -> +4; same room +1; distance 4 -> -2.
        assert_eq!(score(&mk(PickKind::Object, Some(4)), &p), 4);
        assert_eq!(score(&mk(PickKind::Object, None), &p), -99_997);
        assert_eq!(score(&mk(PickKind::LightSwitch, Some(4)), &p), 24);
        assert_eq!(score(&mk(PickKind::Window, None), &p), 7);
    }
}
