//! What fire does to a square: `IsoGridSquare.Burn()`, `BurnWalls`
//! (sheet ropes and window sheets taken down, barricades removed, hutches
//! and feeding troughs burnt as a whole (`handleBurning`), burnt sprites
//! from `IsoSprite.burntTile` or the burnt tilesets, destroyed doors,
//! windows, curtains, radios, generators and attached objects, emptied
//! containers, exploding fuel),
//! `IsoObjectUtils.safelyBurnTileObjectFromSquare` for multi-square
//! objects, `addAshes` and `BurnTick` (`IsoDeadBody.Burn`).
//!
//! Removals go through the square mutations ([`MutationEnv`]):
//! `RemoveTileObject`, `transmitRemoveItemFromSquare`, `removeFromWorld`
//! for each class, `removeCorpse`, `RecalcProperties` and the collision,
//! pathfinding and render notifications.

use crate::fire::manager::{FireWorld, IsoFireManager};
use crate::java::GameRand;
use crate::object_slot::{IsoObjectRef, ObjectSlot};
use crate::objects::{IsoObject, ObjectId, ObjectKind, ObjectName, SpriteName};
use crate::sprite::IsoFlagType;
use crate::square::IsoDirections;
use crate::square_mutation::{ObjectRef, WorldCall};
use crate::world_state::get_barricade_on_square;

/// `IsoGridSquare.FIRE_IMMUNE_THRESHOLD`.
pub const FIRE_IMMUNE_THRESHOLD: i32 = 800_000;
/// `IsoGridSquare.FLOORS_BURNT_SPRITE_PREFIX`.
pub const FLOORS_BURNT_SPRITE_PREFIX: &str = "floors_burnt_01_";

type At = (i32, i32, i32);

fn oref(pos: At, id: ObjectId) -> ObjectRef {
    ObjectRef { pos, id }
}

/// The exception `IsoWindow.removeSheet` throws when the square inside a
/// window outside any room does not exist.
pub const REMOVE_SHEET_NPE: &str = "java.lang.NullPointerException: Cannot invoke \"zombie.iso.IsoGridSquare.getSpecialObjects()\" because \"sq\" is null";

/// `IsoGridSquare.Burn()`: `BurnWalls(true, true)` and `OnGridBurnt`.
/// `Err` is the exception the original throws partway (see
/// [`REMOVE_SHEET_NPE`]); what ran before it stays done.
pub fn burn(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    at: At,
) -> Result<(), String> {
    if w.sq(at.0, at.1, at.2).is_none() {
        return Ok(());
    }
    burn_walls(mgr, w, rand, at, true, true)?;
    w.hooks.lua_event("OnGridBurnt", at.0, at.1, at.2);
    Ok(())
}

/// `IsoGridSquare.Burn(explode)`.
pub fn burn_explode(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    at: At,
    explode: bool,
) -> Result<(), String> {
    if w.sq(at.0, at.1, at.2).is_none() {
        return Ok(());
    }
    burn_walls(mgr, w, rand, at, explode, true)
}

/// `IsoGridSquare.RemoveTileObject(obj)` (the chunk is loaded, so the
/// removal is the safe one).
fn remove_tile_object(w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At, id: ObjectId) {
    w.env(rand).remove_tile_object(at, id);
}

/// `transmitRemoveItemFromSquare(obj)` in single player.
fn transmit_remove(w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At, id: ObjectId) {
    w.env(rand).transmit_remove_item_from_square(at, id, true);
}

fn sq_has(w: &FireWorld<'_>, at: At, f: Option<IsoFlagType>) -> bool {
    f.is_some_and(|f| w.sq(at.0, at.1, at.2).is_some_and(|sq| sq.borrow().has(f)))
}

/// `IsoWindow.isTopOfSheetRopeHere(sq, north)`.
fn is_top_of_sheet_rope_here(w: &FireWorld<'_>, at: At, north: bool) -> bool {
    if w.sq(at.0, at.1, at.2).is_none() {
        return false;
    }
    let [tn, ts, tw, te] = w.flags.climbSheetTop;
    if north {
        sq_has(w, at, tn) || sq_has(w, (at.0, at.1 - 1, at.2), ts)
    } else {
        sq_has(w, at, tw) || sq_has(w, (at.0 - 1, at.1, at.2), te)
    }
}

/// `IsoWindow.removeSheetRope(null, square, north)`: the rope tiles from
/// the top down, and the hook tile of a rope hanging from the next square.
fn remove_sheet_rope(w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At, north: bool) {
    let Some(sq_owner) = w.sq_mut(at.0, at.1, at.2) else {
        return;
    };
    let mut sq = sq_owner.borrow_mut();
    sq.have_sheet_rope = false;
    drop(sq);
    let [tn, ts, tw, te] = w.flags.climbSheetTop;
    let [cn, cs, cw, ce] = w.flags.climbSheet;
    let (own_top, own_rope, adj, adj_top, adj_rope, tile) = if north {
        (tn, cn, (at.0, at.1 - 1, at.2), ts, cs, "crafted_01_4")
    } else {
        (tw, cw, (at.0 - 1, at.1, at.2), te, ce, "crafted_01_3")
    };
    let (t1, t2, mut cur) = if sq_has(w, at, own_top) {
        (own_top, own_rope, Some(at))
    } else if sq_has(w, adj, adj_top) {
        let hook = w.sq(at.0, at.1, at.2).and_then(|sq| {
            sq.borrow()
                .objects
                .iter()
                .map(ObjectSlot::borrow)
                .find(|o| o.sprite.and_then(|s| w.sprites.sprites[s].name.as_deref()) == Some(tile))
                .map(|o| o.id)
        });
        if let Some(id) = hook {
            transmit_remove(w, rand, at, id);
        }
        (adj_top, adj_rope, Some(adj))
    } else {
        return;
    };
    while let Some(p) = cur {
        let Some(sq_owner) = w.sq(p.0, p.1, p.2) else {
            return;
        };
        let sq = sq_owner.borrow();
        let has = |o: &IsoObject, f: Option<IsoFlagType>| f.is_some_and(|f| obj_has(w, o, f));
        let rope = sq
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .find(|o| has(o, t1) || has(o, t2))
            .map(|o| o.id);
        let Some(id) = rope else { return };
        drop(sq);
        transmit_remove(w, rand, p, id);
        let min_level = w
            .cell
            .chunks
            .get(&(p.0.div_euclid(8), p.1.div_euclid(8)))
            .map_or(0, |c| c.min_level);
        if p.2 == min_level {
            return;
        }
        cur = w.sq(p.0, p.1, p.2 - 1).map(|_| (p.0, p.1, p.2 - 1));
    }
}

/// `IsoWindow.removeSheet(null)`: the first curtain on the inside square
/// (the window's own square in a room, else the one north or west, which
/// the original reads without a null check).
fn remove_window_sheet(
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    at: At,
    north: bool,
) -> Result<(), String> {
    let Some(sq_owner) = w.sq(at.0, at.1, at.2) else {
        return Ok(());
    };
    let sq = sq_owner.borrow();
    let s = if sq.room_id != -1 {
        at
    } else if north {
        (at.0, at.1 - 1, at.2)
    } else {
        (at.0 - 1, at.1, at.2)
    };
    drop(sq);
    let Some(sq_owner) = w.sq(s.0, s.1, s.2) else {
        return Err(REMOVE_SHEET_NPE.to_string());
    };
    let sq = sq_owner.borrow();
    let curtain = sq.special_objects.iter().copied().find(|&id| {
        sq.object_index(id)
            .is_some_and(|i| matches!(sq.objects[i].borrow().kind, ObjectKind::Curtain(_)))
    });
    drop(sq);
    if let Some(id) = curtain {
        transmit_remove(w, rand, s, id);
    }
    Ok(())
}

/// `IsoObject.getProperties().has(flag)` (the sprite's).
fn obj_has(w: &FireWorld<'_>, o: &IsoObject, f: IsoFlagType) -> bool {
    o.sprite
        .is_some_and(|s| w.sprites.sprites[s].properties.has(f))
}

/// `IsoThumpable.haveSheetRope()` (`!canAddSheetRope()`), with the
/// thumpable's `north`.
fn thumpable_have_sheet_rope(w: &FireWorld<'_>, at: At, o: &IsoObject) -> Option<(bool, bool)> {
    let ObjectKind::Thumpable(t) = &o.kind else {
        return None;
    };
    let f = &w.sprites.tables.flag;
    let props_hoppable = |s: Option<usize>| {
        s.is_some_and(|s| {
            let p = &w.sprites.sprites[s].properties;
            p.has(f.HoppableN) || p.has(f.HoppableW)
        })
    };
    // isHoppable().
    let hoppable = if t.is_door && !t.open && t.closed_sprite.is_some() {
        props_hoppable(t.closed_sprite)
    } else {
        props_hoppable(o.sprite) || t.is_hoppable
    };
    // isWindow().
    let window = o.sprite.is_some_and(|s| {
        let p = &w.sprites.sprites[s].properties;
        p.has(f.WindowW) || p.has_name(&w.sprites.alias, "WindowN")
    });
    let can_add = if hoppable || window {
        let [tn, ts, tw, te] = w.flags.climbSheetTop;
        if t.north {
            !sq_has(w, at, tn) && !sq_has(w, at, ts)
        } else {
            !sq_has(w, at, tw) && !sq_has(w, at, te)
        }
    } else {
        false
    };
    Some((!can_add, t.north))
}

/// `getNorth()` of a `BarricadeAble` (`IsoDoor`, `IsoWindow`,
/// `IsoThumpable`, `IsoWindowFrame`).
fn barricade_able_north(o: &IsoObject) -> Option<bool> {
    match &o.kind {
        ObjectKind::Door(d) => Some(d.north),
        ObjectKind::Window(w) => Some(w.north),
        ObjectKind::Thumpable(t) => Some(t.north),
        ObjectKind::WindowFrame { north } => Some(*north),
        _ => None,
    }
}

/// `IsoBarricade.GetBarricadeOnSquare(sq, dir)`'s object.
fn barricade_id(sq: &crate::IsoGridSquare, dir: IsoDirections) -> Option<ObjectId> {
    get_barricade_on_square(Some(sq), dir)?;
    sq.special_objects.iter().copied().find(|&id| {
        sq.object_index(id).is_some_and(
            |i| matches!(&sq.objects[i].borrow().kind, ObjectKind::Barricade(b) if b.forward_dir() == dir),
        )
    })
}

/// The first loop of `BurnWalls`, over the special objects as they change:
/// sheet ropes and window sheets come down, then the barricades on both
/// sides of every barricade-able object go.
fn burn_specials(w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At) -> Result<(), String> {
    let (x, y, z) = at;
    let mut i = 0;
    while let Some(id) = w
        .sq(x, y, z)
        .and_then(|sq| sq.borrow().special_objects.get(i).copied())
    {
        let Some(obj) = w.sq(x, y, z).and_then(|sq| {
            sq.borrow()
                .object_index(id)
                .map(|j| sq.borrow().objects[j].borrow().clone())
        }) else {
            i += 1;
            continue;
        };
        if let Some((true, north)) = thumpable_have_sheet_rope(w, at, &obj) {
            remove_sheet_rope(w, rand, at, north);
        }
        if let ObjectKind::Window(win) = &obj.kind {
            if is_top_of_sheet_rope_here(w, at, win.north) {
                remove_sheet_rope(w, rand, at, win.north);
            }
            remove_window_sheet(w, rand, at, win.north)?;
        }
        if let ObjectKind::WindowFrame { north } = obj.kind {
            if is_top_of_sheet_rope_here(w, at, north) {
                remove_sheet_rope(w, rand, at, north);
            }
        }
        if let Some(north) = barricade_able_north(&obj) {
            let (same, opp, op) = if north {
                (IsoDirections::N, IsoDirections::S, (x, y - 1, z))
            } else {
                (IsoDirections::W, IsoDirections::E, (x - 1, y, z))
            };
            let b1 = w
                .sq(x, y, z)
                .and_then(|sq| barricade_id(&(sq).borrow(), same));
            let b2 = w
                .sq(op.0, op.1, op.2)
                .and_then(|sq| barricade_id(&(sq).borrow(), opp));
            if let Some(b) = b1 {
                remove_tile_object(w, rand, at, b);
            }
            if let Some(b) = b2 {
                remove_tile_object(w, rand, op, b);
            }
        }
        i += 1;
    }
    Ok(())
}

/// `IsoGridSquare.addAshes()`: a burnt-floor object (`new IsoObject(sq,
/// sprite)`: a sprite of its own, with no name) on top of the square.
pub fn add_ashes(w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At) {
    if w.sq(at.0, at.1, at.2).is_none() {
        return;
    }
    let name = format!("{FLOORS_BURNT_SPRITE_PREFIX}{}", rand.next_range(1, 2));
    let s = w.sprites.create_sprite();
    let Some(sq_owner) = w.sq_mut(at.0, at.1, at.2) else {
        return;
    };
    let mut sq = sq_owner.borrow_mut();
    let id = sq.new_object_id();
    let mut o = IsoObject::new(id, ObjectKind::Object, Some(s));
    o.sprite_name = Some(SpriteName::Str(name));
    sq.objects.push((ObjectSlot::new(o)).into());
}

/// `getSpriteGridObjects(result, addSelf)` of `r`: the objects with the
/// same sprite grid on the grid's squares (z, then y, then x).
fn sprite_grid_objects(w: &FireWorld<'_>, r: ObjectRef, add_self: bool) -> Vec<ObjectRef> {
    let Some(obj) = w.sq(r.pos.0, r.pos.1, r.pos.2).and_then(|sq| {
        let sq = sq.borrow();
        sq.object_index(r.id).map(|i| sq.objects[i].ref_handle())
    }) else {
        return Vec::new();
    };
    let grid_of = |o: &IsoObject| o.sprite.and_then(|s| w.sprites.sprites[s].sprite_grid);
    let Some(gi) = grid_of(&obj.borrow()) else {
        return if add_self { vec![r] } else { Vec::new() };
    };
    let g = &w.sprites.grids[gi];
    let Some((gx, gy, gz)) = obj.borrow().sprite.and_then(|s| g.sprite_pos(s)) else {
        return Vec::new();
    };
    let (cx, cy, cz) = r.pos;
    let mut out = Vec::new();
    for z in cz - gz..cz - gz + g.levels {
        for y in cy - gy..cy - gy + g.height {
            for x in cx - gx..cx - gx + g.width {
                let Some(sq_owner) = w.sq(x, y, z) else {
                    continue;
                };
                let sq = sq_owner.borrow();
                for o in sq.objects.iter().map(ObjectSlot::borrow) {
                    let is_self = (x, y, z) == r.pos && o.id == r.id;
                    if (is_self && !add_self) || grid_of(&o) != Some(gi) {
                        continue;
                    }
                    out.push(oref((x, y, z), o.id));
                }
            }
        }
    }
    out
}

/// `IsoHutch.getHutch()`: the master hutch object of `r`.
fn hutch_master(w: &FireWorld<'_>, r: ObjectRef) -> Option<ObjectRef> {
    let sq_owner = w.sq(r.pos.0, r.pos.1, r.pos.2)?;
    let sq = sq_owner.borrow();
    let ObjectKind::Hutch(h) = &sq.objects[sq.object_index(r.id)?].borrow().kind else {
        return None;
    };
    if h.master.is_some() {
        return Some(r);
    }
    let p = (h.linked_x, h.linked_y, h.linked_z);
    let lsq_owner = w.sq(p.0, p.1, p.2)?;
    let lsq = lsq_owner.borrow();
    lsq.special_objects.iter().copied().find_map(|id| {
        let i = lsq.object_index(id)?;
        matches!(lsq.objects[i].borrow().kind, ObjectKind::Hutch(_)).then_some(oref(p, id))
    })
}

/// `IsoHutch.handleBurning()`: the other hutch squares lose their hutch
/// objects (`removeHutchObject`) and burn; every hutch square gets ashes.
fn hutch_handle_burning(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    r: ObjectRef,
) -> Result<(), String> {
    // A HashSet of squares in the original (identity hash order); kept in
    // insertion order here.
    let mut burn_squares = vec![r.pos];
    let master = hutch_master(w, r);
    // getAllHutchObjects(): getHutchTiles(this) of the 5x5 squares around.
    let mut hutches = Vec::new();
    for x in r.pos.0 - 2..r.pos.0 + 3 {
        for y in r.pos.1 - 2..r.pos.1 + 3 {
            let p = (x, y, r.pos.2);
            let Some(sq_owner) = w.sq(p.0, p.1, p.2) else {
                continue;
            };
            let sq = sq_owner.borrow();
            for &id in &sq.special_objects {
                let Some(i) = sq.object_index(id) else {
                    continue;
                };
                if matches!(sq.objects[i].borrow().kind, ObjectKind::Hutch(_))
                    && hutch_master(w, oref(p, id)) == master
                {
                    hutches.push(oref(p, id));
                }
            }
        }
    }
    for h in hutches {
        if burn_squares.contains(&h.pos) {
            continue;
        }
        burn_squares.push(h.pos);
        // removeHutchObject: releaseAllAnimals, dropAllEggs,
        // removeFromWorld, transmitRemoveItemFromSquare.
        w.hooks.hutch_release_contents(h);
        w.env(rand).object_remove_from_world(h);
        transmit_remove(w, rand, h.pos, h.id);
    }
    for p in burn_squares {
        add_ashes(w, rand, p);
        if p != r.pos {
            burn_walls(mgr, w, rand, p, true, false)?;
        }
    }
    Ok(())
}

/// `IsoFeedingTrough.handleBurning()`: every part of the trough leaves the
/// world and its squares, which get ashes and burn.
fn trough_handle_burning(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    r: ObjectRef,
) -> Result<(), String> {
    let mut burn_squares = vec![r.pos];
    for o in sprite_grid_objects(w, r, false) {
        let trough = w.sq(o.pos.0, o.pos.1, o.pos.2).is_some_and(|sq| {
            sq.borrow().object_index(o.id).is_some_and(|i| {
                matches!(
                    sq.borrow().objects[i].borrow().kind,
                    ObjectKind::FeedingTrough(_)
                )
            })
        });
        if trough {
            w.env(rand).object_remove_from_world(o);
            transmit_remove(w, rand, o.pos, o.id);
        }
        if !burn_squares.contains(&o.pos) {
            burn_squares.push(o.pos);
        }
    }
    w.env(rand).object_remove_from_world(r);
    w.env(rand).safely_remove_tile_object_from_square(r);
    for p in burn_squares {
        add_ashes(w, rand, p);
        burn_walls(mgr, w, rand, p, true, false)?;
    }
    Ok(())
}

/// `IsoObjectUtils.safelyBurnTileObjectFromSquare(obj)`: the other parts
/// of a multi-square object (double and garage doors, entity sprite
/// configs, sprite grids) burn with `BurnWalls(true, false)`.
fn safely_burn_tile_object(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    r: ObjectRef,
) -> Result<(), String> {
    if !w.env(rand).is_object_multi_square(r) {
        return Ok(());
    }
    let parts = w
        .env(rand)
        .get_all_multi_tile_objects(r)
        .unwrap_or_default();
    for p in parts {
        if p != r {
            burn_walls(mgr, w, rand, p.pos, true, false)?;
        }
    }
    Ok(())
}

/// The suffix after the last `_` of a sprite name.
fn suffix(name: &str) -> &str {
    &name[name.rfind('_').map_or(0, |i| i + 1)..]
}

/// What `BurnWalls` does with one object after its sprite changed.
enum Outcome {
    /// `continue` without the end-of-loop treatment.
    Skip,
    /// An `IsoThumpable` with a `burntTile`: `removeFromWorld`, then
    /// replaced by a plain object with that sprite (`continue`).
    ReplaceWith(usize),
    /// Taken off the square with `objects.remove` (`--n`).
    Drop,
    /// `RemoveTileObject` (`--n`); `true` counts as `removedTileObject`.
    RemoveTile(bool),
    /// Kept, with the end-of-loop treatment (`replaceIt` or a thumpable:
    /// replaced; otherwise `RemoveAttachedAnims` and no overlay), then its
    /// emitter stops.
    Keep { replace: bool },
}

/// `IsoGridSquare.BurnWalls(explode, recursive)`.
pub fn burn_walls(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    at: At,
    explode: bool,
    recursive: bool,
) -> Result<(), String> {
    let (x, y, z) = at;
    if w.sq(x, y, z).is_none() {
        return Ok(());
    }
    burn_specials(w, rand, at)?;

    let burnt_out = w.sprites.tables.flag.burntOut;
    let mut removed_tile_object = false;
    let burnt_already = w.sq(x, y, z).is_some_and(|sq| sq.borrow().has(burnt_out));
    if !burnt_already {
        if recursive {
            // handleBurning (only hutches and feeding troughs do anything).
            let mut n = 0;
            while let Some(o) = w
                .sq(x, y, z)
                .and_then(|s| s.borrow().objects.get(n).map(|o| o.ref_handle()))
            {
                let (r, kind) = {
                    let o = o.borrow();
                    (oref(at, o.id), o.kind.clone())
                };
                match kind {
                    ObjectKind::Hutch(_) => hutch_handle_burning(mgr, w, rand, r)?,
                    ObjectKind::FeedingTrough(_) => trough_handle_burning(mgr, w, rand, r)?,
                    _ => {}
                }
                n += 1;
            }
        }
        let mut power = 0;
        let mut n: isize = 0;
        while let Some(obj) = w.sq(x, y, z).and_then(|s| {
            s.borrow()
                .objects
                .get(n as usize)
                .map(ObjectSlot::ref_handle)
        }) {
            let id = obj.borrow().id;
            let r = oref(at, id);
            if recursive {
                safely_burn_tile_object(mgr, w, rand, r)?;
            }
            let detached = w
                .sq(x, y, z)
                .is_none_or(|s| !s.borrow().objects.iter().any(|s| s.ptr_eq_ref(&obj)));
            let void_wall = basement_wall_next_to_void(w, at, &obj.borrow());
            let outcome = burn_object(w, rand, at, &obj, void_wall, &mut power);
            // The removals are by object (`objects.remove(obj)`,
            // `RemoveTileObject(obj)`; for a detached object the latter
            // only resets the square's region flags, which nothing here
            // reads) and `--n` follows them regardless. The replacements are
            // `objects.set(n, ..)`: after a shift they replace whichever
            // object is at `n` now (or throw past the end), and `obj` keeps
            // its place if it is still there.
            let set_n = n as usize;
            match outcome {
                Outcome::Skip => {}
                Outcome::Drop => {
                    if let Some(sq) = w.sq_mut(x, y, z) {
                        let index = { sq.borrow().objects.iter().position(|s| s.ptr_eq_ref(&obj)) };
                        if let Some(i) = index {
                            sq.borrow_mut().remove_object_slot(i);
                        }
                    }
                    n -= 1;
                }
                Outcome::RemoveTile(counts) => {
                    if !detached {
                        remove_tile_object(w, rand, at, id);
                    }
                    removed_tile_object |= counts;
                    n -= 1;
                }
                Outcome::ReplaceWith(s) => {
                    remove_from_world(w, rand, r, detached, &obj);
                    set_at(w, at, set_n, Some(s))?;
                }
                Outcome::Keep { replace: rep } => {
                    if rep || matches!(obj.borrow().kind, ObjectKind::Thumpable(_)) {
                        let s = obj.borrow().sprite;
                        remove_from_world(w, rand, r, detached, &obj);
                        set_at(w, at, set_n, s)?;
                    } else {
                        let mut o = obj.borrow_mut();
                        o.attached_anim_sprite = None;
                        o.overlay_sprite = None;
                    }
                    w.env(rand).world_call_on(r, &obj, WorldCall::StopEmitter);
                }
            }
            n += 1;
        }
        if power > 0 && explode {
            w.hooks
                .play_world_sound("BurnedObjectExploded", x, y, z, 50.0, 1.0);
            mgr.explode(w, rand, at, power)?;
        }
    }
    if let Some(sq) = w.sq_mut(x, y, z) {
        let live: Vec<ObjectId> = sq
            .borrow()
            .objects
            .iter()
            .map(ObjectSlot::borrow)
            .map(|o| o.id)
            .collect();
        sq.borrow_mut()
            .special_objects
            .retain(|id| live.contains(id));
    }
    let mut env = w.env(rand);
    if !removed_tile_object {
        env.recalc_properties(at);
    }
    if let Some(sq_owner) = env.cell.get_grid_square_mut(x, y, z) {
        let mut sq = sq_owner.borrow_mut();
        sq.properties.set_flag(burnt_out);
        sq.burnt_out = true;
    }
    env.map_collision_square_changed(at);
    env.polygonal_map2_square_changed(at);
    env.invalidate_render(at, 384);
    Ok(())
}

/// `objects.set(n, IsoObject.getNew())` with sprite `s` (the replaced
/// object's `removeFromWorld` already ran, if it was the burning one);
/// `PZArrayList.set`'s `IndexOutOfBoundsException` past the end.
fn set_at(w: &mut FireWorld<'_>, at: At, n: usize, s: Option<usize>) -> Result<(), String> {
    let Some(sq_owner) = w.sq_mut(at.0, at.1, at.2) else {
        return Ok(());
    };
    let mut sq = sq_owner.borrow_mut();
    if n >= sq.objects.len() {
        return Err(format!(
            "java.lang.IndexOutOfBoundsException: Index: {n} Size: {}",
            sq.objects.len()
        ));
    }
    let nid = sq.new_object_id();
    sq.replace_object_slot(
        n,
        ObjectSlot::new(IsoObject::new(nid, ObjectKind::Object, s)),
    );
    Ok(())
}

/// `obj.removeFromWorld()`, with `obj` on the square or `detached`.
fn remove_from_world(
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    r: ObjectRef,
    detached: bool,
    obj: &IsoObjectRef,
) {
    let _ = detached;
    w.env(rand).object_remove_from_world_detached(r, obj);
}

/// The middle of `BurnWalls`' object loop for one object: its new sprite
/// (set on the square's object) and what happens to it. `void_wall` is
/// `isBasementWallAdjacentToTheVoid_North/West(obj)`.
fn burn_object(
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    at: At,
    receiver: &IsoObjectRef,
    void_wall: bool,
    power: &mut i32,
) -> Outcome {
    let obj = receiver.borrow().clone();
    let Some(s) = obj.sprite else {
        return Outcome::Skip;
    };
    let f = w.sprites.tables.flag;
    let ff = w.flags;
    let ty = w.sprites.tables.ty;
    let sp = &w.sprites.sprites[s];
    let Some(name) = sp.name.clone() else {
        return Outcome::Skip;
    };
    let p = sp.properties.clone();
    let has = |fl| p.has(fl);
    let has_opt = |fl: Option<IsoFlagType>| fl.is_some_and(|fl| p.has(fl));
    let has_name = |n: &str| p.has_name(&w.sprites.alias, n);
    if has(f.water)
        || name.contains("_burnt_")
        || sp.firerequirement >= FIRE_IMMUNE_THRESHOLD
        || void_wall
    {
        return Outcome::Skip;
    }
    let burnt_tile = sp.burnt_tile.clone();
    let tile_type = sp.tile_type;
    let thump = matches!(obj.kind, ObjectKind::Thumpable(_));
    if thump {
        if let Some(bt) = &burnt_tile {
            return Outcome::ReplaceWith(w.sprite_named(bt));
        }
    }
    let set_sprite = |w: &mut FireWorld<'_>, name: &str| {
        let ns = w.sprite_named(name);
        receiver.borrow_mut().sprite = Some(ns);
    };
    let clear_overlay = |_w: &mut FireWorld<'_>, anims: bool| {
        let mut o = receiver.borrow_mut();
        if anims {
            o.attached_anim_sprite = None;
        }
        o.overlay_sprite = None;
    };
    if burnt_tile.is_some() || tile_type == ty.tree {
        let n = match &burnt_tile {
            Some(bt) => bt.clone(),
            None => format!("fencing_burnt_01_{}", rand.next_range(15, 19) + 1),
        };
        set_sprite(w, &n);
        clear_overlay(w, true);
        return Outcome::Skip;
    }
    match &obj.kind {
        ObjectKind::Trap(_) => return Outcome::Skip,
        ObjectKind::Barricade(_) | ObjectKind::Mannequin(_) => return Outcome::Drop,
        ObjectKind::Generator(g) => {
            if g.fuel > 0.0 {
                *power += 20;
            }
            if g.activated {
                if let ObjectKind::Generator(g) = &mut receiver.borrow_mut().kind {
                    g.activated = false;
                }
                w.hooks.generator_switched_off(at.0, at.1, at.2);
            }
            return Outcome::RemoveTile(false);
        }
        _ => {}
    }
    if w.object_name(&obj)
        .is_some_and(|n| n.eq_ignore_ascii_case("Campfire"))
    {
        return Outcome::Skip;
    }
    let pick = |rand: &mut dyn GameRand, a: &str, b: &str| {
        if rand.next(2) == 0 {
            a.to_string()
        } else {
            b.to_string()
        }
    };
    let wall_like = tile_type == ty.wall
        && !has(ff.DoorWallW)
        && !has(ff.DoorWallN)
        && !has_name("WindowN")
        && !has(f.WindowW)
        && !name.starts_with("walls_exterior_roofs_")
        && !name.starts_with("fencing_")
        && !name.starts_with("fixtures_railings_");
    let mut replace_it = false;
    if wall_like {
        let (cw, cn) = (has(f.collideW), has(f.collideN));
        let n = if cw && !cn {
            Some(pick(rand, "0", "4"))
        } else if cn && !cw {
            Some(pick(rand, "1", "5"))
        } else if cw && cn {
            Some(pick(rand, "2", "6"))
        } else if has(f.WallSE) {
            Some(pick(rand, "3", "7"))
        } else {
            None
        };
        if let Some(n) = n {
            set_sprite(w, &format!("walls_burnt_01_{n}"));
        }
    } else {
        if matches!(
            obj.kind,
            ObjectKind::Door(_) | ObjectKind::Window(_) | ObjectKind::Curtain(_)
        ) {
            return Outcome::RemoveTile(true);
        }
        let floor_burnt = |w: &mut FireWorld<'_>, rand: &mut dyn GameRand| {
            let n = format!("{FLOORS_BURNT_SPRITE_PREFIX}{}", rand.next_range(1, 2));
            set_sprite(w, &n);
            clear_overlay(w, false);
        };
        if has(f.WindowW) {
            set_sprite(w, &format!("walls_burnt_01_{}", pick(rand, "8", "12")));
        } else if has_name("WindowN") {
            set_sprite(w, &format!("walls_burnt_01_{}", pick(rand, "9", "13")));
        } else if has(ff.DoorWallW) {
            set_sprite(w, &format!("walls_burnt_01_{}", pick(rand, "10", "14")));
        } else if has(ff.DoorWallN) {
            set_sprite(w, &format!("walls_burnt_01_{}", pick(rand, "11", "15")));
        } else if has(f.solidfloor) && !has(f.exterior) {
            set_sprite(w, "floors_burnt_01_0");
        } else if matches!(obj.kind, ObjectKind::Radio(_) | ObjectKind::Television(_)) {
            return Outcome::RemoveTile(true);
        } else if let Some(c) = &obj.container {
            for it in &c.items {
                let Some(it) = it.as_item() else { continue };
                let fuel = (matches!(it.kind, pz_inventory::item::ItemKind::Food(_))
                    && it.is_alcoholic())
                    || it.get_type() == "PetrolCan"
                    || it.get_type() == "Bleach";
                if !fuel {
                    continue;
                }
                *power += 20;
                if *power > 100 {
                    *power = 100;
                    break;
                }
            }
            let n = format!("{FLOORS_BURNT_SPRITE_PREFIX}{}", rand.next_range(1, 2));
            set_sprite(w, &n);
            // removeItemsFromProcessItems() and removeAllItems() of every
            // container, removeAllContainers(), no overlay.
            let r = oref(at, obj.id);
            let count = obj.container_count();
            for c in 0..count {
                w.env(rand).world_call_on(
                    r,
                    receiver,
                    WorldCall::ProcessItems {
                        container: c,
                        add: false,
                    },
                );
            }
            let retired = {
                let mut o = receiver.borrow_mut();
                let primary = o.container.take();
                let secondary = std::mem::take(&mut o.secondary_containers);
                o.overlay_sprite = None;
                (primary, secondary)
            };
            drop(retired);
            replace_it = true;
        } else if has(f.solidtrans) || has(ff.bed) || has(ff.waterPiped) {
            floor_burnt(w, rand);
        } else if name.starts_with("walls_exterior_roofs_") {
            set_sprite(w, &format!("walls_burnt_roofs_01_{}", suffix(&name)));
        } else if !name.starts_with("roofs_accents") {
            if name.starts_with("roofs_") {
                set_sprite(w, &format!("roofs_burnt_01_{}", suffix(&name)));
            } else if (name.starts_with("fencing_") || name.starts_with("fixtures_railings_"))
                && (has(f.HoppableN) || has(f.HoppableW))
            {
                let (tw, tn) = (has(f.transparentW), has(f.transparentN));
                let n = if tw && !tn {
                    "fencing_burnt_01_0"
                } else if tn && !tw {
                    "fencing_burnt_01_1"
                } else {
                    "fencing_burnt_01_2"
                };
                set_sprite(w, n);
            } else {
                if has(f.attachedN)
                    || has(f.attachedW)
                    || has_opt(ff.attachedS)
                    || has_opt(ff.attachedE)
                {
                    return Outcome::RemoveTile(true);
                }
                let multi = w.env(rand).is_object_multi_square(oref(at, obj.id));
                if thump || multi {
                    floor_burnt(w, rand);
                }
            }
        }
    }
    Outcome::Keep {
        replace: replace_it,
    }
}

/// `IsoCell.isBasementWallAdjacentToTheVoid_North/West(obj)`.
fn basement_wall_next_to_void(w: &FireWorld<'_>, at: At, obj: &IsoObject) -> bool {
    let has = |f: Option<IsoFlagType>| f.is_some_and(|f| obj_has(w, obj, f));
    let nw = has(w.flags.WallNW);
    let floorless = |x: i32, y: i32| {
        w.sq(x, y, at.2)
            .is_none_or(|sq| sq.borrow().floor_index(w.sprites).is_none())
    };
    if at.2 >= 0 {
        return false;
    }
    let Some(sq_owner) = w.sq(at.0, at.1, at.2) else {
        return false;
    };
    let sq = sq_owner.borrow();
    let own_floor = sq.floor_index(w.sprites).is_some();
    for (wall, (dx, dy)) in [(w.flags.WallN, (0, -1)), (w.flags.WallW, (-1, 0))] {
        if !has(wall) && !nw {
            continue;
        }
        if !own_floor || floorless(at.0 + dx, at.1 + dy) {
            return true;
        }
    }
    false
}

/// `IsoGridSquare.BurnTick()`: corpses on a burning square burn for ten
/// seconds, then leave burnt remains (`IsoDeadBody.Burn`, then
/// `removeCorpse(body, true)`).
pub fn burn_tick(w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At, multiplied_seconds: f32) {
    let (x, y, z) = at;
    let burning = w.flags.burning;
    let mut i = 0;
    loop {
        let Some(sq_owner) = w.sq_mut(x, y, z) else {
            return;
        };
        let mut sq = sq_owner.borrow_mut();
        if i >= sq.static_moving.len() {
            return;
        }
        if !sq.has(burning) {
            i += 1;
            continue;
        }
        let body = &mut sq.static_moving[i];
        body.burn_timer += multiplied_seconds;
        if body.burn_timer < 10.0 {
            i += 1;
            continue;
        }
        drop(sq);
        let have_remains = {
            let sq_owner = w.sq(x, y, z).unwrap();
            let sq = sq_owner.borrow();
            sq.objects
                .iter()
                .map(ObjectSlot::borrow)
                .any(|o| w.object_name(&o).as_deref() == Some("burnedCorpse"))
        };
        if !have_remains {
            // new IsoObject(square, tile, "burnedCorpse").
            let name = format!("{FLOORS_BURNT_SPRITE_PREFIX}{}", rand.next_range(1, 3));
            let s = w.sprite_named(&name);
            let sq_owner = w.sq_mut(x, y, z).unwrap();
            let mut sq = sq_owner.borrow_mut();
            let id = sq.new_object_id();
            let mut o = IsoObject::new(id, ObjectKind::Object, Some(s));
            o.name = Some(ObjectName::Str("burnedCorpse".into()));
            o.tile = Some(name.clone());
            o.sprite_name = Some(SpriteName::Str(name));
            sq.objects.push((ObjectSlot::new(o)).into());
        }
        w.env(rand).remove_corpse(at, i);
    }
}
