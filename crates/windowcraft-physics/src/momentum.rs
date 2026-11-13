// Made by WindowCraft Dev Team
//! Rubber-banding past a boundary.

/// Progressive resistance past a boundary. The default constant is `0.55`.
pub fn rubberband(overshoot: f64, dimension: f64, constant: f64) -> f64 {
    (overshoot * dimension * constant) / (dimension + constant * overshoot.abs())
}

#[cfg(test)]
mod tests {
    use super::*;

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
