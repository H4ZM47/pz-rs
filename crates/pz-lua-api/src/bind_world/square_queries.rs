//! Square queries over the current world's actual stored properties.

use super::*;
use pz_world::WorldObjectEntry;

/// `square.lighting[playerIndex]` as the engine's renderer decoded it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SquareLighting {
    /// `getLightLevel(playerIndex)`: `max(r, g, b)` of `lightInfo()`
    /// (`getLightLevel2()` for -1).
    pub light_level: f32,
    /// Visibility bits: 1 seen, 2 canSee, 4 couldSee.
    pub vis: u8,
}

/// A square no lighting frame has decoded keeps its constructor's
/// `ColorInfo` (white) and no visibility bits.
const UNDECODED: SquareLighting = SquareLighting { light_level: 1.0, vis: 0 };

pub type SquareLightSource = Box<dyn Fn(i32, i32, i32, i32) -> Option<SquareLighting>>;

thread_local! {
    static SQUARE_LIGHT: RefCell<Option<Rc<SquareLightSource>>> = const { RefCell::new(None) };
}

/// Install (or clear) the engine's lighting for `getLightLevel`,
/// `isSeen`, `isCanSee` and `isCouldSee`.
pub fn install_square_light(source: Option<SquareLightSource>) {
    let retired = SQUARE_LIGHT.with(|s| s.replace(source.map(Rc::new)));
    drop(retired);
}

pub(crate) fn square_lighting(player: i32, x: i32, y: i32, z: i32) -> SquareLighting {
    let source = SQUARE_LIGHT.with(|s| s.borrow().clone());
    source.and_then(|f| f(player, x, y, z)).unwrap_or(UNDECODED)
}

/// `square.lighting[player]`'s visibility bits (1 seen, 2 canSee, 4
/// couldSee) for the character states' world; 0 for an undecoded square.
pub fn square_vis(player: i32, x: i32, y: i32, z: i32) -> u8 {
    square_lighting(player, x, y, z).vis
}

/// `lighting[playerIndex]`: Java throws for an index outside the array.
fn player_index(c: &NativeCall<'_>, i: usize, allow_minus_one: bool) -> LuaResult<i32> {
    let player = int_arg(c, i);
    if (0..4).contains(&player) || allow_minus_one && player == -1 {
        return Ok(player);
    }
    Err(LuaError::Runtime {
        message: format!("Index {player} out of bounds for length 4"),
        class: "java.lang.ArrayIndexOutOfBoundsException",
    })
}

/// Run `f` on the loaded square at `(x, y, z)` of the current cell.
fn with_square<R>(
    x: i32,
    y: i32,
    z: i32,
    f: impl FnOnce(&pz_world::IsoGridSquare) -> LuaResult<R>,
) -> LuaResult<R> {
    cell(|cell| {
        let square_owner = cell
            .get_grid_square(x, y, z)
            .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))?;
        let square = square_owner
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
        f(&square)
    })
}

pub fn square_object_value(owner: &pz_world::IsoObjectRef, (x, y, z): (i32, i32, i32)) -> LuaResult<Value> {
    let value = crate::bind_world_object::object_value(owner)?;
    let square = crate::bind_world::square_value(x, y, z);
    crate::bind_world_object::adopt_square(&value, owner, &square)?;
    crate::bind_world_object::remember_square(&value, [x, y, z]);
    Ok(value)
}

/// The Lua object of the moving object with `id`: a local player
/// (`IsoPlayer.players`).
fn moving_object_value(id: i32) -> LuaResult<Value> {
    for i in 0..4 {
        let Some(player) = crate::bind_players::player(i) else {
            continue;
        };
        let matches = player
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoPlayer is in use"))?
            .id
            == id;
        if matches {
            return crate::bind_character_sim::try_character_value(&player)
                .map_err(crate::bind_world_object::host_error);
        }
    }
    Err(crate::bind_world_object::unsupported(format!(
        "IsoGridSquare.getMovingObjects: moving object {id} is not a local player and has no Lua object"
    )))
}

/// `square.haveRoof` of the loaded square.
pub fn square_have_roof(x: i32, y: i32, z: i32) -> LuaResult<bool> {
    with_square(x, y, z, |square| Ok(square.have_roof))
}

/// `square.isOutside()`: the cached exterior flag of the loaded square.
pub fn square_is_outside(x: i32, y: i32, z: i32) -> LuaResult<bool> {
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites
        .try_borrow()
        .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
    // The original reads properties.has(exterior). Recalculation belongs
    // to square mutations, never to this cached-flag query.
    with_square(x, y, z, |square| Ok(square.is_exterior(&sprites)))
}

/// The square of `object` among `around` and its eight neighbours on the same
/// level, which are the only squares `canReachTo` can accept from `around`.
///
/// The original reads the `square` field of the object. The port records it
/// only for world items, so every other object is looked up by identity in the
/// object lists of those nine squares.
pub(crate) fn object_square_near(
    object: &pz_world::IsoObjectRef,
    around: (i32, i32, i32),
) -> LuaResult<Option<(i32, i32, i32)>> {
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (x, y, z) = (around.0 + dx, around.1 + dy, around.2);
            if !square_exists(x, y, z) {
                continue;
            }
            let found = with_square(x, y, z, |square| {
                Ok(square.objects.iter().any(|slot| Rc::ptr_eq(slot.owner_slot(), object)))
            })?;
            if found {
                return Ok(Some((x, y, z)));
            }
        }
    }
    Ok(None)
}

type EdgeQuery = fn(
    &pz_world::square::CollideEnv<'_>,
    &pz_world::IsoGridSquare,
    Option<&pz_world::IsoGridSquare>,
) -> bool;

/// A boolean query from this square to the square argument (or `nil`),
/// over the current cell, sprite tables and meta grid.
fn edge(b: &ClassBuilder, name: &str, f: EdgeQuery) {
    get(b, name, move |c| {
        let (x, y, z) = self_square(c)?;
        let other = match c.arg(1) {
            Value::Nil => None,
            _ => Some(square_arg(c, 1)?),
        };
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        with_world_state_cell(|w| {
            let w = w.try_borrow().map_err(|_| LuaError::runtime("IsoWorld is in use"))?;
            let cell = w.cell.as_ref().ok_or_else(npe)?;
            let meta = w
                .meta_grid
                .as_ref()
                .ok_or_else(|| LuaError::runtime("IsoMetaGrid is not loaded"))?;
            let env = pz_world::square::CollideEnv::new(cell, &sprites, meta);
            let loaded = |(x, y, z)| {
                cell.get_grid_square(x, y, z)
                    .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))
            };
            let a = loaded((x, y, z))?;
            let a = a.try_borrow().map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
            let other = other.map(loaded).transpose()?;
            let other = match &other {
                Some(o) => Some(o.try_borrow().map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?),
                None => None,
            };
            Ok(f(&env, &a, other.as_deref()).into())
        })
    });
}

/// An object of the square has an attached sprite whose name contains one
/// of `parts` (`haveGraffiti`, `haveGrime`).
fn have_overlay(x: i32, y: i32, z: i32, parts: &[&str]) -> LuaResult<bool> {
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites
        .try_borrow()
        .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
    with_square(x, y, z, |square| {
        Ok(square.objects.iter().any(|slot| {
            let object = slot.borrow();
            object.attached_anim_sprite.iter().flatten().any(|instance| {
                sprites.sprites[instance.sprite]
                    .name
                    .as_deref()
                    .is_some_and(|name| parts.iter().any(|part| name.contains(part)))
            })
        }))
    })
}

/// `haveBlood()`: with blood decals on, a wall splat on an object of the
/// square or a floor splat of the chunk within one square of it.
fn have_blood(t: &mut Thread, x: i32, y: i32, z: i32) -> LuaResult<bool> {
    let get_core = t.globals().rawget(&Value::str("getCore"));
    let core = t.call1(&get_core, &[])?;
    let option = t.table_get(&core, &Value::str("getOptionBloodDecals"))?;
    if t.call1(&option, &[core])?.as_num().unwrap_or(0.0) == 0.0 {
        return Ok(false);
    }
    let on_wall = with_square(x, y, z, |square| {
        Ok(square
            .objects
            .iter()
            .any(|slot| slot.borrow().wall_blood_splats.as_ref().is_some_and(|s| !s.is_empty())))
    })?;
    if on_wall {
        return Ok(true);
    }
    let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
    cell(|cell| {
        let chunk = cell.chunks.get(&(wx, wy)).ok_or_else(npe)?;
        Ok(chunk.saved.floor_blood_splats.iter().any(|splat| {
            let sx = fastfloor(splat.x + (wx * 8) as f32);
            let sy = fastfloor(splat.y + (wy * 8) as f32);
            sx - 1 <= x && sx + 1 >= x && sy - 1 <= y && sy + 1 >= y
        }))
    })
}

pub(super) fn register(b: &ClassBuilder) {
    for (method, zone_type) in [("getZombiesType", "ZombiesType"), ("getLootZone", "LootZone")] {
        get(b, method, move |c| {
            let (x, y, _) = self_square(c)?;
            with_world_state_cell(|world| {
                let world = world.try_borrow().map_err(|_| LuaError::runtime("IsoWorld is in use"))?;
                let meta = world.meta_grid.as_ref().ok_or_else(|| LuaError::runtime("IsoMetaGrid is not loaded"))?;
                Ok(meta.get_zones_at(x, y, 0).into_iter().map(|id| meta.zone(id)).find(|zone|
                    if zone_type == "ZombiesType" { zone.zone_type.eq_ignore_ascii_case(zone_type) }
                    else { zone.zone_type == zone_type }).map_or(Value::Nil, |zone| Value::str(&zone.name)))
            })
        });
    }
    get(b, "HasTree", |c| {
        let (x, y, z) = self_square(c)?;
        with_square(x, y, z, |square| Ok(square.has_tree.into()))
    });
    get(b, "isOutside", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(square_is_outside(x, y, z)?.into())
    });
    // `isFree(bCountOtherCharacters)`. The port does not keep the square's
    // `movingObjects`, so the `true` form, which returns false for a square
    // with a moving object, is unsupported.
    get(b, "isFree", |c| {
        let (x, y, z) = self_square(c)?;
        if bool_arg(c, 1) {
            return Err(crate::bind_world_object::unsupported("IsoGridSquare.isFree(true): the square's movingObjects"));
        }
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        Ok(with_square(x, y, z, |square| Ok(square.is_free_static(&sprites)))?.into())
    });
    // Edge queries between this square and `other` (`nil` allowed).
    edge(b, "isBlockedTo", |env, a, other| env.is_blocked_to(a, other));
    edge(b, "isWallTo", |env, a, other| a.is_wall_to(other, env.sprites, env.cell));
    edge(b, "isWindowTo", |env, a, other| a.is_window_to(other, env.sprites, env.cell));
    edge(b, "isDoorTo", |env, a, other| a.is_door_to(other, env.sprites, env.cell));
    // `getWorldObjects()`: the square's `worldObjects` in order. Lua gets a
    // snapshot ArrayList; writes to it do not reach the square.
    get(b, "getWorldObjects", |c| {
        let (x, y, z) = self_square(c)?;
        let entries = with_square(x, y, z, |square| {
            let list = square
                .world_objects
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoGridSquare.worldObjects is in use"))?;
            Ok(list.snapshot_entries())
        })?;
        let mut items = Vec::with_capacity(entries.len());
        for entry in &entries {
            items.push(match entry {
                WorldObjectEntry::Object(owner) => crate::bind_world_object::object_value(owner)?,
                WorldObjectEntry::Foreign(value) => value.clone(),
            });
        }
        Ok(crate::bind_java_util::list_value(items))
    });
    get(b, "getFloor", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let floor = with_square(x, y, z, |square| {
            let sprites = sprites.try_borrow().map_err(|_| crate::bind_world_object::borrowed("IsoSpriteManager"))?;
            let index = square.try_floor_index(&sprites).map_err(crate::bind_world_object::host_error)?;
            Ok(index.map(|i| square.objects[i].ref_handle()))
        })?;
        let Some(floor) = floor else { return Ok(Value::Nil) };
        square_object_value(&floor, (x, y, z))
    });
    // `getObjects()`: the square's main object list in order, as a
    // snapshot ArrayList.
    get(b, "getObjects", |c| {
        let (x, y, z) = self_square(c)?;
        let objects = with_square(x, y, z, |square| {
            Ok(square.objects.iter().map(|slot| slot.ref_handle()).collect::<Vec<_>>())
        })?;
        let items = objects
            .iter()
            .map(|object| square_object_value(object, (x, y, z)))
            .collect::<LuaResult<Vec<_>>>()?;
        Ok(crate::bind_java_util::list_value(items))
    });
    // `getSpecialObjects()`: the square's special objects (doors, windows,
    // thumpables, curtains, etc.) resolved from `special_objects`.
    get(b, "getSpecialObjects", |c| {
        let (x, y, z) = self_square(c)?;
        let objects = with_square(x, y, z, |square| {
            let mut out = Vec::with_capacity(square.special_objects.len());
            for &id in &square.special_objects {
                if let Some(slot) = square.objects.iter().find(|s| s.borrow().id == id) {
                    out.push(slot.ref_handle());
                }
            }
            Ok(out)
        })?;
        let items = objects
            .iter()
            .map(|object| square_object_value(object, (x, y, z)))
            .collect::<LuaResult<Vec<_>>>()?;
        Ok(crate::bind_java_util::list_value(items))
    });
    get(b, "isSolidTrans", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        Ok(with_square(x, y, z, |square| Ok(square.is_solid_trans(&sprites)))?.into())
    });
    get(b, "TreatAsSolidFloor", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        Ok(with_square(x, y, z, |square| Ok(square.treat_as_solid_floor(&sprites)))?.into())
    });
    get(b, "HasStairs", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        Ok(with_square(x, y, z, |square| Ok(square.has_stairs(&sprites)))?.into())
    });
    get(b, "HasStairsNorth", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        Ok(with_square(x, y, z, |square| Ok(square.has_stairs_north(&sprites)))?.into())
    });
    get(b, "HasStairsWest", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        Ok(with_square(x, y, z, |square| Ok(square.has_stairs_west(&sprites)))?.into())
    });
    // `hasFloor()` / `hasFloor(north)`: the one-argument form also accepts a
    // solid floor on the neighbouring square to the north (`true`) or west.
    get(b, "hasFloor", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let own = with_square(x, y, z, |square| {
            let sprites = sprites
                .try_borrow()
                .map_err(|_| crate::bind_world_object::borrowed("IsoSpriteManager"))?;
            Ok(square
                .try_floor_index(&sprites)
                .map_err(crate::bind_world_object::host_error)?
                .is_some())
        })?;
        if own || c.nargs() < 2 {
            return Ok(own.into());
        }
        let (nx, ny) = if bool_arg(c, 1) { (x, y - 1) } else { (x - 1, y) };
        let sprites = sprites
            .try_borrow()
            .map_err(|_| crate::bind_world_object::borrowed("IsoSpriteManager"))?;
        let solid = sprites.tables.flag.solidfloor;
        let neighbour = cell(|cell| {
            Ok(match cell.get_grid_square(nx, ny, z) {
                Some(square) => square
                    .try_borrow()
                    .map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?
                    .has(solid),
                None => false,
            })
        })?;
        Ok(neighbour.into())
    });
    // `isFreeOrMidair(countOthers[, doZombie])`: no floor needed. Counting
    // other characters, any moving object makes the square not free (with
    // `doZombie`, dead bodies would not count, but the port keeps corpses in
    // static_moving, so every moving object here is a live character). Then
    // solid, solidtrans or a tree block it, and stairs always free it.
    get(b, "isFreeOrMidair", |c| {
        let (x, y, z) = self_square(c)?;
        let count_others = bool_arg(c, 1);
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        let free = with_square(x, y, z, |square| {
            if count_others && !square.moving_objects.is_empty() {
                return Ok(false);
            }
            let (f, ty) = (&sprites.tables.flag, &sprites.tables.ty);
            if square.has_stairs_north(&sprites) || square.has_stairs_west(&sprites) {
                return Ok(true);
            }
            Ok(!(square.has(f.solid) || square.has(f.solidtrans) || square.has_type(ty.tree)))
        })?;
        Ok(free.into())
    });
    b.method("RecalcProperties", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        with_world_state_cell(|w| {
            let w = w.borrow();
            if let Some(sq) = w.cell.as_ref().and_then(|cell| cell.get_grid_square(x, y, z)) {
                if let Ok(mut sq) = sq.try_borrow_mut() {
                    sq.recalc_properties(&sprites);
                }
            }
        });
        Ok(0)
    });
    // `getStaticMovingObjects()`: the corpses on the square.
    get(b, "getStaticMovingObjects", |c| {
        let (x, y, z) = self_square(c)?;
        let bodies = with_square(x, y, z, |square| Ok(square.static_moving.clone()))?;
        let values = bodies.iter().map(crate::bind_dead_body::dead_body_value).collect();
        Ok(crate::bind_java_util::list_value(values))
    });
    // `getMovingObjects()`: the characters whose moving square this is
    // (`movingObjects`), as a copy. Only the local players have Lua objects
    // yet; any other character on the square stops with `unsupported`.
    get(b, "getMovingObjects", |c| {
        let (x, y, z) = self_square(c)?;
        let ids = with_square(x, y, z, |square| Ok(square.moving_objects.clone()))?;
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            items.push(moving_object_value(id)?);
        }
        Ok(crate::bind_java_util::list_value(items))
    });
    // `getVehicleContainer()`: the first vehicle of the chunks within four
    // squares that intersects this square.
    get(b, "getVehicleContainer", |c| {
        let (x, y, z) = self_square(c)?;
        let min_x = fastfloor((x as f32 - 4.0) / 8.0);
        let min_y = fastfloor((y as f32 - 4.0) / 8.0);
        let max_x = ((x as f32 + 4.0) / 8.0).ceil() as i32;
        let max_y = ((y as f32 + 4.0) / 8.0).ceil() as i32;
        let mut vehicles = Vec::new();
        cell(|cell| {
            for cy in min_y..max_y {
                for cx in min_x..max_x {
                    vehicles.extend(
                        pz_vehicles::cell_link::chunk_vehicles(cell, cx, cy).unwrap_or_default(),
                    );
                }
            }
            Ok(())
        })?;
        Ok(vehicles
            .iter()
            .find(|v| v.is_intersecting_square(x, y, z))
            .map_or(Value::Nil, pz_vehicles::lua::vehicle_value))
    });
    get(b, "isVehicleIntersecting", |c| {
        let (x, y, z) = self_square(c)?;
        let min_x = fastfloor((x as f32 - 4.0) / 8.0);
        let min_y = fastfloor((y as f32 - 4.0) / 8.0);
        let max_x = ((x as f32 + 4.0) / 8.0).ceil() as i32;
        let max_y = ((y as f32 + 4.0) / 8.0).ceil() as i32;
        let mut vehicles = Vec::new();
        cell(|cell| {
            for cy in min_y..max_y {
                for cx in min_x..max_x {
                    vehicles.extend(
                        pz_vehicles::cell_link::chunk_vehicles(cell, cx, cy).unwrap_or_default(),
                    );
                }
            }
            Ok(())
        })?;
        Ok(vehicles.iter().any(|v| v.is_intersecting_square(x, y, z)).into())
    });
    get(b, "getLightLevel", |c| {
        let (x, y, z) = self_square(c)?;
        let player = player_index(c, 1, true)?;
        Ok(Value::Num(square_lighting(player, x, y, z).light_level as f64))
    });
    // `getProperties()`: `properties`, as a copy.
    get(b, "getProperties", |c| {
        let (x, y, z) = self_square(c)?;
        let props = with_square(x, y, z, |square| Ok(square.properties.clone()))?;
        Ok(crate::bind_sprites::square_props_value(props))
    });
    // `getNextNonItemObjectIndex(index)`: the first object at or after
    // `index` that is not an IsoWorldInventoryObject, or -1.
    get(b, "getNextNonItemObjectIndex", |c| {
        let (x, y, z) = self_square(c)?;
        let from = int_arg(c, 1);
        let next = with_square(x, y, z, |square| {
            Ok(square
                .objects
                .iter()
                .enumerate()
                .skip(from.max(0) as usize)
                .find(|(_, slot)| !matches!(slot.borrow().kind, pz_world::ObjectKind::WorldInventory(_)))
                .map_or(-1, |(i, _)| i as i32))
        })?;
        Ok(next.into())
    });
    get(b, "getBuilding", |c| {
        let (x, y, z) = self_square(c)?;
        match super::runtime_rooms::square_room(x, y, z)? {
            Some(room) => super::runtime_rooms::building(room),
            None => Ok(Value::Nil),
        }
    });
    // `getDeadBody()`: the first IsoDeadBody of `staticMovingObjects`.
    get(b, "getDeadBody", |c| {
        let (x, y, z) = self_square(c)?;
        let first = with_square(x, y, z, |square| Ok(square.static_moving.first().cloned()))?;
        Ok(first
            .as_ref()
            .map_or(Value::Nil, crate::bind_dead_body::dead_body_value))
    });
    // `haveGraffiti()`: an object with an attached sprite named
    // `overlay_graffiti`, `overlay_messages` or `constructedobjects_signs_01_4`.
    get(b, "haveGraffiti", |c| {
        let (x, y, z) = self_square(c)?;
        let found = have_overlay(
            x,
            y,
            z,
            &["overlay_graffiti", "overlay_messages", "constructedobjects_signs_01_4"],
        )?;
        Ok(found.into())
    });
    get(b, "haveBlood", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(have_blood(c.thread, x, y, z)?.into())
    });
    get(b, "haveGrime", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(have_overlay(x, y, z, &["overlay_grime"])?.into())
    });
    // `haveStains()`: `haveBlood() || haveGrime()`.
    get(b, "haveStains", |c| {
        let (x, y, z) = self_square(c)?;
        Ok((have_blood(c.thread, x, y, z)? || have_overlay(x, y, z, &["overlay_grime"])?).into())
    });
    // `getAnimalTrack()`: the first IsoAnimalTrack of the objects.
    // `getOre()`: the first object that `isOre()`, whose `CustomName`
    // property holds ironOre, copperOre, FlintBoulder or LimestoneBoulder.
    get(b, "getOre", |c| {
        let (x, y, z) = self_square(c)?;
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        let ore = with_square(x, y, z, |square| {
            for slot in &square.objects {
                let sprite = match slot.borrow().sprite {
                    pz_world::objects::ObjectSprite::None => continue,
                    pz_world::objects::ObjectSprite::Tile(idx) => idx,
                    _ => {
                        return Err(crate::bind_world_object::unsupported(
                            "IsoGridSquare.getOre: an object sprite that has no Lua object",
                        ))
                    }
                };
                let name = sprites.sprites[sprite].properties.get(&sprites.alias, "CustomName");
                if name.is_some_and(|n| {
                    ["ironOre", "copperOre", "FlintBoulder", "LimestoneBoulder"]
                        .iter()
                        .any(|ore| n.contains(ore))
                }) {
                    return Ok(Some(slot.ref_handle()));
                }
            }
            Ok(None)
        })?;
        match ore {
            None => Ok(Value::Nil),
            Some(ore) => {
                let value = crate::bind_world_object::object_value(&ore)?;
                crate::bind_world_object::remember_square(&value, [x, y, z]);
                Ok(value)
            }
        }
    });
    get(b, "getAnimalTrack", |c| {
        let (x, y, z) = self_square(c)?;
        let track = with_square(x, y, z, |square| {
            Ok(square
                .objects
                .iter()
                .find(|slot| matches!(slot.borrow().kind, pz_world::ObjectKind::AnimalTrack(_)))
                .map(|slot| slot.ref_handle()))
        })?;
        match track {
            None => Ok(Value::Nil),
            Some(track) => {
                let value = crate::bind_world_object::object_value(&track)?;
                crate::bind_world_object::remember_square(&value, [x, y, z]);
                Ok(value)
            }
        }
    });
    for (name, bit) in [("isSeen", 1u8), ("isCanSee", 2), ("isCouldSee", 4)] {
        get(b, name, move |c| {
            let (x, y, z) = self_square(c)?;
            let player = player_index(c, 1, false)?;
            Ok((square_lighting(player, x, y, z).vis & bit != 0).into())
        });
    }
}

#[cfg(test)]
mod tests;
