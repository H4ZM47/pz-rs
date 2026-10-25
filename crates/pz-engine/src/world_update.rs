//! `IsoWorld.update` (`updateInternal`, `updateWorld`, `updateThread`),
//! `IsoCell.update` and the world systems `IngameState.update` steps: the
//! chunk maps' `ProcessChunkPos`, `ZombiePopulationManager.updateMain`,
//! `WorldSoundManager.update`, `IsoFireManager.Update` and the
//! pathfinder's main-thread pass.
//!
//! The characters live in two places: the local players in
//! `pz_lua_api::bind_players` (`IsoPlayer.players`, shared with Lua) and
//! the real zombies in [`IsoWorld::zombies`] (`IsoCell.getZombieList()`).
//! The cell is `pz_lua_api::bind_world`'s; nothing here calls into Lua
//! while it is borrowed.

use crate::iso_world::{IsoWorld, StandardRand};
use crate::sim::FrameSim;
use pz_characters::character::state::StateCx;
use pz_characters::character::zombie::population::RealZombies;
use pz_characters::character::SimEvent;
use pz_characters::survivor::SurvivorDescRef;
use pz_kahlua::Value;
use pz_lua_api::bind_character_sim::{character_value, LuaGameCharacter, LuaGameCharacterRef};
use pz_lua_api::bind_characters::{shared_characters, LuaItem};
use pz_lua_api::bind_world::with_world;
use pz_lua_api::LuaManager;
use pz_world::streamer::{PlayerPos, Serve, StreamEvent};
use pz_world::{GameRand, IsoCell, IsoMetaGrid, IsoSpriteManager};
use pz_world::isoregion::IsoRegions;
use pz_world_popman::{PopulationFrame, WorldPopulation, WorldPopulationHost};
use pz_world_states::{StatesEngine, StatesRuntime, StatesWorldAdapter, ThumpSettings};
use std::cell::RefCell;
use std::rc::Rc;

/// `threadLoop` iterations of the `WorldStreamer` thread per game frame
/// (the original's thread runs beside the game loop; one chunk and one
/// queued save per iteration).
pub const STREAMER_JOBS_PER_FRAME: usize = 4;

/// `IsoPlayer.MAX`.
const MAX_PLAYERS: usize = 4;

/// The engine's side of the character states: the game's `Rand` and the
/// squares' moving objects outside the zombie list. The rest of
/// `StatesEngine` (other characters by id, LOS, corpses, sounds, the path
/// host) keeps its defaults: TODO(port): those owners.
struct EngineStates {
    rand: StandardRand,
    /// The squares the local players stand on (`getCurrentSquare()`),
    /// taken before the frame lends any of them out. TODO(port): animals.
    movers: Vec<(i32, i32, i32)>,
}

impl StatesEngine for EngineStates {
    fn rand(&mut self) -> &mut dyn GameRand {
        &mut self.rand
    }
    /// `!square.getMovingObjects().isEmpty()` for the players and animals;
    /// the population host counts the real zombies itself.
    fn has_moving_objects(&self, sq: (i32, i32, i32)) -> bool {
        self.movers.contains(&sq)
    }
}

/// The local players' current squares, for [`EngineStates::movers`].
fn player_squares() -> Vec<(i32, i32, i32)> {
    local_players().iter().flatten().filter_map(|p| p.try_borrow().ok().and_then(|c| c.path.current_square)).collect()
}

/// The local players, `IsoPlayer.players[0..MAX]`.
fn local_players() -> [Option<LuaGameCharacterRef>; MAX_PLAYERS] {
    std::array::from_fn(pz_lua_api::bind_players::player)
}

/// `IsoPlayer.players[n]` as the chunk streamer sees it.
fn player_positions() -> [Option<PlayerPos>; MAX_PLAYERS] {
    local_players().map(|p| {
        p.map(|p| {
            let c = p.borrow();
            PlayerPos::moved(c.vars.last_x, c.vars.last_y, c.vars.x, c.vars.y)
        })
    })
}

/// Runs `f` over the character states' world (`StatesWorldAdapter` over
/// the current cell); `None` without a cell.
fn with_states<R>(
    sprites: &mut IsoSpriteManager,
    runtime: &mut StatesRuntime,
    f: impl FnOnce(&mut StatesWorldAdapter<'_>) -> R,
) -> Option<R> {
    let movers = player_squares();
    with_world(|w| {
        let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else { return None };
        let mut engine = EngineStates { rand: StandardRand, movers };
        let mut adapter = StatesWorldAdapter::new(cell, meta, sprites, ThumpSettings::default(), runtime, &mut engine);
        Some(f(&mut adapter))
    })
}

/// `SurvivorFactory.CreateSurvivor()` for a new real zombie.
fn create_survivor(chars: &RefCell<pz_characters::Characters>) -> pz_scripts::ScriptResult<SurvivorDescRef<LuaItem>> {
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let d = chars.borrow_mut().create_survivor(&sm)?;
    Ok(Rc::new(RefCell::new(d)))
}

/// The real zombies and the world the population manager drives, for one
/// call: `(population, host, frame)`.
type PopulationCall<'a, R> = &'a mut dyn FnMut(
    &mut WorldPopulation<i32>,
    &mut WorldPopulationHost<'_, RealZombies<'_, LuaItem, StatesWorldAdapter<'_>>>,
    &PopulationFrame<'_, i32>,
) -> R;

impl IsoWorld {
    /// The `IngameState.updateInternal` loop before `IsoWorld.update`:
    /// `chunkMap[n].ProcessChunkPos(player)` for each local player
    /// (`IsoWorld.doChunkMapUpdate`), then this frame's share of the
    /// `WorldStreamer` thread.
    pub fn process_chunk_pos(&mut self) {
        let players = player_positions();
        self.with_stream(|s, cell, ctx| {
            for (n, p) in players.iter().enumerate().take(s.num_players()) {
                if let Some(p) = p {
                    if !s.chunk_map(n).ignore {
                        s.process_chunk_pos(n, *p, cell, ctx);
                    }
                }
            }
            s.serve(&players, Serve::Jobs(STREAMER_JOBS_PER_FRAME), ctx);
        });
        self.hand_off_events();
    }

    /// `IsoWorld.update()`: `updateInternal`, then `DesignationZone.update`
    /// (TODO(port): `DesignationZone`).
    pub fn update(&mut self, lua: &mut LuaManager, sim: &FrameSim) {
        self.frame_no += 1;
        // TODO(port): `WorldSimulation.instance.update()`,
        // `HutchManager.updateAll`, `ImprovedFog.update`,
        // `helicopter.update`, the current emitters' `tick` (every 30 ms),
        // `metaGrid.getCurrentCellData().checkTriggers()`.
        // TODO(port): `WorldSoundManager.instance.initFrame()`,
        // `ZombieGroupManager.preupdate`, `OnceEvery.update`,
        // `CollisionManager.initUpdate`.
        // `ClimateManager.getInstance().update()` runs with `GameTime.update`
        // in `bind_climate::update_game_time` (`IngameState.UpdateStuff`).
        self.update_world(lua, sim);
        self.update_thread();
    }

    /// `updateWorld()`: `IsoCell.update`, `IsoRegions.update`, then
    /// `MovingObjectUpdateScheduler.postupdate`.
    fn update_world(&mut self, lua: &mut LuaManager, sim: &FrameSim) {
        self.update_cell(lua, sim);
        let player = pz_lua_api::bind_players::player_instance().map(|p| {
            let c = p.borrow();
            (c.vars.x, c.vars.y)
        });
        if let (Some(r), Some(sp)) = (self.regions.as_mut(), self.sprites.as_ref()) {
            // TODO(port): `update_with_buildings` (player-built rooms into
            // the meta grid needs the render invalidation's `BuildingCtx`).
            with_world(|w| {
                if let Some(cell) = w.cell.as_ref() {
                    r.update(cell, sp, player);
                }
            });
        }
        // TODO(port): `HaloTextHelper.update`, `CollisionManager.ResolveContacts`.
        self.postupdate_characters(lua, sim);
    }

    /// `IsoCell.update()`.
    fn update_cell(&mut self, lua: &mut LuaManager, sim: &FrameSim) {
        // TODO(port): `MovingObjectUpdateScheduler.startFrame`,
        // `IsoSprite.alphaStep`, `ProcessSpottedRooms`.
        let alive: Vec<usize> = local_players()
            .iter()
            .enumerate()
            .filter_map(|(n, p)| p.as_ref().filter(|p| !p.borrow().is_dead()).map(|_| n))
            .collect();
        self.with_stream(|s, cell, ctx| {
            for &n in &alive {
                if n < s.num_players() {
                    s.chunk_map_update(n, cell, ctx);
                }
            }
        });
        self.hand_off_events();
        // TODO(port): `ProcessRemoveItems`, `ProcessItems` (with the alarm
        // clocks), `ProcessIsoObject`.
        self.update_characters(lua, sim);
        // TODO(port): `ProcessStaticUpdaters`, `ObjectDeletionAddition`,
        // `IsoDeadBody.updateBodies`, `FishSchoolManager.update`, the rain
        // scroll and `updateWeatherFx`.
    }

    /// `ProcessObjects` through `MovingObjectUpdateScheduler.update`: each
    /// moving object's `preupdate`, `frameStep` and `update`, local players
    /// first, then the zombies, in list order. TODO(port): the simulation
    /// levels (`frameMod` buckets by distance); everything updates every
    /// frame.
    fn update_characters(&mut self, lua: &mut LuaManager, sim: &FrameSim) {
        let input = pz_player_input::EnginePlayerInput::capture();
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let chars = shared_characters();
        for p in local_players().into_iter().flatten() {
            if p.borrow().is_dead() {
                continue;
            }
            // TODO(port): `action_blocks_movement`/`has_actions` from the
            // Lua timed-action queue and the climate's wind movement.
            let r = {
                let mut chr = p.borrow_mut();
                chr.preupdate_player();
                chr.frame_step(&input);
                let IsoWorld { sprites, states_runtime, action_groups, .. } = self;
                let Some(sprites) = sprites.as_mut() else { return };
                with_states(sprites, states_runtime, |w| {
                    fill_square_environment(&mut chr, w.cell);
                    pz_lua_api::bind_states::with_state_registry(|reg| {
                        let chars = chars.borrow();
                        let mut cx = StateCx::new(&sm, &chars, sim, w);
                        chr.update_player(&input, reg, action_groups, &mut cx)
                    })
                })
            };
            match r {
                Some(Ok(u)) => fire_player_events(lua, &p, u.events),
                Some(Err(e)) => self.log_update_error(format!("IsoPlayer.update: {e}")),
                None => {}
            }
            self.fire_sim_events(lua, Some(&p), sim);
            // `IsoGameCharacter.updateTextObjects` queued the chat element's
            // lines; run them and their Lua events.
            if let Err(e) = pz_lua_api::chat::characters::flush_chat(&p) {
                self.log_update_error(format!("ChatElement: {e:?}"));
            }
            pz_lua_api::chat::fire_pending_events(&mut lua.thread);
        }
        input.write_back();
        let IsoWorld { sprites, states_runtime, action_groups, zombies, .. } = self;
        let Some(sprites) = sprites.as_mut() else { return };
        let errors = with_states(sprites, states_runtime, |w| {
            pz_lua_api::bind_states::with_state_registry(|reg| {
                let chars = chars.borrow();
                let mut errors = Vec::new();
                for z in zombies.iter_mut() {
                    z.preupdate_zombie();
                    fill_square_environment(z, w.cell);
                    let mut cx = StateCx::new(&sm, &chars, sim, &mut *w);
                    if let Err(e) = z.update_zombie(reg, action_groups, &mut cx) {
                        errors.push(format!("IsoZombie.update: {e}"));
                    }
                }
                errors
            })
        });
        for e in errors.into_iter().flatten() {
            self.log_update_error(e);
        }
        self.fire_sim_events(lua, None, sim);
    }

    /// `MovingObjectUpdateScheduler.postupdate()`: `IsoPlayer.postupdate`
    /// and `IsoZombie.postupdate` (the move and its collisions, the action
    /// context).
    fn postupdate_characters(&mut self, lua: &mut LuaManager, sim: &FrameSim) {
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let chars = shared_characters();
        let players = local_players();
        let IsoWorld { sprites, states_runtime, zombies, .. } = self;
        let Some(sprites) = sprites.as_mut() else { return };
        with_states(sprites, states_runtime, |w| {
            let chars = chars.borrow();
            for p in players.iter().flatten() {
                let mut cx = StateCx::new(&sm, &chars, sim, &mut *w);
                p.borrow_mut().postupdate_player(&mut cx);
            }
            for z in zombies.iter_mut() {
                let mut cx = StateCx::new(&sm, &chars, sim, &mut *w);
                z.postupdate_zombie(&mut cx);
            }
        });
        self.fire_sim_events(lua, None, sim);
    }

    /// `updateThread()`.
    fn update_thread(&mut self) {
        // TODO(port): `updateBuildings` (`IsoBuilding.update`),
        // `ObjectRenderEffects.updateStatic`, `AddCoopPlayer`,
        // `IsoPlayer.UpdateRemovedEmitters`, `updateDBs`
        // (`PlayerDB.updateMain`, `VehiclesDB2.updateMain`),
        // `AnimalZones.updateVirtualAnimals`,
        // `AnimalTracksDefinitions.loadTracksDefinitions`.
    }

    /// The simulation's side effects for Lua (`LuaEventManager.triggerEvent`
    /// from inside the character code), with `chr` as the character.
    fn fire_sim_events(&mut self, lua: &mut LuaManager, chr: Option<&LuaGameCharacterRef>, sim: &FrameSim) {
        let cv = chr.map(character_value).unwrap_or_default();
        for e in sim.take_events() {
            match e {
                SimEvent::ContainerUpdate => {
                    lua.trigger_event("OnContainerUpdate", &[]);
                }
                SimEvent::PlayerGetDamage { kind, amount } if chr.is_some() => {
                    lua.trigger_event("OnPlayerGetDamage", &[cv.clone(), Value::str(kind), Value::Num(f64::from(amount))]);
                }
                // TODO(port): the other `SimEvent`s (sounds, visuals, XP,
                // game speed, the world's side of combat).
                _ => {}
            }
        }
    }

    fn log_update_error(&mut self, e: String) {
        eprintln!("ERROR: {e}");
        if self.errors.len() < 100 {
            self.errors.push(e);
        }
    }

    /// `ExceptionLogger.logException(ex)` for an exception the original
    /// catches and logs, kept in [`IsoWorld::errors`].
    fn log_exception(&mut self, e: String) {
        pz_core::debug::log_exception(&pz_core::debug::LogThrowable::new(e.clone()), None);
        if self.errors.len() < 100 {
            self.errors.push(e);
        }
    }

    /// Runs `f` with the population manager, the population host over the
    /// real world (`pz_world_states::population_host`: the zombies' world
    /// is the `StatesWorldAdapter`, which is also the sitting checks) and
    /// this frame's inputs. `None` without a population, streamer, sprites
    /// or cell.
    pub fn with_population<R>(&mut self, sim: &FrameSim, f: PopulationCall<'_, R>) -> Option<R> {
        let IsoWorld { population, streamer, sprites, zombies, next_object_id, rand, key_spawn_chance_d100, states_runtime, .. } = self;
        let (Some(pop), Some(streamer), Some(sprites)) = (population.as_mut(), streamer.as_ref(), sprites.as_mut()) else {
            return None;
        };
        let maps = pz_world_popman::player_chunk_maps(streamer);
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let chars = shared_characters();
        let water_flag = sprites.tables.flag.water;
        let movers = player_squares();
        with_world(|w| {
            let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else { return None };
            // `isWaterSquare()`: read before the adapter takes the cell.
            let water_squares: std::collections::HashSet<(i32, i32, i32)> = cell
                .chunks
                .values()
                .flat_map(|c| c.squares())
                .filter(|s| s.has(water_flag))
                .map(|s| (s.x, s.y, s.z))
                .collect();
            let water = |p: (i32, i32, i32)| water_squares.contains(&p);
            let mut create = || create_survivor(&chars);
            let mut engine = EngineStates { rand: StandardRand, movers };
            let mut adapter = StatesWorldAdapter::new(cell, meta, sprites, ThumpSettings::default(), states_runtime, &mut engine);
            let mut ops = RealZombies {
                sm: &sm,
                chars: &chars,
                ctx: sim,
                world: &mut adapter,
                zombies,
                next_id: next_object_id,
                create_survivor: &mut create,
                water: &water,
                key_spawn_chance_d100: *key_spawn_chance_d100,
                building_keys: Vec::new(),
                removed: Vec::new(),
            };
            let infos = ops.infos();
            let frame = PopulationFrame {
                multiplier: sim.multiplier,
                world_age_hours: sim.world_age_hours,
                chunk_maps: &maps,
                real_zombies: &infos,
            };
            let r = {
                let mut host = pz_world_states::population_host(rand, &mut ops);
                f(pop, &mut host, &frame)
            };
            // TODO(port): `checkAndSpawnZombieForBuildingKey` for
            // `ops.building_keys`.
            Some(r)
        })
    }

    /// `ZombiePopulationManager.instance.updateMain()` (with
    /// `MapCollisionData.updateMain` and the in-line `MCDThread` pass).
    /// `Err` is the exception it threw, already logged: the original's
    /// `UpdateStuff` catches it around the population and pathfinder
    /// block, so the caller skips the rest of that block.
    pub fn update_population(&mut self, sim: &FrameSim) -> Result<(), String> {
        let r = self.with_population(sim, &mut |pop, host, frame| pop.update(host, frame));
        if let Some(Err(e)) = r {
            self.log_exception(e.clone());
            return Err(e);
        }
        Ok(())
    }

    /// `MapCollisionData.instance.startGame()` (`IngameState.enter`, which
    /// logs what it throws).
    pub fn start_population(&mut self, sim: &FrameSim) {
        let r = self.with_population(sim, &mut |pop, host, frame| pop.start_game(host, frame));
        if let Some(Err(e)) = r {
            self.log_exception(e);
        }
    }

    /// `WorldSoundManager.instance.update()` over the local players' chunk
    /// maps.
    pub fn update_world_sounds(&mut self, hearing: i32) {
        let Some(s) = self.streamer.as_ref() else { return };
        let half = s.width / 2;
        let mut active = Vec::new();
        for n in 0..s.num_players() {
            let m = s.chunk_map(n);
            if m.ignore || pz_lua_api::bind_players::player(n).is_none() {
                continue;
            }
            for x in m.world_x - half..=m.world_x + half {
                for y in m.world_y - half..=m.world_y + half {
                    active.push((x, y));
                }
            }
        }
        let sounds = pz_lua_api::bind_zombie::shared_world_sounds();
        with_world(|w| {
            let Some(cell) = w.cell.as_ref() else { return };
            // TODO(port): the `SoundEffects` (`OnWorldSound`, fish noise,
            // the population's world sounds).
            let mut fx = pz_world_sound::NoEffects;
            let a = pz_world_sound::WorldSoundAdapter::new(cell, hearing, &mut fx);
            sounds.borrow_mut().update(&a, active);
        });
    }

    /// `IsoFireManager.Update()`. `Err` is the exception a burn threw
    /// (logged here: `UpdateStuff` catches and logs it, then goes on).
    pub fn update_fire(&mut self, sim: &FrameSim, settings: pz_world::fire::FireSettings) -> Result<(), String> {
        if self.fire_flags.is_none() {
            let Some(sprites) = self.sprites.as_ref() else { return Ok(()) };
            match pz_world::fire::manager::FireFlags::resolve(&sprites.tables) {
                Ok(f) => self.fire_flags = Some(f),
                Err(e) => {
                    self.log_update_error(format!("IsoFireManager: {e}"));
                    return Ok(());
                }
            }
        }
        if self.door_tables.is_none() {
            match pz_classfile::Jar::open(self.game_dir.join("projectzomboid.jar")).map_err(|e| e.to_string()).and_then(|j| pz_world::square_mutation::DoorTables::from_jar(&j)) {
                Ok(d) => self.door_tables = Some(d),
                Err(e) => {
                    self.log_update_error(format!("IsoDoor tables: {e}"));
                    return Ok(());
                }
            }
        }
        let g = sim.game_time.borrow();
        let ctx = pz_lua_api::bind_climate::LuaClimateContext::new();
        let frame = pz_world::fire::FireFrame {
            multiplied_seconds: g.multiplied_seconds_since_last_update(&ctx),
            thirty_fps: g.get_thirty_fps_multiplier(&ctx),
            now_millis: sim.calendar_millis,
        };
        drop(g);
        let num_players = pz_lua_api::bind_players::num_players();
        let IsoWorld { fire, fire_flags, door_tables, sprites, rand, regions, render, pathfind, .. } = self;
        let (Some(sprites), Some(flags), Some(doors)) = (sprites.as_mut(), *fire_flags, door_tables.as_ref()) else { return Ok(()) };
        let water = sprites.tables.flag.water;
        let mut no_regions = None;
        let regions = match regions.as_mut() {
            Some(r) => r,
            None => no_regions.get_or_insert_with(|| IsoRegions::new(0)),
        };
        // The pathfinder hears of the changed squares once the fire update
        // has let go of the sprites (`PathfindNative.squareChanged` builds
        // its tasks from the cell as it is then).
        let mut pf = QueuedPathfind::default();
        let r = with_world(|w| {
            let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else { return Ok(()) };
            let mut no_hooks = pz_world::square_mutation::NoMutationHooks;
            let mut no_render = pz_world::square_mutation::NoMutationHooks;
            let mut inv = render.as_mut().map(|r| r.invalidation(water, num_players));
            let render: &mut dyn pz_world::square_mutation::RenderSquareHooks = match inv.as_mut() {
                Some(i) => i,
                None => &mut no_render,
            };
            // TODO(port): the engine's `SquareMutationHooks` (Lua events,
            // the add/remove-from-world registries) and the `FireHooks`
            // (sounds, lights through `FireLightLinks`, burning characters,
            // heat sources, hutches).
            let mutation = pz_world::fire::manager::FireMutation {
                regions,
                doors,
                world: &mut no_hooks,
                pathfind: &mut pf,
                render,
                num_players,
            };
            let mut hooks = pz_world::fire::manager::NoFireHooks;
            let mut fw = pz_world::fire::FireWorld { cell, meta, sprites: &mut *sprites, mutation, flags, settings, hooks: &mut hooks };
            fire.update(&mut fw, rand, frame)
        });
        if let Some(h) = pathfind.as_mut() {
            pf.apply(h, sprites);
        }
        if let Err(e) = &r {
            self.log_exception(e.clone());
        }
        r
    }

    /// `PolygonalMap2.instance.updateMain()` (`PathfindNative.updateMain`):
    /// the pathfinding thread's pass, then the finished requests handed to
    /// their characters.
    pub fn update_pathfinding(&mut self) {
        let IsoWorld { pathfind, zombies, .. } = self;
        let Some(h) = pathfind.as_mut() else { return };
        h.update_thread();
        // TODO(port): the cell's vehicles and `SeatingManager`.
        let mut movers = Movers { zombies };
        h.update_main(&pz_pathfind::game_vehicle::NoVehicles, &pz_pathfind::behavior::hooks::NoSeating, &mut movers);
    }

    /// Takes the streamer's events and runs the world's side of each
    /// chunk that was handed off (`doLoadGridsquare`: the population, the
    /// pathfinder, the renderer) or left the world (`removeFromWorld`).
    pub fn hand_off_events(&mut self) {
        let Some(events) = self.streamer.as_mut().map(|s| s.take_events()) else { return };
        for e in events {
            match e {
                StreamEvent::HandedOff { wx, wy } => self.chunk_added(wx, wy),
                StreamEvent::Unloaded { wx, wy } | StreamEvent::Removed { wx, wy } => self.chunk_removed(wx, wy),
                StreamEvent::SaveFailed { wx, wy, error } => {
                    self.log_update_error(format!("IsoChunk.Save {wx},{wy}: {error}"));
                }
                // TODO(port): `HotSave` (`world_save::hot_save_ancillary`),
                // `GenAnimalsPath`, the vehicle events.
                _ => {}
            }
        }
    }

    /// `IsoChunk.doLoadGridsquare` for a chunk that entered the cell.
    fn chunk_added(&mut self, wx: i32, wy: i32) {
        let IsoWorld { population, sprites, pathfind, chunk_load_id, .. } = self;
        let Some(sp) = sprites.as_ref() else { return };
        *chunk_load_id = chunk_load_id.wrapping_add(1);
        let load_id = *chunk_load_id;
        with_world(|w| {
            let (Some(cell), Some(meta)) = (w.cell.as_ref(), w.meta_grid.as_ref()) else { return };
            if let Some(p) = population.as_mut() {
                p.add_chunk_to_world(cell, meta, sp, wx, wy);
            }
            if let Some(h) = pathfind.as_mut() {
                add_chunk_to_pathfinder(h, cell, meta, sp, wx, wy, load_id);
            }
        });
        // TODO(port): the chunk's vehicles (`AddVehicles`), reanimated
        // players, erosion and the lighting thread's chunk add.
        self.render_chunks.added.push((wx, wy));
    }

    /// `IsoChunk.removeFromWorld` for a chunk that left the cell.
    fn chunk_removed(&mut self, wx: i32, wy: i32) {
        if let Some(h) = self.pathfind.as_mut() {
            h.service_mut().remove_chunk_from_world(wx, wy);
        }
        let sim = FrameSim::capture();
        // TODO(port): `in_room` needs the chunk's rooms, which are gone
        // once the streamer reports the unload: every zombie counts as
        // outside.
        self.with_population(&sim, &mut |pop, host, _| {
            let cz = host.zombies.chunk_zombies(wx, wy, &|_| false);
            pop.remove_chunk_from_world(host, wx, wy, &cz);
        });
        self.render_chunks.removed.push((wx, wy));
    }
}

/// The parts of the character's `CharacterEnvironment` the world answers
/// from `getCurrentSquare()`, refreshed before its update. TODO(port): the
/// room size, building toxicity, can-see, heat sources and the vehicle.
fn fill_square_environment(chr: &mut LuaGameCharacter, cell: &IsoCell) {
    let cur = chr.path.current_square;
    let sq = cur.and_then(|(x, y, z)| cell.get_grid_square(x, y, z));
    let stress = cur.map_or(0.0, |(x, y, z)| pz_lua_api::bind_zombie::shared_world_sounds().borrow().get_stress_from_sounds(x, y, z));
    let e = &mut chr.environment;
    e.has_current_square = sq.is_some();
    e.on_last_square = cur.is_some() && cur == chr.vars.last_square;
    e.square_in_room = sq.is_some_and(|s| s.room_id != -1);
    e.square_have_roof = sq.is_some_and(|s| s.have_roof);
    e.square_z = sq.map_or(0, |s| s.z);
    e.square_has_chunk = sq.is_some();
    e.square_room = sq.filter(|s| s.room_id != -1).map(|s| s.room_id as u64);
    e.stress_from_sounds = stress;
}

/// `PathfindNative.addChunkToWorld(chunk)`.
fn add_chunk_to_pathfinder(
    h: &mut pz_pathfind::host::PathHost,
    cell: &IsoCell,
    meta: &IsoMetaGrid,
    sprites: &IsoSpriteManager,
    wx: i32,
    wy: i32,
    load_id: i16,
) {
    let Some(chunk) = cell.chunks.get(&(wx, wy)) else { return };
    // TODO(port): `ObjectHooks` for thumpables, barricades and doors.
    let hooks = pz_pathfind::square_bits::DefaultBitsHooks;
    match pz_pathfind::square_bits::SquareBits::new(sprites, meta, cell, &hooks) {
        Ok(bits) => h.service_mut().add_chunk_to_world(chunk, load_id, &bits),
        Err(e) => eprintln!("ERROR: PathfindNative.addChunkToWorld: {e}"),
    }
}

/// `PathfindSquareHooks` that queue the square and chunk changes of a
/// mutation for the pathfinder, applied after it with
/// [`QueuedPathfind::apply`].
#[derive(Default)]
struct QueuedPathfind {
    squares: Vec<(i32, i32, i32)>,
    chunks: Vec<(i32, i32)>,
}

impl pz_world::square_mutation::PathfindSquareHooks for QueuedPathfind {
    fn square_changed(&mut self, _cell: &IsoCell, sq: (i32, i32, i32)) {
        self.squares.push(sq);
    }
    // TODO(port): `map_collision_square_changed` (the population's
    // `MapCollisionData.squareChanged`).
    fn add_chunk_to_world(&mut self, _cell: &IsoCell, wx: i32, wy: i32) {
        self.chunks.push((wx, wy));
    }
}

impl QueuedPathfind {
    /// `PathfindNative.squareChanged` / `addChunkToWorld` for what was
    /// queued, in order.
    fn apply(self, h: &mut pz_pathfind::host::PathHost, sprites: &IsoSpriteManager) {
        if self.squares.is_empty() && self.chunks.is_empty() {
            return;
        }
        with_world(|w| {
            let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_ref()) else { return };
            let bits_hooks = pz_world::ObjectHooks::new(cell, sprites);
            let mut m = pz_pathfind::mutation_hooks::PathfindMutationHooks { service: h.service_mut(), sprites, meta, bits_hooks: &bits_hooks };
            for sq in self.squares {
                m.square_changed(cell, sq);
            }
            for (wx, wy) in self.chunks {
                m.add_chunk_to_world(cell, wx, wy);
            }
        });
    }
}

/// The characters a finished path request goes to.
struct Movers<'a> {
    zombies: &'a mut Vec<LuaGameCharacter>,
}

impl pz_pathfind::host::MoverAccess for Movers<'_> {
    fn with_mover(&mut self, id: pz_pathfind::host::CharacterId, f: &mut dyn FnMut(&mut dyn pz_pathfind::mover::PathMover)) {
        if let Some(z) = self.zombies.iter_mut().find(|z| z.id == id) {
            f(z);
            return;
        }
        for p in local_players().into_iter().flatten() {
            if p.borrow().id == id {
                if let Ok(mut c) = p.try_borrow_mut() {
                    f(&mut *c);
                }
                return;
            }
        }
    }
}

/// A local player's `PlayerUpdate` events to Lua.
fn fire_player_events(lua: &mut LuaManager, p: &LuaGameCharacterRef, events: Vec<pz_characters::character::player::PlayerEvent>) {
    use pz_characters::character::player::PlayerEvent;
    let pv = character_value(p);
    for e in events {
        match e {
            PlayerEvent::LuaEvent(name) => {
                lua.trigger_event(name, std::slice::from_ref(&pv));
            }
            PlayerEvent::ContextKey { millis } => {
                lua.trigger_event("OnContextKey", &[pv.clone(), Value::Num(millis)]);
            }
            PlayerEvent::PressWalkTo => {
                lua.trigger_event("OnPressWalkTo", &[Value::from(0), Value::from(0), Value::from(0)]);
            }
            // TODO(port): reload/rack, contextual actions, the reticle mode,
            // dirt, the action queue stops and the cursor.
            _ => {}
        }
    }
}

/// `GameWindow.save`'s Lua events, fired once the save has let go of
/// `Core` and `GameTime` (the handlers reach both through Lua).
#[derive(Default)]
struct QuitSaveHooks {
    on_save: bool,
    on_post_save: bool,
}

impl pz_lua_api::world_save::SaveGameHooks for QuitSaveHooks {
    fn on_save(&mut self) {
        self.on_save = true;
    }
    /// Already done by [`IsoWorld::save_on_quit`] before the rest of the
    /// save (the streamer needs the meta grid the save holds).
    fn save_chunks(&mut self, _meta: &IsoMetaGrid) {}
    fn on_post_save(&mut self) {
        self.on_post_save = true;
    }
}

impl IsoWorld {
    /// `IngameState.updateInternal` on `Core.exiting`: the local players'
    /// rows, `GameWindow.save(true)` and `OnPostSave`
    /// (`world_save::save_on_quit`).
    ///
    /// `IsoCell.save`'s chunk files (`ChunkSaveWorker` drained, then
    /// `chunkMap[n].Save()`) and `MapCollisionData.save` run first here:
    /// the streamer and the population read the meta grid and cell the
    /// rest of the save borrows; the files do not depend on each other.
    pub fn save_on_quit(&mut self, lua: &mut LuaManager) {
        self.with_stream(|s, cell, ctx| {
            s.wait_for_saves(ctx);
            s.save_chunk_maps(cell, ctx);
        });
        self.hand_off_events();
        let sim = FrameSim::capture();
        self.with_population(&sim, &mut |pop, host, _| pop.save(&host.zombies.infos()));
        let world_version = pz_classfile::Jar::open(self.game_dir.join("projectzomboid.jar"))
            .ok()
            .and_then(|j| pz_sandbox::save_bin::world_version(&j).ok())
            .unwrap_or(crate::iso_world::WORLD_VERSION);
        let sandbox = crate::iso_world::sandbox(lua).map(|sb| sb.borrow().save_bin(world_version, &self.preset)).unwrap_or_default();
        // `PlayerDB.saveLocalPlayersForce()`: each local player's row
        // (`PlayerData.set`: `IsoPlayer.save`).
        let mut rows: Vec<(usize, pz_lua_api::player_db::PlayerData)> = Vec::new();
        {
            let sm = pz_lua_api::bind_registry::shared_script_manager();
            let chars = shared_characters();
            let mut errors = Vec::new();
            if let Some(tables) = self.save_tables.as_ref() {
                let items: &dyn pz_inventory::env::ItemEnv = match self.items.as_ref() {
                    Some(i) => i,
                    None => &pz_world::save::items::AllItems,
                };
                let env = pz_characters::character::save::SaveEnv { items, tables };
                for (n, p) in local_players().iter().enumerate() {
                    let Some(p) = p else { continue };
                    let r = pz_lua_api::world_save::local_player_data(&mut p.borrow_mut(), self.player_sql_ids[n], &sm, &chars.borrow(), &sim, &env);
                    match r {
                        Ok(d) => rows.push((n, d)),
                        Err(e) => errors.push(format!("PlayerDB.saveLocalPlayersForce: IsoPlayer.save: {e}")),
                    }
                }
            }
            for e in errors {
                self.log_exception(e);
            }
        }
        let mut data: Vec<pz_lua_api::player_db::PlayerData> = rows.iter().map(|(_, d)| d.clone()).collect();
        // The meta grid leaves the shared world while the save holds it.
        let (meta, has_cell) = with_world(|w| (w.meta_grid.take(), w.cell.is_some()));
        let Some(mut meta) = meta else { return };
        let gt = pz_lua_api::bind_climate::shared_game_time();
        let core = pz_lua_api::bind_core::shared_core();
        let report = {
            let (gt, core) = (gt.borrow(), core.borrow());
            let local_players: Vec<bool> = local_players().iter().map(Option::is_some).collect();
            let mut g = pz_lua_api::world_save::SaveGame {
                save_dir: &self.save_dir,
                no_save: core.is_no_save(),
                has_cell,
                game_mode: &core.game_mode,
                game_map: Some(&core.game_map),
                sandbox: &sandbox,
                game_time: &gt,
                core: &core,
                meta: &mut meta,
                meta_world: &self.meta_world,
                spawned_zones: &self.spawned_zones,
                uuids: &mut self.uuids,
                players: self.player_db.as_mut(),
                can_save_players: true,
                // TODO(port): `ReanimatedPlayers.saveReanimatedPlayers`.
                reanimated: None,
                world_gen: &self.world_gen_params,
                instance_tracker: &self.instance_tracker,
                cache_dir: Some(&self.cache_dir),
                meta_tracker: &self.meta_tracker,
                statistics: &self.statistics,
                local_players: &local_players,
                on_game_thread: true,
            };
            let mut hooks = QuitSaveHooks::default();
            let r = pz_lua_api::world_save::save_on_quit(&mut g, &mut data, &mut hooks);
            (r, hooks)
        };
        let (report, hooks) = report;
        with_world(|w| w.meta_grid = Some(meta));
        // A new player keeps the id its first save gave it.
        for ((n, _), d) in rows.iter().zip(&data) {
            self.player_sql_ids[*n] = d.sql_id;
        }
        // `OnSave` (mid-save in the original, before the thumbnail and the
        // other systems' files, none of which read what its handlers do
        // yet) and `OnPostSave`.
        if hooks.on_save {
            lua.trigger_event("OnSave", &[]);
        }
        if hooks.on_post_save {
            lua.trigger_event("OnPostSave", &[]);
        }
        for e in report.errors {
            self.log_update_error(e);
        }
    }

    /// The stops of `IngameState.exit`: `WorldStreamer.stop` (the queued
    /// saves, then every chunk's `removeFromWorld`),
    /// `ZombiePopulationManager.stop`, the pathfinder, then
    /// `PlayerDB.close` and `VehiclesDB2.Reset` (`world_save::close_stores`).
    pub fn stop(&mut self) {
        self.with_stream(|s, cell, ctx| s.stop(cell, ctx));
        self.hand_off_events();
        // TODO(port): `LightingThread.stop`, `AnimalPopulationManager.stop`,
        // `AnimalInstanceManager.stop`, `IsoPuddles.freeHMTextureBuffer`.
        if let Some(p) = self.population.as_mut() {
            p.stop();
        }
        self.pathfind = None;
        let pending = self.streamer.as_mut().map(|s| s.take_vehicle_queue()).unwrap_or_default();
        let r = pz_lua_api::world_save::close_stores(self.player_db.as_mut(), &mut self.vehicles, pending);
        for (_, e) in r.errors {
            self.log_update_error(format!("VehiclesDB2.Reset: {e}"));
        }
        self.regions = None;
    }

    /// `MapCollisionData.instance.updateGameState()` (`GameWindow.logic`).
    pub fn update_game_state(&mut self, no_save: bool, paused: bool) {
        if let Some(p) = self.population.as_mut() {
            p.update_game_state(no_save, paused);
        }
    }
}
