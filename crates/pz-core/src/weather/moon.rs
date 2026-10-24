//! `zombie.iso.weather.ClimateMoon`: the moon phase of a date.

const DAY_YEAR: [i32; 13] = [-1, -1, 30, 58, 89, 119, 150, 180, 211, 241, 272, 303, 333];
/// `getPhaseName()` by phase.
pub const MOON_PHASE_NAMES: [&str; 8] = [
    "New",
    "Waxing crescent",
    "First quarter",
    "Waxing gibbous",
    "Full",
    "Waning gibbous",
    "Third quarter",
    "Waning crescent",
];
const UNITS: [f32; 8] = [0.0, 0.25, 0.5, 0.75, 1.0, 0.75, 0.5, 0.25];

/// `ClimateMoon` (a singleton in the original; the climate owns it here).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClimateMoon {
    last_year: i32,
    last_month: i32,
    last_day: i32,
    current_phase: i32,
    current_float: f32,
}

impl ClimateMoon {
    /// `updatePhase(year, month, day)`.
    pub fn update_phase(&mut self, year: i32, month: i32, day: i32) {
        if year != self.last_year || month != self.last_month || day != self.last_day {
            self.last_year = year;
            self.last_month = month;
            self.last_day = day;
            self.current_phase = Self::get_moon_phase(year, month, day).clamp(0, 7);
            self.current_float = UNITS[self.current_phase as usize];
        }
    }

    pub fn get_phase_name(&self) -> &'static str {
        MOON_PHASE_NAMES[self.current_phase as usize]
    }

    pub fn get_moon_float(&self) -> f32 {
        self.current_float
    }

    pub fn get_current_moon_phase(&self) -> i32 {
        self.current_phase
    }

    fn is_leap_year_p(year: i32) -> bool {
        year % 4 == 0 && (year % 400 == 0 || year % 100 != 0)
    }

    fn get_moon_phase(year: i32, month: i32, day: i32) -> i32 {
        let month = if !(0..=12).contains(&month) { 0 } else { month };
        let mut diy = day + DAY_YEAR[month as usize];
        if month > 2 && Self::is_leap_year_p(year) {
            diy += 1;
        }
        let golden = year % 19 + 1;
        let cent = year / 100 + 1;
        let mut epact = (11 * golden + 20 + (8 * cent + 5) / 25 - 5 - (3 * cent / 4 - 12)) % 30;
        if epact <= 0 {
            epact += 30;
        }
        if epact == 25 && golden > 11 || epact == 24 {
            epact += 1;
        }
        (((diy + epact) * 6 + 11) % 177 / 22) & 7
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        *self = ClimateMoon::default();
    }
}
