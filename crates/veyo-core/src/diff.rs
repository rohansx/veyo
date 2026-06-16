//! Gate-1 cheap change detection.
//!
//! Operates on a region already downscaled to an 8×8 grayscale [`Cell`] (the capture /
//! eval layer does the decode + downscale; the differ stays pure and tiny). The signal
//! is a **mean absolute difference** per pixel, normalized to `[0,1]` — chosen over a
//! perceptual/average hash because raw difference also catches uniform brightness
//! changes (a region going blank → content), which a brightness-invariant hash misses.
//!
//! `epsilon_noise` is interpreted in these magnitude units: `changed = magnitude ≥ ε`.

/// Side length of a downscaled region cell.
pub const CELL_SIDE: usize = 8;
/// Number of grayscale samples in a [`Cell`] (`CELL_SIDE²`).
pub const CELL_LEN: usize = CELL_SIDE * CELL_SIDE;

/// A region downscaled to 8×8 grayscale — the unit Gate 1 compares.
pub type Cell = [u8; CELL_LEN];

/// Mean absolute difference per pixel between two cells, normalized to `[0,1]`.
///
/// `0.0` = identical; `1.0` = maximally different (solid black vs solid white).
pub fn magnitude(a: &Cell, b: &Cell) -> f32 {
    let sum: u32 = a
        .iter()
        .zip(b.iter())
        .map(|(&x, &y)| x.abs_diff(y) as u32)
        .sum();
    (sum as f32 / CELL_LEN as f32) / 255.0
}

/// Convenience: did the region change past the noise floor?
pub fn changed(a: &Cell, b: &Cell, epsilon_noise: f32) -> bool {
    magnitude(a, b) >= epsilon_noise
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected ~{b}, got {a}");
    }

    #[test]
    fn identical_cells_have_zero_magnitude() {
        approx(magnitude(&[7; CELL_LEN], &[7; CELL_LEN]), 0.0);
    }

    #[test]
    fn black_vs_white_is_maximal() {
        approx(magnitude(&[0; CELL_LEN], &[255; CELL_LEN]), 1.0);
    }

    #[test]
    fn half_the_pixels_flipped_is_half_magnitude() {
        let mut b = [0u8; CELL_LEN];
        for p in b.iter_mut().take(CELL_LEN / 2) {
            *p = 255;
        }
        // 32 pixels differ by 255 → mean abs = 255*32/64 = 127.5 → /255 = 0.5
        approx(magnitude(&[0; CELL_LEN], &b), 0.5);
    }

    #[test]
    fn changed_respects_the_epsilon_floor() {
        let a = [10; CELL_LEN];
        let b = [12; CELL_LEN]; // mean abs diff = 2/255 ≈ 0.0078
        assert!(
            !changed(&a, &b, 0.03),
            "tiny diff should be below the floor"
        );
        assert!(changed(&[0; CELL_LEN], &[255; CELL_LEN], 0.03));
    }
}
