//! Per-frame `IsoFireplace` and `IsoBarbecue` engine wiring:
//! - Runs [`pz_world_render::HearthLink::update_fireplace`] and
//!   [`pz_world_render::HearthLink::update_barbecue`] (fuel burn/smoulder,
//!   flame/smoke/fuel attached sprites, propane tank sprite, and fireplace
//!   [`WorldLighting`] lamppost synchronization).
//! - Applies [`pz_iso_render::hearth::FireplaceState::apply`] and
//!   [`pz_iso_render::hearth::BarbecueState::apply`] onto the live
//!   [`pz_iso_render::world::RenderObject`]s in [`WorldRender`].
//! - Tracks active hearth [`pz_core::weather::manager::HeatSource`]s for
//!   `fill_square_environment`.
//! - Drives looped `FireplaceRunning` / `BBQPropaneRunning` / `BBQRegularRunning`
//!   audio on per-hearth [`FmodSoundEmitter`]s.

use std::collections::{HashMap, HashSet};

use pz_core::weather::manager::HeatSource;
use pz_iso_render::fire::RandSource;
use pz_render::Texture;
use pz_sound::emitter::{FmodSoundEmitter, ParentObject};
use pz_world::objects::ObjectKind;
use pz_world::{IsoCell, IsoSpriteManager};
use pz_world_render::HearthLink;

use crate::world_render::WorldRender;

type HearthKey = (i32, i32, i32, u32);

struct EngineHearthRand;

impl RandSource for EngineHearthRand {
    fn next(&self, max: i32) -> i32 {
        pz_core::random::next(max)
    }
    fn next_range(&self, min: i32, max: i32) -> i32 {
        pz_core::random::next_range(min, max)
    }
    fn next_float(&self, min: f32, max: f32) -> f32 {
        pz_core::random::next_float(min, max)
    }
}

struct HearthEntry {
    link: HearthLink,
    /// `(radius, temperature)` from `IsoFireplace::heat_source()` /
    /// `IsoBarbecue::heat_source()`, `None` when unlit.
    heat: Option<(i32, i32)>,
    in_room: bool,
    emitter: Option<FmodSoundEmitter>,
    sound_handle: i64,
    sound_name: Option<String>,
}

impl HearthEntry {
    fn new() -> Self {
        Self {
            link: HearthLink::new(),
            heat: None,
            in_room: false,
            emitter: None,
            sound_handle: 0,
            sound_name: None,
        }
    }

    fn sync_sound(&mut self, pos: (i32, i32, i32), desired: Option<&str>) {
        let sys = pz_lua_api::bind_audio::sound_system();
        let Ok(mut sys) = sys.try_borrow_mut() else {
            return;
        };
        sys.with(|_m, ctx| match desired {
            Some(name) => {
                let emitter = self
                    .emitter
                    .get_or_insert_with(|| FmodSoundEmitter::new(0));
                emitter.set_pos(pos.0 as f32 + 0.5, pos.1 as f32 + 0.5, pos.2 as f32);
                let name_changed = self.sound_name.as_deref() != Some(name);
                if name_changed && self.sound_handle != 0 {
                    emitter.stop_sound(self.sound_handle, ctx);
                    self.sound_handle = 0;
                }
                if self.sound_handle == 0 || !emitter.is_playing(self.sound_handle) {
                    self.sound_handle = emitter.play_sound_impl(name, ParentObject::None, ctx);
                    self.sound_name = Some(name.to_string());
                }
                emitter.tick(ctx);
            }
            None => {
                if let Some(emitter) = &mut self.emitter {
                    if self.sound_handle != 0 {
                        emitter.stop_sound(self.sound_handle, ctx);
                        self.sound_handle = 0;
                    }
                    emitter.tick(ctx);
                }
                self.sound_name = None;
            }
        });
    }

    fn stop_sound(&mut self) {
        let Some(mut emitter) = self.emitter.take() else {
            self.sound_handle = 0;
            self.sound_name = None;
            return;
        };
        let sys = pz_lua_api::bind_audio::sound_system();
        if let Ok(mut sys) = sys.try_borrow_mut() {
            sys.with(|_m, ctx| emitter.stop_all(ctx));
        }
        self.sound_handle = 0;
        self.sound_name = None;
    }
}

/// Engine-side per-world state for all loaded fireplaces and barbecues.
#[derive(Default)]
pub struct WorldHearths {
    entries: HashMap<HearthKey, HearthEntry>,
}

impl WorldHearths {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of tracked loaded hearths (for diagnostics/tests).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Advances all loaded fireplaces and barbecues for one frame:
    /// fuel burn/smoulder, visual flame/smoke/log states, fireplace lighting,
    /// `RenderObject` attachment sync, heat sources, and looping audio.
    pub fn update(
        &mut self,
        cell: &IsoCell,
        sprites: &IsoSpriteManager,
        mut render: Option<&mut WorldRender>,
        world_age_hours: f64,
        dt: f32,
    ) {
        let mut seen: HashSet<HearthKey> = HashSet::new();
        let rand = EngineHearthRand;
        let tex_fn = |name: &str| -> Option<Texture> { pz_render::get_shared_texture(name) };
        let frame_size = |name: &str| -> Option<(i32, i32)> {
            pz_render::get_shared_texture(name).map(|t| {
                (
                    t.width_orig.max(t.width as i32),
                    t.height_orig.max(t.height as i32),
                )
            })
        };

        for chunk in cell.chunks.values() {
            for sq_ref in chunk.squares() {
                let Ok(sq) = sq_ref.try_borrow() else {
                    continue;
                };
                let (x, y, z) = (sq.x, sq.y, sq.z);
                let in_room = sq.room_id != -1;
                for slot in &sq.objects {
                    let Ok(mut obj) = slot.owner_slot().try_borrow_mut() else {
                        continue;
                    };
                    let is_hearth = matches!(
                        obj.kind,
                        ObjectKind::Fireplace(_) | ObjectKind::Barbecue(_)
                    );
                    if !is_hearth {
                        continue;
                    }
                    let key = (x, y, z, obj.id);
                    seen.insert(key);
                    let entry = self.entries.entry(key).or_insert_with(HearthEntry::new);
                    entry.in_room = in_room;

                    match &obj.kind {
                        ObjectKind::Fireplace(_) => {
                            if let Some(wr) = render.as_deref_mut() {
                                let _ = entry.link.update_fireplace(
                                    &mut wr.lighting,
                                    &mut obj,
                                    sprites,
                                    (x, y, z),
                                    world_age_hours,
                                    dt,
                                    &rand,
                                    &frame_size,
                                );
                                if let Some(rsq) = wr.world.squares.get_mut(&(x, y, z)) {
                                    if let Some(ro) = rsq.objects.iter_mut().find(|o| o.id == obj.id)
                                    {
                                        entry.link.fireplace.apply(ro, &tex_fn);
                                    }
                                }
                            } else if let ObjectKind::Fireplace(fp) = &mut obj.kind {
                                fp.update_fuel(world_age_hours);
                                let fp_copy = fp.clone();
                                if let Ok(input) =
                                    pz_world_render::fireplace_input(&obj, &fp_copy, sprites)
                                {
                                    entry.link.fireplace.update(
                                        &input.as_input(),
                                        dt,
                                        &rand,
                                        &frame_size,
                                    );
                                }
                            }
                            if let ObjectKind::Fireplace(fp) = &obj.kind {
                                entry.heat = fp.heat_source();
                                let sound = fp.running_sound(None);
                                entry.sync_sound((x, y, z), sound);
                            }
                        }
                        ObjectKind::Barbecue(_) => {
                            let _ = entry.link.update_barbecue(
                                &mut obj,
                                sprites,
                                world_age_hours,
                                dt,
                            );
                            if let Some(wr) = render.as_deref_mut() {
                                if let Some(rsq) = wr.world.squares.get_mut(&(x, y, z)) {
                                    if let Some(ro) = rsq.objects.iter_mut().find(|o| o.id == obj.id)
                                    {
                                        entry.link.barbecue.apply(ro, &tex_fn);
                                    }
                                }
                            }
                            let propane = obj
                                .sprite
                                .try_read(sprites)
                                .ok()
                                .flatten()
                                .is_some_and(|sp| {
                                    sp.properties.has_name(&sprites.alias, "PropaneTank")
                                });
                            if let ObjectKind::Barbecue(bbq) = &obj.kind {
                                entry.heat = bbq.heat_source();
                                let sound = bbq.running_sound(propane);
                                entry.sync_sound((x, y, z), sound);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Prune hearths whose chunks/objects were unloaded or removed.
        let stale: Vec<HearthKey> = self
            .entries
            .keys()
            .copied()
            .filter(|k| !seen.contains(k))
            .collect();
        for key in stale {
            if let Some(mut entry) = self.entries.remove(&key) {
                if let Some(wr) = render.as_deref_mut() {
                    entry.link.remove(&mut wr.lighting);
                }
                entry.stop_sound();
            }
        }
    }

    /// Active hearth [`HeatSource`]s on level `z` within their radius of `(x, y)`.
    pub fn heat_sources_for(&self, x: i32, y: i32, z: i32) -> Vec<HeatSource> {
        let mut out = Vec::new();
        for (&(hx, hy, hz, _), entry) in &self.entries {
            if hz != z {
                continue;
            }
            let Some((radius, temperature)) = entry.heat else {
                continue;
            };
            let dx = (x - hx) as f32;
            let dy = (y - hy) as f32;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= (radius as f32) * (radius as f32) {
                out.push(HeatSource {
                    dist_sq,
                    radius,
                    temperature,
                    source_in_room: Some(entry.in_room),
                    line_clear: true,
                });
            }
        }
        out
    }

    /// Removes all fireplace lights and stops all looping hearth emitters.
    pub fn reset(&mut self, mut render: Option<&mut WorldRender>) {
        for (_, mut entry) in self.entries.drain() {
            if let Some(wr) = render.as_deref_mut() {
                entry.link.remove(&mut wr.lighting);
            }
            entry.stop_sound();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_world::objects::{IsoBarbecue, IsoFireplace, IsoObject};
    use pz_world::{IsoChunk, IsoGridSquare};

    #[test]
    fn fireplace_and_barbecue_update_fuel_heat_sources_and_cleanup_on_unload() {
        let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(1, 2);
        let mut sq = IsoGridSquare::new(10, 20, 0);
        sq.room_id = 42;

        let mut fp = IsoFireplace::default();
        fp.add_fuel(60);
        fp.turn_on();
        sq.objects
            .push(IsoObject::new(1, ObjectKind::Fireplace(fp), None).into());

        let mut bbq = IsoBarbecue::default();
        bbq.add_fuel(30);
        bbq.turn_on();
        sq.objects
            .push(IsoObject::new(2, ObjectKind::Barbecue(bbq), None).into());

        chunk.set_square(2, 4, 0, Some(sq.into()));
        cell.chunks.insert((1, 2), chunk);

        let mut hearths = WorldHearths::new();
        // First tick initializes last_update_time at hour 10.0.
        hearths.update(&cell, &sprites, None, 10.0, 1.0);
        assert_eq!(hearths.len(), 2);

        // Advance 10 in-game minutes (1/6 hour): both should burn 10 minutes of fuel.
        hearths.update(&cell, &sprites, None, 10.0 + 10.0 / 60.0, 1.0);
        {
            let sq_ref = cell.get_grid_square(10, 20, 0).unwrap();
            let sq_b = sq_ref.borrow();
            let obj0 = sq_b.objects[0].borrow();
            let ObjectKind::Fireplace(fp_after) = &obj0.kind else {
                panic!("expected fireplace");
            };
            assert_eq!(fp_after.fuel_amount(), 50);
            let obj1 = sq_b.objects[1].borrow();
            let ObjectKind::Barbecue(bbq_after) = &obj1.kind else {
                panic!("expected barbecue");
            };
            assert_eq!(bbq_after.fuel_amount(), 20);
        }

        // Both lit hearths contribute HeatSources on z = 0 within radius.
        let near = hearths.heat_sources_for(11, 20, 0);
        assert_eq!(near.len(), 2);
        assert!(near.iter().any(|h| h.temperature == 35 && h.source_in_room == Some(true)));
        assert!(near.iter().any(|h| h.temperature == 25 && h.radius == 3));
        // Wrong Z level sees none.
        assert!(hearths.heat_sources_for(11, 20, 1).is_empty());

        // Unloading the chunk prunes tracked hearth entries and heat sources.
        cell.chunks.clear();
        hearths.update(&cell, &sprites, None, 10.5, 1.0);
        assert!(hearths.is_empty());
        assert!(hearths.heat_sources_for(10, 20, 0).is_empty());
    }
}
