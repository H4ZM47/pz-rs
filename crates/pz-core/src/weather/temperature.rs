//! `zombie.iso.weather.Temperature`: conversions and the clothing and wind
//! chill formulas.

pub const CELSIUS_POSTFIX: &str = "\u{b0}C";
pub const FAHRENHEIT_POSTFIX: &str = "\u{b0}F";

/// `CelsiusToFahrenheit(celsius)`.
pub fn celsius_to_fahrenheit(c: f32) -> f32 {
    c * 1.8 + 32.0
}

/// `FahrenheitToCelsius(fahrenheit)`.
pub fn fahrenheit_to_celsius(f: f32) -> f32 {
    (f - 32.0) / 1.8
}

/// `WindchillCelsiusKph(t, v)`.
pub fn windchill_celsius_kph(t: f32, v: f32) -> f32 {
    let p = (v as f64).powf(0.16f32 as f64) as f32;
    let w = 13.12 + 0.6215 * t - 11.37 * p + 0.3965 * t * p;
    if w < t {
        w
    } else {
        t
    }
}

/// `getTrueInsulationValue(insulation)`.
pub fn true_insulation_value(i: f32) -> f32 {
    i * 2.0 + 0.5 * i * i * i
}

/// `getTrueWindresistanceValue(windresist)`.
pub fn true_windresistance_value(w: f32) -> f32 {
    w * 1.0 + 0.5 * w * w
}

/// `getTemperaturePostfix()`.
pub fn temperature_postfix(celsius_display: bool) -> &'static str {
    if celsius_display {
        CELSIUS_POSTFIX
    } else {
        FAHRENHEIT_POSTFIX
    }
}

/// `getTemperatureString(celsius)`: one decimal, `Float.toString` style.
pub fn temperature_string(celsius: f32, celsius_display: bool) -> String {
    let v = if celsius_display { celsius } else { celsius_to_fahrenheit(celsius) };
    let v = ((v * 10.0) as f64 + 0.5).floor() as i32 as f32 / 10.0;
    let s = if v == v.trunc() && v.abs() < 1e7 { format!("{v:.1}") } else { format!("{v}") };
    format!("{s} {}", temperature_postfix(celsius_display))
}

/// `getRoundedDisplayTemperature(celsius)` (`PZMath.roundToInt`).
pub fn rounded_display_temperature(celsius: f32, celsius_display: bool) -> i32 {
    let v = if celsius_display { celsius } else { celsius_to_fahrenheit(celsius) };
    (v as f64 + 0.5).floor() as i32
}

/// `getFractionForRealTimeRatePerMin(rate)` with the sandbox day length
/// (`getDayLengthMinutes()`) and its default (`getDayLengthMinutesDefault()`).
pub fn fraction_for_real_time_rate_per_min(rate: f32, day_length_minutes: i32, day_length_default: i32) -> f32 {
    let mut m = day_length_minutes as f32 / day_length_default as f32;
    if m < 1.0 {
        m = 0.5 + 0.5 * m;
    } else if m > 1.0 {
        m = 1.0 + m / 16.0;
    }
    rate / (1440.0 / day_length_minutes as f32) * m
}

/// `getValueColor(val)` as RGBA (`col_0 .. col_100` interpolated).
pub fn value_color(val: f32) -> [f32; 4] {
    let c = |r: i32, g: i32, b: i32| [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0];
    let cols = [c(29, 34, 237), c(0, 255, 234), c(84, 255, 55), c(255, 246, 0), c(255, 0, 0)];
    let val = super::clamp(0.0, 1.0, val);
    let (i, t) = if val < 0.25 {
        (0, val / 0.25)
    } else if val < 0.5 {
        (1, (val - 0.25) / 0.25)
    } else if val < 0.75 {
        (2, (val - 0.5) / 0.25)
    } else {
        (3, (val - 0.75) / 0.25)
    };
    let (a, b) = (cols[i], cols[i + 1]);
    let mut o = [0.0; 4];
    for k in 0..4 {
        o[k] = a[k] + (b[k] - a[k]) * t;
    }
    o
}
