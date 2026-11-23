//! GameWindow.mainThreadStep keeps the unused part of a capped frame interval.
use pz_core::core::PerformanceSettings;
use std::time::{Duration, Instant};

pub(super) struct FramePacer {
    previous: Instant,
    accumulated_ns: u128,
}

impl FramePacer {
    pub(super) fn new(now: Instant) -> Self {
        Self { previous: now, accumulated_ns: 0 }
    }

    /// Return the deadline, or None when a frame is due. A wakeup requests a
    /// redraw; only delivery of that redraw consumes the interval.
    pub(super) fn step(
        &mut self,
        now: Instant,
        settings: &PerformanceSettings,
        redraw: bool,
    ) -> Result<Option<Instant>, &'static str> {
        let elapsed = now.saturating_duration_since(self.previous);
        self.previous = now;
        if settings.uncapped_fps {
            return Ok(None);
        }
        let interval = 1_000_000_000_i64.checked_div(i64::from(settings.lock_fps))
            .filter(|v| *v != 0)
            .ok_or("GameWindow.mainThreadStep: zero frame interval or frame rate")?;
        self.accumulated_ns += elapsed.as_nanos();
        if interval > 0 && self.accumulated_ns < interval as u128 {
            let remaining = interval as u64 - self.accumulated_ns as u64;
            return Ok(Some(now + Duration::from_nanos(remaining)));
        }
        if redraw {
            self.accumulated_ns %= u128::from(interval.unsigned_abs());
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_core::core::Core;

    #[test]
    fn late_redraw_preserves_phase_and_cannot_run_twice() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut core = Core::new();
        core.set_framerate(6); // 120 FPS, including integer-nanosecond truncation
        let mut clock = FramePacer::new(start);
        let period = Duration::from_nanos(8_333_333);
        assert_eq!(clock.step(at(1), &core.performance, true), Ok(Some(start + period)));
        assert_eq!(clock.step(at(25), &core.performance, false), Ok(None));
        assert_eq!(clock.step(at(26), &core.performance, true), Ok(None));
        assert_eq!(clock.step(at(26), &core.performance, true), Ok(Some(start + 4 * period)));
    }

    #[test]
    fn options_changes_reuse_the_remainder_and_uncapped_time_does_not_accrue() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut core = Core::new();
        let mut clock = FramePacer::new(start);
        core.set_framerate(13); // 30 FPS
        assert!(clock.step(at(12), &core.performance, true).unwrap().is_some());
        core.set_framerate(6); // same elapsed time already covers a 120 FPS frame
        assert_eq!(clock.step(at(12), &core.performance, true), Ok(None));
        core.set_framerate(1);
        assert_eq!(clock.step(at(100), &core.performance, true), Ok(None));
        assert_eq!(clock.step(at(200), &core.performance, true), Ok(None));
        core.set_framerate(10);
        assert_eq!(clock.step(at(201), &core.performance, false),
            Ok(Some(at(200) + Duration::from_nanos(12_999_999))));
    }
}
