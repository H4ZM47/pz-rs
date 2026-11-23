//! Engine wiring for `IsoFireManager` (`FireHooks`, `FireSoundHooks`,
//! `FireLightLinks`, and the cell's `IsoHeatSource` registry).
//!
//! Keeps 20 `FmodSoundEmitter` slots (`IsoFireManager.FireSounds`),
//! synchronises permanent-fire and hearth `IsoHeatSource`s for
//! `ClimateManager::get_air_temperature_for_square`, replays fire light
//! additions/updates/removals onto `WorldRender`'s `FireLightLinks`, and
//! queues world sounds, one-shot world audio (`BurnedObjectExploded`), and
//! Lua events (`OnNewFire`, `OnGridBurnt`) so they run outside `bind_world`'s
//! `WORLD` borrow.

use std::collections::HashMap;

use pz_core::weather::HeatSource;
use pz_sound::emitter::ParentObject;
use pz_sound::world_emitters::EmitterRef;
use pz_world::chunk::IsoCell;
use pz_world::fire::manager::{FireHooks, FireRef};
use pz_world::fire::sound::{FireSoundEvent, FireSoundHooks, FIRE_SOUND_SLOTS};
use pz_world::fire::{IsoFire, HEAT_SOURCE_TEMPERATURE};
use pz_world::isoregion::IsoRegions;
use pz_world::los::{line_clear, TestResults};
use pz_world::meta_grid::IsoMetaGrid;
use pz_world::sprite::IsoSpriteManager;
use pz_world::square::VisionEnv;

/// One of `IsoFireManager.FireSounds`'s 20 emitter slots.
#[derive(Default)]
struct FireSoundEmitterSlot {
    emitter: Option<EmitterRef>,
    handle: i64,
    event: Option<FireSoundEvent>,
}

/// `IsoFireManager.fireSounds` emitters on the engine side: 20 pooled
/// `FmodSoundEmitter` instances driven by [`FireSoundHooks`].
pub struct FireSoundSlots {
    slots: Vec<FireSoundEmitterSlot>,
}

impl Default for FireSoundSlots {
    fn default() -> Self {
        Self::new()
    }
}

impl FireSoundSlots {
    pub fn new() -> Self {
        Self {
            slots: std::iter::repeat_with(FireSoundEmitterSlot::default)
                .take(FIRE_SOUND_SLOTS)
                .collect(),
        }
    }

    /// `FireSounds.Slot.playSound(fire)`: positions the slot's emitter,
    /// starts `event` if not already playing, updates `FireSize`, and ticks
    /// the emitter.
    pub fn play(
        &mut self,
        slot: usize,
        pos: (f32, f32, i32),
        fire_size: i32,
        event: Option<FireSoundEvent>,
    ) {
        let Some(entry) = self.slots.get_mut(slot) else {
            return;
        };
        let ss = pz_lua_api::bind_audio::sound_system();
        let Ok(mut ss) = ss.try_borrow_mut() else {
            return;
        };
        ss.with_world_emitters(|pool, ctx| {
            let emitter_ref = entry
                .emitter
                .get_or_insert_with(|| pool.new_emitter())
                .clone();
            let mut em = emitter_ref.borrow_mut();
            em.set_pos(pos.0, pos.1, pos.2 as f32);
            if let Some(ev) = event {
                let name = ev.name();
                if entry.event != Some(ev) || !em.is_playing_name(name) {
                    if entry.handle != 0 {
                        em.stop_all(ctx);
                    }
                    entry.handle = em.play_sound_impl(name, ParentObject::None, ctx);
                    entry.event = Some(ev);
                }
                if entry.handle != 0 {
                    em.set_parameter_value_by_name(entry.handle, "FireSize", fire_size as f32, ctx);
                }
            }
            em.tick(ctx);
        });
    }

    /// `FireSounds.Slot.stopPlaying()`: ticks an idle non-empty emitter, or
    /// triggers the sustain cue / stops all on an active instance and forgets
    /// its handle.
    pub fn stop(&mut self, slot: usize) {
        let Some(entry) = self.slots.get_mut(slot) else {
            return;
        };
        let Some(emitter_ref) = entry.emitter.clone() else {
            return;
        };
        let ss = pz_lua_api::bind_audio::sound_system();
        let Ok(mut ss) = ss.try_borrow_mut() else {
            return;
        };
        ss.with_world_emitters(|_, ctx| {
            let mut em = emitter_ref.borrow_mut();
            if entry.handle == 0 {
                if !em.is_empty() {
                    em.tick(ctx);
                }
            } else {
                if em.has_sustain_points(entry.handle, ctx.sounds) {
                    em.trigger_cue(entry.handle, ctx);
                } else {
                    em.stop_all(ctx);
                }
                entry.handle = 0;
                entry.event = None;
            }
        });
    }

    /// Stops every active slot emitter (called on `IngameState::exit`).
    pub fn stop_all(&mut self) {
        let ss = pz_lua_api::bind_audio::sound_system();
        let Ok(mut ss) = ss.try_borrow_mut() else {
            for entry in &mut self.slots {
                entry.handle = 0;
                entry.event = None;
            }
            return;
        };
        ss.with_world_emitters(|_, ctx| {
            for entry in &mut self.slots {
                if let Some(emitter_ref) = &entry.emitter {
                    emitter_ref.borrow_mut().stop_all(ctx);
                }
                entry.handle = 0;
                entry.event = None;
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn slot_event(&self, slot: usize) -> Option<FireSoundEvent> {
        self.slots.get(slot).and_then(|s| s.event)
    }
}

/// Identifies the owner of a registered `IsoHeatSource`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeatSourceKey {
    Fire(FireRef),
    Object((i32, i32, i32), u32),
}

/// One live `IsoHeatSource` in the cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeatSourceEntry {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i32,
    pub temperature: i32,
}

/// The cell's `IsoHeatSource` list (`IsoCell.addHeatSource` /
/// `removeHeatSource`), keyed by its owning fire or hearth object so updates
/// resize in place and removals clean up deterministically.
#[derive(Clone, Debug, Default)]
pub struct HeatSourceRegistry {
    entries: HashMap<HeatSourceKey, HeatSourceEntry>,
}

impl HeatSourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Synchronises a fire's permanent `IsoHeatSource` (`IsoFire::heat_source`).
    pub fn sync_fire(&mut self, r: FireRef, fire: &IsoFire) {
        match fire.heat_source {
            Some(radius) if radius > 0 => {
                self.entries.insert(
                    HeatSourceKey::Fire(r),
                    HeatSourceEntry {
                        x: r.x,
                        y: r.y,
                        z: r.z,
                        radius,
                        temperature: HEAT_SOURCE_TEMPERATURE,
                    },
                );
            }
            _ => {
                self.entries.remove(&HeatSourceKey::Fire(r));
            }
        }
    }

    /// Removes a fire's `IsoHeatSource` (`IsoFire.removeFromWorld`).
    pub fn remove_fire(&mut self, r: FireRef) {
        self.entries.remove(&HeatSourceKey::Fire(r));
    }

    /// Synchronises a hearth (`IsoFireplace` / `IsoBarbecue`) `IsoHeatSource`.
    pub fn sync_object(&mut self, pos: (i32, i32, i32), id: u32, heat: Option<(i32, i32)>) {
        match heat {
            Some((radius, temperature)) if radius > 0 => {
                self.entries.insert(
                    HeatSourceKey::Object(pos, id),
                    HeatSourceEntry {
                        x: pos.0,
                        y: pos.1,
                        z: pos.2,
                        radius,
                        temperature,
                    },
                );
            }
            _ => {
                self.entries.remove(&HeatSourceKey::Object(pos, id));
            }
        }
    }

    /// Removes a hearth's `IsoHeatSource` (`WorldCall::RemoveHeatSource`).
    pub fn remove_object(&mut self, pos: (i32, i32, i32), id: u32) {
        self.entries.remove(&HeatSourceKey::Object(pos, id));
    }

    /// Builds the `pz_core::weather::HeatSource` slice for a square at
    /// `(x, y, z)`, matching `IsoCell.getHeatSourceHighestTemperature`.
    pub fn query_for_square(
        &self,
        x: i32,
        y: i32,
        z: i32,
        cell: &IsoCell,
        meta: &IsoMetaGrid,
        sprites: Option<&IsoSpriteManager>,
        regions: Option<&IsoRegions>,
    ) -> Vec<HeatSource> {
        if self.entries.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for entry in self.entries.values() {
            if entry.z != z || entry.radius <= 0 {
                continue;
            }
            let dx = (x - entry.x) as f32;
            let dy = (y - entry.y) as f32;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq >= (entry.radius * entry.radius) as f32 {
                continue;
            }
            let source_in_room = cell
                .get_grid_square(entry.x, entry.y, entry.z)
                .and_then(|sq| sq.try_borrow().ok().map(|s| {
                    s.room_def(meta).is_some()
                        || regions.and_then(|r| s.player_room_size(r)).is_some()
                }));
            let is_clear = if (x, y, z) == (entry.x, entry.y, entry.z) {
                true
            } else if let Some(sprites) = sprites {
                let env = VisionEnv::new(cell, sprites, &pz_world::square::NoSquareHooks);
                matches!(
                    line_clear(&env, entry.x, entry.y, entry.z, x, y, z, false),
                    TestResults::Clear | TestResults::ClearThroughOpenDoor
                )
            } else {
                true
            };
            out.push(HeatSource {
                dist_sq,
                radius: entry.radius,
                temperature: entry.temperature,
                source_in_room,
                line_clear: is_clear,
            });
        }
        out
    }
}

/// Ordered light-source sync operations recorded during `IsoFireManager::update`
/// and replayed onto `WorldRender`'s `FireLightLinks` right after
/// `RenderInvalidation` releases its borrow of `WorldRender`.
#[derive(Clone, Debug)]
pub enum FireLightOp {
    Updated(FireRef, IsoFire),
    Removed(FireRef),
}

/// Deferred side effects collected while `IsoFireManager::update` holds
/// `bind_world`'s `WORLD` borrow, drained immediately after `with_world`.
#[derive(Default)]
pub struct FireDeferredEffects {
    pub light_ops: Vec<FireLightOp>,
    pub world_sounds: Vec<(i32, i32, i32, i32, i32)>,
    pub play_sounds: Vec<(String, i32, i32, i32)>,
    pub lua_events: Vec<(String, i32, i32, i32)>,
}

/// Engine [`FireHooks`] + [`FireSoundHooks`] adapter for `IsoFireManager::update`.
pub struct EngineFireHooks<'a> {
    pub sounds: &'a mut FireSoundSlots,
    pub heat_sources: &'a mut HeatSourceRegistry,
    pub object_names: &'a HashMap<i8, String>,
    /// Active non-deaf local-player listener positions `(x, y, z * 3.0)`.
    pub listeners: Vec<[f32; 3]>,
    pub deferred: FireDeferredEffects,
}

impl FireSoundHooks for EngineFireHooks<'_> {
    fn fire_slot_play(
        &mut self,
        slot: usize,
        pos: (f32, f32, i32),
        fire_size: i32,
        event: Option<FireSoundEvent>,
    ) {
        self.sounds.play(slot, pos, fire_size, event);
    }

    fn fire_slot_stop(&mut self, slot: usize) {
        self.sounds.stop(slot);
    }
}

impl FireHooks for EngineFireHooks<'_> {
    fn world_sound(&mut self, x: i32, y: i32, z: i32, radius: i32, volume: i32) {
        self.deferred
            .world_sounds
            .push((x, y, z, radius, volume));
    }

    fn play_world_sound(
        &mut self,
        name: &str,
        x: i32,
        y: i32,
        z: i32,
        _radius: f32,
        _gain: f32,
    ) {
        self.deferred
            .play_sounds
            .push((name.to_owned(), x, y, z));
    }

    fn fire_updated(&mut self, r: FireRef, fire: &IsoFire) {
        self.heat_sources.sync_fire(r, fire);
        self.deferred
            .light_ops
            .push(FireLightOp::Updated(r, fire.clone()));
    }

    fn fire_removed(&mut self, r: FireRef) {
        self.heat_sources.remove_fire(r);
        self.deferred.light_ops.push(FireLightOp::Removed(r));
    }

    fn listener_distance_sq(&self, x: f32, y: f32, z: i32) -> f32 {
        let mut best = f32::MAX;
        let fz = z as f32 * 3.0;
        for &[lx, ly, lz] in &self.listeners {
            let dx = x - lx;
            let dy = y - ly;
            let dz = fz - lz;
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 < best {
                best = d2;
            }
        }
        best
    }

    fn lua_event(&mut self, name: &str, x: i32, y: i32, z: i32) {
        self.deferred.lua_events.push((name.to_owned(), x, y, z));
    }

    fn dict_object_name(&self, id: i8) -> Option<String> {
        self.object_names.get(&id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::chunk::{IsoChunk, IsoGridSquare};
    use pz_world::fire::IsoFire;

    #[test]
    fn engine_fire_hooks_tracks_heat_sources_lights_listeners_and_deferred_events() {
        let mut sounds = FireSoundSlots::new();
        let mut heat_sources = HeatSourceRegistry::new();
        let mut object_names = HashMap::new();
        object_names.insert(4, "Campfire".to_owned());

        let r = FireRef {
            x: 10,
            y: 12,
            z: 0,
            id: 1,
        };
        let mut fire = IsoFire::permanent();
        fire.heat_source = Some(5);

        let mut hooks = EngineFireHooks {
            sounds: &mut sounds,
            heat_sources: &mut heat_sources,
            object_names: &object_names,
            listeners: vec![[13.0, 16.0, 0.0], [100.0, 100.0, 0.0]],
            deferred: FireDeferredEffects::default(),
        };

        // Closest listener distance squared: (10-13)^2 + (12-16)^2 = 9 + 16 = 25.
        assert_eq!(hooks.listener_distance_sq(10.0, 12.0, 0), 25.0);
        assert_eq!(hooks.dict_object_name(4).as_deref(), Some("Campfire"));

        hooks.fire_updated(r, &fire);
        hooks.world_sound(10, 12, 0, 20, 20);
        hooks.play_world_sound("BurnedObjectExploded", 10, 12, 0, 10.0, 1.0);
        hooks.lua_event("OnNewFire", 10, 12, 0);
        hooks.fire_slot_play(0, (10.5, 12.5, 0), 2, Some(FireSoundEvent::Fire));

        assert_eq!(hooks.heat_sources.len(), 1);
        assert_eq!(hooks.deferred.light_ops.len(), 1);
        assert_eq!(hooks.deferred.world_sounds, vec![(10, 12, 0, 20, 20)]);
        assert_eq!(hooks.deferred.play_sounds.len(), 1);
        assert_eq!(
            hooks.deferred.lua_events,
            vec![("OnNewFire".to_owned(), 10, 12, 0)]
        );
        assert_eq!(hooks.sounds.slot_event(0), Some(FireSoundEvent::Fire));

        // Querying heat sources on a square within radius 5 finds the fire.
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(1, 1);
        let sq = pz_world::SquareSlot::new(IsoGridSquare::new(10, 12, 0));
        chunk.set_square(2, 4, 0, Some(sq));
        cell.chunks.insert((1, 1), chunk);
        let meta = IsoMetaGrid::empty();
        let found = hooks
            .heat_sources
            .query_for_square(12, 12, 0, &cell, &meta, None, None);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].dist_sq, 4.0);
        assert_eq!(found[0].radius, 5);
        assert_eq!(found[0].temperature, HEAT_SOURCE_TEMPERATURE);
        assert!(found[0].line_clear);

        // Removing the fire clears its heat source and records light removal.
        hooks.fire_removed(r);
        hooks.fire_slot_stop(0);
        assert!(hooks.heat_sources.is_empty());
        assert_eq!(hooks.deferred.light_ops.len(), 2);
        assert!(matches!(hooks.deferred.light_ops[1], FireLightOp::Removed(x) if x == r));
    }
}
