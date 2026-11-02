// Made by ycangignacy
//! Momentum projection and rubber-banding.

/// Distance a flick travels before friction stops it.
///
/// `deceleration_rate` is `0.998` for a normal scroll or sheet feel and `0.99`
/// for a snappier settle.
pub fn project_momentum(initial_velocity_px_per_sec: f64, deceleration_rate: f64) -> f64 {
    (initial_velocity_px_per_sec / 1000.0) * deceleration_rate / (1.0 - deceleration_rate)
}

/// Progressive resistance past a boundary. The default constant is `0.55`.
pub fn rubberband(overshoot: f64, dimension: f64, constant: f64) -> f64 {
    (overshoot * dimension * constant) / (dimension + constant * overshoot.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn momentum_matches_reference_values() {
        assert!((project_momentum(1_000.0, 0.998) - 499.0).abs() < 1e-9);
        assert!((project_momentum(2_000.0, 0.998) - 998.0).abs() < 1e-9);
        assert!((project_momentum(1_000.0, 0.99) - 99.0).abs() < 1e-9);
        assert!((project_momentum(-1_000.0, 0.998) + 499.0).abs() < 1e-9);
        assert_eq!(project_momentum(0.0, 0.998), 0.0);
    }

    #[test]
    fn rubberband_approaches_but_never_reaches_dimension() {
        let dimension = 340.0;
        let limit = dimension;
        let mut previous = 0.0;
        for overshoot in [1.0, 10.0, 100.0, 1_000.0, 100_000.0, 10_000_000.0] {
            let value = rubberband(overshoot, dimension, 0.55);
            assert!(value > previous, "must be monotonic");
            assert!(value < limit, "{value} exceeded asymptote");
            previous = value;
        }
        assert!((rubberband(1e12, dimension, 0.55) - limit).abs() < 1e-3);
    }

    #[test]
    fn rubberband_starts_at_zero_with_slope_of_the_constant() {
        assert_eq!(rubberband(0.0, 340.0, 0.55), 0.0);
        let slope = rubberband(0.001, 340.0, 0.55) / 0.001;
        assert!((slope - 0.55).abs() < 1e-3);
    }

    #[test]
    fn rubberband_is_odd() {
        let positive = rubberband(120.0, 340.0, 0.55);
        assert_eq!(rubberband(-120.0, 340.0, 0.55), -positive);
    }
}
