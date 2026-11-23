//! Client AmbientStreamManager globals sampled between the UI and music updates.
use pz_core::game_time::GameTime;
use pz_lua_api::{bind_climate, bind_players, bind_registry, bind_world};
use pz_sound::backend::AudioBackend;
use pz_sound::params::{FmodParameter, FmodParameterList};
use pz_world::{IsoMetaGrid, ZoneId};
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

// ParameterZone uses these exact map types, including the spelling of Vegitation.
const ZONES: [(&str, &str); 7] = [
    ("ZoneDeepForest", "DeepForest"),
    ("ZoneFarm", "Farm"),
    ("ZoneForest", "Forest"),
    ("ZoneNav", "Nav"),
    ("ZoneVegetation", "Vegitation"),
    ("ZoneTown", "TownZone"),
    ("ZoneTrailerPark", "TrailerPark"),
];
const PARAM_COUNT: usize = NAMES.len() + ZONES.len();

#[derive(Default)]
pub(crate) struct AmbientParameters {
    values: Rc<Cell<[f32; PARAM_COUNT]>>,
    nearby_zones: Vec<ZoneId>,
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
                selected = Some((
                    actor.current_square_xyz(),
                    actor.vars.x,
                    actor.vars.y,
                    actor.vars.z,
                ));
                selected_dead = dead;
                selected_deaf = deaf;
            }
        }
        let Some((square, x, y, elevation)) = selected else {
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
        let mut current = [0.0; PARAM_COUNT];
        current[..NAMES.len()].copy_from_slice(&values(
            &bind_climate::shared_game_time().borrow(),
            streamer,
            elevation,
            room_size,
        ));
        let distances = bind_world::with_world(|w| {
            zone_distances(w.meta_grid.as_ref(), x, y, &mut self.nearby_zones)
        });
        current[NAMES.len()..].copy_from_slice(&distances);
        self.values.set(current);
        if self.params.parameters.is_empty() {
            let names = NAMES.into_iter().chain(ZONES.map(|(name, _)| name));
            for (i, name) in names.enumerate() {
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

fn zone_distances(
    grid: Option<&IsoMetaGrid>,
    x: f32,
    y: f32,
    nearby: &mut Vec<ZoneId>,
) -> [f32; ZONES.len()] {
    let mut closest_sq = [40.0_f32 * 40.0; ZONES.len()];
    if let Some(grid) = grid {
        let tile_x = pz_world::java::fastfloor(x);
        let tile_y = pz_world::java::fastfloor(y);
        nearby.clear();
        grid.get_zones_intersecting_into(tile_x - 40, tile_y - 40, 0, 80, 80, nearby);
        for &id in nearby.iter() {
            let zone = grid.zone(id);
            let kind = &zone.zone_type;
            let Some(index) = ZONES.iter().position(|&(_, wanted)| {
                kind.eq_ignore_ascii_case(wanted)
                    || (wanted == "Forest"
                        && !kind.eq_ignore_ascii_case("DeepForest")
                        && kind.ends_with("Forest"))
            }) else {
                continue;
            };
            if closest_sq[index] == 0.0 {
                continue;
            }
            if zone.contains(tile_x, tile_y, 0) {
                closest_sq[index] = 0.0;
            } else {
                let half_w = zone.w as f32 * 0.5;
                let half_h = zone.h as f32 * 0.5;
                let dx = ((x - (zone.x as f32 + half_w)).abs() - half_w).max(0.0);
                let dy = ((y - (zone.y as f32 + half_h)).abs() - half_h).max(0.0);
                closest_sq[index] = closest_sq[index].min(dx * dx + dy * dy);
            }
        }
    }
    closest_sq.map(|distance| distance.sqrt().trunc())
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
    fn nearby_zones_use_ground_geometry_and_distinct_forest_types() {
        let mut grid = IsoMetaGrid::with_bounds(0, 0, 1, 1);
        grid.register_zone("", "deepFOREST", 251, 91, 0, 7, 12);
        grid.register_zone("", "BirchForest", 262, 108, 0, 10, 10);
        grid.register_zone("", "TownZone", 250, 91, 1, 10, 10);
        grid.register_zone("", "Farm", 290, 135, 0, 10, 10);
        grid.register_zone("", "VEGITATION", 250, 99, 0, 2, 2);
        let mut nearby = Vec::new();
        assert_eq!(
            zone_distances(Some(&grid), 252.5, 97.5, &mut nearby),
            [0., 40., 14., 40., 1., 40., 40.]
        );
        assert_eq!(
            zone_distances(Some(&grid), 310., 200., &mut nearby),
            [40.; 7]
        );
    }

    #[test]
    fn daylight_parameter_changes_at_the_native_boundaries() {
        let labels = [4.99, 5.0, 6.99, 7.0, 8.0, 18.0, 19.0, 20.99, 21.0]
            .map(|time| time_band(time, 6.0, 20.0));
        assert_eq!(labels, [5., 0., 0., 1., 2., 3., 4., 4., 5.]);
    }
}
