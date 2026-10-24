//! `FBORenderCutaways` and the `IsoCell` cutaway helpers it relies on:
//! which squares render this frame (buildings collapsed above the player,
//! orphan structures hidden), which wall squares are cut away
//! (`IsoGridSquare.playerCutawayFlags`: 1 = north wall, 2 = west wall),
//! and the occlusion grid.
//!
//! The original updates this state incrementally from many per-frame
//! checks (`checkPlayerRoom`, `SetCutawayRoomsForPlayer`,
//! `checkExteriorWalls`, `checkSlopedSurfaces`, `checkOccludedRooms`,
//! `doCutawayVisitSquares`, `CalculatePointsOfInterest`,
//! `CalculateBuildingsToCollapse`, `checkHiddenBuildingLevels`,
//! `prepareChunkForUpdating`, `calculateOccludingSquares`) and invalidates
//! chunk textures as it goes. [`Cutaways::update`] recomputes the same end
//! state from scratch each frame instead; [`CutawayState::dirty_chunk_levels`]
//! replaces the invalidation by diffing two frames' states.

use std::collections::{HashMap, HashSet};

use pz_render::iso::fastfloor;

use crate::occlusion::{chunk_of, OccludedGrid};
use crate::world::{BuildingId, GarageDoorSpan, RenderObject, RenderSquare, RoomId, SlopeDir, SpriteFlags};

/// `playerCutawayFlags` bit for a cut north wall.
pub const CUTAWAY_N: u8 = 1;
/// `playerCutawayFlags` bit for a cut west wall.
pub const CUTAWAY_W: u8 = 2;

/// A `BuildingDef`'s bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildingDef {
    pub x: i32,
    pub y: i32,
    pub x2: i32,
    pub y2: i32,
    /// `isEntirelyEmptyOutside()`.
    pub entirely_empty_outside: bool,
}

impl BuildingDef {
    pub fn w(&self) -> i32 {
        self.x2 - self.x
    }

    pub fn h(&self) -> i32 {
        self.y2 - self.y
    }

    /// `new Rectangle(x - 1, y - 1, w + 2, h + 2).contains(px, py)`.
    fn expanded_contains(&self, px: i32, py: i32) -> bool {
        rect_contains(self.x - 1, self.y - 1, self.w() + 2, self.h() + 2, px, py)
    }
}

/// `java.awt.Rectangle.contains(x, y)`.
fn rect_contains(rx: i32, ry: i32, rw: i32, rh: i32, x: i32, y: i32) -> bool {
    rw >= 0 && rh >= 0 && x >= rx && y >= ry && x < rx + rw && y < ry + rh
}

/// The world as cutaways read it: `IsoCell.getGridSquare`, building
/// definitions and the loaded chunks.
pub trait CutawayWorld {
    /// `getGridSquare(x, y, z)`.
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare>;
    /// The building's definition.
    fn building(&self, id: BuildingId) -> Option<BuildingDef>;
    /// `IsoChunk.minLevel/maxLevel` of the loaded chunk `(wx, wy)`.
    fn chunk_levels(&self, wx: i32, wy: i32) -> Option<(i32, i32)>;
    /// The player's on-screen chunks (`perPlayerData.onScreenChunks`).
    fn on_screen_chunks(&self) -> Vec<(i32, i32)>;
    /// `IsoCell.maxHeight`.
    fn max_height(&self) -> i32 {
        32
    }
    /// `BuildingDef.getRoofRoomID(level)`: the synthetic room id of a
    /// building's roof at `level`. The original numbers these per cell; any
    /// id distinct from real rooms works.
    fn roof_room_id(&self, building: BuildingId, level: i32) -> Option<RoomId> {
        Some(i64::MIN / 2 + building.0 as i64 * 64 + (level + 32) as i64)
    }
}

/// A plain [`CutawayWorld`]: squares and buildings in hash maps.
#[derive(Clone, Debug, Default)]
pub struct GridWorld {
    pub squares: HashMap<(i32, i32, i32), RenderSquare>,
    pub buildings: HashMap<BuildingId, BuildingDef>,
    /// The on-screen chunks; when empty, every chunk holding a square.
    pub chunks: Vec<(i32, i32)>,
    /// Per-chunk level ranges; chunks not listed span their squares'
    /// levels.
    pub levels: HashMap<(i32, i32), (i32, i32)>,
    /// Per-chunk floor blood ([`crate::weather::SurfaceWorld::chunk_blood`]).
    pub blood: HashMap<(i32, i32), crate::blood::ChunkBlood>,
}

impl GridWorld {
    pub fn insert(&mut self, sq: RenderSquare) {
        self.squares.insert((sq.x, sq.y, sq.z), sq);
    }

    pub fn square_mut(&mut self, x: i32, y: i32, z: i32) -> &mut RenderSquare {
        self.squares.entry((x, y, z)).or_insert_with(|| RenderSquare::new(x, y, z))
    }
}

impl crate::weather::SurfaceWorld for GridWorld {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
        self.squares.get(&(x, y, z))
    }

    fn chunk_blood(&self, wx: i32, wy: i32) -> Option<&crate::blood::ChunkBlood> {
        self.blood.get(&(wx, wy))
    }
}

impl CutawayWorld for GridWorld {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
        self.squares.get(&(x, y, z))
    }

    fn building(&self, id: BuildingId) -> Option<BuildingDef> {
        self.buildings.get(&id).copied()
    }

    fn chunk_levels(&self, wx: i32, wy: i32) -> Option<(i32, i32)> {
        if let Some(l) = self.levels.get(&(wx, wy)) {
            return Some(*l);
        }
        let zs = self.squares.keys().filter(|(x, y, _)| chunk_of(*x, *y) == (wx, wy)).map(|k| k.2);
        zs.fold(None, |acc, z| match acc {
            None => Some((z, z)),
            Some((lo, hi)) => Some((lo.min(z), hi.max(z))),
        })
    }

    fn on_screen_chunks(&self) -> Vec<(i32, i32)> {
        if !self.chunks.is_empty() {
            return self.chunks.clone();
        }
        let mut c: Vec<(i32, i32)> = self.squares.keys().map(|k| chunk_of(k.0, k.1)).collect::<HashSet<_>>().into_iter().collect();
        c.sort();
        c
    }
}

/// `IsoCell.playerWindowPeekingRoomId` and the room's building.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeekedRoom {
    pub room: RoomId,
    pub building: BuildingId,
}

/// The player (`IsoCamera.frameState.camCharacter`, `IsoPlayer`) as the
/// cutaways read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CutawayPlayer {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getForwardDirection()`.
    pub forward: [f32; 2],
    /// `isAiming()` (player 0).
    pub aiming: bool,
    /// The mouse point of interest (player 0 aiming without a joypad):
    /// see [`mouse_point_of_interest`].
    pub mouse_poi: Option<[f32; 2]>,
    /// The room the player peeks into through a window or door while
    /// outside (see [`peeked_in_room`]).
    pub peeking: Option<PeekedRoom>,
    /// The occlusion grid bounds ([`crate::occlusion::grid_bounds`]);
    /// `None` disables occlusion.
    pub occlusion_bounds: Option<[i32; 4]>,
}

impl CutawayPlayer {
    pub fn at(x: f32, y: f32, z: f32) -> Self {
        CutawayPlayer { x, y, z, forward: [1.0, 0.0], aiming: false, mouse_poi: None, peeking: None, occlusion_bounds: None }
    }
}

/// `CalculatePointsOfInterest`'s mouse point: the mouse's screen position
/// (`Mouse.getX/getY`, plus the camera offset) projected to level `z`.
pub fn mouse_point_of_interest(cam: &crate::Camera, mouse_x: f32, mouse_y: f32, z: i32) -> [f32; 2] {
    let my = mouse_y + (52 * pz_render::iso::TILE_SCALE) as f32;
    let (px, py) = (mouse_x + cam.off_x as f32, my + cam.off_y as f32);
    [pz_render::iso::x_to_iso(px, py, z as f32), pz_render::iso::y_to_iso(px, py, z as f32)]
}

/// A compass direction (`IsoDirections`, without `Max`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir8 {
    N,
    NW,
    W,
    SW,
    S,
    SE,
    E,
    NE,
}

impl Dir8 {
    pub const ALL: [Dir8; 8] = [Dir8::N, Dir8::NW, Dir8::W, Dir8::SW, Dir8::S, Dir8::SE, Dir8::E, Dir8::NE];

    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir8::N => (0, -1),
            Dir8::NW => (-1, -1),
            Dir8::W => (-1, 0),
            Dir8::SW => (-1, 1),
            Dir8::S => (0, 1),
            Dir8::SE => (1, 1),
            Dir8::E => (1, 0),
            Dir8::NE => (1, -1),
        }
    }
}

/// `IsoCell.GetPeekedInBuilding`: the room of the building square next to
/// the player's (outside) square that the player looks into, when nothing
/// blocks the step (`clear(dx, dy)` is `LosUtil.lineClear(...) !=
/// Blocked` for the step from the player's square).
pub fn peeked_in_room(world: &dyn CutawayWorld, square: &RenderSquare, look: Dir8, clear: impl Fn(i32, i32) -> bool) -> Option<PeekedRoom> {
    use Dir8::*;
    let tries: [(&[Dir8], (i32, i32)); 4] = [(&[NW, N, NE], (0, -1)), (&[SW, W, NW], (-1, 0)), (&[SE, S, SW], (0, 1)), (&[NE, E, SE], (1, 0))];
    for (dirs, (dx, dy)) in tries {
        if !dirs.contains(&look) || !clear(dx, dy) {
            continue;
        }
        let Some(t) = world.square(square.x + dx, square.y + dy, square.z) else { continue };
        if let Some(building) = t.cut.building {
            // The building comes from the room, so the room exists.
            return t.cut.room.map(|room| PeekedRoom { room, building });
        }
    }
    None
}

/// `FBORenderCutaways.PointOfInterest`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointOfInterest {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub mouse_pointer: bool,
}

// Square helpers: IsoGridSquare.has(IsoFlagType) is the union of the
// objects' sprite properties.

fn has(sq: &RenderSquare, f: impl Fn(&SpriteFlags) -> bool) -> bool {
    sq.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| f(&s.flags)))
}

fn has_cut_n(sq: &RenderSquare) -> bool {
    sq.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| s.cut_n))
}

fn has_cut_w(sq: &RenderSquare) -> bool {
    sq.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| s.cut_w))
}

/// `getWall(bNorth)`.
fn get_wall(sq: &RenderSquare, north: bool) -> Option<&RenderObject> {
    sq.objects.iter().find(|o| o.sprite.as_ref().is_some_and(|s| !o.is_wall_se() && ((s.cut_n && north) || (s.cut_w && !north))))
}

/// `getGarageDoor(bNorth)`.
fn get_garage_door(sq: &RenderSquare, north: bool) -> Option<GarageDoorSpan> {
    sq.objects.iter().filter_map(|o| o.garage_door).find(|g| g.north == north)
}

/// `TreatAsSolidFloor`.
fn treat_as_solid_floor(sq: &RenderSquare) -> bool {
    has(sq, |f| f.solidfloor) || sq.cut.has_stairs || sq.cut.sloped.is_some()
}

/// `getFirstMultiLevelObject`.
fn first_multi_level_object(sq: &RenderSquare) -> Option<&RenderObject> {
    sq.objects.iter().find(|o| o.sprite.as_ref().is_some_and(|s| s.grid_levels > 1))
}

fn adjacent<'w>(world: &'w dyn CutawayWorld, sq: &RenderSquare, d: Dir8) -> Option<&'w RenderSquare> {
    let (dx, dy) = d.delta();
    world.square(sq.x + dx, sq.y + dy, sq.z)
}

/// `HasStairsBelow`.
fn has_stairs_below(world: &dyn CutawayWorld, sq: &RenderSquare) -> bool {
    world.square(sq.x, sq.y, sq.z - 1).is_some_and(|b| b.cut.has_stairs)
}

/// `hasFloorAtTopOfStairs`.
fn has_floor_at_top_of_stairs(world: &dyn CutawayWorld, sq: &RenderSquare) -> bool {
    if sq.floor().is_none() {
        return false;
    }
    adjacent(world, sq, Dir8::S).is_some_and(|s| has_stairs_below(world, s)) || adjacent(world, sq, Dir8::E).is_some_and(|e| has_stairs_below(world, e))
}

/// `isRoofRoomSquare`.
pub fn is_roof_room_square(sq: &RenderSquare) -> bool {
    sq.z != 0 && sq.cut.room.is_none() && sq.cut.associated_building.is_some() && treat_as_solid_floor(sq)
}

/// `getRoomID()`, or the roof room of a roof square at `level`.
fn room_or_roof(world: &dyn CutawayWorld, sq: &RenderSquare, level: i32) -> Option<RoomId> {
    if sq.cut.room.is_some() {
        return sq.cut.room;
    }
    if is_roof_room_square(sq) {
        return world.roof_room_id(sq.cut.associated_building?, level);
    }
    None
}

/// `IsoGridOcclusionData.GetBuildingFloorsProjectedOnSquare`: the
/// buildings whose floors (at any level above) project onto `(x, y, z)`,
/// and whether an orphan structure could be in the way.
fn floors_projected_on_square(world: &dyn CutawayWorld, out: &mut Vec<BuildingId>, x: i32, y: i32, z: i32) -> bool {
    let mut orphan = false;
    let (mut x, mut y, mut z) = (x, y, z);
    let empty_outside = |b: BuildingId| world.building(b).is_some_and(|d| d.entirely_empty_outside);
    while z < world.max_height() {
        if let Some(test) = world.square(x, y, z) {
            let mut building = test.cut.building;
            if building.is_none() {
                building = test.cut.roof_hide_building;
                if test.z > 0 && building.is_some_and(empty_outside) {
                    building = None;
                    orphan = true;
                }
            }
            if let Some(b) = building {
                push_unique(out, b);
            }
            let mut drop_z = z - 1;
            while drop_z >= 0 && building.is_none() {
                if let Some(d) = world.square(x, y, drop_z) {
                    building = d.cut.building.or(d.cut.roof_hide_building);
                    if let Some(b) = building {
                        push_unique(out, b);
                    }
                }
                drop_z -= 1;
            }
            if building.is_none() && !orphan && test.z != 0 && test.cut.player_built_floor {
                orphan = true;
            }
        }
        z += 1;
        x += 3;
        y += 3;
    }
    orphan
}

fn push_unique<T: PartialEq>(v: &mut Vec<T>, t: T) {
    if !v.contains(&t) {
        v.push(t);
    }
}

/// `IsoGridOcclusionData.OcclusionFilter`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcclusionFilter {
    Left,
    Right,
    All,
}

/// `IsoGridOcclusionData` for one square (`LazyInitializeSoftOccluders`):
/// the buildings that could hide it, split by the side of the square they
/// cover.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OcclusionData {
    pub left: Vec<BuildingId>,
    pub right: Vec<BuildingId>,
    pub all: Vec<BuildingId>,
    pub left_orphan: bool,
    pub right_orphan: bool,
}

impl OcclusionData {
    pub fn compute(world: &dyn CutawayWorld, x: i32, y: i32, z: i32) -> Self {
        let mut all = Vec::new();
        let mut left = Vec::new();
        let mut right = Vec::new();
        let mut orphan = false;
        for d in 0..4 {
            orphan |= floors_projected_on_square(world, &mut all, x + d, y + d, z);
        }
        let mut left_orphan = false;
        for d in 0..3 {
            left_orphan |= floors_projected_on_square(world, &mut left, x + d, y + 1 + d, z);
        }
        let mut right_orphan = false;
        for d in 0..3 {
            right_orphan |= floors_projected_on_square(world, &mut right, x + 1 + d, y + d, z);
        }
        left_orphan |= orphan;
        right_orphan |= orphan;
        for b in &all {
            push_unique(&mut left, *b);
            push_unique(&mut right, *b);
        }
        let mut all = left.clone();
        for b in &right {
            push_unique(&mut all, *b);
        }
        OcclusionData { left, right, all, left_orphan, right_orphan }
    }

    pub fn buildings(&self, f: OcclusionFilter) -> &[BuildingId] {
        match f {
            OcclusionFilter::Left => &self.left,
            OcclusionFilter::Right => &self.right,
            OcclusionFilter::All => &self.all,
        }
    }

    pub fn orphan(&self, f: OcclusionFilter) -> bool {
        match f {
            OcclusionFilter::Left => self.left_orphan,
            OcclusionFilter::Right => self.right_orphan,
            OcclusionFilter::All => self.left_orphan || self.right_orphan,
        }
    }
}

/// `IsoCell.GetBuildingsInFrontOfCharacter`: the buildings that could
/// hide `square` from the camera, and `occludedByOrphanStructureFlag`.
pub fn buildings_in_front_of_character(world: &dyn CutawayWorld, square: Option<&RenderSquare>, right_of_square: bool) -> (Vec<BuildingId>, bool) {
    let mut out = Vec::new();
    let mut orphan = false;
    let Some(sq) = square else { return (out, orphan) };
    let (x, y, z) = (sq.x, sq.y, sq.z);
    let mut visit = |x: i32, y: i32, z: i32, right: bool| buildings_in_front_of_square(world, x, y, z, right, &mut out, &mut orphan);
    visit(x, y, z, right_of_square);
    if z < world.max_height() {
        visit(x - 1 + 3, y - 1 + 3, z + 1, right_of_square);
        visit(x - 2 + 3, y - 2 + 3, z + 1, right_of_square);
        if right_of_square {
            visit(x + 3, y - 1 + 3, z + 1, !right_of_square);
            visit(x - 1 + 3, y - 2 + 3, z + 1, !right_of_square);
        } else {
            visit(x - 1 + 3, y + 3, z + 1, !right_of_square);
            visit(x - 2 + 3, y - 1 + 3, z + 1, !right_of_square);
        }
    }
    (out, orphan)
}

/// `GetBuildingsInFrontOfCharacterSquare`.
fn buildings_in_front_of_square(world: &dyn CutawayWorld, x: i32, y: i32, z: i32, right: bool, out: &mut Vec<BuildingId>, orphan: &mut bool) {
    if world.square(x, y, z).is_none() {
        if z < world.max_height() {
            buildings_in_front_of_square(world, x + 3, y + 3, z + 1, right, out, orphan);
        }
        return;
    }
    let data = OcclusionData::compute(world, x, y, z);
    let filter = if right { OcclusionFilter::Right } else { OcclusionFilter::Left };
    *orphan |= data.orphan(filter);
    for b in data.buildings(filter) {
        push_unique(out, *b);
    }
}

/// `IsoCell.IsBehindStuff`: whether anything above could hide the square
/// (the old roof-hiding test; `hideFloors`).
pub fn is_behind_stuff(world: &dyn CutawayWorld, sq: &RenderSquare) -> bool {
    if !has(sq, |f| f.exterior) {
        return true;
    }
    let mut z = 1;
    while z < 64 && sq.z + z < world.max_height() {
        for y in -5..=6 {
            for x in -5..=6 {
                if x < y - 5 || x > y + 5 {
                    continue;
                }
                let Some(sq2) = world.square(sq.x + x + z * 3, sq.y + y + z * 3, sq.z + z) else { continue };
                if sq2.objects.is_empty() {
                    continue;
                }
                if z == 1 && sq2.objects.len() == 1 && sq2.objects[0].sprite.as_ref().is_some_and(|s| s.name.starts_with("lighting_outdoor")) {
                    continue;
                }
                return true;
            }
        }
        z += 1;
    }
    false
}

/// `FBORenderCutaways.CutawayWall`: a run of wall squares along a chunk
/// row (`y1 == y2`, north walls) or column (west walls), `[x1, x2)` /
/// `[y1, y2)`.
#[derive(Clone, Debug, PartialEq)]
pub struct CutawayWall {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    pub level: i32,
    /// `occludedRoomIds`.
    pub occluded_room_ids: Vec<RoomId>,
    /// `occludedSquaresMaskForSeenRooms`.
    pub occluded_mask: u32,
}

const NORTH_WALL_DXY: [i32; 12] = [-1, -1, -2, -2, -3, -3, 0, -1, -1, -2, -2, -3];
const WEST_WALL_DXY: [i32; 12] = [-1, -1, -2, -2, -3, -3, -1, 0, -2, -1, -3, -2];

impl CutawayWall {
    fn new(x1: i32, y1: i32, level: i32) -> Self {
        CutawayWall { x1, y1, x2: x1, y2: y1, level, occluded_room_ids: Vec::new(), occluded_mask: 0 }
    }

    pub fn is_horizontal(&self) -> bool {
        self.y1 == self.y2
    }

    /// The wall's squares' coordinates.
    fn squares(&self) -> Vec<(i32, i32)> {
        if self.is_horizontal() {
            (self.x1..self.x2).map(|x| (x, self.y1)).collect()
        } else {
            (self.y1..self.y2).map(|y| (self.x1, y)).collect()
        }
    }

    /// The squares behind the wall square at offset `i`.
    fn behind(&self, i: i32) -> impl Iterator<Item = (i32, i32)> + '_ {
        let (dxy, horizontal) = if self.is_horizontal() { (&NORTH_WALL_DXY, true) } else { (&WEST_WALL_DXY, false) };
        (0..dxy.len() - 1).step_by(2).map(move |j| if horizontal { (self.x1 + i + dxy[j], self.y1 + dxy[j + 1]) } else { (self.x1 + dxy[j], self.y1 + i + dxy[j + 1]) })
    }

    fn len(&self) -> i32 {
        if self.is_horizontal() {
            self.x2 - self.x1
        } else {
            self.y2 - self.y1
        }
    }

    /// `calculateOccludedRooms`.
    fn calculate_occluded_rooms(&mut self, world: &dyn CutawayWorld) {
        for i in 0..self.len() {
            let pts: Vec<(i32, i32)> = self.behind(i).collect();
            for (x, y) in pts {
                let Some(sq) = world.square(x, y, self.level) else { continue };
                if let Some(room) = room_or_roof(world, sq, sq.z) {
                    push_unique(&mut self.occluded_room_ids, room);
                }
            }
        }
    }

    /// `calculateOccludedSquaresMask`.
    fn occluded_squares_mask(&self, world: &dyn CutawayWorld, player_z: i32, room: RoomId) -> u32 {
        if !self.occluded_room_ids.contains(&room) {
            return 0;
        }
        let mut mask = 0;
        for i in 0..self.len() {
            for (x, y) in self.behind(i) {
                let Some(sq) = world.square(x, y, player_z) else { continue };
                if sq.objects.is_empty() || !sq.cut.could_see {
                    continue;
                }
                if room_or_roof(world, sq, player_z) != Some(room) {
                    continue;
                }
                mask |= 1 << i;
                break;
            }
        }
        mask
    }

    /// `isPartOfBuilding`.
    fn is_part_of_building(&self, world: &dyn CutawayWorld, id: BuildingId, def: &BuildingDef) -> bool {
        let is = |x: i32, y: i32| world.square(x, y, self.level).is_some_and(|s| s.cut.building == Some(id));
        if self.is_horizontal() {
            if self.y1 < def.y || self.y1 > def.y2 {
                return false;
            }
            (self.x1..self.x2).any(|x| is(x, self.y1) || is(x, self.y1 - 1))
        } else {
            if self.x1 < def.x || self.x1 > def.x2 {
                return false;
            }
            (self.y1..self.y2).any(|y| is(self.x1, y) || is(self.x1 - 1, y))
        }
    }
}

/// `FBORenderCutaways.SlopedSurface`: a run of sloped squares; unlike
/// walls its flags cover `[x1, x2]` / `[y1, y2]` inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlopedSurface {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl SlopedSurface {
    pub fn is_horizontal(&self) -> bool {
        self.x2 > self.x1
    }
}

/// Runs of squares along a chunk level's rows (`horizontal`) or columns,
/// as `recreateLevel_*` builds them: `(x1, y1, x2, y2)` with the end one
/// past the last square.
fn runs(wx: i32, wy: i32, horizontal: bool, mut pick: impl FnMut(i32, i32) -> bool) -> Vec<(i32, i32, i32, i32)> {
    let mut out = Vec::new();
    for a in 0..8 {
        let mut start: Option<(i32, i32)> = None;
        for b in 0..8 {
            let (lx, ly) = if horizontal { (b, a) } else { (a, b) };
            let (x, y) = (wx * 8 + lx, wy * 8 + ly);
            if pick(x, y) {
                if start.is_none() {
                    start = Some((x, y));
                }
                continue;
            }
            if let Some((x1, y1)) = start.take() {
                out.push((x1, y1, x, y));
            }
        }
        if let Some((x1, y1)) = start {
            let end = if horizontal { (wx * 8 + 8, wy * 8 + a) } else { (wx * 8 + a, wy * 8 + 8) };
            out.push((x1, y1, end.0, end.1));
        }
    }
    out
}

/// The cutaway state of one frame for one player.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CutawayState {
    pub player_x: f32,
    pub player_y: f32,
    /// `fastfloor(camCharacterZ)`.
    pub player_z: i32,
    /// Whether `camCharacterSquare` exists, and its room.
    pub player_square: bool,
    pub player_room: Option<RoomId>,
    pub points_of_interest: Vec<PointOfInterest>,
    /// `getCollapsedBuildings()`.
    pub buildings_to_collapse: Vec<BuildingId>,
    collapse_defs: Vec<(BuildingId, BuildingDef)>,
    /// `IsoCell.occludedByOrphanStructureFlag`.
    pub occluded_by_orphan_structures: bool,
    /// `IsoCell.tempPlayerCutawayRoomIds` (sorted).
    pub cutaway_room_ids: Vec<RoomId>,
    pub peeking: Option<PeekedRoom>,
    /// The north/west wall runs of the player's level.
    pub exterior_walls: Vec<CutawayWall>,
    pub all_walls: Vec<CutawayWall>,
    pub sloped_surfaces: Vec<SlopedSurface>,
    /// Per chunk level: `squareFlags & CLDSF_SHOULD_RENDER` bits
    /// (`lx + ly * 8`).
    render: HashMap<(i32, i32, i32), u64>,
    /// Per chunk level: the rendered squares not occluded.
    visible: HashMap<(i32, i32, i32), u64>,
    /// Per chunk level: `playerCutawayFlags`.
    flags: HashMap<(i32, i32, i32), [u8; 64]>,
    force_render: HashSet<(i32, i32, i32)>,
    pub occlusion: Option<OccludedGrid>,
    /// Objects hiding the player and fading back in
    /// ([`crate::obscuring::Obscuring::update`]); set by the caller.
    pub obscuring: crate::obscuring::Obscuring,
}

fn key(x: i32, y: i32, z: i32) -> ((i32, i32, i32), usize) {
    let (wx, wy) = chunk_of(x, y);
    ((wx, wy, z), ((x - wx * 8) + (y - wy * 8) * 8) as usize)
}

impl CutawayState {
    /// `ChunkLevelData.shouldRenderSquare`: false for squares outside the
    /// on-screen chunks.
    pub fn should_render_square(&self, x: i32, y: i32, z: i32) -> bool {
        let (k, i) = key(x, y, z);
        self.render.get(&k).is_some_and(|m| m & (1 << i) != 0)
    }

    /// `FBORenderOcclusion.isOccluded`.
    pub fn is_occluded(&self, x: i32, y: i32, z: i32) -> bool {
        self.occlusion.as_ref().is_some_and(|g| g.is_occluded(x, y, z))
    }

    /// Rendered and not occluded: the test `renderOneLevel` applies before
    /// classifying a square's objects.
    pub fn is_visible(&self, x: i32, y: i32, z: i32) -> bool {
        let (k, i) = key(x, y, z);
        self.visible.get(&k).is_some_and(|m| m & (1 << i) != 0)
    }

    /// `getPlayerCutawayFlag(playerIndex, ...)`: [`CUTAWAY_N`] |
    /// [`CUTAWAY_W`].
    pub fn cutaway_flags(&self, x: i32, y: i32, z: i32) -> u8 {
        let (k, i) = key(x, y, z);
        self.flags.get(&k).map_or(0, |f| f[i])
    }

    fn add_flag(&mut self, x: i32, y: i32, z: i32, flag: u8) {
        let (k, i) = key(x, y, z);
        self.flags.entry(k).or_insert([0; 64])[i] |= flag;
    }

    /// `isForceRenderSquare`: a square of a collapsed building above the
    /// player drawn only for its multi-level objects that reach down to
    /// the player's level.
    pub fn is_force_render_square(&self, x: i32, y: i32, z: i32) -> bool {
        self.force_render.contains(&(x, y, z))
    }

    /// `isBuildingCollapsed`.
    pub fn is_building_collapsed(&self, b: BuildingId) -> bool {
        self.buildings_to_collapse.contains(&b)
    }

    /// `FBORenderCutaways.CanBuildingSquareOccludePlayer`: the square lies
    /// within (or one square around) a collapsed building.
    pub fn can_building_square_occlude_player(&self, x: i32, y: i32) -> bool {
        self.collapse_defs.iter().any(|(_, d)| d.expanded_contains(x, y))
    }

    /// `shouldHideElevatedFloor`: raised floors of a collapsed building on
    /// the player's level.
    pub fn should_hide_elevated_floor(&self, sq: &RenderSquare, obj: &RenderObject) -> bool {
        let Some(s) = obj.sprite.as_ref() else { return false };
        if !s.flags.floor_height_one_third && !s.flags.floor_height_two_thirds {
            return false;
        }
        sq.z == self.player_z && sq.cut.associated_building.is_some_and(|b| self.is_building_collapsed(b))
    }

    /// The chunk levels whose textures must be re-recorded because their
    /// rendered squares or cutaway flags differ from `prev`'s
    /// (`invalidateLevel(..., CUTAWAYS / OCCLUSION)`); a level whose north
    /// walls on its west/east edge or west walls on its north/south edge
    /// change also invalidates that neighbour.
    pub fn dirty_chunk_levels(&self, prev: &CutawayState) -> Vec<(i32, i32, i32)> {
        let mut out = HashSet::new();
        let keys: HashSet<&(i32, i32, i32)> = self.visible.keys().chain(prev.visible.keys()).chain(self.flags.keys()).chain(prev.flags.keys()).collect();
        for k in keys {
            if self.visible.get(k).copied().unwrap_or(0) != prev.visible.get(k).copied().unwrap_or(0) {
                out.insert(*k);
            }
            let a = self.flags.get(k).copied().unwrap_or([0; 64]);
            let b = prev.flags.get(k).copied().unwrap_or([0; 64]);
            if a == b {
                continue;
            }
            out.insert(*k);
            let &(wx, wy, z) = k;
            for i in 0..64 {
                let changed = a[i] ^ b[i];
                let (lx, ly) = (i % 8, i / 8);
                if changed & CUTAWAY_N != 0 {
                    if lx == 0 {
                        out.insert((wx - 1, wy, z));
                    }
                    if lx == 7 {
                        out.insert((wx + 1, wy, z));
                    }
                }
                if changed & CUTAWAY_W != 0 {
                    if ly == 0 {
                        out.insert((wx, wy - 1, z));
                    }
                    if ly == 7 {
                        out.insert((wx, wy + 1, z));
                    }
                }
            }
        }
        let mut v: Vec<_> = out.into_iter().collect();
        v.sort();
        v
    }
}

/// `FBORenderCutaways` for one player.
#[derive(Clone, Debug)]
pub struct Cutaways {
    /// `FBORenderOcclusion.enabled`.
    pub occlusion_enabled: bool,
}

impl Default for Cutaways {
    fn default() -> Self {
        Cutaways { occlusion_enabled: true }
    }
}

impl Cutaways {
    pub fn new() -> Self {
        Self::default()
    }

    /// One frame's cutaway state, in `renderTilesInternal`'s order: points
    /// of interest and collapsed buildings (`recalculateGridStacks`), the
    /// player's cutaway rooms, the rendered squares, exterior walls and
    /// sloped surfaces, occluded rooms and the cutaway visitor, then
    /// occlusion.
    pub fn update(&mut self, world: &dyn CutawayWorld, player: &CutawayPlayer) -> CutawayState {
        let pz = fastfloor(player.z);
        let psq = world.square(fastfloor(player.x), fastfloor(player.y), pz);
        let mut st = CutawayState {
            player_x: player.x,
            player_y: player.y,
            player_z: pz,
            player_square: psq.is_some(),
            player_room: psq.and_then(|s| s.cut.room),
            peeking: player.peeking,
            ..Default::default()
        };
        st.points_of_interest = points_of_interest(world, player, psq);
        calculate_buildings_to_collapse(world, &mut st);
        st.cutaway_room_ids = cutaway_rooms_for_player(world, player, psq);
        let chunks = world.on_screen_chunks();
        let mut orphans = OrphanCache::default();
        for &(wx, wy) in &chunks {
            let Some((lo, hi)) = world.chunk_levels(wx, wy) else { continue };
            for z in lo..=hi {
                let mut mask = 0u64;
                for i in 0..64 {
                    let Some(sq) = world.square(wx * 8 + (i % 8), wy * 8 + (i / 8), z) else { continue };
                    if should_render_building_square(world, &mut st, &mut orphans, sq) {
                        mask |= 1 << i;
                    }
                }
                st.render.insert((wx, wy, z), mask);
            }
        }
        // Walls and sloped surfaces of the player's level.
        for &(wx, wy) in &chunks {
            let Some((lo, hi)) = world.chunk_levels(wx, wy) else { continue };
            if pz < lo || pz > hi {
                continue;
            }
            st.exterior_walls.extend(exterior_walls(world, wx, wy, pz));
            st.all_walls.extend(all_walls(world, wx, wy, pz));
            st.sloped_surfaces.extend(sloped_surfaces(world, wx, wy, pz));
        }
        // checkExteriorWalls: fences and other walls outside buildings.
        let fences: Vec<CutawayWall> = st.exterior_walls.iter().filter(|w| should_cutaway_fence(world, &st, player, w)).cloned().collect();
        for w in &fences {
            let flag = if w.is_horizontal() { CUTAWAY_N } else { CUTAWAY_W };
            for (x, y) in w.squares() {
                if world.square(x, y, w.level).is_some() {
                    st.add_flag(x, y, w.level, flag);
                }
            }
        }
        // checkSlopedSurfaces.
        let slopes: Vec<SlopedSurface> = st.sloped_surfaces.iter().copied().filter(|s| sloped_should_cutaway(world, &st, player, psq, s)).collect();
        for s in slopes {
            if s.is_horizontal() {
                for x in s.x1..=s.x2 {
                    if world.square(x, s.y1, pz).is_some() {
                        st.add_flag(x, s.y1, pz, CUTAWAY_N);
                    }
                }
            } else {
                for y in s.y1..=s.y2 {
                    if world.square(s.x1, y, pz).is_some() {
                        st.add_flag(s.x1, y, pz, CUTAWAY_W);
                    }
                }
            }
        }
        // checkOccludedRooms.
        let rooms = st.cutaway_room_ids.clone();
        for w in &mut st.all_walls {
            if rooms.iter().any(|r| w.occluded_room_ids.contains(r)) {
                w.occluded_mask = rooms.iter().fold(0, |m, r| m | w.occluded_squares_mask(world, pz, *r));
            }
        }
        // doCutawayVisitSquares.
        let (north, west) = cutaway_visit(world, &st);
        for (x, y) in north {
            st.add_flag(x, y, pz, CUTAWAY_N);
        }
        for (x, y) in west {
            st.add_flag(x, y, pz, CUTAWAY_W);
        }
        // FBORenderCell.calculateOccludingSquares.
        if let (true, Some(bounds)) = (self.occlusion_enabled, player.occlusion_bounds) {
            let mut grid = OccludedGrid::new(bounds);
            for &(wx, wy) in &chunks {
                let Some((lo, hi)) = world.chunk_levels(wx, wy) else { continue };
                for z in lo..=hi {
                    for i in 0..64 {
                        let (x, y) = (wx * 8 + (i % 8), wy * 8 + (i / 8));
                        let Some(sq) = world.square(x, y, z) else { continue };
                        if !st.should_render_square(x, y, z) {
                            continue;
                        }
                        let below = if z > lo { world.square(x, y, z - 1) } else { None };
                        if !sq.cut.vision_blocked_below && below.is_some() {
                            continue;
                        }
                        grid.add_occluder(x, y, z);
                    }
                }
            }
            st.occlusion = Some(grid);
        }
        let mut visible = HashMap::new();
        for (&(wx, wy, z), &mask) in &st.render {
            let mut v = mask;
            for i in 0..64 {
                if v & (1 << i) != 0 && st.is_occluded(wx * 8 + (i % 8), wy * 8 + (i / 8), z) {
                    v &= !(1 << i);
                }
            }
            visible.insert((wx, wy, z), v);
        }
        st.visible = visible;
        st
    }
}

/// `CalculatePointsOfInterest`.
fn points_of_interest(world: &dyn CutawayWorld, player: &CutawayPlayer, psq: Option<&RenderSquare>) -> Vec<PointOfInterest> {
    let poi = |x: f32, y: f32, z: f32, mouse_pointer: bool| PointOfInterest { x: fastfloor(x), y: fastfloor(y), z: fastfloor(z), mouse_pointer };
    let mut out = vec![poi(player.x, player.y, player.z, false)];
    if let Some([mx, my]) = player.mouse_poi {
        out.push(poi(mx, my, player.z, true));
    }
    let Some(psq) = psq else { return out };
    let (left, right) = squares_around_player(world, player, psq.z);
    for sq in left.into_iter().chain(right) {
        if !sq.cut.could_see || (sq.cut.building.is_some() && sq.cut.building != psq.cut.building) {
            continue;
        }
        out.push(poi(sq.x as f32, sq.y as f32, sq.z as f32, false));
    }
    out
}

/// `IsoCell.GetSquaresAroundPlayerSquare`: the squares in a band in front
/// of the player, split into those left and right of the player's screen
/// column.
fn squares_around_player<'w>(world: &'w dyn CutawayWorld, player: &CutawayPlayer, z: i32) -> (Vec<&'w RenderSquare>, Vec<&'w RenderSquare>) {
    let start_x = fastfloor(player.x - 4.0);
    let start_y = fastfloor(player.y - 4.0);
    let (px, py) = (fastfloor(player.x), fastfloor(player.y));
    let mut left = Vec::new();
    let mut right = Vec::new();
    for y in start_y..start_y + 10 {
        for x in start_x..start_x + 10 {
            if (x < px && y < py) || (x == px && y == py) {
                continue;
            }
            let dx = x as f32 - player.x;
            let dy = y as f32 - player.y;
            if !((dy as f64) < dx as f64 + 4.5 && (dy as f64) > dx as f64 - 4.5) {
                continue;
            }
            let Some(sq) = world.square(x, y, z) else { continue };
            if dy >= dx {
                left.push(sq);
            }
            if dy <= dx {
                right.push(sq);
            }
        }
    }
    (left, right)
}

/// `CalculateBuildingsToCollapse`.
fn calculate_buildings_to_collapse(world: &dyn CutawayWorld, st: &mut CutawayState) {
    let mut out = Vec::new();
    let mut orphan = false;
    for poi in st.points_of_interest.iter().filter(|p| !p.mouse_pointer) {
        let sq = world.square(poi.x, poi.y, poi.z);
        let (mut b, o) = buildings_in_front_of_character(world, sq, false);
        orphan |= o;
        if b.is_empty() {
            let (b2, o2) = buildings_in_front_of_character(world, sq, true);
            b = b2;
            orphan |= o2;
        }
        for id in b {
            push_unique(&mut out, id);
        }
    }
    if let Some(p) = st.peeking {
        push_unique(&mut out, p.building);
    }
    st.occluded_by_orphan_structures = orphan;
    st.collapse_defs = out.iter().filter_map(|&id| world.building(id).map(|d| (id, d))).collect();
    st.buildings_to_collapse = out;
}

/// `IsoCell.SetCutawayRoomsForPlayer`: the rooms around the player (within
/// 1.5 squares, visible, ahead of the player) whose walls get cut away.
fn cutaway_rooms_for_player(world: &dyn CutawayWorld, player: &CutawayPlayer, psq: Option<&RenderSquare>) -> Vec<RoomId> {
    let mut out = Vec::new();
    let Some(psq) = psq else { return out };
    let Some(_) = room_or_roof(world, psq, psq.z) else {
        if let Some(p) = player.peeking {
            out.push(p.room);
        }
        return out;
    };
    let range = 1.5f32;
    let (min_x, min_y) = ((player.x - range) as i32, (player.y - range) as i32);
    let (max_x, max_y) = ((player.x + range) as i32, (player.y + range) as i32);
    for x in min_x..=max_x {
        for y in min_y..=max_y {
            let Some(sq) = world.square(x, y, psq.z) else { continue };
            let is_player = sq.x == psq.x && sq.y == psq.y;
            let Some(room) = room_or_roof(world, sq, sq.z) else { continue };
            if (!sq.cut.can_see && !is_player) || out.contains(&room) {
                continue;
            }
            let v = [sq.x as f32 + 0.5 - player.x, sq.y as f32 + 0.5 - player.y];
            if !is_player && player.forward[0] * v[0] + player.forward[1] * v[1] <= 0.0 {
                continue;
            }
            out.push(room);
        }
    }
    out.sort();
    out
}

/// Per chunk level `OrphanStructures.isOrphanStructureSquare` bits.
#[derive(Default)]
struct OrphanCache {
    levels: HashMap<(i32, i32, i32), u64>,
}

impl OrphanCache {
    /// `OrphanStructures.calculate` for a chunk level.
    fn level(&mut self, world: &dyn CutawayWorld, wx: i32, wy: i32, z: i32) -> u64 {
        if let Some(m) = self.levels.get(&(wx, wy, z)) {
            return *m;
        }
        let mut m = 0u64;
        if world.chunk_levels(wx, wy).is_some_and(|(lo, hi)| z >= lo && z <= hi) {
            for i in 0..64 {
                if let Some(sq) = world.square(wx * 8 + (i % 8), wy * 8 + (i / 8), z) {
                    if is_orphan_structure_square_calc(world, sq) {
                        m |= 1 << i;
                    }
                }
            }
        }
        self.levels.insert((wx, wy, z), m);
        m
    }

    /// `isOrphanStructureSquare`.
    fn is_orphan(&mut self, world: &dyn CutawayWorld, sq: Option<&RenderSquare>) -> bool {
        let Some(sq) = sq else { return false };
        let (wx, wy) = chunk_of(sq.x, sq.y);
        let (_, i) = key(sq.x, sq.y, sq.z);
        self.level(world, wx, wy, sq.z) & (1 << i) != 0
    }
}

/// `isPlayerBuiltSquare`.
fn is_player_built_square(sq: &RenderSquare) -> bool {
    sq.cut.player_built_floor || sq.cut.has_stairs || sq.cut.stair_pillar
}

/// `calculateOrphanStructureSquare`: a player-built structure above ground
/// that belongs to no building.
fn is_orphan_structure_square_calc(world: &dyn CutawayWorld, sq: &RenderSquare) -> bool {
    let mut building = sq.cut.building;
    if building.is_none() {
        building = sq.cut.roof_hide_building;
        if building.is_some_and(|b| world.building(b).is_some_and(|d| d.entirely_empty_outside)) {
            return true;
        }
    }
    let mut drop_z = sq.z - 1;
    while drop_z >= 0 && building.is_none() {
        if let Some(d) = world.square(sq.x, sq.y, drop_z) {
            building = d.cut.building;
            if building.is_none() {
                building = d.cut.roof_hide_building;
            }
        }
        drop_z -= 1;
    }
    if building.is_some() || sq.cut.associated_building.is_some() {
        return false;
    }
    if is_player_built_square(sq) {
        return true;
    }
    let neighbour = |d: Dir8| adjacent(world, sq, d).filter(|n| n.cut.building.is_none()).is_some_and(|n| is_player_built_square(n) || has_stairs_below(world, n));
    if neighbour(Dir8::N) || neighbour(Dir8::W) {
        return true;
    }
    has(sq, |f| f.wall_se) && neighbour(Dir8::NW)
}

/// `shouldRenderBuildingSquare` (with `isForceRenderSquare` recorded).
fn should_render_building_square(world: &dyn CutawayWorld, st: &mut CutawayState, orphans: &mut OrphanCache, sq: &RenderSquare) -> bool {
    let pz = st.player_z;
    if sq.cut.associated_building.is_some_and(|b| st.is_building_collapsed(b)) && sq.z > pz {
        if let Some(o) = first_multi_level_object(sq) {
            let pos_z = o.sprite.as_ref().map_or(0, |s| s.grid_pos_z);
            if sq.z - pos_z <= pz {
                st.force_render.insert((sq.x, sq.y, sq.z));
                return true;
            }
        }
        return false;
    }
    if sq.z > pz && pz < 0 {
        return false;
    }
    if sq.z == pz + 1 && has_floor_at_top_of_stairs(world, sq) {
        return true;
    }
    if sq.z > pz {
        let (wx, wy) = chunk_of(sq.x, sq.y);
        let has_orphans = orphans.level(world, wx, wy, sq.z) != 0;
        // OrphanStructures.shouldCutaway: the orphan flag and a level
        // above the player (true here); levels below 1 are never checked.
        let in_range = st.occluded_by_orphan_structures && sq.z >= 1;
        if has_orphans && in_range {
            if orphans.is_orphan(world, Some(sq)) {
                return false;
            }
            for d in Dir8::ALL {
                if orphans.is_orphan(world, adjacent(world, sq, d)) {
                    return false;
                }
            }
        }
    }
    true
}

/// `ChunkLevelsData.hasRoomBelow`.
fn has_room_below(world: &dyn CutawayWorld, sq: &RenderSquare) -> bool {
    if sq.z == 0 {
        return false;
    }
    let (wx, wy) = chunk_of(sq.x, sq.y);
    let Some((lo, _)) = world.chunk_levels(wx, wy) else { return false };
    (lo..sq.z).rev().any(|z| world.square(sq.x, sq.y, z).is_some_and(|s| s.cut.building.is_some() || s.cut.roof_hide_building.is_some()))
}

/// `ChunkLevelsData.isAdjacentToRoom`.
fn is_adjacent_to_room(world: &dyn CutawayWorld, sq: &RenderSquare, d: Dir8) -> bool {
    if let Some(adj) = adjacent(world, sq, d) {
        if adj.cut.building.is_some() || adj.cut.roof_hide_building.is_some() || has_room_below(world, adj) {
            return true;
        }
    }
    sq.cut.room.is_some() || sq.cut.roof_hide_building.is_some() || has_room_below(world, sq)
}

/// `recreateLevel_ExteriorWalls`: wall runs that are not part of any
/// building (fences, walls of yards).
pub fn exterior_walls(world: &dyn CutawayWorld, wx: i32, wy: i32, level: i32) -> Vec<CutawayWall> {
    let mut out = Vec::new();
    for north in [true, false] {
        let pick = |x: i32, y: i32| {
            world.square(x, y, level).is_some_and(|sq| {
                get_wall(sq, north).is_some()
                    && if north { has(sq, |f| f.wall_n || f.wall_nw || f.door_wall_n || f.window_n) } else { has(sq, |f| f.wall_w || f.wall_nw || f.door_wall_w || f.window_w) }
                    && !is_adjacent_to_room(world, sq, if north { Dir8::N } else { Dir8::W })
            })
        };
        for (x1, y1, x2, y2) in runs(wx, wy, north, pick) {
            out.push(CutawayWall { x2, y2, ..CutawayWall::new(x1, y1, level) });
        }
    }
    out
}

/// `recreateLevel_AllWalls`: every wall run, with the rooms behind it.
pub fn all_walls(world: &dyn CutawayWorld, wx: i32, wy: i32, level: i32) -> Vec<CutawayWall> {
    let mut out = Vec::new();
    for north in [true, false] {
        let pick = |x: i32, y: i32| {
            let Some(sq) = world.square(x, y, level) else { return false };
            let window = has(sq, |f| if north { f.window_n } else { f.window_w });
            let wall_like = (get_wall(sq, north).is_some() || window)
                && if north { has(sq, |f| f.wall_n || f.wall_nw || f.door_wall_n || f.window_n) } else { has(sq, |f| f.wall_w || f.wall_nw || f.door_wall_w || f.window_w) };
            wall_like || get_garage_door(sq, north).is_some()
        };
        for (x1, y1, x2, y2) in runs(wx, wy, north, pick) {
            let mut w = CutawayWall { x2, y2, ..CutawayWall::new(x1, y1, level) };
            w.calculate_occluded_rooms(world);
            out.push(w);
        }
    }
    out
}

/// `recreateLevel_SlopedSurfaces`.
pub fn sloped_surfaces(world: &dyn CutawayWorld, wx: i32, wy: i32, level: i32) -> Vec<SlopedSurface> {
    let mut out = Vec::new();
    for horizontal in [true, false] {
        let pick = |x: i32, y: i32| {
            world.square(x, y, level).and_then(|s| s.cut.sloped).is_some_and(|d| if horizontal { matches!(d, SlopeDir::W | SlopeDir::E) } else { matches!(d, SlopeDir::N | SlopeDir::S) })
        };
        for (x1, y1, x2, y2) in runs(wx, wy, horizontal, pick) {
            out.push(SlopedSurface { x1, y1, x2, y2 });
        }
    }
    out
}

/// The `(index, range)` pairs `shouldCutawayFence` and
/// `SlopedSurface.shouldCutaway` test the points of interest with: the
/// first (two when aiming) at range 6, the rest at 3.
fn poi_ranges<'a>(st: &'a CutawayState, player: &CutawayPlayer) -> impl Iterator<Item = (&'a PointOfInterest, i32)> + 'a {
    let max = if player.aiming { 2 } else { 1 }.min(st.points_of_interest.len());
    st.points_of_interest.iter().enumerate().map(move |(i, p)| (p, if i >= max { 3 } else { 6 }))
}

/// `CutawayWall.shouldCutawayFence`.
fn should_cutaway_fence(world: &dyn CutawayWorld, st: &CutawayState, player: &CutawayPlayer, w: &CutawayWall) -> bool {
    poi_ranges(st, player).any(|(p, range)| {
        let Some(sq) = world.square(p.x, p.y, p.z) else { return false };
        if !sq.cut.can_see {
            return false;
        }
        if w.is_horizontal() {
            sq.y < w.y1 && sq.y >= w.y1 - range
        } else {
            sq.x < w.x1 && sq.x >= w.x1 - range
        }
    })
}

/// `SlopedSurface.shouldCutaway`.
fn sloped_should_cutaway(world: &dyn CutawayWorld, st: &CutawayState, player: &CutawayPlayer, psq: Option<&RenderSquare>, s: &SlopedSurface) -> bool {
    poi_ranges(st, player).any(|(p, range)| {
        let Some(sq) = world.square(p.x, p.y, p.z) else { return false };
        if psq.is_some_and(|ps| ps.cut.sloped.is_some()) || !sq.cut.can_see {
            return false;
        }
        if s.is_horizontal() {
            sq.y < s.y1 && sq.y >= s.y1 - range
        } else {
            sq.x < s.x1 && sq.x >= s.x1 - range && sq.y >= s.y1 && sq.y < s.y2
        }
    })
}

/// Squares `(x, y)` of one level.
type SquareSet = HashSet<(i32, i32)>;

/// `doCutawayVisitSquares` / `cutawayVisit`: the wall squares of the
/// player's level to cut, as north and west results.
fn cutaway_visit(world: &dyn CutawayWorld, st: &CutawayState) -> (SquareSet, SquareSet) {
    let pz = st.player_z;
    let mut north = HashSet::new();
    let mut west = HashSet::new();
    let mut visited_n = HashSet::new();
    let mut visited_w = HashSet::new();
    for poi in &st.points_of_interest {
        if poi.z != pz || world.square(poi.x, poi.y, poi.z).is_none() || visited_n.contains(&(poi.x, poi.y)) || visited_w.contains(&(poi.x, poi.y)) {
            continue;
        }
        for w in &st.all_walls {
            let horizontal = w.is_horizontal();
            for (x, y) in w.squares() {
                let Some(test) = world.square(x, y, pz) else { continue };
                let first = if horizontal { visited_n.insert((x, y)) } else { visited_w.insert((x, y)) };
                if !st.should_render_square(x, y, w.level) || test.objects.is_empty() || !first || !is_cutaway_square(world, st, w, test) {
                    continue;
                }
                let wall_se = has(test, |f| f.wall_se);
                if horizontal {
                    north.insert((x, y));
                    if wall_se {
                        west.insert((x, y));
                    }
                } else {
                    west.insert((x, y));
                    if wall_se {
                        north.insert((x, y));
                    }
                }
            }
        }
    }
    // Exterior walls not out of range are added as visited results
    // (setVisitedSquares); checkExteriorWalls already set their flags.
    (north, west)
}

/// `IsCutawaySquare`.
fn is_cutaway_square(world: &dyn CutawayWorld, st: &CutawayState, w: &CutawayWall, sq: &RenderSquare) -> bool {
    if st.cutaway_room_ids.is_empty() {
        return is_collapsible_building_square(world, st, w, sq);
    }
    if is_cutaway_due_to_peeking(st, w, sq) {
        return true;
    }
    for room in &st.cutaway_room_ids {
        if !w.occluded_room_ids.contains(room) {
            continue;
        }
        let bit = if w.is_horizontal() { sq.x - w.x1 } else { sq.y - w.y1 };
        return w.occluded_mask & (1 << bit) != 0;
    }
    false
}

/// `isCutawayDueToPeeking`: the wall squares in front of a player peeking
/// in from outside, or the whole garage door they stand at.
fn is_cutaway_due_to_peeking(st: &CutawayState, w: &CutawayWall, sq: &RenderSquare) -> bool {
    if st.peeking.is_none() {
        return false;
    }
    let (px, py) = (fastfloor(st.player_x), fastfloor(st.player_y));
    let garage = has(sq, |f| f.garage_door);
    if w.is_horizontal() {
        if py == w.y1 - 1 || py == w.y1 {
            if garage {
                if let Some(g) = get_garage_door(sq, true) {
                    return sq.x >= g.first && sq.x <= g.last;
                }
            }
            if sq.x >= px && sq.x <= px + 1 {
                return true;
            }
        }
    } else if px == w.x1 - 1 || px == w.x1 {
        if garage {
            if let Some(g) = get_garage_door(sq, false) {
                return sq.y >= g.first && sq.y <= g.last;
            }
        }
        if sq.y >= py && sq.y <= py + 1 {
            return true;
        }
    }
    false
}

/// `IsCollapsibleBuildingSquare`.
fn is_collapsible_building_square(world: &dyn CutawayWorld, st: &CutawayState, w: &CutawayWall, sq: &RenderSquare) -> bool {
    if has(sq, |f| f.force_render) || !st.player_square {
        return false;
    }
    st.collapse_defs.iter().any(|(id, def)| w.is_part_of_building(world, *id, def) && def.expanded_contains(sq.x, sq.y))
}

/// Square-level `cutN`/`cutW` (`square.has(IsoFlagType.cutN/cutW)`).
pub fn square_cut(sq: &RenderSquare) -> (bool, bool) {
    (has_cut_n(sq), has_cut_w(sq))
}
