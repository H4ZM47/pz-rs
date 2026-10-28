//! The population manager over the real world: [`SittingChecks`] on the
//! adapter (`getSquareForSittingZombie`'s `movingObjects` test and
//! `PolygonalMap2.lineClearCollide`), and a [`WorldPopulationHost`] that
//! reads the cell through `RealZombies` while it holds the adapter.

use pz_characters::character::zombie::population::RealZombies;
use pz_pathfind::main_thread::{LineClearCollideMain, MainThreadMap};
use pz_pathfind::square_bits::SquareBits;
use pz_world::{GameRand, ObjectHooks};
use pz_world_popman::{PopulationView, SittingChecks, WorldPopulationHost};

use crate::StatesWorldAdapter;

pz_vehicles::pathfind_adapter!(pz_pathfind);

impl SittingChecks for StatesWorldAdapter<'_> {
    /// [`StatesEngine::has_moving_objects`](crate::StatesEngine::has_moving_objects).
    fn has_moving_objects(&self, x: i32, y: i32, z: i32) -> bool {
        self.engine.has_moving_objects((x, y, z))
    }

    /// `PolygonalMap2.instance.lineClearCollide(fromX, fromY, toX, toY, z,
    /// null, bIgnoreDoors, bCloseToWalls)` over the cell, with the cell's
    /// vehicles (`pz-vehicles`' pathfind view): true when blocked.
    fn line_clear_collide(
        &self,
        from_x: f32,
        from_y: f32,
        to_x: f32,
        to_y: f32,
        z: i32,
        ignore_doors: bool,
        close_to_walls: bool,
    ) -> bool {
        let cell = &*self.cell;
        let sprites = &*self.sprites;
        let hooks = ObjectHooks::new(cell, sprites);
        let Ok(bits) = SquareBits::new(sprites, self.meta, cell, &hooks) else {
            return false;
        };
        let mut map = MainThreadMap::new(&bits, &PathfindVehicles);
        map.is_server = self.settings.server;
        LineClearCollideMain::new(&map).line_clear_collide(
            from_x,
            from_y,
            to_x,
            to_y,
            z,
            None,
            ignore_doors,
            close_to_walls,
        )
    }
}

/// The world `zombies` holds, for [`WorldPopulationHost::from_ops`].
pub fn population_view<'z, I: 'static>(
    zombies: &'z RealZombies<'_, I, StatesWorldAdapter<'_>>,
) -> PopulationView<'z> {
    let w = &*zombies.world;
    PopulationView {
        cell: &*w.cell,
        meta: w.meta,
        sprites: &*w.sprites,
        checks: w,
    }
}

/// A real zombie of `zombies`' list stands on `(x, y, z)`: it is on that
/// square's `movingObjects` (`createRealZombieAlways` adds it there at
/// once), which the engine's moving-object answer cannot see while the list
/// is lent out.
pub fn real_zombie_on_square<I: 'static>(
    zombies: &RealZombies<'_, I, StatesWorldAdapter<'_>>,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    zombies
        .zombies
        .iter()
        .any(|c| c.path.current_square == Some((x, y, z)))
}

/// The population host over the real world: squares, wall types and the
/// sitting checks read through `zombies`' adapter between zombie calls.
pub fn population_host<'a, 'r, 'w, I: 'static>(
    rand: &'a mut dyn GameRand,
    zombies: &'a mut RealZombies<'r, I, StatesWorldAdapter<'w>>,
) -> WorldPopulationHost<'a, RealZombies<'r, I, StatesWorldAdapter<'w>>> {
    WorldPopulationHost::from_ops(
        population_view::<I>,
        Some(real_zombie_on_square::<I>),
        rand,
        zombies,
    )
}
