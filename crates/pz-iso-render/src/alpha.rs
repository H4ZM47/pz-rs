//! `FBORenderCell.calculateObjectTargetAlpha` and friends: the alpha an
//! object fades to given this frame's cutaways, the render layer that
//! alpha implies (`calculateObjectRenderInfo`), and the cutaway state a
//! wall is drawn with (`renderMinusFloor_DoorOrWall`).

use crate::cell::{object_layer, Layer};
use crate::cutaways::{square_cut, CutawayState, CutawayWorld, CUTAWAY_N, CUTAWAY_W};
use crate::world::{ObjectKind, RenderObject, RenderSquare, TileType};

/// A frame's cutaway state with the world it was computed from, as the
/// renderer reads it.
#[derive(Clone, Copy)]
pub struct CutawayView<'a> {
    pub state: &'a CutawayState,
    pub world: &'a dyn CutawayWorld,
}

/// Which wall a cutaway draw is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallSide {
    N,
    W,
}

/// The cutaway inputs of `IsoGridSquare.DoWallLightingN/W/NW` for one
/// wall object: the cutaway flags of the square and its neighbours and
/// whether the cut door or window has been seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WallCutaway {
    pub cutaway_self: u8,
    pub cutaway_n: u8,
    pub cutaway_s: u8,
    pub cutaway_w: u8,
    pub cutaway_e: u8,
    pub has_seen_door_n: bool,
    pub has_seen_door_w: bool,
    pub has_seen_window_n: bool,
    pub has_seen_window_w: bool,
    /// `IsoGridSquare.circleStencil`: the object is not `NeverCutaway`.
    pub circle_stencil: bool,
}

/// How `performDrawWallSegmentSingle` draws one side of a wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallDraw {
    /// `performDrawWall`. `no_wall_lighting` draws with the square's light
    /// instead of the wall shaper's vertex lights (a cut-away door or
    /// window, or a `NoWallLighting` sprite).
    Normal { no_wall_lighting: bool },
    /// `DoCutawayShader`: the wall drawn cut down with the `wallcutaways`
    /// masks under the circle stencil.
    Cutaway,
}

/// `isWindowOrWindowFrame(obj, north)`.
fn is_window_or_frame(obj: &RenderObject, north: bool) -> bool {
    let Some(s) = obj.sprite.as_ref() else { return false };
    if (north && s.flags.window_n) || (!north && s.flags.window_w) {
        return true;
    }
    matches!(obj.kind, ObjectKind::Window { north: n } if n == north)
}

fn is_door(t: TileType, side: WallSide) -> bool {
    match side {
        WallSide::N => matches!(t, TileType::DoorFrN | TileType::DoorN),
        WallSide::W => matches!(t, TileType::DoorFrW | TileType::DoorW),
    }
}

fn square_has_door(sq: &RenderSquare, side: WallSide) -> bool {
    sq.objects.iter().any(|o| is_door(o.tile_type(), side))
}

fn square_has_window(sq: &RenderSquare, side: WallSide) -> bool {
    sq.objects.iter().any(|o| is_window_or_frame(o, side == WallSide::N))
}

impl CutawayView<'_> {
    fn flags_at(&self, x: i32, y: i32, z: i32) -> u8 {
        self.state.cutaway_flags(x, y, z)
    }

    /// `renderOneLevel`'s per-square test: rendered and not occluded.
    pub fn is_visible(&self, sq: &RenderSquare) -> bool {
        self.state.is_visible(sq.x, sq.y, sq.z)
    }

    /// `calculateObjectRenderLayer` with `shouldHideElevatedFloor`: raised
    /// floors of a collapsed building on the player's level are neither
    /// floors nor translucent floors, and (being floors) nothing else
    /// either.
    pub fn object_layer(&self, sq: &RenderSquare, obj: &RenderObject) -> Layer {
        let layer = object_layer(sq, obj);
        if matches!(layer, Layer::Floor | Layer::TranslucentFloor) && self.state.should_hide_elevated_floor(sq, obj) {
            return Layer::None;
        }
        layer
    }

    /// `calculateObjectRenderInfo`'s layer: `MinusFloor(SE)` objects that
    /// fade become `Translucent(SE)`.
    pub fn render_layer(&self, sq: &RenderSquare, obj: &RenderObject) -> Layer {
        let layer = self.object_layer(sq, obj);
        if self.target_alpha(sq, obj, layer) < 1.0 {
            return match layer {
                Layer::MinusFloor => Layer::Translucent,
                Layer::MinusFloorSE => Layer::TranslucentSE,
                l => l,
            };
        }
        layer
    }

    /// `calculateObjectTargetAlpha`.
    pub fn target_alpha(&self, sq: &RenderSquare, obj: &RenderObject, layer: Layer) -> f32 {
        if !matches!(layer, Layer::MinusFloor | Layer::MinusFloorSE | Layer::Translucent | Layer::TranslucentSE) {
            return 1.0;
        }
        if matches!(obj.kind, ObjectKind::Door { open: true }) && !obj.flags().garage_door {
            return 0.6;
        }
        let t = obj.tile_type();
        let cut_w = obj.sprite.as_ref().is_some_and(|s| s.cut_w);
        let cut_n = obj.sprite.as_ref().is_some_and(|s| s.cut_n);
        if is_door(t, WallSide::W) || cut_w || is_door(t, WallSide::N) || cut_n {
            self.target_alpha_door_or_wall(sq, obj)
        } else {
            self.target_alpha_not_door_or_wall(sq, obj)
        }
    }

    /// `shouldHideFascia`.
    fn should_hide_fascia(&self, obj: &RenderObject) -> bool {
        obj.fascia_attached.is_some_and(|[x, y, z]| !self.state.should_render_square(x, y, z))
    }

    /// `calculateObjectTargetAlpha_DoorOrWall`: cut-away doors and windows
    /// in a wall fade to 0.4.
    pub fn target_alpha_door_or_wall(&self, sq: &RenderSquare, obj: &RenderObject) -> f32 {
        let Some(s) = obj.sprite.as_ref() else { return 1.0 };
        if s.cut_w && s.cut_n {
            return 1.0;
        }
        if self.should_hide_fascia(obj) {
            return 0.0;
        }
        let t = s.tile_type;
        let cutaway_self = self.flags_at(sq.x, sq.y, sq.z);
        let side = if is_door(t, WallSide::W) || s.cut_w {
            WallSide::W
        } else if is_door(t, WallSide::N) || s.cut_n {
            WallSide::N
        } else {
            return 1.0;
        };
        let bit = if side == WallSide::W { CUTAWAY_W } else { CUTAWAY_N };
        let door = is_door(t, side);
        let window = matches!(obj.kind, ObjectKind::Window { .. });
        if (door || window) && cutaway_self & bit != 0 {
            // hasSeenDoorW/N and hasSeenWindowW/N only look for the door
            // or window on the square.
            if door && !square_has_door(sq, side) {
                return 0.0;
            }
            if window && !square_has_window(sq, side) {
                return 0.0;
            }
            return 0.4;
        }
        1.0
    }

    /// `calculateObjectTargetAlpha_NotDoorOrWall`.
    pub fn target_alpha_not_door_or_wall(&self, sq: &RenderSquare, obj: &RenderObject) -> f32 {
        let st = self.state;
        let cutaway_self = self.flags_at(sq.x, sq.y, sq.z);
        let (cut_north, cut_west) = (cutaway_self & CUTAWAY_N != 0, cutaway_self & CUTAWAY_W != 0);
        if let ObjectKind::Window { north } = obj.kind {
            let (ox, oy) = if north { (sq.x, sq.y - 1) } else { (sq.x - 1, sq.y) };
            return self.window_target_alpha(sq, obj, self.world.square(ox, oy, sq.z), north);
        }
        let t = obj.tile_type();
        let roof = matches!(t, TileType::WestRoofB | TileType::WestRoofM | TileType::WestRoofT);
        if roof && st.player_z == sq.z && sq.cut.building.is_none() && st.can_building_square_occlude_player(sq.x, sq.y) {
            return 0.05;
        }
        if self.should_hide_fascia(obj) {
            return 0.0;
        }
        if !(st.player_square && st.player_room == sq.cut.room) {
            let (has_n, has_w) = square_cut(sq);
            let cutaway = if has_n && has_w {
                cut_north || cut_west
            } else if has_w {
                cut_west
            } else if has_n {
                cut_north
            } else {
                false
            };
            if cutaway {
                return if sq.cut.can_see { 0.25 } else { 0.0 };
            }
        }
        // Table tops and other objects obscuring the player
        // (isSquareObscuringPlayer) are not ported.
        1.0
    }

    /// `calculateWindowTargetAlpha`: a cut-away window fades with the
    /// player's distance, from 0.75 next to it down to 0.1 five squares
    /// away. `obj.alpha` stands in for the object's previous target alpha.
    pub fn window_target_alpha(&self, sq: &RenderSquare, obj: &RenderObject, opposite: Option<&RenderSquare>, north: bool) -> f32 {
        let st = self.state;
        let cutaway_self = self.flags_at(sq.x, sq.y, sq.z);
        let (cut_north, cut_west) = (cutaway_self & CUTAWAY_N != 0, cutaway_self & CUTAWAY_W != 0);
        let mut target = 1.0f32;
        if let Some(o) = opposite {
            if obj.alpha < 1.0e-4 && !(o.x == sq.x && o.y == sq.y && o.z == sq.z) && o.cut.seen {
                target = o.cut.dark_multi * 2.0;
            }
        }
        if target > 0.75 && ((cut_north && north) || (cut_west && !north)) {
            let dx = st.player_x - (sq.x as f32 + 0.5);
            let dy = st.player_y - (sq.y as f32 + 0.5);
            let d2 = (dx * dx + dy * dy).min(25.0);
            let fade = 0.1 + (0.75 - 0.1) * (1.0 - d2 / 25.0);
            target = fade.max(0.1);
        }
        target
    }

    /// `renderMinusFloor_DoorOrWall`'s cutaway inputs for `obj` on `sq`.
    pub fn wall_cutaway(&self, sq: &RenderSquare, obj: &RenderObject) -> WallCutaway {
        let (x, y, z) = (sq.x, sq.y, sq.z);
        let cutaway_self = self.flags_at(x, y, z);
        let mut c = WallCutaway {
            cutaway_self,
            cutaway_n: self.flags_at(x, y - 1, z),
            cutaway_s: self.flags_at(x, y + 1, z),
            cutaway_w: self.flags_at(x - 1, y, z),
            cutaway_e: self.flags_at(x + 1, y, z),
            circle_stencil: !obj.flags().never_cutaway,
            ..Default::default()
        };
        let could_see = sq.cut.could_see;
        let saw = |dx: i32, dy: i32| could_see || self.world.square(x + dx, y + dy, z).is_some_and(|s| s.cut.could_see);
        let (mut lowest_n, mut lowest_w) = (false, false);
        for o in &sq.objects {
            let t = o.tile_type();
            if !lowest_n && is_window_or_frame(o, true) && cutaway_self & CUTAWAY_N != 0 {
                c.has_seen_window_n = saw(0, -1);
                lowest_n = true;
            }
            if !lowest_w && is_window_or_frame(o, false) && cutaway_self & CUTAWAY_W != 0 {
                c.has_seen_window_w = saw(-1, 0);
                lowest_w = true;
            }
            if !lowest_n && o.sprite.is_some() && (is_door(t, WallSide::N) || o.flags().door_wall_n) && cutaway_self & CUTAWAY_N != 0 {
                c.has_seen_door_n = saw(0, -1);
                lowest_n = true;
            }
            if !lowest_w && o.sprite.is_some() && (is_door(t, WallSide::W) || o.flags().door_wall_w) && cutaway_self & CUTAWAY_W != 0 {
                c.has_seen_door_w = saw(-1, 0);
                lowest_w = true;
            }
        }
        c
    }

    /// `performDrawWallSegmentSingle`'s choice for a north or west wall
    /// (`DoWallLightingN/W`), with `calculateWallAlphaAndCircleStencilEdge`.
    pub fn wall_draw(&self, sq: &RenderSquare, obj: &RenderObject, side: WallSide, c: &WallCutaway) -> WallDraw {
        let Some(s) = obj.sprite.as_ref() else { return WallDraw::Normal { no_wall_lighting: false } };
        let bit = if side == WallSide::N { CUTAWAY_N } else { CUTAWAY_W };
        let is_cutaway = c.cutaway_self & bit != 0;
        let mut circle = c.circle_stencil && sq.z == self.state.player_z;
        let door = is_door(s.tile_type, side);
        let window = matches!(obj.kind, ObjectKind::Window { .. });
        let no_wall_lighting = ((door || window) && is_cutaway) || s.flags.no_wall_lighting;
        if (door || window) && !s.flags.garage_door {
            circle = false;
        }
        // calculateWallAlphaCommon's alpha comes from the target alpha here,
        // and the 0.3 light floor it gives cut doors is overwritten by
        // performDrawWall resetting lightInfoTemp.
        let transparent = if side == WallSide::N { s.flags.transparent_n } else { s.flags.transparent_w };
        let window_flag = if side == WallSide::N { s.flags.window_n } else { s.flags.window_w };
        let burnt = s.tile_type == TileType::Wall && transparent && s.tileset == "walls_burnt_01";
        let keep = !circle || burnt || s.tile_type != TileType::Wall || !transparent || s.flags.exterior || window_flag;
        if !keep {
            circle = false;
        }
        if circle && is_cutaway {
            return WallDraw::Cutaway;
        }
        WallDraw::Normal { no_wall_lighting }
    }
}

/// The render layer of `obj` with or without cutaways.
pub fn render_layer(cv: Option<&CutawayView<'_>>, sq: &RenderSquare, obj: &RenderObject) -> Layer {
    match cv {
        Some(cv) => cv.render_layer(sq, obj),
        None => crate::cell::render_layer(sq, obj),
    }
}

/// The target alpha of `obj` with or without cutaways.
pub fn target_alpha(cv: Option<&CutawayView<'_>>, sq: &RenderSquare, obj: &RenderObject) -> f32 {
    match cv {
        Some(cv) => cv.target_alpha(sq, obj, cv.object_layer(sq, obj)),
        None => crate::cell::target_alpha(obj, object_layer(sq, obj)),
    }
}

/// Whether `obj` on a force-rendered square (`isForceRenderSquare`) is
/// skipped by `renderMinusFloor`: everything but multi-level objects.
pub fn skip_on_force_render_square(cv: Option<&CutawayView<'_>>, sq: &RenderSquare, obj: &RenderObject) -> bool {
    let Some(cv) = cv else { return false };
    cv.state.is_force_render_square(sq.x, sq.y, sq.z) && obj.sprite.as_ref().is_none_or(|s| s.grid_levels == 1 || s.grid_levels == 0)
}
