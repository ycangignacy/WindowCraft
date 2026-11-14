// Made by Nordan
//! Closed-form damped harmonic oscillator using Apple's parameterization.
//!
//! `omega_n = 2 * PI / response` and `c = 2 * damping * omega_n`, with unit
//! mass. Integrating analytically makes each step exact and independent of the
//! frame rate, which matters because frames are only produced on demand.

use std::f64::consts::TAU;

/// A spring is at rest when both position error and velocity fall below this.
pub const SETTLE_EPSILON: f64 = 0.0005;

const CRITICAL_BAND: f64 = 1e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringConfig {
    pub damping: f64,
    pub response: f64,
}

impl SpringConfig {
    /// Critically damped. Used for tab indicators, accordions, the inspector
    /// drawer and the action dock.
    pub const UI: Self = Self { damping: 1.0, response: 0.32 };
    /// Under-damped. Only used after a gesture supplied real velocity.
    pub const FLICK: Self = Self { damping: 0.82, response: 0.30 };

    pub fn natural_frequency(self) -> f64 {
        TAU / self.response
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spring1D {
    position: f64,
    velocity: f64,
    pub(crate) target: f64,
    config: SpringConfig,
    /// Position error below which the spring counts as at rest, in its own units.
    settle_position: f64,
    /// Speed below which the spring counts as at rest, in units per second.
    settle_velocity: f64,
}

impl Spring1D {
    pub fn new(value: f64, config: SpringConfig) -> Self {
        Self {
            position: value,
            velocity: 0.0,
            target: value,
            config,
            settle_position: SETTLE_EPSILON,
            settle_velocity: SETTLE_EPSILON,
        }
    }

    /// Sets the smallest change that is still visible, in the units of this
    /// spring. Motion below it is invisible, and every extra frame costs a
    /// full view rebuild, so the spring rests once both the error and the
    /// distance covered in a tenth of a second fall below it.
    pub fn set_precision(&mut self, precision: f64) {
        let precision = precision.abs().max(f64::EPSILON);
        self.settle_position = precision;
        self.settle_velocity = precision * 10.0;
    }

    /// The live presentation value. Drawing code reads this, never the target.
    pub fn position(&self) -> f64 {
        self.position
    }

    pub fn velocity(&self) -> f64 {
        self.velocity
    }

    /// Redirects the spring. Position and velocity carry over untouched, so an
    /// interrupted animation continues from exactly where it is on screen.
    pub fn retarget(&mut self, target: f64) {
        self.target = target;
    }

    pub fn retarget_with(&mut self, target: f64, config: SpringConfig) {
        self.target = target;
        self.config = config;
    }

    /// Hands a gesture over to the spring, using the release velocity in
    /// units per second as the initial velocity.
    pub fn release(&mut self, position: f64, velocity: f64, target: f64, config: SpringConfig) {
        self.position = position;
        self.velocity = velocity;
        self.target = target;
        self.config = config;
    }

    /// Pins the spring to a value while a pointer owns it.
    pub fn track(&mut self, position: f64) {
        self.position = position;
        self.velocity = 0.0;
        self.target = position;
    }

    /// Ends all motion immediately.
    pub fn snap(&mut self, value: f64) {
        self.track(value);
    }

    pub fn is_settled(&self) -> bool {
        (self.position - self.target).abs() < self.settle_position && self.velocity.abs() < self.settle_velocity
    }

    /// Advances by `dt` seconds. Returns true while motion continues.
    pub fn step(&mut self, dt: f64) -> bool {
        if dt > 0.0 {
            let (displacement, velocity) = self.solve(self.position - self.target, self.velocity, dt);
            self.position = self.target + displacement;
            self.velocity = velocity;
        }
        if self.is_settled() {
            self.position = self.target;
            self.velocity = 0.0;
            return false;
        }
        true
    }

    fn solve(&self, x0: f64, v0: f64, t: f64) -> (f64, f64) {
        let omega = self.config.natural_frequency();
        let zeta = self.config.damping;

        if (zeta - 1.0).abs() < CRITICAL_BAND {
            let decay = (-omega * t).exp();
            let b = v0 + omega * x0;
            return (decay * (x0 + b * t), decay * (v0 - omega * b * t));
        }

        if zeta < 1.0 {
            let omega_d = omega * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * omega * t).exp();
            let b = (v0 + zeta * omega * x0) / omega_d;
            let (sin, cos) = (omega_d * t).sin_cos();
            let x = decay * (x0 * cos + b * sin);
            let v = decay * (v0 * cos - (x0 * omega_d + zeta * omega * b) * sin);
            return (x, v);
        }

        let root = (zeta * zeta - 1.0).sqrt();
        let r1 = -omega * (zeta - root);
        let r2 = -omega * (zeta + root);
        let c1 = (v0 - r2 * x0) / (r1 - r2);
        let c2 = x0 - c1;
        let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
        (c1 * e1 + c2 * e2, c1 * r1 * e1 + c2 * r2 * e2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f64 = 1.0 / 240.0;

    fn run(spring: &mut Spring1D, mut visit: impl FnMut(&Spring1D)) -> usize {
        let mut steps = 0;
        while spring.step(DT) {
            visit(spring);
            steps += 1;
            assert!(steps < 60_000, "spring failed to settle");
        }
        steps
    }

    #[test]
    fn critically_damped_never_overshoots() {
        let mut spring = Spring1D::new(0.0, SpringConfig::UI);
        spring.retarget(340.0);
        let mut previous = 0.0;
        run(&mut spring, |s| {
            assert!(s.position() <= 340.0 + 1e-9, "overshot to {}", s.position());
            assert!(s.position() >= previous - 1e-9, "reversed direction");
            previous = s.position();
        });
        assert_eq!(spring.position(), 340.0);
    }

    #[test]
    fn under_damped_overshoots_and_settles() {
        let mut spring = Spring1D::new(0.0, SpringConfig::FLICK);
        spring.release(0.0, 2_000.0, 100.0, SpringConfig::FLICK);
        let mut peak: f64 = 0.0;
        run(&mut spring, |s| peak = peak.max(s.position()));
        assert!(peak > 100.0, "expected overshoot, peak was {peak}");
        assert_eq!(spring.position(), 100.0);
    }

    #[test]
    fn over_damped_settles_without_overshoot() {
        let config = SpringConfig { damping: 1.6, response: 0.4 };
        let mut spring = Spring1D::new(0.0, config);
        spring.retarget(50.0);
        run(&mut spring, |s| assert!(s.position() <= 50.0 + 1e-9));
        assert_eq!(spring.position(), 50.0);
    }

    #[test]
    fn analytic_velocity_matches_numeric_derivative() {
        for config in [SpringConfig::UI, SpringConfig::FLICK, SpringConfig { damping: 1.5, response: 0.3 }] {
            let mut spring = Spring1D::new(0.0, config);
            spring.release(0.0, 700.0, 240.0, config);
            for _ in 0..60 {
                let h = 1e-6;
                let mut probe = spring;
                probe.step(h);
                let numeric = (probe.position() - spring.position()) / h;
                assert!(
                    (numeric - spring.velocity()).abs() < 0.05,
                    "{config:?}: numeric {numeric} vs analytic {}",
                    spring.velocity()
                );
                spring.step(1.0 / 120.0);
            }
        }
    }

    #[test]
    fn retarget_preserves_position_and_velocity() {
        let mut spring = Spring1D::new(0.0, SpringConfig::UI);
        spring.retarget(340.0);
        for _ in 0..30 {
            spring.step(DT);
        }
        let (position, velocity) = (spring.position(), spring.velocity());
        spring.retarget(0.0);
        assert_eq!(spring.position(), position);
        assert_eq!(spring.velocity(), velocity);

        // The next step must not jump: the change is bounded by v*dt plus a
        // small acceleration term rather than a teleport.
        spring.step(DT);
        assert!((spring.position() - (position + velocity * DT)).abs() < 2.0);
        assert!(spring.velocity() < velocity, "reversal should decelerate, not reset");
    }

    #[test]
    fn steps_are_frame_rate_independent() {
        let mut coarse = Spring1D::new(0.0, SpringConfig::FLICK);
        let mut fine = coarse;
        coarse.retarget(100.0);
        fine.retarget(100.0);
        coarse.step(0.2);
        for _ in 0..2_000 {
            fine.step(0.0001);
        }
        assert!((coarse.position() - fine.position()).abs() < 1e-6);
    }

    #[test]
    fn settles_exactly_on_target_within_epsilon() {
        let mut spring = Spring1D::new(0.0, SpringConfig::UI);
        spring.retarget(1.0);
        run(&mut spring, |_| {});
        assert!(spring.is_settled());
        assert_eq!(spring.velocity(), 0.0);
        assert!(!spring.step(DT), "a settled spring must stop requesting frames");
    }

    #[test]
    fn settle_time_is_bounded_for_ui_distances() {
        let mut spring = Spring1D::new(0.0, SpringConfig::UI);
        spring.retarget(1_200.0);
        let steps = run(&mut spring, |_| {});
        assert!((steps as f64) * DT < 1.5, "took {}s", steps as f64 * DT);
    }

    #[test]
    fn a_coarse_precision_stops_once_motion_is_invisible() {
        let mut fine = Spring1D::new(0.0, SpringConfig::UI);
        let mut coarse = fine;
        coarse.set_precision(0.25);
        fine.retarget(340.0);
        coarse.retarget(340.0);
        let fine_steps = run(&mut fine, |_| {});
        let coarse_steps = run(&mut coarse, |_| {});
        assert!(coarse_steps * 3 < fine_steps * 2, "coarse {coarse_steps} vs fine {fine_steps}");
        assert_eq!(coarse.position(), 340.0, "a resting spring lands exactly on target");
    }

}
