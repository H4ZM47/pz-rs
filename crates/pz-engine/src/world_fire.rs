//! `IsoFireManager` engine integration: 20-slot fire sound emitters
//! (`FmodSoundEmitter` + `ParameterFireSize`), `FireLightLinks` lampposts,
//! permanent-fire `IsoHeatSource` tracking (feeding character ambient heat),
//! world sounds, `OnNewFire` / `OnGridBurnt` Lua event dispatch, and cleanup
//! on `IngameState.exit`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use pz_audio::world_sound::SoundSource;
use pz_core::weather::HeatSource;
use pz_lua_api::bind_world::with_world;
use pz_lua_api::LuaManager;
use pz_sound::emitter::ParentObject;
use pz_sound::params::FmodParameter;
use pz_sound::world_emitters::EmitterRef;
use pz_world::fire::manager::{FireHooks, FireRef};
use pz_world::fire::sound::{FireSoundEvent, FireSoundHooks, FIRE_SOUND_SLOTS};
use pz_world::fire::{IsoFire, HEAT_SOURCE_TEMPERATURE};
use pz_world::IsoCell;
use pz_world_render::FireLightLinks;

use crate::world_render::WorldRender;

const MAX_PLAYERS: usize = 4;

/// Closest living local player's squared distance to `(x, y, z)`
/// (`FMODParameterUtils.getClosestListenerDistanceSquared`).
pub(crate) fn closest_fire_listener_distance_sq(x: f32, y: f32, z: i32) -> f32 {
    let mut best = f32::MAX;
    for slot in 0..MAX_PLAYERS {
        let Some(p) = pz_lua_api::bind_players::player(slot) else {
            continue;
        };
        let Ok(c) = p.try_borrow() else {
            continue;
        };
        if c.is_dead() {
            continue;
        }
        let dx = c.vars.x - x;
        let dy = c.vars.y - y;
        let dz = (c.vars.z - z as f32) * 3.0;
        let d2 = dx * dx + dy * dy + dz * dz;
        if d2 < best {
            best = d2;
        }
    }
    if best == f32::MAX {
        0.0
    } else {
        best
    }
}

/// One of `IsoFireManager.FireSounds`'s 20 emitter slots.
struct FireSlotEmitter {
    emitter: EmitterRef,
    fire_size: Rc<Cell<f32>>,
    playing: Option<(FireSoundEvent, i64)>,
}

/// Pool of 20 owned world emitters with local `FireSize` parameters
/// (`IsoFireManager.FireSounds.Slot`).
pub(crate) struct FireSoundSlots {
    slots: [Option<FireSlotEmitter>; FIRE_SOUND_SLOTS],
}

impl Default for FireSoundSlots {
    fn default() -> Self {
        Self {
            slots: std::array::from_fn(|_| None),
        }
    }
}

impl FireSoundSlots {
    /// Stops and returns every acquired fire slot emitter back to the shared
    /// `SoundSystem::world_emitters` pool.
    pub fn reset(&mut self) {
        let drained: Vec<EmitterRef> = self
            .slots
            .iter_mut()
            .filter_map(|s| s.take().map(|e| e.emitter))
            .collect();
        if drained.is_empty() {
            return;
        }
        let sound = pz_lua_api::bind_audio::sound_system();
        let mut sys = sound.borrow_mut();
        sys.with_world_emitters(|we, ctx| {
            for e in drained {
                e.borrow_mut().stop_all(ctx);
                e.borrow_mut().clear_parameters();
                we.return_ownership(e);
            }
        });
    }

    /// Number of sound slots currently holding a live emitter (tests).
    #[cfg(test)]
    pub fn active_slot_count(&self) -> usize {
        self.slots.iter().filter(|s| s.is_some()).count()
    }
}

impl FireSoundHooks for FireSoundSlots {
    fn fire_slot_play(
        &mut self,
        slot: usize,
        pos: (f32, f32, i32),
        fire_size: i32,
        event: Option<FireSoundEvent>,
    ) {
        let Some(slot_entry) = self.slots.get_mut(slot) else {
            return;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        let mut sys = sound.borrow_mut();
        sys.with_world_emitters(|we, ctx| {
            let entry = slot_entry.get_or_insert_with(|| {
                let emitter = we.get_free();
                we.take_ownership(&emitter);
                let size_cell = Rc::new(Cell::new(0.0f32));
                let size_src = size_cell.clone();
                {
                    let mut em = emitter.borrow_mut();
                    let desc = ctx.backend.parameter_description("FireSize");
                    em.add_parameter(FmodParameter::local(
                        "FireSize",
                        desc,
                        Box::new(move || size_src.get()),
                    ));
                }
                FireSlotEmitter {
                    emitter,
                    fire_size: size_cell,
                    playing: None,
                }
            });
            entry.fire_size.set(fire_size as f32);
            let mut em = entry.emitter.borrow_mut();
            em.set_pos(pos.0, pos.1, pos.2 as f32);
            if let Some(ev) = event {
                let still_playing = entry
                    .playing
                    .is_some_and(|(cur, h)| cur == ev && em.is_playing(h));
                if !still_playing {
                    if let Some((_, old)) = entry.playing.take() {
                        if em.has_sustain_points(old, ctx.sounds) {
                            em.trigger_cue(old, ctx);
                        } else {
                            em.stop_all(ctx);
                        }
                    }
                    let handle = em.play_sound_impl(ev.name(), ParentObject::None, ctx);
                    if handle != 0 {
                        entry.playing = Some((ev, handle));
                    }
                }
            }
            em.tick(ctx);
        });
    }

    fn fire_slot_stop(&mut self, slot: usize) {
        let Some(Some(entry)) = self.slots.get_mut(slot) else {
            return;
        };
        let sound = pz_lua_api::bind_audio::sound_system();
        let mut sys = sound.borrow_mut();
        sys.with_world_emitters(|_we, ctx| {
            let mut em = entry.emitter.borrow_mut();
            if let Some((_, handle)) = entry.playing.take() {
                if em.has_sustain_points(handle, ctx.sounds) {
                    em.trigger_cue(handle, ctx);
                } else {
                    em.stop_all(ctx);
                }
            } else if !em.is_empty() {
                em.tick(ctx);
            }
        });
    }
}

/// Active `IsoHeatSource` maintained by a permanent fire (`IsoFire::heat_source`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FireHeatEntry {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i32,
    pub temperature: i32,
}

/// Deferred light update recorded during `FireHooks` while `WorldRender` is
/// borrowed for `RenderSquareHooks`, then applied to `WorldLighting`.
pub(crate) enum FireLightOp {
    Updated(FireRef, IsoFire),
    Removed(FireRef),
}

/// Engine-side state paired with [`IsoWorld::fire`] (`IsoFireManager`):
/// lamppost light links, permanent-fire heat sources, and the 20 sound slots.
#[derive(Default)]
pub struct WorldFireState {
    pub(crate) lights: FireLightLinks,
    pub(crate) heat_sources: HashMap<FireRef, FireHeatEntry>,
    pub(crate) sound_slots: FireSoundSlots,
}

impl WorldFireState {
    /// Builds [`HeatSource`] entries relative to `(x, y, z)` for
    /// `CharacterEnvironment::heat_sources` (`IsoCell.getHeatSourceHighestTemperature`).
    pub fn heat_sources_for(&self, x: f32, y: f32, z: f32, cell: &IsoCell) -> Vec<HeatSource> {
        if self.heat_sources.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(self.heat_sources.len());
        let iz = pz_world::java::fastfloor(z);
        for hs in self.heat_sources.values() {
            if hs.z != iz || hs.radius <= 0 {
                continue;
            }
            let dx = x - (hs.x as f32 + 0.5);
            let dy = y - (hs.y as f32 + 0.5);
            let dist_sq = dx * dx + dy * dy;
            let r = hs.radius as f32;
            if dist_sq >= r * r {
                continue;
            }
            let source_in_room = cell
                .get_grid_square(hs.x, hs.y, hs.z)
                .map(|sq| sq.borrow().room_id != -1);
            out.push(HeatSource {
                dist_sq,
                radius: hs.radius,
                temperature: hs.temperature,
                source_in_room,
                line_clear: true,
            });
        }
        out
    }

    /// Applies queued fire lamppost updates to [`WorldRender::lighting`].
    pub(crate) fn apply_light_ops(
        &mut self,
        ops: Vec<FireLightOp>,
        render: &Rc<RefCell<Option<WorldRender>>>,
    ) {
        if ops.is_empty() {
            return;
        }
        let Ok(mut r) = render.try_borrow_mut() else {
            return;
        };
        let Some(wr) = r.as_mut() else {
            return;
        };
        for op in ops {
            match op {
                FireLightOp::Updated(fr, fire) => {
                    self.lights.updated(&mut wr.lighting, fr, &fire);
                }
                FireLightOp::Removed(fr) => {
                    self.lights.removed(&mut wr.lighting, fr);
                }
            }
        }
    }

    /// Releases all fire lampposts, heat sources, and sound slot emitters on
    /// `IngameState.exit` / world reset.
    pub fn reset(&mut self, render: &Rc<RefCell<Option<WorldRender>>>) {
        if let Ok(mut r) = render.try_borrow_mut() {
            if let Some(wr) = r.as_mut() {
                for (_, mut link) in self.lights.links.drain() {
                    link.remove(&mut wr.lighting);
                }
            } else {
                self.lights.links.clear();
            }
        } else {
            self.lights.links.clear();
        }
        self.heat_sources.clear();
        self.sound_slots.reset();
    }
}

/// Deferred side effects collected by [`EngineFireHooks`] during a fire or
/// character-fire pass, flushed cleanly once cell/sprite borrows drop.
#[derive(Default)]
pub(crate) struct FireEffectsQueue {
    pub(crate) light_ops: Vec<FireLightOp>,
    pub(crate) world_sounds: Vec<(i32, i32, i32, i32, i32)>,
    pub(crate) play_sounds: Vec<(String, i32, i32, i32, f32, f32)>,
    pub(crate) lua_events: Vec<(String, i32, i32, i32)>,
    pub(crate) generators_off: Vec<(i32, i32, i32)>,
}

impl FireEffectsQueue {
    /// Applies all queued lights, world sounds, one-shot sounds, and Lua
    /// events (`OnNewFire`, `OnGridBurnt`).
    pub(crate) fn flush(
        &mut self,
        state: &mut WorldFireState,
        render: &Rc<RefCell<Option<WorldRender>>>,
        hearing: i32,
        lua: Option<&mut LuaManager>,
    ) {
        let light_ops = std::mem::take(&mut self.light_ops);
        state.apply_light_ops(light_ops, render);

        let world_sounds = std::mem::take(&mut self.world_sounds);
        if !world_sounds.is_empty() {
            let ws = pz_lua_api::bind_zombie::shared_world_sounds();
            with_world(|w| {
                let Some(cell) = w.cell.as_ref() else {
                    return;
                };
                let mut fx = pz_world_sound::NoEffects;
                let mut adapter = pz_world_sound::WorldSoundAdapter::new(cell, hearing, &mut fx);
                let mut mgr = ws.borrow_mut();
                for (x, y, z, radius, volume) in world_sounds {
                    let _ = mgr.add_sound(
                        &mut adapter,
                        Some(SoundSource::default()),
                        x,
                        y,
                        z,
                        radius,
                        volume,
                    );
                }
            });
        }

        let play_sounds = std::mem::take(&mut self.play_sounds);
        if !play_sounds.is_empty() {
            let sound = pz_lua_api::bind_audio::sound_system();
            let mut sys = sound.borrow_mut();
            sys.with_world_emitters(|we, ctx| {
                for (name, x, y, z, _radius, _gain) in play_sounds {
                    let e = we.get_free();
                    let mut em = e.borrow_mut();
                    em.set_pos(x as f32 + 0.5, y as f32 + 0.5, z as f32);
                    em.play_sound_impl(&name, ParentObject::None, ctx);
                }
            });
        }

        let lua_events = std::mem::take(&mut self.lua_events);
        if let Some(lua) = lua {
            for (name, x, y, z) in lua_events {
                let sq = pz_lua_api::bind_world::square_value(x, y, z);
                lua.trigger_event(&name, &[sq]);
            }
        }

        self.generators_off.clear();
    }
}

/// Live [`FireHooks`] implementation for `IsoWorld::update_fire` and
/// `StatesEngine::fire`.
pub(crate) struct EngineFireHooks<'a> {
    pub(crate) state: &'a mut WorldFireState,
    pub(crate) queue: &'a mut FireEffectsQueue,
    pub(crate) object_names: &'a HashMap<i8, String>,
}

impl FireSoundHooks for EngineFireHooks<'_> {
    fn fire_slot_play(
        &mut self,
        slot: usize,
        pos: (f32, f32, i32),
        fire_size: i32,
        event: Option<FireSoundEvent>,
    ) {
        self.state
            .sound_slots
            .fire_slot_play(slot, pos, fire_size, event);
    }

    fn fire_slot_stop(&mut self, slot: usize) {
        self.state.sound_slots.fire_slot_stop(slot);
    }
}

impl FireHooks for EngineFireHooks<'_> {
    fn world_sound(&mut self, x: i32, y: i32, z: i32, radius: i32, volume: i32) {
        self.queue.world_sounds.push((x, y, z, radius, volume));
    }

    fn play_world_sound(&mut self, name: &str, x: i32, y: i32, z: i32, radius: f32, gain: f32) {
        self.queue
            .play_sounds
            .push((name.to_string(), x, y, z, radius, gain));
    }

    fn fire_updated(&mut self, r: FireRef, fire: &IsoFire) {
        if let Some(radius) = fire.heat_source {
            self.state.heat_sources.insert(
                r,
                FireHeatEntry {
                    x: r.x,
                    y: r.y,
                    z: r.z,
                    radius,
                    temperature: HEAT_SOURCE_TEMPERATURE,
                },
            );
        } else {
            self.state.heat_sources.remove(&r);
        }
        self.queue.light_ops.push(FireLightOp::Updated(r, fire.clone()));
    }

    fn fire_removed(&mut self, r: FireRef) {
        self.state.heat_sources.remove(&r);
        self.queue.light_ops.push(FireLightOp::Removed(r));
    }

    fn listener_distance_sq(&self, x: f32, y: f32, z: i32) -> f32 {
        closest_fire_listener_distance_sq(x, y, z)
    }

    fn lua_event(&mut self, name: &str, x: i32, y: i32, z: i32) {
        self.queue.lua_events.push((name.to_string(), x, y, z));
    }

    fn generator_switched_off(&mut self, x: i32, y: i32, z: i32) {
        self.queue.generators_off.push((x, y, z));
    }

    fn dict_object_name(&self, id: i8) -> Option<String> {
        self.object_names.get(&id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::{IsoChunk, IsoGridSquare};

    #[test]
    fn fire_sound_slots_acquire_update_stop_and_reset() {
        let mut slots = FireSoundSlots::default();
        assert_eq!(slots.active_slot_count(), 0);

        slots.fire_slot_play(0, (10.5, 20.5, 0), 2, Some(FireSoundEvent::Fire));
        assert_eq!(slots.active_slot_count(), 1);
        assert_eq!(slots.slots[0].as_ref().unwrap().fire_size.get(), 2.0);

        slots.fire_slot_play(0, (10.5, 20.5, 0), 1, Some(FireSoundEvent::CampfireRunning));
        assert_eq!(slots.slots[0].as_ref().unwrap().fire_size.get(), 1.0);

        slots.fire_slot_stop(0);
        assert!(slots.slots[0].as_ref().unwrap().playing.is_none());

        slots.reset();
        assert_eq!(slots.active_slot_count(), 0);
    }

    #[test]
    fn fire_hooks_track_heat_sources_lights_and_listener_distance() {
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(10i32.div_euclid(8), 20i32.div_euclid(8));
        chunk.set_square(2, 4, 0, Some(IsoGridSquare::new(10, 20, 0).into()));
        cell.chunks.insert((10i32.div_euclid(8), 20i32.div_euclid(8)), chunk);

        let sm = pz_lua_api::bind_registry::shared_script_manager();
        let player = Rc::new(RefCell::new(
            pz_lua_api::bind_character_sim::LuaGameCharacter::new(
                &sm,
                pz_characters::CharacterKind::Player,
            ),
        ));
        {
            let mut p = player.borrow_mut();
            p.vars.x = 13.5;
            p.vars.y = 24.5;
            p.vars.z = 0.0;
        }
        pz_lua_api::bind_players::set_player(0, Some(player));

        let mut state = WorldFireState::default();
        let mut queue = FireEffectsQueue::default();
        let mut names = HashMap::new();
        names.insert(7i8, "Campfire".to_string());

        let fr = FireRef {
            x: 10,
            y: 20,
            z: 0,
            id: 1,
        };
        let mut fire = IsoFire::permanent();
        fire.heat_source = Some(6);

        {
            let mut hooks = EngineFireHooks {
                state: &mut state,
                queue: &mut queue,
                object_names: &names,
            };
            let d2 = hooks.listener_distance_sq(10.5, 20.5, 0);
            assert!((d2 - 25.0).abs() < 1e-3, "expected 3^2 + 4^2 = 25, got {d2}");
            assert_eq!(hooks.dict_object_name(7).as_deref(), Some("Campfire"));

            hooks.fire_updated(fr, &fire);
            hooks.lua_event("OnNewFire", 10, 20, 0);
            hooks.world_sound(10, 20, 0, 12, 12);
        }

        assert_eq!(state.heat_sources.len(), 1);
        let hs = state.heat_sources_for(12.5, 20.5, 0.0, &cell);
        assert_eq!(hs.len(), 1);
        assert_eq!(hs[0].radius, 6);
        assert_eq!(hs[0].temperature, HEAT_SOURCE_TEMPERATURE);
        assert!((hs[0].dist_sq - 4.0).abs() < 1e-3);
        // Far away player receives no heat source.
        assert!(state.heat_sources_for(100.0, 100.0, 0.0, &cell).is_empty());
        assert_eq!(queue.light_ops.len(), 1);
        assert_eq!(queue.lua_events.len(), 1);
        assert_eq!(queue.world_sounds.len(), 1);

        {
            let mut hooks = EngineFireHooks {
                state: &mut state,
                queue: &mut queue,
                object_names: &names,
            };
            hooks.fire_removed(fr);
        }
        assert!(state.heat_sources.is_empty());

        let render = Rc::new(RefCell::new(None));
        state.reset(&render);
        pz_lua_api::bind_players::set_player(0, None);
    }
}
