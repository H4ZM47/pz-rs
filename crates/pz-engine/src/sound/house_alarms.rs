//! The active house alarms loaded from metadata.bin or started by an armed room.
use crate::iso_world::{opt_int, sandbox, IsoWorld};
use pz_audio::world_sound::{SoundSource, WorldSoundManager};
use pz_core::save_trackers::AlarmSave;
use pz_kahlua::{value::LuaObject, Value};
use pz_lua_api::{bind_audio, bind_climate, bind_players, bind_world::with_world, LuaManager};
use pz_sound::backend::{InstanceHandle, PlaybackState};
use std::rc::Rc;

struct AlarmSource;
impl LuaObject for AlarmSource {
    fn class_name(&self) -> &str {
        "zombie.iso.Alarm"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Native Alarm.inst is shared by active alarms. Per-alarm sources remain
/// identifiable to Lua and the world-sound manager until each alarm finishes.
#[derive(Default)]
pub(crate) struct HouseAlarms {
    sources: Vec<Rc<AlarmSource>>,
    instance: InstanceHandle,
}

impl HouseAlarms {
    pub(crate) fn stop(&mut self) {
        if self.instance > 0 {
            let system = bind_audio::sound_system();
            let mut system = system.borrow_mut();
            let instance = std::mem::take(&mut self.instance);
            system.backend.instance_stop(instance, false);
            system.backend.instance_release(instance);
        }
    }

    fn update_sound(&mut self, alarm: AlarmSave, occlusion: f32) -> bool {
        let system = bind_audio::sound_system();
        let mut system = system.borrow_mut();
        if system.sounds.env.sound_disabled {
            return false;
        }
        system.with(|_, ctx| {
            if self.instance <= 0 {
                let Some(event) = ctx.backend.event_description("event:/Meta/HouseAlarm") else {
                    return false;
                };
                ctx.backend.load_event_sample_data(&event);
                self.instance = ctx.backend.create_event_instance(&event);
                if self.instance <= 0 {
                    return false;
                }
            }
            let instance = self.instance;
            let volume = ctx
                .sounds
                .get_or_create_sound("HouseAlarm", ctx.backend, ctx.media_file_exists)
                .map_or(1.0, |sound| {
                    pz_sound::game_sounds::user_volume(&ctx.sounds.env, &sound.borrow().game_sound)
                });
            ctx.backend.instance_set_volume(instance, volume);
            ctx.backend
                .instance_set_3d_position(instance, alarm.x as f32, alarm.y as f32, 0.0);
            match ctx.backend.instance_playback_state(instance) {
                PlaybackState::Playing | PlaybackState::Starting => (),
                PlaybackState::Stopping => return true,
                _ => ctx.backend.instance_start(instance),
            }
            ctx.backend
                .instance_set_parameter_by_name(instance, "Occlusion", occlusion);
            false
        })
    }
}

impl IsoWorld {
    pub(crate) fn update_house_alarms(&mut self, lua: &mut LuaManager) {
        if self.meta_tracker.borrow().alarms.is_empty() || bind_climate::is_game_paused() {
            return;
        }
        let count = bind_players::num_players().clamp(0, bind_players::MAX_PLAYERS as i32) as usize;
        let players: Vec<_> = (0..count).filter_map(bind_players::player).collect();
        let scripts = pz_lua_api::bind_registry::shared_script_manager();
        let listener = players
            .iter()
            .find(|p| !p.borrow().has_trait(&scripts, "DEAF").unwrap_or(false))
            .or(players.first());
        let Some(at) = listener.and_then(|p| p.borrow().current_square_xyz()) else {
            return;
        };
        let hearing = sandbox(lua).map_or(2, |s| opt_int(&s.borrow(), "ZombieLore.Hearing"));
        let time = bind_climate::shared_game_time();
        let mut i = 0;
        loop {
            let Some(alarm) = self.meta_tracker.borrow().alarms.get(i).copied() else {
                break;
            };
            if i == self.house_alarms.sources.len() {
                self.house_alarms.sources.push(Rc::new(AlarmSource));
            }
            let source = self.house_alarms.sources[i].clone();
            let id = Rc::as_ptr(&source) as usize as u64 | (1 << 63);
            let (x, y) = (alarm.x, alarm.y);
            // WorldSound.init raises this event before registering in chunks.
            // A listener may query the world or start a second alarm.
            lua.trigger_event(
                "OnWorldSound",
                &[
                    x.into(),
                    y.into(),
                    0.into(),
                    600.into(),
                    600.into(),
                    Value::Object(source),
                ],
            );
            let occlusion = with_world(|w| {
                let Some(cell) = w.cell.as_ref() else {
                    return 0.0;
                };
                let mut effects = pz_world_sound::NoEffects;
                let mut adapter =
                    pz_world_sound::WorldSoundAdapter::new(cell, hearing, &mut effects);
                let source = SoundSource {
                    id,
                    ..Default::default()
                };
                pz_lua_api::bind_zombie::shared_world_sounds()
                    .borrow_mut()
                    .add_sound(&mut adapter, Some(source), x, y, 0, 600, 600);
                let Some(meta) = w.meta_grid.as_ref() else {
                    return 0.0;
                };
                let Some(sprites) = self.sprites.as_ref() else {
                    return 0.0;
                };
                let Some(listener) = cell.get_grid_square(at.0, at.1, at.2) else {
                    return 0.0;
                };
                let listener = listener.borrow();
                if count != 1 || listener.has(sprites.borrow().tables.flag.exterior) {
                    return 0.0;
                }
                let same_building = cell.get_grid_square(x, y, 0).is_some_and(|s| {
                    s.borrow().room_def(meta).and_then(|r| r.building)
                        == listener.room_def(meta).and_then(|r| r.building)
                });
                if same_building {
                    0.0
                } else {
                    0.2
                }
            });
            if let Some(pop) = self.population.as_mut() {
                pop.manager.add_world_sound(
                    x,
                    y,
                    600,
                    600,
                    false,
                    true,
                    WorldSoundManager::hearing_multiplier(hearing),
                );
            }
            let stopping = self.house_alarms.update_sound(alarm, occlusion);
            for player in &players {
                let p = player.borrow();
                let (dx, dy) = (p.vars.x - x as f32, p.vars.y - y as f32);
                if !p.is_dead()
                    && !p.has_trait(&scripts, "DEAF").unwrap_or(false)
                    && dx * dx + dy * dy <= 2500.0
                {
                    bind_audio::intensity_config()
                        .borrow()
                        .trigger(Some("AlarmNearby"), &p.player.music_intensity_events);
                    break;
                }
            }
            if time.borrow().world_age_hours() >= f64::from(alarm.end_game_time) || stopping {
                self.house_alarms.stop();
                self.meta_tracker.borrow_mut().alarms.remove(i);
                self.house_alarms.sources.remove(i);
            } else {
                i += 1;
            }
        }
    }
}
