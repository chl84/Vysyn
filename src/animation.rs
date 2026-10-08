use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct Animation {
    pub frame: usize,
    pub deadline: Option<Instant>,
    completed: u32,
    loops: Option<u32>,
}

impl Animation {
    pub fn start(delays: &[Duration], loops: Option<u32>, now: Instant) -> Self {
        Self {
            frame: 0,
            deadline: (delays.len() > 1).then(|| now + delays[0]),
            completed: 0,
            loops,
        }
    }
    pub fn advance(&mut self, delays: &[Duration], now: Instant) -> bool {
        let Some(mut deadline) = self.deadline else {
            return false;
        };
        if now < deadline || delays.len() < 2 {
            return false;
        }
        let previous = self.frame;
        // Catch up using absolute deadlines; do not accumulate rendering latency.
        for _ in 0..4096 {
            if now < deadline {
                break;
            }
            if self.frame + 1 == delays.len() {
                self.completed = self.completed.saturating_add(1);
                if self.loops.is_some_and(|n| self.completed >= n) {
                    self.deadline = None;
                    return self.frame != previous;
                }
            }
            self.frame = (self.frame + 1) % delays.len();
            deadline += delays[self.frame];
        }
        // Bound catch-up work after a long suspension.
        if deadline <= now {
            deadline = now + delays[self.frame];
        }
        self.deadline = Some(deadline);
        self.frame != previous
    }
}

pub fn frame_delay(numerator_ms: u32, denominator: u32) -> Duration {
    // Zero GIF delays have no useful deadline; use the common 10 ms minimum.
    Duration::from_secs_f64(
        (f64::from(numerator_ms) / f64::from(denominator.max(1)) / 1000.0).max(0.01),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timing_catches_up_without_drift_and_honors_finite_loops() {
        let t = Instant::now();
        let d = [Duration::from_millis(20), Duration::from_millis(50)];
        let mut a = Animation::start(&d, Some(1), t);
        assert!(!a.advance(&d, t + Duration::from_millis(19)));
        assert!(a.advance(&d, t + Duration::from_millis(30)));
        assert_eq!(a.deadline, Some(t + Duration::from_millis(70)));
        a.advance(&d, t + Duration::from_millis(80));
        assert_eq!(a.frame, 1);
        assert!(a.deadline.is_none());
    }
    #[test]
    fn zero_delays_are_bounded() {
        assert_eq!(frame_delay(0, 0), Duration::from_millis(10));
    }
}
