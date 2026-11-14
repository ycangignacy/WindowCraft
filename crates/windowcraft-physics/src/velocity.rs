// Made by Nordan
//! Release velocity estimation from recent pointer samples.

const CAPACITY: usize = 20;
/// Samples older than this say nothing about the speed at release.
const HORIZON_SECONDS: f64 = 0.10;
/// A pointer that rested this long before release was not flicked.
const STALE_SECONDS: f64 = 0.05;

/// Fixed-size ring buffer of `(position, timestamp)` samples.
#[derive(Debug, Clone)]
pub struct VelocityTracker {
    samples: [(f64, f64); CAPACITY],
    head: usize,
    len: usize,
}

impl Default for VelocityTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl VelocityTracker {
    pub fn new() -> Self {
        Self { samples: [(0.0, 0.0); CAPACITY], head: 0, len: 0 }
    }

    pub fn reset(&mut self) {
        self.len = 0;
        self.head = 0;
    }

    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Records a sample. `timestamp` is in seconds and must not go backwards.
    pub fn push(&mut self, position: f64, timestamp: f64) {
        self.samples[self.head] = (position, timestamp);
        self.head = (self.head + 1) % CAPACITY;
        self.len = (self.len + 1).min(CAPACITY);
    }

    fn sample_from_newest(&self, age: usize) -> (f64, f64) {
        let index = (self.head + CAPACITY - 1 - age) % CAPACITY;
        self.samples[index]
    }

    /// Velocity in units per second as seen at `now`, using a least-squares
    /// fit over the recent window so one noisy sample cannot flip the sign.
    pub fn velocity(&self, now: f64) -> f64 {
        if self.len < 2 {
            return 0.0;
        }
        let (_, newest_time) = self.sample_from_newest(0);
        if now - newest_time > STALE_SECONDS {
            return 0.0;
        }

        let (mut count, mut sum_t, mut sum_x, mut sum_tt, mut sum_tx) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for age in 0..self.len {
            let (position, time) = self.sample_from_newest(age);
            let t = time - newest_time;
            if -t > HORIZON_SECONDS {
                break;
            }
            count += 1.0;
            sum_t += t;
            sum_x += position;
            sum_tt += t * t;
            sum_tx += t * position;
        }

        let denominator = count * sum_tt - sum_t * sum_t;
        if count < 2.0 || denominator.abs() < f64::EPSILON {
            return 0.0;
        }
        (count * sum_tx - sum_t * sum_x) / denominator
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_motion_reports_exact_velocity() {
        let mut tracker = VelocityTracker::new();
        for i in 0..10 {
            let t = i as f64 * 0.008;
            tracker.push(t * 1_500.0, t);
        }
        assert!((tracker.velocity(0.072) - 1_500.0).abs() < 1e-6);
    }

    #[test]
    fn direction_change_uses_only_recent_samples() {
        let mut tracker = VelocityTracker::new();
        for i in 0..10 {
            tracker.push(i as f64 * 10.0, i as f64 * 0.016);
        }
        let turn = 9.0 * 0.016 + 0.2;
        for i in 0..6 {
            tracker.push(90.0 - i as f64 * 10.0, turn + i as f64 * 0.016);
        }
        assert!(tracker.velocity(turn + 0.08) < 0.0);
    }

    #[test]
    fn stale_pointer_reports_zero() {
        let mut tracker = VelocityTracker::new();
        tracker.push(0.0, 0.0);
        tracker.push(100.0, 0.01);
        assert_eq!(tracker.velocity(0.5), 0.0);
    }

    #[test]
    fn fewer_than_two_samples_reports_zero() {
        let mut tracker = VelocityTracker::new();
        assert_eq!(tracker.velocity(0.0), 0.0);
        tracker.push(10.0, 0.0);
        assert_eq!(tracker.velocity(0.0), 0.0);
    }

    #[test]
    fn identical_timestamps_do_not_divide_by_zero() {
        let mut tracker = VelocityTracker::new();
        tracker.push(0.0, 1.0);
        tracker.push(50.0, 1.0);
        assert_eq!(tracker.velocity(1.0), 0.0);
    }

    #[test]
    fn buffer_wraps_without_growing() {
        let mut tracker = VelocityTracker::new();
        for i in 0..1_000 {
            let t = i as f64 * 0.004;
            tracker.push(t * 300.0, t);
        }
        assert_eq!(tracker.len(), CAPACITY);
        assert!((tracker.velocity(3.996) - 300.0).abs() < 1e-6);
    }
}
