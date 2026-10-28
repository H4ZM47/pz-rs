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

fn square_lighting(player: i32, x: i32, y: i32, z: i32) -> SquareLighting {
    let source = SQUARE_LIGHT.with(|s| s.borrow().clone());
    source.and_then(|f| f(player, x, y, z)).unwrap_or(UNDECODED)
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

pub(super) fn register(b: &ClassBuilder) {
    get(b, "isOutside", |c| {
        let (x, y, z) = self_square(c)?;
        Ok(square_is_outside(x, y, z)?.into())
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
    // `getObjects()`: the square's main object list in order, as a
    // snapshot ArrayList.
    get(b, "getObjects", |c| {
        let (x, y, z) = self_square(c)?;
        let objects = with_square(x, y, z, |square| {
            Ok(square.objects.iter().map(|slot| slot.ref_handle()).collect::<Vec<_>>())
        })?;
        let items = objects
            .iter()
            .map(crate::bind_world_object::object_value)
            .collect::<LuaResult<Vec<_>>>()?;
        Ok(crate::bind_java_util::list_value(items))
    });
    // `getStaticMovingObjects()`: corpses. `IsoDeadBody` has no Lua object
    // yet, so only an empty list can be answered faithfully.
    get(b, "getStaticMovingObjects", |c| {
        let (x, y, z) = self_square(c)?;
        let count = with_square(x, y, z, |square| Ok(square.static_moving.len()))?;
        if count != 0 {
            return Err(crate::bind_world_object::unsupported(
                "IsoGridSquare.getStaticMovingObjects: IsoDeadBody is not exposed to Lua",
            ));
        }
        Ok(crate::bind_java_util::list_value(Vec::new()))
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
    get(b, "getLightLevel", |c| {
        let (x, y, z) = self_square(c)?;
        let player = player_index(c, 1, true)?;
        Ok(Value::Num(square_lighting(player, x, y, z).light_level as f64))
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
