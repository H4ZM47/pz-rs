use super::*;

#[test]
fn time_lerp_wraps_midnight() {
    assert_eq!(time_lerp(0.5, 0.0, 1.0, false), 1.0);
    assert_eq!(time_lerp(0.2, 0.3, 0.6, false), 0.0);
    // 22:00 to 04:00: 01:00 is the midpoint.
    let v = time_lerp_hours(1.0, 22.0, 4.0, false);
    assert!((v - 1.0).abs() < 1e-6, "{v}");
    assert_eq!(time_lerp_hours(12.0, 22.0, 4.0, true), 0.0);
}

#[test]
fn helpers() {
    assert!(clamp(0.0, 1.0, f32::NAN).is_nan());
    assert_eq!(clerp(0.0, 2.0, 4.0), 2.0);
    assert_eq!(clerp(1.0, 2.0, 4.0), 4.0);
    assert_eq!(pos_to_pos_neg_range(0.25), -0.5);
    assert_eq!(wind_angle_string(10.0), "SE");
    assert_eq!(wind_angle_string(250.0), "NE");
    assert_eq!(air_mass_noise_frequency_mod(1), 300.0);
    assert_eq!(rain_time_multiplier_mod(5), 1.5);
}

use crate::calendar::JavaZone;
use crate::game_time::GameTime;
use crate::random::Rand;
use calendar::set_default_zone;

struct Ctx {
    rand: Rand,
    events: Vec<ClimateEvent>,
}

impl ClimateContext for Ctx {
    fn rand_next(&mut self, max: i32) -> i32 {
        self.rand.next(max)
    }
    fn rand_next_range(&mut self, min: i32, max: i32) -> i32 {
        self.rand.next_range(min, max)
    }
    fn rand_next_float(&mut self, min: f32, max: f32) -> f32 {
        self.rand.next_float(min, max)
    }
    fn players(&self) -> [Option<(f32, f32)>; 4] {
        [Some((500.0, 700.0)), None, None, None]
    }
    fn event(&mut self, e: ClimateEvent) {
        self.events.push(e);
    }
}

impl crate::game_time::GameTimeContext for Ctx {
    fn meta_event(&self) -> i32 {
        1
    }
}

fn world() -> (GameTime, Ctx) {
    set_default_zone(Some(JavaZone::utc()));
    let mut ctx = Ctx { rand: Rand::new(3), events: Vec::new() };
    let mut gt = GameTime { multiplier: 20.0, helicopter_day1: -1, day: 3, month: 1, year: 1994, ..GameTime::default() };
    gt.climate = ClimateManager::with_offsets([100.0, 9000.0, -500.0, -9000.0]);
    gt.set_minutes_stamp();
    let season = ErosionSeason::from_config(&SeasonConfig::default(), 3, gt.day, gt.month, gt.year);
    let clock = gt.clock(&ctx);
    gt.climate.init(season, Some((0, 0, 60, 50)), &clock, &mut ctx);
    (gt, ctx)
}

#[test]
fn a_day_of_winter() {
    let (mut gt, mut ctx) = world();
    assert_eq!(gt.climate.climate_forecaster.forecasts.len(), 40);
    assert_eq!(gt.climate.thunder_storm.map_max_x, 60 * 256 + 4000);
    let mut temps = Vec::new();
    while gt.nights_survived < 2 {
        gt.update(&mut ctx);
        gt.update_climate(&mut ctx);
        temps.push(gt.climate.get_temperature());
    }
    assert!(ctx.events.contains(&ClimateEvent::ClimateTick));
    let (min, max) = temps.iter().fold((f32::MAX, f32::MIN), |(a, b), &t| (a.min(t), b.max(t)));
    assert!(min > -40.0 && max < 30.0 && max > min, "{min} {max}");
    assert_eq!(gt.climate.get_season().unwrap().get_season(), season::SEASON_WINTER);
    assert!(gt.view_dist_max > gt.view_dist_min);
    let h = gt.climate.get_humidity();
    assert!((0.0..=1.0).contains(&h));
}

#[test]
fn custom_weather_and_overrides() {
    let (mut gt, mut ctx) = world();
    gt.update(&mut ctx);
    gt.update_climate(&mut ctx);
    let clock = gt.clock(&ctx);
    assert!(gt.climate.trigger_custom_weather_stage(period::STAGE_STORM, 6.0, &clock, &mut ctx));
    let wp = &gt.climate.weather_period;
    assert!(wp.is_running);
    let ids: Vec<i32> = wp.weather_stages.iter().map(|s| s.stage_id).collect();
    assert_eq!(ids, [0, 3, 4]);
    assert!(!gt.climate.trigger_custom_weather(0.5, true, &clock, &mut ctx));
    for _ in 0..2000 {
        gt.update(&mut ctx);
        gt.update_climate(&mut ctx);
    }
    assert!(gt.climate.thunder_storm.clouds.iter().any(|c| c.is_running) || !gt.climate.weather_period.is_running);
    gt.climate.stop_weather_and_thunder(&mut ctx);
    assert!(!gt.climate.weather_period.is_running);
    assert!(gt.climate.thunder_storm.clouds.iter().all(|c| !c.is_running));

    // Admin beats modded beats override.
    let f = &mut gt.climate.vars.floats[manager::FLOAT_TEMPERATURE];
    f.internal_value = 10.0;
    f.set_override(20.0, 0.5);
    f.calculate(false);
    assert_eq!(f.final_value, 15.0);
    f.set_enable_modded(true);
    f.set_modded_value(30.0);
    f.set_modded_interpolate(1.0);
    f.calculate(false);
    assert_eq!(f.internal_value, 30.0);
    f.set_admin_value(500.0);
    f.set_enable_admin(true);
    f.calculate(false);
    assert_eq!(f.final_value, 80.0);
}

#[test]
fn air_temperature_for_squares() {
    let (mut gt, mut ctx) = world();
    gt.update(&mut ctx);
    gt.update_climate(&mut ctx);
    let c = &mut gt.climate;
    c.vars.floats[manager::FLOAT_TEMPERATURE].final_value = -10.0;
    c.wind_power = 0.5;
    c.day_light_lagged = 0.5;
    let outside = manager::AirTemperatureQuery {
        in_room: false,
        z: 0,
        vehicle_inside_temperature: None,
        hydro_power_on: false,
        heat_source_highest: None,
    };
    assert_eq!(c.get_air_temperature_for_square(Some(&outside), false), -10.0);
    let chill = c.get_air_temperature_for_square(Some(&outside), true);
    assert!(chill < -10.0);
    assert_eq!(c.wind_chill_amount(Some(&outside), false), -10.0 - chill);
    let room = manager::AirTemperatureQuery { in_room: true, ..outside };
    // 32 below 22, 0.4 + 0.2 * 0.5 of it recovered.
    assert_eq!(c.get_air_temperature_for_square(Some(&room), true), -10.0 + 32.0 * 0.5);
    let powered = manager::AirTemperatureQuery { hydro_power_on: true, ..room };
    assert_eq!(c.get_air_temperature_for_square(Some(&powered), false), 22.0);
    let fire = |t: f32| t + 30.0;
    let heated = manager::AirTemperatureQuery { heat_source_highest: Some(&fire), ..outside };
    assert_eq!(c.get_air_temperature_for_square(Some(&heated), false), 20.0);
    assert_eq!(c.get_air_temperature_for_square(None, true), -10.0);
}

#[test]
fn render_values_and_lightning() {
    let (mut gt, mut ctx) = world();
    gt.update(&mut ctx);
    gt.update_climate(&mut ctx);
    let v = gt.climate.render_values(0, false);
    assert_eq!(v.cm_ambient, gt.climate.get_ambient());
    assert_eq!(v.cm_global_light, *gt.climate.get_global_light().borrow());
    // A strike next to the player flashes its lightning (the first one
    // only clears the initial strength of 1, as in the original).
    let mut dl = 0.2;
    for _ in 0..2 {
        dl = 0.2;
        gt.climate.thunder_storm.trigger_thunder_event(510, 700, true, true, false, &mut ctx);
        gt.climate.thunder_storm.update(gt.climate.world_age_hours, &mut dl, 0.01, &mut ctx);
    }
    assert!(dl > 0.2);
    let lit = gt.climate.render_values(0, false);
    assert!(lit.cm_ambient >= v.cm_ambient);
    assert!(ctx.events.iter().any(|e| matches!(e, ClimateEvent::ThunderEvent { strike: true, .. })));
}

#[test]
fn moon_phases() {
    let mut m = ClimateMoon::default();
    m.update_phase(1993, 6, 3);
    let a = m.get_current_moon_phase();
    m.update_phase(1993, 6, 18);
    assert_ne!(a, m.get_current_moon_phase());
    assert!((0.0..=1.0).contains(&m.get_moon_float()));
}
