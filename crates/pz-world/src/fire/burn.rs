//! What fire does to a square: `IsoGridSquare.Burn()`, `BurnWalls`
//! (burnt sprites from `IsoSprite.burntTile` or the burnt tilesets,
//! destroyed doors, windows, curtains, barricades, radios, generators and
//! attached objects, emptied containers, exploding fuel), `BurnTick`
//! (`IsoDeadBody.Burn`) and `IsoObjectUtils.safelyBurnTileObjectFromSquare`
//! for sprite-grid objects.
//!
//! Not ported: sheet ropes and window sheets (`removeSheetRope`,
//! `IsoWindow.removeSheet`), double and garage doors and entity sprite
//! configs as multi-square objects (sprite grids are), sound emitters on
//! burnt objects.

use crate::fire::manager::{FireWorld, IsoFireManager};
use crate::java::GameRand;
use crate::objects::{IsoObject, ObjectKind};
use crate::square::IsoDirections;
use crate::world_state::{barricade_able_north, get_barricade_on_square};

/// `IsoGridSquare.FIRE_IMMUNE_THRESHOLD`.
pub const FIRE_IMMUNE_THRESHOLD: i32 = 800_000;
/// `IsoGridSquare.FLOORS_BURNT_SPRITE_PREFIX`.
pub const FLOORS_BURNT_SPRITE_PREFIX: &str = "floors_burnt_01_";

type At = (i32, i32, i32);

/// `IsoGridSquare.Burn()`: `BurnWalls(true, true)` and `OnGridBurnt`.
pub fn burn(mgr: &mut IsoFireManager, w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At) {
    if w.sq(at.0, at.1, at.2).is_none() {
        return;
    }
    burn_walls(mgr, w, rand, at, true, true);
    w.hooks.lua_event("OnGridBurnt", at.0, at.1, at.2);
}

/// `IsoGridSquare.Burn(explode)`.
pub fn burn_explode(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    at: At,
    explode: bool,
) {
    if w.sq(at.0, at.1, at.2).is_none() {
        return;
    }
    burn_walls(mgr, w, rand, at, explode, true);
}

/// `IsoGridSquare.RemoveTileObject(obj, false)` of the object at `index`
/// (not a world inventory object): out of the square, then
/// `RecalcAllWithNeighbours(true)`.
pub fn remove_tile_object(w: &mut FireWorld<'_>, at: At, index: usize) -> Option<IsoObject> {
    let (x, y, z) = at;
    w.hooks.lua_event("OnObjectAboutToBeRemoved", x, y, z);
    w.hooks.invalidate_render_chunk_level(x, y, z, 128);
    let sq = w.sq_mut(x, y, z)?;
    let obj = sq.objects.remove(index);
    sq.special_objects.retain(|&id| id != obj.id);
    sq.world_objects.retain(|&id| id != obj.id);
    w.hooks.object_removed(x, y, z, &obj);
    w.cell.recalc_all_with_neighbours(x, y, z, w.meta, w.sprites);
    w.hooks.square_changed(x, y, z);
    w.hooks.lua_event("OnTileRemoved", x, y, z);
    Some(obj)
}

/// `IsoCell.isBasementWallAdjacentToTheVoid_North/West(obj)`.
fn basement_wall_next_to_void(w: &FireWorld<'_>, at: At, obj: &IsoObject) -> bool {
    let Some(s) = obj.sprite else { return false };
    let p = &w.sprites.sprites[s].properties;
    let has = |f: Option<crate::sprite::IsoFlagType>| f.is_some_and(|f| p.has(f));
    let nw = has(w.flags.WallNW);
    let floorless = |x: i32, y: i32| {
        w.sq(x, y, at.2)
            .is_none_or(|sq| sq.floor_index(w.sprites).is_none())
    };
    if at.2 >= 0 {
        return false;
    }
    let Some(sq) = w.sq(at.0, at.1, at.2) else {
        return false;
    };
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

/// The other parts of a sprite-grid object at `index` on `at`
/// (`IsoObjectUtils.getAllMultiTileObjects` for sprite grids): `None`
/// when the object is not multi-square or a part is missing.
fn sprite_grid_parts(w: &FireWorld<'_>, at: At, obj: &IsoObject) -> Option<Vec<At>> {
    let s = obj.sprite?;
    let g = &w.sprites.grids[w.sprites.sprites[s].sprite_grid?];
    let (ox, oy, oz) = g.sprite_pos(s)?;
    let mut out = Vec::new();
    for z in 0..g.levels {
        for x in 0..g.width {
            for y in 0..g.height {
                let p = (at.0 + x - ox, at.1 + y - oy, at.2 + z - oz);
                let test = g.get_sprite(x, y, z);
                let found = w.sq(p.0, p.1, p.2).is_some_and(|sq| {
                    sq.objects
                        .iter()
                        .any(|o| o.sprite.is_some() && o.sprite == test)
                });
                if !found {
                    return None;
                }
                out.push(p);
            }
        }
    }
    Some(out)
}

/// `IsoObjectUtils.isObjectMultiSquare(obj)` for sprite grids.
fn is_multi_square(w: &FireWorld<'_>, obj: &IsoObject) -> bool {
    obj.sprite
        .is_some_and(|s| w.sprites.sprites[s].sprite_grid.is_some())
}

/// The suffix after the last `_` of a sprite name.
fn suffix(name: &str) -> &str {
    &name[name.rfind('_').map_or(0, |i| i + 1)..]
}

/// What `BurnWalls` does to one object.
enum Outcome {
    /// `continue` without touching it.
    Skip,
    /// Replaced by a plain object with sprite `s` (`IsoObject.getNew`).
    ReplaceWith(Option<usize>),
    /// Taken off the square with `objects.remove` (`--n`).
    Drop,
    /// `RemoveTileObject` (`--n`); `true` counts as `removedTileObject`.
    RemoveTile(bool),
    /// Kept, with the end-of-loop treatment (`replaceIt` or
    /// `RemoveAttachedAnims` + no overlay).
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
) {
    let (x, y, z) = at;
    let Some(sq) = w.sq(x, y, z) else { return };
    // Barricades on the doors, windows and thumpables here and opposite.
    let mut removals: Vec<(At, u32)> = Vec::new();
    for &id in &sq.special_objects {
        let Some(i) = sq.object_index(id) else {
            continue;
        };
        let Some(north) = barricade_able_north(&sq.objects[i]) else {
            continue;
        };
        let (same, opp, op) = if north {
            (IsoDirections::N, IsoDirections::S, (x, y - 1, z))
        } else {
            (IsoDirections::W, IsoDirections::E, (x - 1, y, z))
        };
        let b1 = get_barricade_on_square(Some(sq), same).map(|_| ());
        if b1.is_some() {
            if let Some(bid) = barricade_id(sq, same) {
                removals.push((at, bid));
            }
        }
        let osq = w.sq(op.0, op.1, op.2);
        if get_barricade_on_square(osq, opp).is_some() {
            if let Some(bid) = osq.and_then(|o| barricade_id(o, opp)) {
                removals.push((op, bid));
            }
        }
    }
    for (p, id) in removals {
        if let Some(i) = w.sq(p.0, p.1, p.2).and_then(|s| s.object_index(id)) {
            remove_tile_object(w, p, i);
        }
    }

    let burnt_out = w.sprites.tables.flag.burntOut;
    let mut removed_tile_object = false;
    let burnt_already = w.sq(x, y, z).is_some_and(|sq| sq.has(burnt_out));
    if !burnt_already {
        if recursive {
            let mut n = 0;
            // IsoObject.handleBurning does nothing; hutches and feeding
            // troughs override it.
            while let Some(o) = w.sq(x, y, z).and_then(|s| s.objects.get(n)) {
                if matches!(o.kind, ObjectKind::Hutch(_) | ObjectKind::FeedingTrough(_)) {
                    let o = o.clone();
                    for p in w.hooks.handle_burning(x, y, z, &o) {
                        burn_walls(mgr, w, rand, p, true, false);
                    }
                }
                n += 1;
            }
        }
        let mut power = 0;
        let mut n: isize = 0;
        while let Some(obj) = w
            .sq(x, y, z)
            .and_then(|s| s.objects.get(n as usize))
            .cloned()
        {
            if recursive {
                // safelyBurnTileObjectFromSquare.
                if is_multi_square(w, &obj) {
                    for p in sprite_grid_parts(w, at, &obj).unwrap_or_default() {
                        if p != at {
                            burn_walls(mgr, w, rand, p, true, false);
                        }
                    }
                }
            }
            let outcome = burn_object(w, rand, at, &obj, &mut power);
            // The object may have moved if a recursive burn changed this
            // square; find it by identity.
            let idx = w.sq(x, y, z).and_then(|s| s.object_index(obj.id));
            match (outcome, idx) {
                (_, None) | (Outcome::Skip, _) => {}
                (Outcome::ReplaceWith(s), Some(i)) => replace(w, at, i, s),
                (Outcome::Drop, Some(i)) => {
                    if let Some(sq) = w.sq_mut(x, y, z) {
                        let o = sq.objects.remove(i);
                        w.hooks.object_removed(x, y, z, &o);
                    }
                    n -= 1;
                }
                (Outcome::RemoveTile(counts), Some(i)) => {
                    remove_tile_object(w, at, i);
                    removed_tile_object |= counts;
                    n -= 1;
                }
                (Outcome::Keep { replace: r }, Some(i)) => {
                    let thump = matches!(obj.kind, ObjectKind::Thumpable(_));
                    if r || thump {
                        let s = w.sq(x, y, z).and_then(|s| s.objects[i].sprite);
                        replace(w, at, i, s);
                    } else if let Some(sq) = w.sq_mut(x, y, z) {
                        let o = &mut sq.objects[i];
                        o.attached_anim_sprite = None;
                        o.overlay_sprite = None;
                    }
                }
            }
            n += 1;
        }
        if power > 0 && explode {
            w.hooks
                .play_world_sound("BurnedObjectExploded", x, y, z, 50.0, 1.0);
            mgr.explode(w, rand, at, power);
        }
    }
    let m = &*w.sprites;
    if let Some(sq) = w.cell.get_grid_square_mut(x, y, z) {
        let live: Vec<u32> = sq.objects.iter().map(|o| o.id).collect();
        sq.special_objects.retain(|id| live.contains(id));
        if !removed_tile_object {
            sq.recalc_properties(m);
        }
        sq.properties.set_flag(burnt_out);
        sq.burnt_out = true;
    }
    w.hooks.square_changed(x, y, z);
    w.hooks.invalidate_render_chunk_level(x, y, z, 384);
}

fn barricade_id(sq: &crate::IsoGridSquare, dir: IsoDirections) -> Option<u32> {
    sq.special_objects.iter().copied().find(|&id| {
        sq.object_index(id).is_some_and(|i| {
            matches!(&sq.objects[i].kind, ObjectKind::Barricade(b) if b.forward_dir() == dir)
        })
    })
}

/// `objects.set(n, IsoObject.getNew())` with sprite `s`, after
/// `obj.removeFromWorld()`.
fn replace(w: &mut FireWorld<'_>, at: At, i: usize, s: Option<usize>) {
    let Some(sq) = w.sq_mut(at.0, at.1, at.2) else {
        return;
    };
    let id = sq.new_object_id();
    let old = std::mem::replace(&mut sq.objects[i], IsoObject::new(id, ObjectKind::Object, s));
    sq.special_objects.retain(|&x| x != old.id);
    w.hooks.object_removed(at.0, at.1, at.2, &old);
}

/// The middle of `BurnWalls`' object loop for one object: its new sprite
/// (set on the square's object) and what happens to it.
fn burn_object(
    w: &mut FireWorld<'_>,
    rand: &mut dyn GameRand,
    at: At,
    obj: &IsoObject,
    power: &mut i32,
) -> Outcome {
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
    let has_opt = |fl: Option<crate::sprite::IsoFlagType>| fl.is_some_and(|fl| p.has(fl));
    let has_name = |n: &str| p.has_name(&w.sprites.alias, n);
    if has(f.water)
        || name.contains("_burnt_")
        || sp.firerequirement >= FIRE_IMMUNE_THRESHOLD
        || basement_wall_next_to_void(w, at, obj)
    {
        return Outcome::Skip;
    }
    let burnt_tile = sp.burnt_tile.clone();
    let tile_type = sp.tile_type;
    let thump = matches!(obj.kind, ObjectKind::Thumpable(_));
    if thump {
        if let Some(bt) = &burnt_tile {
            let ns = w.sprite_named(bt);
            return Outcome::ReplaceWith(Some(ns));
        }
    }
    let set_sprite = |w: &mut FireWorld<'_>, name: &str| {
        let ns = w.sprite_named(name);
        if let Some(i) = w.sq(at.0, at.1, at.2).and_then(|s| s.object_index(obj.id)) {
            if let Some(sq) = w.sq_mut(at.0, at.1, at.2) {
                sq.objects[i].sprite = Some(ns);
            }
        }
    };
    if burnt_tile.is_some() || tile_type == ty.tree {
        let n = match &burnt_tile {
            Some(bt) => bt.clone(),
            None => format!("fencing_burnt_01_{}", rand.next_range(15, 19) + 1),
        };
        set_sprite(w, &n);
        if let Some(i) = w.sq(at.0, at.1, at.2).and_then(|s| s.object_index(obj.id)) {
            if let Some(sq) = w.sq_mut(at.0, at.1, at.2) {
                sq.objects[i].attached_anim_sprite = None;
                sq.objects[i].overlay_sprite = None;
            }
        }
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
                if let Some(i) = w.sq(at.0, at.1, at.2).and_then(|s| s.object_index(obj.id)) {
                    if let Some(sq) = w.sq_mut(at.0, at.1, at.2) {
                        if let ObjectKind::Generator(g) = &mut sq.objects[i].kind {
                            g.activated = false;
                        }
                    }
                }
                w.hooks.generator_switched_off(at.0, at.1, at.2);
            }
            return Outcome::RemoveTile(false);
        }
        _ => {}
    }
    if w
        .object_name(obj)
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
            if let Some(i) = w.sq(at.0, at.1, at.2).and_then(|s| s.object_index(obj.id)) {
                if let Some(sq) = w.sq_mut(at.0, at.1, at.2) {
                    sq.objects[i].overlay_sprite = None;
                }
            }
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
                let fuel = (matches!(it.kind, pz_inventory::item::ItemKind::Food(_)) && it.is_alcoholic())
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
            floor_burnt(w, rand);
            if let Some(i) = w.sq(at.0, at.1, at.2).and_then(|s| s.object_index(obj.id)) {
                if let Some(sq) = w.sq_mut(at.0, at.1, at.2) {
                    sq.objects[i].container = None;
                    sq.objects[i].secondary_containers.clear();
                }
            }
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
                if thump || is_multi_square(w, obj) {
                    floor_burnt(w, rand);
                }
            }
        }
    }
    Outcome::Keep {
        replace: replace_it,
    }
}

/// `IsoGridSquare.BurnTick()`: corpses on a burning square burn for ten
/// seconds, then leave burnt remains (`IsoDeadBody.Burn`).
pub fn burn_tick(w: &mut FireWorld<'_>, rand: &mut dyn GameRand, at: At, multiplied_seconds: f32) {
    let (x, y, z) = at;
    let burning = w.flags.burning;
    let mut i = 0;
    loop {
        let Some(sq) = w.sq_mut(x, y, z) else { return };
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
        let have_remains = {
            let sq = w.sq(x, y, z).unwrap();
            sq.objects
                .iter()
                .any(|o| w.object_name(o).as_deref() == Some("burnedCorpse"))
        };
        if !have_remains {
            let name = format!("{FLOORS_BURNT_SPRITE_PREFIX}{}", rand.next_range(1, 3));
            let s = w.sprite_named(&name);
            let sq = w.sq_mut(x, y, z).unwrap();
            let id = sq.new_object_id();
            let mut o = IsoObject::new(id, ObjectKind::Object, Some(s));
            o.name = Some(crate::objects::ObjectName::Str("burnedCorpse".into()));
            o.tile = Some(name.clone());
            o.sprite_name = Some(crate::objects::SpriteName::Str(name));
            sq.objects.push(o);
        }
        w.hooks.invalidate_render_chunk_level(x, y, z, 130);
        w.sq_mut(x, y, z).unwrap().static_moving.remove(i);
        w.hooks.corpse_burnt(x, y, z);
        w.hooks.lua_event("OnContainerUpdate", x, y, z);
    }
}
