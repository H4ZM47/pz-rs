//! `zombie.iso.weather.ThunderStorm`: the moving thunder clouds, their
//! strike and rumble events, and the per-player lightning flash.

use super::color::ClimateColorInfo;
use super::{clamp01, lerp, to_radians, ClimateContext, ClimateEvent};

/// `GameTime.AnimTimer`: counts game-speed-scaled seconds at 30 ticks per
/// second.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimTimer {
    pub elapsed: f32,
    pub duration: f32,
    pub finished: bool,
    pub ticks: i32,
}

impl Default for AnimTimer {
    fn default() -> Self {
        AnimTimer { elapsed: 0.0, duration: 0.0, finished: true, ticks: 0 }
    }
}

impl AnimTimer {
    /// `init(ticks)`.
    pub fn init(&mut self, ticks: i32) {
        self.ticks = ticks;
        self.elapsed = 0.0;
        self.duration = ticks as f32 / 30.0;
        self.finished = false;
    }

    /// `update()` with `GameTime.getMultipliedSecondsSinceLastUpdate()`.
    pub fn update(&mut self, multiplied_seconds: f32) {
        self.elapsed += multiplied_seconds * 60.0 / 30.0;
        if self.elapsed >= self.duration {
            self.elapsed = self.duration;
            self.finished = true;
        }
    }

    /// `ratio()`.
    pub fn ratio(&self) -> f32 {
        self.elapsed / self.duration
    }
}

/// `ThunderStorm.ThunderCloud`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThunderCloud {
    pub is_running: bool,
    pub current_x: i32,
    pub current_y: i32,
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub radius: f32,
    pub angle: f32,
    pub strength: f32,
    pub thunder_ratio: f32,
    pub event_frequency: f32,
    pub start_time: f64,
    pub end_time: f64,
    pub duration: f64,
    pub percentage_offset: f32,
    pub suspend_timer: AnimTimer,
}

impl ThunderCloud {
    /// `lifeTime()`.
    pub fn life_time(&self) -> f64 {
        (self.start_time - self.end_time) / self.duration
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct ThunderEventSlot {
    event_x: i32,
    event_y: i32,
    do_lightning: bool,
    do_rumble: bool,
    do_strike: bool,
    sound_delay: AnimTimer,
    is_running: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct PlayerLightningInfo {
    apply_lightning: bool,
    timer: AnimTimer,
    lightning_strength: f32,
    lightning_mod: f32,
    lightning_color: ClimateColorInfo,
    x: i32,
    y: i32,
    distance: i32,
}

impl Default for PlayerLightningInfo {
    fn default() -> Self {
        PlayerLightningInfo {
            apply_lightning: false,
            timer: AnimTimer::default(),
            lightning_strength: 1.0,
            lightning_mod: 0.0,
            lightning_color: ClimateColorInfo::new(1.0, 1.0, 1.0, 1.0),
            x: 0,
            y: 0,
            distance: 0,
        }
    }
}

/// `ThunderStorm`.
#[derive(Clone, Debug, PartialEq)]
pub struct ThunderStorm {
    has_active_thunder_clouds: bool,
    events: Vec<ThunderEventSlot>,
    /// `clouds` (always three).
    pub clouds: [ThunderCloud; 3],
    strike_radius: i32,
    lightning_infos: [PlayerLightningInfo; 4],
    network_thunder_event: ThunderEventSlot,
    /// `dummyCloud`: where a save's clouds past the third load into.
    pub dummy_cloud: Option<ThunderCloud>,
    /// `ThunderStorm.mapMinX` .. `mapMaxY` (statics; `ClimateManager.init`
    /// sets them from the meta grid).
    pub map_min_x: i32,
    pub map_min_y: i32,
    pub map_max_x: i32,
    pub map_max_y: i32,
}

impl Default for ThunderStorm {
    fn default() -> Self {
        ThunderStorm {
            has_active_thunder_clouds: false,
            events: vec![ThunderEventSlot::default(); 30],
            clouds: Default::default(),
            strike_radius: 4000,
            lightning_infos: Default::default(),
            network_thunder_event: ThunderEventSlot::default(),
            dummy_cloud: None,
            map_min_x: -3000,
            map_min_y: -3000,
            map_max_x: 25000,
            map_max_y: 20000,
        }
    }
}

fn add_to_angle(angle: f32, addition: f32) -> f32 {
    let mut a = angle + addition;
    if a > 360.0 {
        a -= 360.0;
    } else if a < 0.0 {
        a += 360.0;
    }
    a
}

fn distance(dx: i32, dy: i32, sx: i32, sy: i32) -> i32 {
    let a = (dx - sx) as f64;
    let b = (dy - sy) as f64;
    (a * a + b * b).sqrt() as i32
}

impl ThunderStorm {
    /// `HasActiveThunderClouds()`.
    pub fn has_active_thunder_clouds(&self) -> bool {
        self.has_active_thunder_clouds
    }

    /// `stopAllClouds()`.
    pub fn stop_all_clouds(&mut self) {
        for c in &mut self.clouds {
            c.is_running = false;
        }
    }

    /// `stopCloud(id)`.
    pub fn stop_cloud(&mut self, id: i32) {
        if let Some(c) = usize::try_from(id).ok().and_then(|i| self.clouds.get_mut(i)) {
            c.is_running = false;
        }
    }

    /// `getMapDiagonal()`.
    pub fn map_diagonal(&self) -> i32 {
        let w = (self.map_max_x - self.map_min_x) as f64;
        let h = (self.map_max_y - self.map_min_y) as f64;
        (w * w + h * h).sqrt() as i32 / 2
    }

    /// `isModifyingNight()`.
    pub fn is_modifying_night(&self) -> bool {
        false
    }

    /// `ThunderCloud.setCenter(centerX, centerY, angle)` on cloud `idx`.
    pub fn set_cloud_center(&mut self, idx: usize, cx: i32, cy: i32, angle: f32, ctx: &mut dyn ClimateContext) {
        let diag = self.map_diagonal();
        let opposing = add_to_angle(angle, 180.0);
        let mut rand_dist = diag + ctx.rand_next_range(1500, 7500);
        let sx = (cx as f64 + rand_dist as f64 * to_radians(opposing as f64).cos()) as i32;
        let sy = (cy as f64 + rand_dist as f64 * to_radians(opposing as f64).sin()) as i32;
        rand_dist = diag + ctx.rand_next_range(1500, 7500);
        let ex = (cx as f64 + rand_dist as f64 * to_radians(angle as f64).cos()) as i32;
        let ey = (cy as f64 + rand_dist as f64 * to_radians(angle as f64).sin()) as i32;
        let c = &mut self.clouds[idx];
        c.start_x = sx;
        c.start_y = sy;
        c.end_x = ex;
        c.end_y = ey;
        c.current_x = sx;
        c.current_y = sy;
    }

    /// `startThunderCloud(str, angle, radius, eventFreq, thunderRatio,
    /// duration, targetRandomPlayer, percentageOffset)`; the index of the
    /// started cloud.
    #[allow(clippy::too_many_arguments)]
    pub fn start_thunder_cloud(
        &mut self,
        strength: f32,
        angle: f32,
        radius: f32,
        event_freq: f32,
        thunder_ratio: f32,
        duration: f64,
        target_random_player: bool,
        percentage_offset: f32,
        world_age_hours: f64,
        ctx: &mut dyn ClimateContext,
    ) -> Option<usize> {
        if ctx.is_client() {
            return None;
        }
        let idx = self.clouds.iter().position(|c| !c.is_running)?;
        let angle = add_to_angle(angle, ctx.rand_next_float(-10.0, 10.0));
        {
            let c = &mut self.clouds[idx];
            c.start_time = world_age_hours;
            c.end_time = c.start_time + duration;
            c.duration = duration;
            c.strength = clamp01(strength);
            c.angle = angle;
            c.radius = if radius > 20000.0 { 20000.0 } else { radius };
            c.event_frequency = event_freq;
            c.thunder_ratio = clamp01(thunder_ratio);
            c.percentage_offset = clamp01(percentage_offset);
        }
        let width = self.map_max_x - self.map_min_x;
        let height = self.map_max_y - self.map_min_y;
        let mut cx = ctx.rand_next_range(self.map_min_x + width / 5, self.map_max_x - width / 5);
        let mut cy = ctx.rand_next_range(self.map_min_y + height / 5, self.map_max_y - height / 5);
        if target_random_player {
            let players: Vec<(f32, f32)> = ctx.players().iter().flatten().copied().collect();
            if !ctx.is_server() {
                if let Some(&(x, y)) = ctx.players()[0].as_ref() {
                    cx = x as i32;
                    cy = y as i32;
                }
            } else if !players.is_empty() {
                let (x, y) = players[ctx.rand_next(players.len() as i32) as usize];
                cx = x as i32;
                cy = y as i32;
            } else {
                // "Thundercloud couldnt target player..."
                return None;
            }
        }
        self.set_cloud_center(idx, cx, cy, angle, ctx);
        self.clouds[idx].is_running = true;
        self.clouds[idx].suspend_timer.init(3);
        Some(idx)
    }

    /// `update(currentTime)`; `day_light_strength_final` is
    /// `ClimateManager.dayLightStrength.finalValue`, which lightning raises.
    pub fn update(
        &mut self,
        current_time: f64,
        day_light_strength_final: &mut f32,
        multiplied_seconds: f32,
        ctx: &mut dyn ClimateContext,
    ) {
        if !ctx.is_client() || ctx.is_server() {
            self.has_active_thunder_clouds = false;
            for i in 0..self.clouds.len() {
                if !self.clouds[i].is_running {
                    continue;
                }
                if current_time >= self.clouds[i].end_time {
                    self.clouds[i].is_running = false;
                    continue;
                }
                let (cxx, cyy, thunder_ratio, cstrength, cradius);
                {
                    let c = &mut self.clouds[i];
                    let mut t = ((current_time - c.start_time) / c.duration) as f32;
                    if c.percentage_offset > 0.0 {
                        t = c.percentage_offset + (1.0 - c.percentage_offset) * t;
                    }
                    c.current_x = lerp(t, c.start_x as f32, c.end_x as f32) as i32;
                    c.current_y = lerp(t, c.start_y as f32, c.end_y as f32) as i32;
                    c.suspend_timer.update(multiplied_seconds);
                    self.has_active_thunder_clouds = true;
                    if !c.suspend_timer.finished {
                        continue;
                    }
                    (cxx, cyy, thunder_ratio, cstrength, cradius) =
                        (c.current_x, c.current_y, c.thunder_ratio, c.strength, c.radius);
                }
                let suspend_next = ctx.rand_next_float(3.5 - 3.0 * cstrength, 24.0 - 20.0 * cstrength);
                self.clouds[i].suspend_timer.init((suspend_next * 60.0) as i32);
                let r = ctx.rand_next_float(0.0, 1.0);
                let half = (cradius / 2.0) as i32;
                self.strike_radius = if r < 0.6 {
                    half / 3
                } else if r < 0.9 {
                    half / 4 * 3
                } else {
                    half
                };
                let sr = self.strike_radius;
                if ctx.rand_next_float(0.0, 1.0) < thunder_ratio {
                    let x = ctx.rand_next_range(cxx - sr, cxx + sr);
                    let y = ctx.rand_next_range(cyy - sr, cyy + sr);
                    let lightning = !ctx.disable_lightning_during_storms();
                    let rumble = ctx.rand_next_float(0.0, 1.0) > 0.4;
                    self.trigger_thunder_event(x, y, true, lightning, rumble, ctx);
                } else {
                    let x = ctx.rand_next_range(cxx - sr, cxx + sr);
                    let y = ctx.rand_next_range(cyy - sr, cyy + sr);
                    self.trigger_thunder_event(x, y, false, false, true, ctx);
                }
            }
        }
        if ctx.is_client() || !ctx.is_server() {
            let players = ctx.players();
            for (i, linfo) in self.lightning_infos.iter_mut().enumerate() {
                if !linfo.apply_lightning {
                    continue;
                }
                linfo.timer.update(multiplied_seconds);
                if !linfo.timer.finished {
                    linfo.lightning_mod = clamp01(linfo.timer.ratio());
                    *day_light_strength_final += (1.0 - *day_light_strength_final) * (1.0 - linfo.lightning_mod);
                    let _ = players[i];
                    continue;
                }
                linfo.timer.init(2);
                linfo.lightning_strength = 0.0;
                linfo.apply_lightning = false;
            }
            let fast_forward = ctx.current_game_speed() > 1;
            let (mut strike, mut rumble) = (false, false);
            for e in &mut self.events {
                if !e.is_running {
                    continue;
                }
                e.sound_delay.update(multiplied_seconds);
                if e.sound_delay.finished {
                    e.is_running = false;
                    if ctx.current_game_speed() > 1 || ctx.sound_disabled() {
                        continue;
                    }
                    if e.do_strike && !(fast_forward && strike) {
                        ctx.event(ClimateEvent::ThunderSound { x: e.event_x, y: e.event_y, rumble: false });
                    }
                    if e.do_rumble && !(fast_forward && rumble) {
                        ctx.event(ClimateEvent::ThunderSound { x: e.event_x, y: e.event_y, rumble: true });
                    }
                    continue;
                }
                strike = strike || e.do_strike;
                rumble = rumble || e.do_rumble;
            }
        }
    }

    /// `triggerThunderEvent(x, y, doStrike, doLightning, doRumble)`.
    pub fn trigger_thunder_event(
        &mut self,
        x: i32,
        y: i32,
        strike: bool,
        lightning: bool,
        rumble: bool,
        ctx: &mut dyn ClimateContext,
    ) {
        if ctx.is_server() {
            let n = &mut self.network_thunder_event;
            n.event_x = x;
            n.event_y = y;
            n.do_strike = strike;
            n.do_lightning = lightning;
            n.do_rumble = rumble;
            ctx.event(ClimateEvent::TransmitClimatePacket(2));
        } else if !ctx.is_client() {
            self.enqueue_thunder_event(x, y, strike, lightning, rumble, ctx);
        }
    }

    /// `enqueueThunderEvent(x, y, doStrike, doLightning, doRumble)`.
    pub fn enqueue_thunder_event(
        &mut self,
        x: i32,
        y: i32,
        strike: bool,
        lightning: bool,
        rumble: bool,
        ctx: &mut dyn ClimateContext,
    ) {
        ctx.event(ClimateEvent::ThunderEvent { x, y, strike, lightning, rumble });
        if !(strike || rumble) {
            return;
        }
        let players = ctx.players();
        let mut dist = 9_999_999;
        for (i, p) in players.iter().enumerate() {
            let Some((px, py)) = p else { continue };
            let pdist = distance(*px as i32, *py as i32, x, y);
            if pdist < dist {
                dist = pdist;
            }
            if lightning {
                let l = &mut self.lightning_infos[i];
                l.distance = pdist;
                l.x = x;
                l.y = y;
            }
        }
        if dist >= 10000 {
            return;
        }
        let Some(e) = self.events.iter_mut().find(|e| !e.is_running) else { return };
        e.do_rumble = rumble;
        e.do_strike = strike;
        e.event_x = x;
        e.event_y = y;
        e.is_running = true;
        e.sound_delay.init((dist as f32 / 300.0 * 60.0) as i32);
        if lightning {
            for (i, p) in players.iter().enumerate() {
                let l = &mut self.lightning_infos[i];
                if p.is_none() || l.distance as f32 >= 7500.0 {
                    continue;
                }
                let ls = 1.0 - l.distance as f32 / 7500.0;
                l.apply_lightning = true;
                if ls > l.lightning_strength {
                    l.lightning_strength = ls;
                    l.timer.init(20 + (80.0 * l.lightning_strength) as i32);
                }
            }
        }
    }

    /// `applyLightningForPlayer(renderSettings, plrIndex, player)` on the
    /// climate values the renderer copied.
    pub fn apply_lightning_for_player(&self, v: &mut super::ClimateRenderValues, plr: usize, weather_shader: bool) {
        let Some(l) = self.lightning_infos.get(plr) else { return };
        if !l.apply_lightning {
            return;
        }
        let gl = v.cm_global_light;
        let mut lc = l.lightning_color;
        for k in 0..3 {
            lc.exterior[k] = gl.exterior[k] + l.lightning_strength * (1.0 - gl.exterior[k]);
            lc.interior[k] = gl.interior[k] + l.lightning_strength * (1.0 - gl.interior[k]);
        }
        let out = lc.interp(&gl, l.lightning_mod);
        for k in 0..3 {
            v.cm_global_light.exterior[k] = out.exterior[k];
            v.cm_global_light.interior[k] = out.interior[k];
        }
        v.cm_ambient = lerp(l.lightning_mod, 1.0, v.cm_ambient);
        v.cm_day_light_strength = lerp(l.lightning_mod, 1.0, v.cm_day_light_strength);
        v.cm_desaturation = lerp(l.lightning_mod, 0.0, v.cm_desaturation);
        v.cm_global_light_intensity = if weather_shader {
            lerp(l.lightning_mod, 1.0, v.cm_global_light_intensity)
        } else {
            lerp(l.lightning_mod, 0.0, v.cm_global_light_intensity)
        };
    }
}
