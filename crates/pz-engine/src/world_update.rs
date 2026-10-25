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
use crate::world_models::WorldModels;
use pz_characters::character::state::StateCx;
use pz_characters::character::zombie::population::RealZombies;
use pz_characters::character::SimEvent;
use pz_characters::survivor::SurvivorDescRef;
use pz_kahlua::Value;
use pz_lua_api::bind_character_sim::{character_value, LuaGameCharacter, LuaGameCharacterRef};
use pz_lua_api::bind_characters::{shared_characters, LuaItem};
use pz_lua_api::bind_world::with_world;
use pz_lua_api::LuaManager;
use pz_world::isoregion::IsoRegions;
use pz_world::streamer::{PlayerPos, Serve, StreamEvent};
use pz_world::{GameRand, IsoCell, IsoMetaGrid, IsoSpriteManager};
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

#[cfg(test)]
mod animation_bridge_tests {
    use super::*;
    use pz_characters::character::state::StateWorld;
    use pz_fs::GameFs;
    use std::path::Path;

    #[test]
    fn states_world_reaches_the_drawn_rig_and_consumes_actual_root_motion() {
        let ident = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let mut sd = pz_model::SkinningData {
            bind_pose: vec![ident; 2],
            inverse_bind_pose: vec![ident; 2],
            bone_offset: vec![ident; 2],
            skeleton_hierarchy: vec![-1, 0],
            ..Default::default()
        };
        sd.bone_indices.insert("Root".into(), 0);
        sd.bone_indices.insert("Bip01".into(), 1);
        let mut keys = Vec::new();
        for bone in 0..2 {
            for time in [0.0, 1.0] {
                keys.push(pz_model::Keyframe {
                    bone,
                    bone_name: String::new(),
                    time,
                    position: if bone == 1 {
                        [2.0 * time, 0.0, 0.5 * time]
                    } else {
                        [0.0; 3]
                    },
                    rotation: pz_model::Quat {
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                        w: 1.0,
                    },
                    scale: [1.0; 3],
                });
            }
        }
        sd.animation_clips.insert(
            "synthetic".into(),
            pz_model::AnimationClip {
                name: "synthetic".into(),
                duration: 1.0,
                keep_last_frame: true,
                is_ragdoll: false,
                keyframes: keys,
            },
        );
        let mut player = pz_anim::AnimationPlayer::new(Rc::new(sd));
        let xml = pz_anim::xml::parse(
            "<animNode><m_Name>IdleSynthetic</m_Name><m_AnimName>synthetic</m_AnimName>\
             <m_Looped>true</m_Looped><m_BlendTime>0</m_BlendTime>\
             <m_DeferredBoneName>Bip01</m_DeferredBoneName><m_useDeferredMovement>true</m_useDeferredMovement>\
             <m_Events><m_EventName>ThumpFrame</m_EventName><m_TimePc>0.5</m_TimePc></m_Events></animNode>",
        ).unwrap();
        let mut state = pz_anim::AnimState::new("idle");
        state.add_node(pz_anim::AnimNode::from_element(&xml));
        let mut set = pz_anim::AnimationSet::default();
        set.add_state(state);
        let mut animator = pz_anim::AdvancedAnimator::new();
        animator.set_anim_set(Some(Rc::new(set)));
        let mut vars = pz_anim::MapVariableSource::new();
        animator.set_state(&mut player, &mut vars, "idle", &[]);
        animator.animate(&mut player, &mut vars, 0.25);
        let raw = player.deferred_movement(false);
        assert!(raw.length_squared() > 0.0);
        let expected_angle = player.rendered_angle();
        let mut models = WorldModels::new(
            Path::new("unused-synthetic-root"),
            Rc::new(RefCell::new(GameFs::new("unused-synthetic-root"))),
            Rc::new(RefCell::new(pz_render::TextureManager::new())),
        );
        models.insert_test_animation(7, player, animator);
        let mut cell = IsoCell::default();
        let meta = IsoMetaGrid::empty();
        let mut sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let mut runtime = StatesRuntime::default();
        {
            let mut engine = EngineStates {
                rand: StandardRand,
                movers: Vec::new(),
                models: RefCell::new(Some(&mut models)),
                animation_time_delta: 0.5,
            };
            let mut world = StatesWorldAdapter::new(
                &mut cell,
                &meta,
                &mut sprites,
                ThumpSettings::default(),
                &mut runtime,
                &mut engine,
            );
            assert!(StateWorld::<LuaItem>::animation_player_ready(&world, 7));
            assert!(!StateWorld::<LuaItem>::animation_player_ready(&world, 8));
            assert_eq!(
                StateWorld::<LuaItem>::rendered_angle(&world, 7),
                Some(expected_angle)
            );
            assert_eq!(StateWorld::<LuaItem>::rendered_angle(&world, 8), None);
            assert!(StateWorld::<LuaItem>::contains_any_idle_nodes(&world, 7));
            assert_eq!(StateWorld::<LuaItem>::thumps_this_frame(&mut world, 7), 1);
            assert_eq!(
                StateWorld::<LuaItem>::deferred_movement_length_sq(&world, 7),
                raw.length_squared()
            );
            let stopped = pz_anim::DeferredMovementInput {
                animation_updating_this_frame: false,
                ..Default::default()
            };
            assert_eq!(
                StateWorld::<LuaItem>::deferred_movement_step(&mut world, 7, &stopped),
                None
            );
            assert_eq!(
                StateWorld::<LuaItem>::deferred_movement_length_sq(&world, 7),
                raw.length_squared()
            );
            assert_eq!(
                StateWorld::<LuaItem>::deferred_movement_step(&mut world, 7, &Default::default()),
                Some((raw.x, raw.y))
            );
            StateWorld::<LuaItem>::turn_alerted_enter(&mut world, 7, 0.75);
        }
        let (player, animator) = models.animation_mut(7).unwrap();
        assert_eq!(player.target_angle(), 0.75);
        animator.animate(player, &mut vars, 0.25);
        // The preceding state-world step reset the actual accumulator:
        // the new quarter-second contains one step rather than two.
        let next = player.deferred_movement(false);
        assert!((next.length_squared() - raw.length_squared()).abs() < 1.0e-6);
    }
}

/// The engine's side of the character states: the game's `Rand` and the
/// squares' moving objects outside the zombie list. The rest of
/// `StatesEngine` (other characters by id, LOS, corpses, sounds, the path
/// host) keeps its defaults: TODO(port): those owners.
struct EngineStates<'a> {
    rand: StandardRand,
    /// The squares the local players stand on (`getCurrentSquare()`),
    /// taken before the frame lends any of them out. TODO(port): animals.
    movers: Vec<(i32, i32, i32)>,
    /// The same animation players that the world draws, lent for this
    /// states-world call. Interior borrowing permits a non-consuming
    /// root-motion query through StateWorld's immutable query method.
    models: RefCell<Option<&'a mut WorldModels>>,
    animation_time_delta: f32,
}

impl StatesEngine for EngineStates<'_> {
    fn rand(&mut self) -> &mut dyn GameRand {
        &mut self.rand
    }
    /// `!square.getMovingObjects().isEmpty()` for the players and animals;
    /// the population host counts the real zombies itself.
    fn has_moving_objects(&self, sq: (i32, i32, i32)) -> bool {
        self.movers.contains(&sq)
    }

    fn thumps_this_frame(&mut self, chr: i32) -> i32 {
        let mut models = self.models.borrow_mut();
        let Some((player, animator)) = models.as_deref_mut().and_then(|m| m.animation_mut(chr))
        else {
            return 0;
        };
        pz_anim::CharacterAnim { player, animator }
            .thumps_this_frame(true, self.animation_time_delta)
    }
    fn turn_alerted_enter(&mut self, chr: i32, target_angle: f32) {
        let mut models = self.models.borrow_mut();
        if let Some((player, animator)) = models.as_deref_mut().and_then(|m| m.animation_mut(chr)) {
            pz_anim::CharacterAnim { player, animator }.turn_alerted_enter(target_angle);
        }
    }
    fn deferred_movement_step(
        &mut self,
        chr: i32,
        input: &pz_anim::DeferredMovementInput,
    ) -> Option<(f32, f32)> {
        let mut models = self.models.borrow_mut();
        let (player, animator) = models.as_deref_mut()?.animation_mut(chr)?;
        pz_anim::CharacterAnim { player, animator }.deferred_movement_step(input)
    }
    fn deferred_movement_length_sq(&self, chr: i32) -> f32 {
        let mut models = self.models.borrow_mut();
        models
            .as_deref_mut()
            .and_then(|m| m.animation_mut(chr))
            .map_or(0.0, |(p, _)| p.deferred_movement(false).length_squared())
    }
    fn contains_any_idle_nodes(&self, chr: i32) -> bool {
        self.models
            .borrow()
            .as_deref()
            .and_then(|m| m.animation(chr))
            .is_some_and(|(_, a)| a.contains_any_idle_nodes())
    }
    fn animation_player_ready(&self, chr: i32) -> bool {
        // A rig is inserted only after loading the actual body mesh's
        // skinning data. Texture preparation is a separate readiness.
        self.models
            .borrow()
            .as_deref()
            .and_then(|m| m.animation(chr))
            .is_some()
    }
    fn rendered_angle(&self, chr: i32) -> Option<f32> {
        self.models
            .borrow()
            .as_deref()
            .and_then(|m| m.animation(chr))
            .map(|(p, _)| p.rendered_angle())
    }
    fn primary_hand_model_ready(&self, chr: i32) -> bool {
        self.models
            .borrow()
            .as_deref()
            .is_some_and(|m| m.primary_hand_model_ready(chr))
    }
}

/// The local players' current squares, for [`EngineStates::movers`].
fn player_squares() -> Vec<(i32, i32, i32)> {
    local_players()
        .iter()
        .flatten()
        .filter_map(|p| p.try_borrow().ok().and_then(|c| c.path.current_square))
        .collect()
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
    sprites: &Rc<RefCell<IsoSpriteManager>>,
    runtime: &mut StatesRuntime,
    models: Option<&mut WorldModels>,
    sim: &FrameSim,
    f: impl FnOnce(&mut StatesWorldAdapter<'_>) -> R,
) -> Option<R> {
    let movers = player_squares();
    let animation_time_delta = sim
        .game_time
        .borrow()
        .get_time_delta(&pz_lua_api::bind_climate::LuaClimateContext::new());
    with_world(|w| {
        let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else {
            return None;
        };
        let mut engine = EngineStates {
            rand: StandardRand,
            movers,
            models: RefCell::new(models),
            animation_time_delta,
        };
        let mut sprites = sprites.borrow_mut();
        let mut adapter = StatesWorldAdapter::new(
            cell,
            meta,
            &mut sprites,
            ThumpSettings::default(),
            runtime,
            &mut engine,
        );
        Some(f(&mut adapter))
    })
}

/// `SurvivorFactory.CreateSurvivor()` for a new real zombie.
fn create_survivor(
    chars: &RefCell<pz_characters::Characters>,
) -> pz_scripts::ScriptResult<SurvivorDescRef<LuaItem>> {
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
                    r.update(cell, &sp.borrow(), player);
                }
            });
        }
        // TODO(port): `HaloTextHelper.update`, `CollisionManager.ResolveContacts`.
        self.postupdate_characters(lua, sim);
        // `IsoGameCharacter.postUpdateAnimating` follows movement and
        // action-context updates, before the rest of UpdateStuff.
        self.update_models(lua, sim);
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
            pz_lua_api::bind_players::set_player_instance(Some(p.clone()));
            pz_lua_api::timed_actions::sync_flags(&p);
            let r = {
                let mut chr = p.borrow_mut();
                chr.preupdate_player();
                chr.frame_step(&input);
                let IsoWorld {
                    sprites,
                    states_runtime,
                    action_groups,
                    models,
                    ..
                } = self;
                let Some(sprites) = sprites.as_mut() else {
                    return;
                };
                with_states(sprites, states_runtime, models.as_mut(), sim, |w| {
                    fill_square_environment(&mut chr, w.cell);
                    pz_lua_api::bind_states::with_state_registry(|reg| {
                        let chars = chars.borrow();
                        let mut cx = StateCx::new(&sm, &chars, sim, w);
                        chr.update_player_before_actions(&input, reg, action_groups, &mut cx)
                    })
                })
            };
            match r {
                Some(Ok((mut u, resume))) => {
                    fire_player_events(lua, &p, std::mem::take(&mut u.events));
                    self.fire_sim_events(lua, Some(&p), sim);
                    if resume {
                        pz_lua_api::timed_actions::update(&mut lua.thread, &p);
                        let IsoWorld { sprites, states_runtime, models, .. } = self;
                        if let Some(sprites) = sprites.as_mut() {
                            let result = with_states(sprites, states_runtime, models.as_mut(), sim, |w| {
                                pz_lua_api::bind_states::with_state_registry(|_| {
                                    let chars = chars.borrow();
                                    let mut cx = StateCx::new(&sm, &chars, sim, w);
                                    p.borrow_mut().update_player_after_actions(&mut u, &mut cx)
                                })
                            });
                            if let Some(Err(e)) = result { self.log_update_error(format!("IsoPlayer.update: {e}")); }
                        }
                        fire_player_events(lua, &p, u.events);
                    }
                }
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
        let IsoWorld {
            sprites,
            states_runtime,
            action_groups,
            zombies,
            models,
            ..
        } = self;
        let Some(sprites) = sprites.as_mut() else {
            return;
        };
        let errors = with_states(sprites, states_runtime, models.as_mut(), sim, |w| {
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
        let IsoWorld {
            sprites,
            states_runtime,
            zombies,
            models,
            ..
        } = self;
        let Some(sprites) = sprites.as_mut() else {
            return;
        };
        with_states(sprites, states_runtime, models.as_mut(), sim, |w| {
            let chars = chars.borrow();
            for p in players.iter().flatten() {
                pz_lua_api::bind_players::set_player_instance(Some(p.clone()));
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
    fn fire_sim_events(
        &mut self,
        lua: &mut LuaManager,
        chr: Option<&LuaGameCharacterRef>,
        sim: &FrameSim,
    ) {
        let cv = chr.map(character_value).unwrap_or_default();
        for e in sim.take_events() {
            match e {
                SimEvent::ContainerUpdate => {
                    lua.trigger_event("OnContainerUpdate", &[]);
                }
                SimEvent::PlayerGetDamage { kind, amount } if chr.is_some() => {
                    lua.trigger_event(
                        "OnPlayerGetDamage",
                        &[cv.clone(), Value::str(kind), Value::Num(f64::from(amount))],
                    );
                }
                SimEvent::ForceStopAction if chr.is_some() => {
                    pz_lua_api::timed_actions::force_stop(chr.unwrap());
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

    /// `ModelManager.update`: advance the live models after simulation,
    /// delivering animation events to the same character and state world.
    /// Asset resolution runs before borrowing the character registry.
    pub fn update_models(&mut self, lua: &mut LuaManager, sim: &FrameSim) {
        let mut models = self.models.take().unwrap_or_else(|| {
            crate::world_models::WorldModels::new(
                &self.game_dir,
                lua.fs.clone(),
                pz_lua_api::bind_render::textures(),
            )
        });
        let dt = sim
            .game_time
            .borrow()
            .get_time_delta(&pz_lua_api::bind_climate::LuaClimateContext::new());
        let days = (sim.world_age_hours / 24.0) as f32;
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let chars = shared_characters();
        let mut ids = Vec::new();
        let mut errors = Vec::new();
        for p in local_players().into_iter().flatten() {
            pz_lua_api::bind_players::set_player_instance(Some(p.clone()));
            let prepared = {
                let mut chr = p.borrow_mut();
                ids.push(chr.id);
                models.update_character_before_action_modifiers(&mut chr, dt, days)
            };
            let events = prepared.and_then(|()| {
                let initial = models.action_movement_modifiers(&p.borrow());
                let modifiers = initial.map(|m| pz_lua_api::timed_actions::movement_modifiers(&mut lua.thread, &p, m));
                models.finish_character_update(&mut p.borrow_mut(), dt, modifiers)
            });
            match events {
                Ok(events) if !events.is_empty() => {
                    for event in events {
                      if let Some(sprites) = self.sprites.as_ref() {
                        with_states(
                            sprites,
                            &mut self.states_runtime,
                            Some(&mut models),
                            sim,
                            |w| {
                                let chars = chars.borrow();
                                let mut cx = StateCx::new(&sm, &chars, sim, w);
                                let mut chr = p.borrow_mut();
                                chr.on_anim_event(&event, &mut cx);
                            },
                        );
                      }
                      pz_lua_api::timed_actions::anim_event(&mut lua.thread, &p, &event.event.event_name, event.event.parameter_value.as_deref());
                    }
                }
                Err(e) => errors.push(e),
                _ => {}
            }
            self.fire_sim_events(lua, Some(&p), sim);
        }
        for chr in &mut self.zombies {
            ids.push(chr.id);
            match models.update_character(chr, dt, days) {
                Ok(events) if !events.is_empty() => {
                    if let Some(sprites) = self.sprites.as_ref() {
                        with_states(
                            sprites,
                            &mut self.states_runtime,
                            Some(&mut models),
                            sim,
                            |w| {
                                let chars = chars.borrow();
                                let mut cx = StateCx::new(&sm, &chars, sim, w);
                                for event in events {
                                    chr.on_anim_event(&event, &mut cx);
                                }
                            },
                        );
                    }
                }
                Err(e) => errors.push(e),
                _ => {}
            }
        }
        self.fire_sim_events(lua, None, sim);
        models.retain_characters(ids);
        self.models = Some(models);
        for e in errors {
            self.log_update_error(format!("ModelManager.update: {e}"));
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
        let IsoWorld {
            population,
            streamer,
            sprites,
            zombies,
            next_object_id,
            rand,
            key_spawn_chance_d100,
            states_runtime,
            models,
            ..
        } = self;
        let (Some(pop), Some(streamer), Some(sprites)) =
            (population.as_mut(), streamer.as_ref(), sprites.as_mut())
        else {
            return None;
        };
        let maps = pz_world_popman::player_chunk_maps(streamer);
        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let chars = shared_characters();
        let mut sprites = sprites.borrow_mut();
        let water_flag = sprites.tables.flag.water;
        let movers = player_squares();
        with_world(|w| {
            let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else {
                return None;
            };
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
            let animation_time_delta = sim
                .game_time
                .borrow()
                .get_time_delta(&pz_lua_api::bind_climate::LuaClimateContext::new());
            let mut engine = EngineStates {
                rand: StandardRand,
                movers,
                models: RefCell::new(models.as_mut()),
                animation_time_delta,
            };
            let mut adapter = StatesWorldAdapter::new(
                cell,
                meta,
                &mut sprites,
                ThumpSettings::default(),
                states_runtime,
                &mut engine,
            );
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
        let Some(s) = self.streamer.as_ref() else {
            return;
        };
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
    pub fn update_fire(
        &mut self,
        sim: &FrameSim,
        settings: pz_world::fire::FireSettings,
    ) -> Result<(), String> {
        if self.fire_flags.is_none() {
            let Some(sprites) = self.sprites.as_ref() else {
                return Ok(());
            };
            let flags = pz_world::fire::manager::FireFlags::resolve(&sprites.borrow().tables);
            match flags {
                Ok(f) => self.fire_flags = Some(f),
                Err(e) => {
                    self.log_update_error(format!("IsoFireManager: {e}"));
                    return Ok(());
                }
            }
        }
        if self.door_tables.is_none() {
            match pz_classfile::Jar::open(self.game_dir.join("projectzomboid.jar"))
                .map_err(|e| e.to_string())
                .and_then(|j| pz_world::square_mutation::DoorTables::from_jar(&j))
            {
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
        let IsoWorld {
            fire,
            fire_flags,
            door_tables,
            sprites,
            rand,
            regions,
            render,
            pathfind,
            ..
        } = self;
        let (Some(sprites), Some(flags), Some(doors)) =
            (sprites.as_mut(), *fire_flags, door_tables.as_ref())
        else {
            return Ok(());
        };
        let mut sprites = sprites.borrow_mut();
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
            let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else {
                return Ok(());
            };
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
            let mut fw = pz_world::fire::FireWorld {
                cell,
                meta,
                sprites: &mut *sprites,
                mutation,
                flags,
                settings,
                hooks: &mut hooks,
            };
            fire.update(&mut fw, rand, frame)
        });
        if let Some(h) = pathfind.as_mut() {
            pf.apply(h, &sprites);
        }
        drop(sprites);
        if let Err(e) = &r {
            self.log_exception(e.clone());
        }
        r
    }

    /// `PolygonalMap2.instance.updateMain()` (`PathfindNative.updateMain`):
    /// the pathfinding thread's pass, then the finished requests handed to
    /// their characters.
    pub fn update_pathfinding(&mut self) {
        let IsoWorld {
            pathfind, zombies, ..
        } = self;
        let Some(h) = pathfind.as_mut() else { return };
        h.update_thread();
        // TODO(port): the cell's vehicles and `SeatingManager`.
        let mut movers = Movers { zombies };
        h.update_main(
            &pz_pathfind::game_vehicle::NoVehicles,
            &pz_pathfind::behavior::hooks::NoSeating,
            &mut movers,
        );
    }

    /// Takes the streamer's events and runs the world's side of each
    /// chunk that was handed off (`doLoadGridsquare`: the population, the
    /// pathfinder, the renderer) or left the world (`removeFromWorld`).
    pub fn hand_off_events(&mut self) {
        let Some(events) = self.streamer.as_mut().map(|s| s.take_events()) else {
            return;
        };
        for e in events {
            match e {
                StreamEvent::HandedOff { wx, wy } => self.chunk_added(wx, wy),
                StreamEvent::Unloaded { wx, wy } | StreamEvent::Removed { wx, wy } => {
                    self.chunk_removed(wx, wy)
                }
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
        let IsoWorld {
            population,
            sprites,
            pathfind,
            chunk_load_id,
            ..
        } = self;
        let Some(sp) = sprites.as_ref() else { return };
        *chunk_load_id = chunk_load_id.wrapping_add(1);
        let load_id = *chunk_load_id;
        with_world(|w| {
            let (Some(cell), Some(meta)) = (w.cell.as_ref(), w.meta_grid.as_ref()) else {
                return;
            };
            if let Some(p) = population.as_mut() {
                p.add_chunk_to_world(cell, meta, &sp.borrow(), wx, wy);
            }
            if let Some(h) = pathfind.as_mut() {
                add_chunk_to_pathfinder(h, cell, meta, &sp.borrow(), wx, wy, load_id);
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
    let stress = cur.map_or(0.0, |(x, y, z)| {
        pz_lua_api::bind_zombie::shared_world_sounds()
            .borrow()
            .get_stress_from_sounds(x, y, z)
    });
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
    let Some(chunk) = cell.chunks.get(&(wx, wy)) else {
        return;
    };
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
            let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_ref()) else {
                return;
            };
            let bits_hooks = pz_world::ObjectHooks::new(cell, sprites);
            let mut m = pz_pathfind::mutation_hooks::PathfindMutationHooks {
                service: h.service_mut(),
                sprites,
                meta,
                bits_hooks: &bits_hooks,
            };
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
    fn with_mover(
        &mut self,
        id: pz_pathfind::host::CharacterId,
        f: &mut dyn FnMut(&mut dyn pz_pathfind::mover::PathMover),
    ) {
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
fn fire_player_events(
    lua: &mut LuaManager,
    p: &LuaGameCharacterRef,
    events: Vec<pz_characters::character::player::PlayerEvent>,
) {
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
                lua.trigger_event(
                    "OnPressWalkTo",
                    &[Value::from(0), Value::from(0), Value::from(0)],
                );
            }
            PlayerEvent::StopActionQueue(reason) => {
                pz_lua_api::timed_actions::cancel(&mut lua.thread, p, reason);
            }
            // TODO(port): reload/rack, contextual actions, the reticle mode,
            // dirt and the cursor.
            _ => {}
        }
    }
}

/// The engine saves chunks and population between the borrow-scoped file
/// phases below. Lua events run directly between those scopes, while
/// their live world, `Core` and `GameTime` remain accessible.
#[derive(Default)]
struct QuitSaveHooks;

impl pz_lua_api::world_save::SaveGameHooks for QuitSaveHooks {
    /// Already done after `OnSave` by [`IsoWorld::save_on_quit`], before
    /// the cell's files (the streamer borrows the same live meta grid).
    fn save_chunks(&mut self, _meta: &IsoMetaGrid) {}
}

impl IsoWorld {
    /// `IngameState.updateInternal` on `Core.exiting`: the local players'
    /// rows, `GameWindow.save(true)` and `OnPostSave`
    /// (`world_save::save_on_quit`).
    ///
    /// Each file phase releases the singletons before Lua runs. `OnSave`
    /// observes the live world after the prelude and before the chunks,
    /// cell and population, so its changes reach the saved files.
    pub fn save_on_quit(&mut self, lua: &mut LuaManager) {
        use pz_lua_api::world_save::{
            save_game_before_event, save_game_cell, save_game_finish, SaveGame, SaveReport,
        };

        // Lend the live meta grid only for one file phase. No reference
        // to it, `GameTime` or `Core` crosses either Lua event.
        fn with_game<R>(
            world: &mut IsoWorld,
            sandbox: &[u8],
            f: impl FnOnce(&mut SaveGame<'_>) -> R,
        ) -> Option<R> {
            let gt = pz_lua_api::bind_climate::shared_game_time();
            let core = pz_lua_api::bind_core::shared_core();
            with_world(|w| {
                let has_cell = w.cell.is_some();
                let meta = w.meta_grid.as_mut()?;
                let (gt, core) = (gt.borrow(), core.borrow());
                let world_gen_params = world.world_gen_params.borrow();
                let players: Vec<bool> = local_players().iter().map(Option::is_some).collect();
                let mut g = SaveGame {
                    save_dir: &world.save_dir,
                    no_save: core.is_no_save(),
                    has_cell,
                    game_mode: &core.game_mode,
                    game_map: Some(&core.game_map),
                    sandbox,
                    game_time: &gt,
                    core: &core,
                    meta,
                    meta_world: &world.meta_world,
                    spawned_zones: &world.spawned_zones,
                    uuids: &mut world.uuids,
                    players: world.player_db.as_mut(),
                    // `saveLocalPlayersForce` below already queued them.
                    can_save_players: false,
                    reanimated: None,
                    world_gen: &world_gen_params,
                    instance_tracker: &world.instance_tracker,
                    cache_dir: Some(&world.cache_dir),
                    meta_tracker: &world.meta_tracker,
                    statistics: &world.statistics,
                    local_players: &players,
                    on_game_thread: true,
                };
                Some(f(&mut g))
            })
        }

        let sim = FrameSim::capture();
        let world_version = pz_classfile::Jar::open(self.game_dir.join("projectzomboid.jar"))
            .ok()
            .and_then(|j| pz_sandbox::save_bin::world_version(&j).ok())
            .unwrap_or(crate::iso_world::WORLD_VERSION);
        let sandbox = crate::iso_world::sandbox(lua)
            .map(|sb| sb.borrow().save_bin(world_version, &self.preset))
            .unwrap_or_default();
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
                    let r = pz_lua_api::world_save::local_player_data(
                        &mut p.borrow_mut(),
                        self.player_sql_ids[n],
                        &sm,
                        &chars.borrow(),
                        &sim,
                        &env,
                    );
                    match r {
                        Ok(d) => rows.push((n, d)),
                        Err(e) => errors.push(format!(
                            "PlayerDB.saveLocalPlayersForce: IsoPlayer.save: {e}"
                        )),
                    }
                }
            }
            for e in errors {
                self.log_exception(e);
            }
        }
        let mut data: Vec<pz_lua_api::player_db::PlayerData> =
            rows.iter().map(|(_, d)| d.clone()).collect();
        if let Some(db) = self.player_db.as_mut() {
            for player in &mut data {
                db.save_player(player);
            }
        }
        // A new player keeps the id its first save gave it.
        for ((n, _), d) in rows.iter().zip(&data) {
            self.player_sql_ids[*n] = d.sql_id;
        }
        let mut report = match with_game(self, &sandbox, |g| save_game_before_event(g)) {
            Some(Ok(mut report)) => {
                if report.saved {
                    lua.trigger_event("OnSave", &[]);
                    self.with_stream(|s, cell, ctx| {
                        s.wait_for_saves(ctx);
                        s.save_chunk_maps(cell, ctx);
                    });
                    self.hand_off_events();
                    let mut hooks = QuitSaveHooks;
                    with_game(self, &sandbox, |g| {
                        save_game_cell(g, &mut hooks, &mut report)
                    });
                    let sim = FrameSim::capture();
                    self.with_population(&sim, &mut |pop, host, _| pop.save(&host.zombies.infos()));
                    with_game(self, &sandbox, |g| {
                        save_game_finish(g, &mut hooks, &mut report)
                    });
                }
                report
            }
            Some(Err(e)) => {
                eprintln!("ERROR: GameWindow.save: {e}");
                SaveReport {
                    saved: true,
                    errors: vec![format!("GameWindow.save: {e}")],
                    ..Default::default()
                }
            }
            None => SaveReport::default(),
        };
        // Quitting delivers this even when saving is disabled or the
        // prelude threw, matching `world_save::save_on_quit`.
        lua.trigger_event("OnPostSave", &[]);
        for e in report.errors.drain(..) {
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
        let pending = self
            .streamer
            .as_mut()
            .map(|s| s.take_vehicle_queue())
            .unwrap_or_default();
        let r = pz_lua_api::world_save::close_stores(
            self.player_db.as_mut(),
            &mut self.vehicles,
            pending,
        );
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

#[cfg(test)]
mod quit_save_tests {
    use super::*;
    use pz_core::core::Core;
    use pz_core::game_time::GameTime;
    use pz_kahlua::NativeFunction;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    struct Fixture {
        dir: PathBuf,
        old_core: Rc<RefCell<Core>>,
        old_time: Rc<RefCell<GameTime>>,
        old_world: pz_lua_api::bind_world::WorldState,
    }

    impl Fixture {
        fn new(no_save: bool, mode: &str, has_cell: bool) -> (Self, IsoWorld, LuaManager) {
            let dir = std::env::temp_dir().join(format!(
                "pz-engine-quit-save-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            let old_core = pz_lua_api::bind_core::shared_core();
            let old_time = pz_lua_api::bind_climate::shared_game_time();
            let old_world = with_world(std::mem::take);
            let mut core = Core::default();
            core.set_no_save(no_save);
            core.set_game_mode(mode);
            pz_lua_api::bind_core::install_core(Rc::new(RefCell::new(core)));
            pz_lua_api::bind_climate::install_game_time(Rc::new(RefCell::new(GameTime::default())));
            pz_lua_api::bind_players::reset();
            pz_lua_api::bind_world::set_meta_grid(Some(IsoMetaGrid::with_bounds(0, 0, 1, 1)));
            pz_lua_api::bind_world::set_cell(has_cell.then(IsoCell::default));
            let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&dir)));
            fs.borrow_mut().set_cache_dir(&dir);
            let lua = LuaManager::init(fs).unwrap();
            let world = IsoWorld::new(&dir, &dir, dir.clone());
            (
                Fixture {
                    dir,
                    old_core,
                    old_time,
                    old_world,
                },
                world,
                lua,
            )
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            with_world(|w| *w = std::mem::take(&mut self.old_world));
            pz_lua_api::bind_core::install_core(self.old_core.clone());
            pz_lua_api::bind_climate::install_game_time(self.old_time.clone());
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn run_lua(lua: &mut LuaManager, source: &str) {
        // `assert` is defined by the installed game's stdlib.lua, which
        // this synthetic fixture deliberately does not contain.
        let source = format!(
            "local function check(c) if not c then error('save check failed') end end\n{source}"
        );
        let f = pz_kahlua::stdlib::load_string(
            &source,
            Some("quit-save-test"),
            lua.thread.globals().clone(),
        )
        .unwrap();
        lua.thread.pcall(&f, &[]).unwrap();
    }

    #[test]
    fn on_save_has_live_unborrowed_owners_and_its_mutations_are_saved() {
        let (fixture, mut world, mut lua) = Fixture::new(false, "Sandbox", true);
        let dir = fixture.dir.clone();
        lua.thread.globals().set_str(
            "checkSaveBoundary",
            Value::Native(NativeFunction::new("checkSaveBoundary", move |_| {
                assert!(dir.join("map_ver.bin").is_file());
                assert!(dir.join("statistics.bin").is_file());
                assert!(!dir.join("map.bin").exists());
                assert!(!dir.join("map_t.bin").exists());
                Ok(0)
            })),
        );
        run_lua(
            &mut lua,
            r#"
            saveCount, postSaveCount = 0, 0
            Events.OnSave.Add(function()
                checkSaveBoundary()
                check(getWorld():getCell() ~= nil)
                check(getWorld():getMetaGrid() ~= nil)
                getGameTime():setTimeOfDay(17.25)
                getCore():setPoisonousBerry("Invented.SaveBerry")
                saveCount = saveCount + 1
            end)
            Events.OnPostSave.Add(function()
                check(getGameTime():getTimeOfDay() == 17.25)
                postSaveCount = postSaveCount + 1
            end)
        "#,
        );
        world.save_on_quit(&mut lua);
        assert_eq!(
            lua.thread.globals().get_str("saveCount").as_num(),
            Some(1.0)
        );
        assert_eq!(
            lua.thread.globals().get_str("postSaveCount").as_num(),
            Some(1.0)
        );
        let mut loaded_time = GameTime::default();
        let mut loaded_core = Core::default();
        pz_core::game_time_save::load_map_t_file(
            &mut loaded_time,
            &mut loaded_core,
            &fixture.dir,
            -1,
        )
        .unwrap();
        assert_eq!(loaded_time.time_of_day, 17.25);
        assert_eq!(
            loaded_core.get_poisonous_berry().as_deref(),
            Some("Invented.SaveBerry")
        );
    }

    #[test]
    fn skipped_and_failed_quit_saves_only_deliver_on_post_save() {
        for (no_save, mode, has_cell, fail_prefix) in [
            (true, "Sandbox", true, false),
            (false, "Tutorial", true, false),
            (false, "LastStand", true, false),
            (false, "Sandbox", false, false),
            (false, "Sandbox", true, true),
        ] {
            let (fixture, mut world, mut lua) = Fixture::new(no_save, mode, has_cell);
            if fail_prefix {
                std::fs::create_dir(fixture.dir.join("map_ver.bin")).unwrap();
            }
            run_lua(
                &mut lua,
                r#"
                saveCount, postSaveCount = 0, 0
                Events.OnSave.Add(function() saveCount = saveCount + 1 end)
                Events.OnPostSave.Add(function() postSaveCount = postSaveCount + 1 end)
            "#,
            );
            world.save_on_quit(&mut lua);
            assert_eq!(
                lua.thread.globals().get_str("saveCount").as_num(),
                Some(0.0),
                "{no_save} {mode} {has_cell} {fail_prefix}"
            );
            assert_eq!(
                lua.thread.globals().get_str("postSaveCount").as_num(),
                Some(1.0)
            );
            assert!(!fixture.dir.join("map.bin").exists());
            assert!(!fixture.dir.join("map_t.bin").exists());
            assert_eq!(
                world
                    .errors
                    .iter()
                    .any(|e| e.starts_with("GameWindow.save:")),
                fail_prefix
            );
        }
    }
}
