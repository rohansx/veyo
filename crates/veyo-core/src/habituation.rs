//! Habituation / novelty.
//!
//! A region's rolling change-frequency estimate. Repetitive change drives habituation
//! up and `novelty = 1 - habituation` toward 0 (spinners/video stop spamming); when a
//! habituated pattern breaks, novelty recovers and events fire again. The exact decay
//! law is a **tunable**, not gospel — this starts as an EWMA.

use serde::{Deserialize, Serialize};

/// Rolling habituation estimate for one region.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoveltyBaseline {
    habituation: f32,
    decay: f32,
}

impl NoveltyBaseline {
    /// `decay` ∈ (0,1): higher = slower to habituate and slower to forget. Values
    /// outside the open interval are clamped, since they'd break the EWMA.
    pub fn new(decay: f32) -> Self {
        Self {
            habituation: 0.0,
            decay: decay.clamp(f32::EPSILON, 1.0 - f32::EPSILON),
        }
    }

    /// Fold in one window's observation (did the region change?).
    ///
    /// An EWMA toward 1.0 while the region keeps changing, toward 0.0 while it's
    /// quiet — so a steady spinner habituates and a long-static region stays novel.
    /// The decay law is a deliberate starting point, not gospel; it's a tunable the
    /// eval harness will refine.
    pub fn observe(&mut self, changed: bool) {
        let target = if changed { 1.0 } else { 0.0 };
        self.habituation = self.decay * self.habituation + (1.0 - self.decay) * target;
    }

    /// Current novelty ∈ [0,1] = `1 - habituation`.
    pub fn novelty(&self) -> f32 {
        (1.0 - self.habituation).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECAY: f32 = 0.9;

    #[test]
    fn a_fresh_region_is_maximally_novel() {
        assert!(NoveltyBaseline::new(DECAY).novelty() > 0.99);
    }

    #[test]
    fn repetitive_change_habituates_toward_silence() {
        // A spinner / autoplay video: changes every window.
        let mut b = NoveltyBaseline::new(DECAY);
        for _ in 0..50 {
            b.observe(true);
        }
        assert!(b.novelty() < 0.05, "novelty stayed high: {}", b.novelty());
    }

    #[test]
    fn a_quiet_region_stays_novel() {
        let mut b = NoveltyBaseline::new(DECAY);
        for _ in 0..50 {
            b.observe(false);
        }
        assert!(b.novelty() > 0.99);
    }

    /// The anti-spam moat's crucial property: habituation is *reversible*. After a
    /// repetitive pattern stops, novelty recovers — so the next real change fires.
    #[test]
    fn novelty_recovers_after_the_pattern_breaks() {
        let mut b = NoveltyBaseline::new(DECAY);
        for _ in 0..50 {
            b.observe(true); // habituated
        }
        assert!(b.novelty() < 0.05);
        for _ in 0..50 {
            b.observe(false); // the spinner stopped
        }
        assert!(
            b.novelty() > 0.9,
            "novelty did not recover: {}",
            b.novelty()
        );
    }
}
