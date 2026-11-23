//! Client AmbientStreamManager globals sampled between the UI and music updates.
use pz_core::game_time::GameTime;
use pz_lua_api::{bind_climate, bind_players, bind_registry, bind_world};
use pz_sound::backend::AudioBackend;
use pz_sound::params::{FmodParameter, FmodParameterList};
use std::cell::Cell;
use std::rc::Rc;

const NAMES: [&str; 12] = [
    "FogIntensity",
    "RainIntensity",
    "Season",
    "SnowIntensity",
    "Storm",
    "StreamerMode",
    "Temperature",
    "TimeOfDay",
    "WeatherEvent",
    "WindIntensity",
    "CharacterElevation",
    "RoomSize",
];

#[derive(Default)]
pub(crate) struct AmbientParameters {
    values: Rc<Cell<[f32; NAMES.len()]>>,
    params: FmodParameterList,
}

impl AmbientParameters {
    #[cfg(test)]
    pub(crate) fn current_value(&self, name: &str) -> Option<f32> {
        self.params
            .parameters
            .iter()
            .find(|p| p.name() == name)
            .map(FmodParameter::current_value)
    }

    pub(crate) fn update(&mut self, backend: &mut dyn AudioBackend, streamer: bool) {
        let scripts = bind_registry::shared_script_manager();
        let mut gate = None;
        let mut gate_deaf = true;
        let mut selected = None;
        let mut selected_dead = true;
        let mut selected_deaf = true;
        for player in (0..bind_players::MAX_PLAYERS).filter_map(bind_players::player) {
            let actor = player.borrow();
            let deaf = actor.has_trait(&scripts, "DEAF").unwrap_or(false);
            let dead = actor.is_dead();
            if gate.is_none() || (gate_deaf && !deaf) {
                gate = Some(actor.current_square_xyz());
                gate_deaf = deaf;
            }
            // FMODParameterUtils.getFirstListener promotes a living or hearing player.
            if selected.is_none() || (selected_dead && !dead) || (selected_deaf && !deaf) {
                selected = Some((actor.current_square_xyz(), actor.vars.z));
                selected_dead = dead;
                selected_deaf = deaf;
            }
        }
        let Some((square, elevation)) = selected else {
            return;
        };
        if gate.flatten().is_none() {
            return;
        }
        let room_size = bind_world::with_world(|w| {
            let Some((x, y, z)) = square else { return 0 };
            let Some(owner) = w.cell.as_ref().and_then(|c| c.get_grid_square(x, y, z)) else {
                return 0;
            };
            let sq = owner.borrow();
            if let Some(room) = w.meta_grid.as_ref().and_then(|m| sq.room_def(m)) {
                return room.area;
            }
            w.regions
                .as_ref()
                .and_then(|r| sq.player_room_size(&r.borrow()))
                .unwrap_or(0)
        });
        self.values.set(values(
            &bind_climate::shared_game_time().borrow(),
            streamer,
            elevation,
            room_size,
        ));
        if self.params.parameters.is_empty() {
            for (i, name) in NAMES.into_iter().enumerate() {
                let inputs = self.values.clone();
                let description = backend.parameter_description(name);
                self.params.add(FmodParameter::global(
                    name,
                    description,
                    Box::new(move || inputs.get()[i]),
                ));
            }
        }
        self.params.update(backend);
    }
}

// ParameterTimeOfDay's half-open intervals are measured from the season's dawn and dusk.
fn time_band(time: f32, dawn: f32, dusk: f32) -> f32 {
    if time < dawn - 1.0 || time >= dusk + 1.0 {
        5.0
    } else if time < dawn + 1.0 {
        0.0
    } else if time < dawn + 2.0 {
        1.0
    } else if time < dusk - 2.0 {
        2.0
    } else if time < dusk - 1.0 {
        3.0
    } else {
        4.0
    }
}

fn values(time: &GameTime, streamer: bool, elevation: f32, room_size: i32) -> [f32; NAMES.len()] {
    let cm = &time.climate;
    let season = cm.current_day.as_ref().and_then(|d| d.season.as_ref());
    let season_label = season.map_or(0.0, |s| match s.get_season() {
        1 => 0.0,
        4 => 2.0,
        5 => 3.0,
        _ => 1.0,
    });
    let band = season.map_or(1.0, |s| {
        time_band(time.time_of_day, s.get_dawn(), s.get_dusk())
    });
    let weather = &cm.weather_period;
    let storm = if !weather.is_running {
        0.0
    } else if weather.is_thunder_storm {
        1.0
    } else if weather.is_tropical_storm {
        2.0
    } else if weather.is_blizzard {
        3.0
    } else {
        0.0
    };
    [
        cm.get_fog_intensity(),
        cm.get_rain_intensity(),
        season_label,
        cm.get_snow_intensity(),
        storm,
        f32::from(streamer),
        (cm.get_temperature() * 100.0) as i32 as f32 / 100.0,
        band,
        0.0,
        (cm.get_wind_intensity() * 1000.0) as i32 as f32 / 1000.0,
        elevation,
        room_size as f32,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn daylight_parameter_changes_at_the_native_boundaries() {
        let labels = [4.99, 5.0, 6.99, 7.0, 8.0, 18.0, 19.0, 20.99, 21.0]
            .map(|time| time_band(time, 6.0, 20.0));
        assert_eq!(labels, [5., 0., 0., 1., 2., 3., 4., 4., 5.]);
    }
}
