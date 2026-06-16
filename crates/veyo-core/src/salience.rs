//! Salience scoring — the emission gate.
//!
//! `salience = w_focus · magnitude · novelty`, clamped to `[0,1]`. A change is only
//! emitted upstream when its salience clears `salience_min`. This is the "too
//! sensitive vs too lax" control surface; the factors come from the policy-engine doc.

/// `salience = w_focus · magnitude · novelty`, clamped to `[0,1]`.
///
/// - `focus_weight` — pass [`focus_multiplier`]'s result (1.0 for background).
/// - `magnitude` — normalized diff (pHash distance / SAD), `[0,1]`.
/// - `novelty` — `1 - habituation`, `[0,1]`.
pub fn salience(focus_weight: f32, magnitude: f32, novelty: f32) -> f32 {
    (focus_weight * magnitude * novelty).clamp(0.0, 1.0)
}

/// The focus multiplier: `focus_weight` for the focused surface, else `1.0`.
pub fn focus_multiplier(focused: bool, focus_weight: f32) -> f32 {
    if focused {
        focus_weight
    } else {
        1.0
    }
}

/// Whether a delta clears the emission threshold (inclusive).
pub fn should_emit(salience: f32, salience_min: f32) -> bool {
    salience >= salience_min
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "expected ~{b}, got {a}");
    }

    #[test]
    fn salience_is_the_product_of_its_factors() {
        approx(salience(1.0, 0.5, 0.8), 0.40);
    }

    #[test]
    fn salience_clamps_to_one() {
        approx(salience(1.5, 1.0, 1.0), 1.0);
    }

    #[test]
    fn zero_novelty_kills_salience() {
        approx(salience(1.5, 1.0, 0.0), 0.0);
    }

    #[test]
    fn focus_multiplier_boosts_only_the_focused_surface() {
        approx(focus_multiplier(true, 1.5), 1.5);
        approx(focus_multiplier(false, 1.5), 1.0);
    }

    #[test]
    fn emits_at_or_above_threshold() {
        assert!(should_emit(0.5, 0.4));
        assert!(should_emit(0.4, 0.4)); // boundary is inclusive
    }

    #[test]
    fn suppresses_below_threshold() {
        assert!(!should_emit(0.39, 0.4));
    }
}
