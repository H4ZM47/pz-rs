//! `IsoAnimal.load`/`save`: the animal's own record, which replaces the
//! `IsoPlayer`/`IsoGameCharacter` one (neither calls `super`). The record
//! is `pz-animals`' [`Animal::load`] / [`Animal::to_saved`] with
//! `pz-world`'s codec (`pz_world::save::animal::IsoAnimal`).
//!
//! A loaded animal is `new IsoAnimal(cell)` ([`GameCharacter::new_animal`]:
//! the descriptor's six `doStats` draws and the patience draw) followed by
//! `load`, which makes `init(breed)`'s draws, one draw per loaded gene and
//! `setWild`'s; together they are the original's `Rand` order. Worn items
//! stay `None`: `IsoAnimal.load` reads none.

use pz_animals::{Animal, AnimalCtx, AnimalEnv, AnimalSandbox};
use crate::character::state::{StateFailure, StateResult};
use pz_world::save::io::{Reader, Writer};

use super::{ AnimalDefValues, AnimalHost, CtxClock};
use crate::character::fire::CtxRand;
use crate::character::state::{ActionGroups, StateRegistry};
use crate::character::{GameCharacter, SimContext};
use crate::Characters;

impl<I: 'static> GameCharacter<I> {
    /// `IsoAnimal.load(input, worldVersion, isDebugSave)` into this
    /// animal (made by [`GameCharacter::new_animal`]), after the serialise
    /// flag and class id the caller read. The character parts of
    /// `init(breed)` (states, action group, turn delta) are set up too.
    #[allow(clippy::too_many_arguments)]
    pub fn load_animal(
        &mut self,
        chars: &Characters,
        r: &mut Reader<'_>,
        world_version: i32,
        registry: &mut StateRegistry<I>,
        groups: &mut ActionGroups,
        ctx: &dyn SimContext,
        host: &mut AnimalHost<'_>,
    ) -> StateResult {
        let Some(f) = self.animal.as_ref() else {
            return Err(crate::npe().into());
        };
        let construction = f.construction;
        let clock = CtxClock(ctx);
        let sandbox: AnimalSandbox = ctx.animal_sandbox();
        let mut rand = CtxRand(ctx);
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: host.defs,
                clock: &clock,
                sandbox: &sandbox,
            },
            rand: &mut rand,
            world: &mut *host.world,
            stat_defs: f.stat_defs.clone(),
            patience_range: (Self::PATIENCE_MIN, Self::PATIENCE_MAX),
        };
        let mut loaded = Animal::load(r, world_version, &mut env).map_err(StateFailure::from)?;
        loaded.chr.construction = construction;
        let def = loaded
            .animal
            .animal_type
            .as_deref()
            .and_then(|t| host.defs.def(t))
            .map(AnimalDefValues::of);
        let f = self.animal.as_mut().ok_or_else(crate::npe)?;
        f.animal = loaded.animal;
        f.initialized = def.is_some();
        f.def = def;
        self.apply_animal_character_values(chars, &loaded.chr);
        self.animal_character_init(registry, groups);
        Ok(())
    }

    /// `IsoAnimal.save(output, isDebugSave, serialize)`: the codec record
    /// of this animal (`serialize` writes the flag and the `Animal` class
    /// id first). `mother_id` is the mother's `animalId` when she is
    /// alive.
    #[allow(clippy::too_many_arguments)]
    pub fn save_animal(
        &self,
        chars: &Characters,
        ctx: &dyn SimContext,
        defs: &pz_animals::AnimalDefinitionTables,
        sctx: &pz_world::save::SaveCtx<'_>,
        w: &mut Writer,
        serialize: bool,
        mother_id: Option<i32>,
    ) -> StateResult {
        let f = self.animal.as_ref().ok_or_else(crate::npe)?;
        let chr = self.animal_character_values(chars).ok_or_else(crate::npe)?;
        let clock = CtxClock(ctx);
        let sandbox = ctx.animal_sandbox();
        let actx = AnimalCtx {
            defs,
            clock: &clock,
            sandbox: &sandbox,
        };
        let rec = f.animal.to_saved(&chr, actx, mother_id).map_err(StateFailure::from)?;
        rec.save(w, sctx, serialize)
            .map_err(crate::character::save::io_error).map_err(StateFailure::from)
    }
}
