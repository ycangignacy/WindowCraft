// Made by ycangignacy
//! Demand-driven animation clock.
//!
//! The UI subscribes to display frames only while [`Timeline::needs_frame`] is
//! true. When every channel settles the subscription drops away and the
//! process goes back to sleeping on the message queue.

use crate::spring::{Spring1D, SpringConfig};

/// Duration of the cross-fade that replaces springs under reduced motion.
pub const REDUCED_MOTION_SECONDS: f64 = 0.18;
/// A long stall must not turn into one huge integration step.
const MAX_FRAME_SECONDS: f64 = 1.0 / 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MotionMode {
    #[default]
    Spring,
    /// Windows animations are off or `prefers-reduced-motion` is set.
    Reduced,
}

/// One animated value.
#[derive(Debug, Clone, Copy)]
pub struct Channel {
    spring: Spring1D,
    fade: Option<Fade>,
}

#[derive(Debug, Clone, Copy)]
struct Fade {
    from: f64,
    elapsed: f64,
}

impl Channel {
    pub fn new(value: f64) -> Self {
        Self { spring: Spring1D::new(value, SpringConfig::UI), fade: None }
    }

    pub fn value(&self) -> f64 {
        self.spring.position()
    }

    pub fn velocity(&self) -> f64 {
        self.spring.velocity()
    }

    pub fn is_settled(&self) -> bool {
        self.fade.is_none() && self.spring.is_settled()
    }

    /// Animates towards `target` from the live presentation value.
    pub fn animate_to(&mut self, target: f64, mode: MotionMode) {
        match mode {
            MotionMode::Spring => {
                self.fade = None;
                self.spring.retarget_with(target, SpringConfig::UI);
            }
            MotionMode::Reduced => {
                let from = self.value();
                // A fade to where the value already is would only burn frames.
                if self.is_settled() && from == target {
                    return;
                }
                self.spring.release(from, 0.0, target, SpringConfig::UI);
                self.fade = Some(Fade { from, elapsed: 0.0 });
            }
        }
    }

    /// Hands a released gesture to the flick spring, keeping its velocity.
    pub fn fling_to(&mut self, position: f64, velocity: f64, target: f64, mode: MotionMode) {
        match mode {
            MotionMode::Spring => {
                self.fade = None;
                self.spring.release(position, velocity, target, SpringConfig::FLICK);
            }
            MotionMode::Reduced => {
                self.spring.track(position);
                self.animate_to(target, mode);
            }
        }
    }

    /// Lets a pointer own the value directly.
    pub fn track(&mut self, value: f64) {
        self.fade = None;
        self.spring.track(value);
    }

    pub fn snap(&mut self, value: f64) {
        self.track(value);
    }

    fn step(&mut self, dt: f64) -> bool {
        if let Some(fade) = &mut self.fade {
            fade.elapsed += dt;
            let progress = (fade.elapsed / REDUCED_MOTION_SECONDS).min(1.0);
            let target = self.spring.target;
            let value = fade.from + (target - fade.from) * progress;
            if progress >= 1.0 {
                self.fade = None;
                self.spring.track(target);
                return false;
            }
            self.spring.track(value);
            self.spring.retarget(target);
            return true;
        }
        self.spring.step(dt)
    }
}

/// A set of channels advanced together by a single clock.
#[derive(Debug, Clone)]
pub struct Timeline {
    pub channels: Vec<Channel>,
    last_tick: Option<f64>,
    pub mode: MotionMode,
}

impl Timeline {
    pub fn new(values: impl IntoIterator<Item = f64>) -> Self {
        Self { channels: values.into_iter().map(Channel::new).collect(), last_tick: None, mode: MotionMode::Spring }
    }

    pub fn needs_frame(&self) -> bool {
        self.channels.iter().any(|channel| !channel.is_settled())
    }

    /// Advances to `now` seconds. Returns true while frames are still needed.
    pub fn tick(&mut self, now: f64) -> bool {
        let dt = self.last_tick.map_or(0.0, |last| (now - last).clamp(0.0, MAX_FRAME_SECONDS));
        self.last_tick = Some(now);
        let mut active = false;
        for channel in &mut self.channels {
            active |= channel.step(dt);
        }
        if !active {
            self.last_tick = None;
        }
        active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_stops_requesting_frames_once_settled() {
        let mut timeline = Timeline::new([0.0, 10.0]);
        assert!(!timeline.needs_frame(), "idle timeline must not schedule frames");

        timeline.channels[0].animate_to(340.0, MotionMode::Spring);
        assert!(timeline.needs_frame());

        let mut now = 0.0;
        let mut frames = 0;
        while timeline.tick(now) {
            now += 1.0 / 60.0;
            frames += 1;
            assert!(frames < 600, "animation loop never settled");
        }
        assert!(!timeline.needs_frame());
        assert_eq!(timeline.channels[0].value(), 340.0);
        assert_eq!(timeline.channels[1].value(), 10.0);
    }

    #[test]
    fn stalled_frame_is_clamped() {
        let mut timeline = Timeline::new([0.0]);
        timeline.channels[0].animate_to(100.0, MotionMode::Spring);
        timeline.tick(0.0);
        timeline.tick(30.0);
        assert!(timeline.channels[0].value() < 100.0, "a 30s stall must not teleport");
    }

    #[test]
    fn interruption_continues_from_live_value() {
        let mut timeline = Timeline::new([0.0]);
        timeline.channels[0].animate_to(100.0, MotionMode::Spring);
        timeline.tick(0.0);
        timeline.tick(0.05);
        let (value, velocity) = (timeline.channels[0].value(), timeline.channels[0].velocity());
        timeline.channels[0].animate_to(0.0, MotionMode::Spring);
        assert_eq!(timeline.channels[0].value(), value);
        assert_eq!(timeline.channels[0].velocity(), velocity);
    }

    #[test]
    fn reduced_motion_is_a_180ms_linear_fade() {
        let mut timeline = Timeline::new([0.0]);
        timeline.mode = MotionMode::Reduced;
        timeline.channels[0].animate_to(1.0, MotionMode::Reduced);
        timeline.tick(0.0);
        timeline.tick(0.045);
        assert!((timeline.channels[0].value() - 0.25).abs() < 1e-9);

        let mut now = 0.045;
        while timeline.tick(now) {
            now += 0.045;
        }
        assert_eq!(timeline.channels[0].value(), 1.0);
        assert!(now <= 0.23, "fade took {now}s");
    }

    #[test]
    fn reduced_motion_ignores_a_target_it_already_holds() {
        let mut timeline = Timeline::new([1.0]);
        timeline.mode = MotionMode::Reduced;
        timeline.channels[0].animate_to(1.0, MotionMode::Reduced);
        assert!(!timeline.needs_frame());
    }

    #[test]
    fn tracking_a_channel_stops_it() {
        let mut channel = Channel::new(0.0);
        channel.animate_to(50.0, MotionMode::Spring);
        channel.track(20.0);
        assert!(channel.is_settled());
        assert_eq!(channel.value(), 20.0);
    }
}
