//! Generator power on the actual loaded square. Parent object power is a
//! different native query and is deliberately not aliased here.

use super::*;
use pz_core::config::OptionKind;
use pz_sandbox::SandboxOptions;
use pz_world::world_state::{self, WorldState as NativeWorldState};

type SandboxRef = Rc<RefCell<SandboxOptions>>;

thread_local! {
    static SANDBOX: RefCell<Option<SandboxRef>> = const { RefCell::new(None) };
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

pub(super) fn register(b: &ClassBuilder) {
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
