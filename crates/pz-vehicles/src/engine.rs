//! `VehicleEngine`, `TransmissionNumber`, `BaseVehicle.engineStateTypes`,
//! `VehicleEngineStateChangeReason` and `UpdateLimit`.
//!
//! The engine's state machine lives here; what the original does through
//! its listener (`BaseVehicle.onEngineStateChanged`) and its owner
//! (battery, fuel, `isEngineWorking`, world sounds) is done by the vehicle
//! (see `vehicle.rs`), which drives these transitions.

use crate::env;

/// `BaseVehicle.engineStateTypes`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EngineState {
    #[default]
    Idle,
    Starting,
    RetryingStarting,
    StartingSuccess,
    StartingFailed,
    Running,
    Stalling,
    ShuttingDown,
}

impl EngineState {
    pub const VALUES: [EngineState; 8] = [
        EngineState::Idle,
        EngineState::Starting,
        EngineState::RetryingStarting,
        EngineState::StartingSuccess,
        EngineState::StartingFailed,
        EngineState::Running,
        EngineState::Stalling,
        EngineState::ShuttingDown,
    ];

    pub fn ordinal(self) -> i32 {
        self as i32
    }

    pub fn name(self) -> &'static str {
        match self {
            EngineState::Idle => "Idle",
            EngineState::Starting => "Starting",
            EngineState::RetryingStarting => "RetryingStarting",
            EngineState::StartingSuccess => "StartingSuccess",
            EngineState::StartingFailed => "StartingFailed",
            EngineState::Running => "Running",
            EngineState::Stalling => "Stalling",
            EngineState::ShuttingDown => "ShuttingDown",
        }
    }
}

/// `VehicleEngineStateChangeReason`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineReason {
    EngineConditionLow,
    EngineNotWorking,
    EngineQualityLow,
    NoPower,
    OutOfFuel,
    ScriptReloaded,
    TurnedOff,
}

/// `TransmissionNumber`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Transmission {
    R,
    #[default]
    N,
    Speed1,
    Speed2,
    Speed3,
    Speed4,
    Speed5,
    Speed6,
    Speed7,
    Speed8,
}

impl Transmission {
    /// `getIndex`.
    pub fn index(self) -> i32 {
        match self {
            Transmission::R => -1,
            Transmission::N => 0,
            Transmission::Speed1 => 1,
            Transmission::Speed2 => 2,
            Transmission::Speed3 => 3,
            Transmission::Speed4 => 4,
            Transmission::Speed5 => 5,
            Transmission::Speed6 => 6,
            Transmission::Speed7 => 7,
            Transmission::Speed8 => 8,
        }
    }

    /// `fromIndex` (unknown indices are `N`).
    pub fn from_index(index: i32) -> Self {
        match index {
            -1 => Transmission::R,
            1 => Transmission::Speed1,
            2 => Transmission::Speed2,
            3 => Transmission::Speed3,
            4 => Transmission::Speed4,
            5 => Transmission::Speed5,
            6 => Transmission::Speed6,
            7 => Transmission::Speed7,
            8 => Transmission::Speed8,
            _ => Transmission::N,
        }
    }

    /// `getNext(gearRatioCount)`.
    pub fn next(self, gear_ratio_count: i32) -> Self {
        if self.index() == -1 || self.index() == gear_ratio_count {
            return self;
        }
        Self::from_index(self.index() + 1)
    }

    /// `getPrev(gearRatioCount)`.
    pub fn prev(self, gear_ratio_count: i32) -> Self {
        if self.index() == -1 || self.index() == gear_ratio_count {
            return self;
        }
        Self::from_index(self.index() - 1)
    }

    /// `getString`.
    pub fn letter(self) -> &'static str {
        match self {
            Transmission::R => "R",
            Transmission::N => "N",
            Transmission::Speed1 => "1",
            Transmission::Speed2 => "2",
            Transmission::Speed3 => "3",
            Transmission::Speed4 => "4",
            Transmission::Speed5 => "5",
            Transmission::Speed6 => "6",
            Transmission::Speed7 => "7",
            Transmission::Speed8 => "8",
        }
    }

    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            Transmission::R => "R",
            Transmission::N => "N",
            Transmission::Speed1 => "Speed1",
            Transmission::Speed2 => "Speed2",
            Transmission::Speed3 => "Speed3",
            Transmission::Speed4 => "Speed4",
            Transmission::Speed5 => "Speed5",
            Transmission::Speed6 => "Speed6",
            Transmission::Speed7 => "Speed7",
            Transmission::Speed8 => "Speed8",
        }
    }
}

/// `zombie.core.utils.UpdateLimit` over [`env::now_ms`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UpdateLimit {
    pub delay: i64,
    pub last: i64,
    pub last_period: i64,
}

impl UpdateLimit {
    /// `new UpdateLimit(ms)`: starts at the current time.
    pub fn new(ms: i64) -> Self {
        let now = env::now_ms();
        UpdateLimit {
            delay: ms,
            last: now,
            last_period: now,
        }
    }

    /// `Check()`.
    pub fn check(&mut self) -> bool {
        let ms = env::now_ms();
        if ms - self.last > self.delay {
            self.last = if ms - self.last > 3 * self.delay {
                ms
            } else {
                self.last + self.delay
            };
            return true;
        }
        false
    }

    /// `Reset()`.
    pub fn reset(&mut self) {
        let now = env::now_ms();
        self.last = now;
        self.last_period = now;
    }
}

/// `VehicleEngine` (the state; transitions are driven by the vehicle).
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleEngine {
    pub state: EngineState,
    pub speed: f64,
    pub quality: i32,
    pub loudness: i32,
    pub power: i32,
    pub engine_check_time: i64,
    pub update_state_time: i64,
    pub reason: Option<EngineReason>,
}

impl Default for VehicleEngine {
    fn default() -> Self {
        VehicleEngine {
            state: EngineState::Idle,
            speed: 0.0,
            quality: 0,
            loudness: 0,
            power: 0,
            engine_check_time: 0,
            update_state_time: 0,
            reason: None,
        }
    }
}

impl VehicleEngine {
    /// `MYSTERY_LOUDNESS_SCALE`.
    pub const LOUDNESS_SCALE: f32 = 0.370_370_36;

    /// `setFeatures(quality, loudness, power)`.
    pub fn set_features(&mut self, quality: i32, loudness: i32, power: i32) {
        self.quality = quality.clamp(0, 100);
        self.loudness = (loudness as f32 * Self::LOUDNESS_SCALE) as i32;
        self.power = power;
    }

    /// `load(engineRunning, loudness, quality, power)`.
    pub fn load(&mut self, running: bool, loudness: i32, quality: i32, power: i32) {
        if running {
            self.state = EngineState::Running;
        }
        self.loudness = loudness;
        self.quality = quality;
        self.power = power;
    }

    /// `isRunning`.
    pub fn is_running(&self) -> bool {
        self.state == EngineState::Running
    }

    /// `isStarting`.
    pub fn is_starting(&self) -> bool {
        matches!(
            self.state,
            EngineState::Starting | EngineState::StartingFailed | EngineState::StartingSuccess
        )
    }

    /// `isStarted`.
    pub fn is_started(&self) -> bool {
        matches!(
            self.state,
            EngineState::Starting
                | EngineState::StartingFailed
                | EngineState::StartingSuccess
                | EngineState::RetryingStarting
        )
    }

    /// The transition shared by the `do*` methods: sets the state and the
    /// time, returns the old state.
    pub fn transition(&mut self, state: EngineState) -> EngineState {
        let old = self.state;
        self.state = state;
        self.update_state_time = env::now_ms();
        old
    }
}
