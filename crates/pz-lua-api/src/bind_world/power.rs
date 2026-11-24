//! Generator power on the actual loaded square. Parent object power is a
//! different native query and is deliberately not aliased here.

use super::*;
use pz_core::config::OptionKind;
use pz_sandbox::SandboxOptions;
use pz_world::world_state::{self, WorldState as NativeWorldState};

type SandboxRef = Rc<RefCell<SandboxOptions>>;

thread_local! {
    static HYDRO_POWER_ON: Cell<bool> = const { Cell::new(false) };
    static SANDBOX: RefCell<Option<SandboxRef>> = const { RefCell::new(None) };
}

/// IsoWorld's grid supply can be read while a square or cell is borrowed.
pub fn hydro_power_on() -> bool {
    HYDRO_POWER_ON.with(Cell::get)
}

pub fn set_hydro_power_on(on: bool) {
    HYDRO_POWER_ON.with(|power| power.set(on));
}

/// Install the same sandbox owner used by the engine, before streaming
/// saved generator constructors. Replacing/retiring it preserves the native
/// class-static range latch until the next actual constructor.
pub fn install_sandbox(options: Option<SandboxRef>) {
    let installed = options.is_some();
    let retired = SANDBOX.with(|current| current.replace(options));
    world_state::install_generator_range_source(installed.then_some(
        world_state::GeneratorRangeSource {
            vertical: constructor_vertical_range,
            radius: constructor_radius,
        },
    ));
    drop(retired);
}

fn sandbox() -> LuaResult<SandboxRef> {
    SANDBOX
        .with(|current| current.borrow().clone())
        .ok_or_else(|| LuaError::runtime("IsoGenerator sandbox owner is not installed"))
}

/// An integer option of the installed sandbox (`None` before the world
/// installs it, or for a missing or non-integer option).
pub fn sandbox_integer(name: &str) -> Option<i32> {
    let options = SANDBOX.with(|current| current.borrow().clone())?;
    let options = options.try_borrow().ok()?;
    integer(&options, name).ok()
}

fn integer(options: &SandboxOptions, name: &str) -> Result<i32, String> {
    match options
        .option_by_name(name)
        .map(|option| &option.config.kind)
    {
        Some(OptionKind::Integer(value)) => Ok(value.value),
        _ => Err(format!("SandboxOptions.{name} is not an integer option")),
    }
}

fn constructor_integer(name: &str) -> pz_world::save::io::SaveResult<i32> {
    use pz_world::save::io::SaveError;
    let options = sandbox().map_err(|error| SaveError(format!("{error:?}")))?;
    let options = options
        .try_borrow()
        .map_err(|_| SaveError("IsoGenerator sandbox owner is in use".into()))?;
    integer(&options, name).map_err(SaveError)
}

fn constructor_vertical_range() -> pz_world::save::io::SaveResult<i32> {
    constructor_integer("GeneratorVerticalPowerRange")
}

fn constructor_radius() -> pz_world::save::io::SaveResult<i32> {
    constructor_integer("GeneratorTileRange")
}

/// Run the window alarm before the container-update callback and store it in
/// the same tracker that loads and writes metadata.bin.
pub fn trigger_room_alarm(room: i64) -> LuaResult<()> {
    let options = sandbox()?;
    let options = options.try_borrow().map_err(|_| crate::bind_world_object::borrowed("alarm sandbox"))?;
    let time = crate::bind_climate::shared_game_time();
    let time = time.try_borrow().map_err(|_| crate::bind_world_object::borrowed("alarm clock"))?;
    let hours = time.world_age_hours() as f32;
    let end_game_time = hours + 0.013611111_f32 * (1440.0 / time.minutes_per_day);
    with_world_state_cell(|w| {
        let mut world = w.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("alarm world"))?;
        let world = &mut *world;
        let (Some(cell), Some(meta)) = (&world.cell, &mut world.meta_grid) else { return Ok(()) };
        let tracker = world.meta_tracker.as_ref().ok_or_else(|| crate::bind_world_object::unsupported("house alarm without world tracker"))?;
        let mut tracker = tracker.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("house alarm tracker"))?;
        let center = world_state::trigger_room_alarm(cell, meta, room, |decay| options.does_power_grid_exist(hours / 24.0, decay))
            .map_err(crate::bind_world_object::host_error)?;
        if let Some((x, y)) = center {
            tracker.alarms.push(pz_core::save_trackers::AlarmSave { x, y, end_game_time });
        }
        Ok(())
    })
}

fn square_utility(c: &NativeCall<'_>, check: fn(&IsoMetaGrid, &pz_world::IsoGridSquare) -> bool) -> LuaResult<bool> {
    let (x, y, z) = self_square(c)?;
    with_world_state_cell(|w| {
        let w = w.try_borrow().map_err(|_| LuaError::runtime("IsoWorld is in use"))?;
        let meta = w.meta_grid.as_ref().ok_or_else(|| LuaError::runtime("IsoMetaGrid is not loaded"))?;
        let square = w.cell.as_ref().and_then(|cell| cell.get_grid_square(x, y, z))
            .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))?;
        let square = square.try_borrow().map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
        Ok(check(meta, &square))
    })
}

pub(super) fn register(b: &ClassBuilder) {
    get(b, "isDerelict", |c| Ok(square_utility(c, world_state::square_is_derelict)?.into()));
    get(b, "isNoPower", |c| Ok(square_utility(c, world_state::square_has_no_power)?.into()));
    get(b, "isNoGas", |c| Ok(square_utility(c, world_state::square_has_no_gas)?.into()));
    get(b, "isNoWater", |c| Ok(square_utility(c, world_state::square_has_no_water)?.into()));
    get(b, "hasGridPower", |c| {
        if square_utility(c, world_state::square_has_no_power)? { return Ok(false.into()); }
        let options = sandbox()?;
        let sb = options.try_borrow().map_err(|_| LuaError::runtime("SandboxOptions is in use"))?;
        let time = crate::bind_climate::shared_game_time();
        let clock = time.try_borrow().map_err(|_| LuaError::runtime("GameTime is in use"))?;
        let days = clock.world_age_days_since_begin(sb.time_since_apo());
        Ok(sb.does_power_grid_exist(days as f32, int_arg(c, 1)).into())
    });
    get(b, "haveElectricity", |c| {
        let (x, y, z) = self_square(c)?;
        // Resolve external owners and read settings before WORLD: this path
        // makes no Lua callback with world, square or sprite guards held.
        let options = sandbox()?;
        let allow_exterior = {
            let options = options
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoGenerator sandbox owner is in use"))?;
            match options
                .option_by_name("AllowExteriorGenerator")
                .map(|option| &option.config.kind)
            {
                Some(OptionKind::Boolean(value)) => value.value,
                _ => {
                    return Err(LuaError::runtime(
                        "SandboxOptions.AllowExteriorGenerator is not a boolean option",
                    ))
                }
            }
        };
        let sprites = crate::bind_sprites::manager()?;
        let range = world_state::generator_power_range();
        with_world_state_cell(|world| {
            let world = world
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoWorld is in use"))?;
            let cell = world
                .cell
                .as_ref()
                .ok_or_else(|| LuaError::runtime("IsoCell is not installed"))?;
            let square = cell
                .get_grid_square(x, y, z)
                .ok_or_else(|| LuaError::runtime("IsoGridSquare is no longer loaded"))?;
            let square = square
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
            let sprites = sprites
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
            Ok(NativeWorldState::square_generator_electricity(
                cell,
                &sprites,
                &square,
                allow_exterior,
                range,
            )
            .into())
        })
    });
}

#[cfg(test)]
mod tests;
