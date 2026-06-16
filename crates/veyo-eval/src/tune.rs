//! The tuning loop: treat the four knobs as hyperparameters and grid-search them,
//! maximizing recall subject to an emission-rate ceiling. The winning knobs become the
//! locked `veyo.toml` defaults; the session fixtures become CI regression guards.

use crate::frame::SessionFrame;
use crate::run_codec;
use crate::score::{score, Scored};
use crate::session::Annotation;
use veyo_core::{CodecConfig, SurfaceRef};

/// One point in knob-space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knobs {
    pub epsilon_noise: f32,
    pub settle_window_ms: u64,
    pub salience_min: f32,
    pub novelty_decay: f32,
}

/// The coarse search grid: candidate values per knob.
#[derive(Debug, Clone)]
pub struct Grid {
    pub epsilon_noise: Vec<f32>,
    pub settle_window_ms: Vec<u64>,
    pub salience_min: Vec<f32>,
    pub novelty_decay: Vec<f32>,
}

impl Grid {
    /// Total number of knob combinations.
    pub fn len(&self) -> usize {
        self.epsilon_noise.len()
            * self.settle_window_ms.len()
            * self.salience_min.len()
            * self.novelty_decay.len()
    }

    /// True if any axis is empty (no combinations to search).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The winning knobs plus their score and how many combinations were tried.
#[derive(Debug, Clone)]
pub struct TuneResult {
    pub best: Knobs,
    pub score: Scored,
    pub trials: usize,
}

/// Grid-search the four knobs over one session. Keeps the highest recall whose emission
/// rate stays strictly `< emission_ceiling` (matching the gate's strict `< 0.01`),
/// tie-breaking toward lower emission. Returns `None` only if the grid is empty or no
/// combination meets the ceiling.
#[allow(clippy::too_many_arguments)]
pub fn grid_search(
    frames: &[SessionFrame],
    surface: &SurfaceRef,
    dims: (u32, u32),
    base: &CodecConfig,
    grid: &Grid,
    annotations: &[Annotation],
    match_tolerance_ms: u64,
    emission_ceiling: f32,
) -> Option<TuneResult> {
    let duration_ms = frames.last().map(|f| f.t_ms).unwrap_or(0);
    let mut best: Option<(Knobs, Scored)> = None;
    let mut trials = 0usize;

    for &epsilon_noise in &grid.epsilon_noise {
        for &settle_window_ms in &grid.settle_window_ms {
            for &salience_min in &grid.salience_min {
                for &novelty_decay in &grid.novelty_decay {
                    trials += 1;
                    let cfg = CodecConfig {
                        epsilon_noise,
                        settle_window_ms,
                        salience_min,
                        novelty_decay,
                        ..base.clone()
                    };
                    let deltas = run_codec(frames, cfg, surface.clone(), dims);
                    let scored = score(
                        &deltas,
                        annotations,
                        frames.len(),
                        duration_ms,
                        match_tolerance_ms,
                    );
                    if scored.emission_rate >= emission_ceiling {
                        continue;
                    }
                    let knobs = Knobs {
                        epsilon_noise,
                        settle_window_ms,
                        salience_min,
                        novelty_decay,
                    };
                    let better = match &best {
                        None => true,
                        Some((_, b)) => {
                            scored.recall > b.recall
                                || (scored.recall == b.recall
                                    && scored.emission_rate < b.emission_rate)
                        }
                    };
                    if better {
                        best = Some((knobs, scored));
                    }
                }
            }
        }
    }

    best.map(|(best, score)| TuneResult {
        best,
        score,
        trials,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{screen_surface, synthetic::settle_session};

    #[test]
    fn grid_len_is_the_product_of_axes() {
        let g = Grid {
            epsilon_noise: vec![0.02, 0.05],
            settle_window_ms: vec![400],
            salience_min: vec![0.3, 0.5],
            novelty_decay: vec![0.9],
        };
        assert_eq!(g.len(), 4);
        assert!(!g.is_empty());
    }

    #[test]
    fn grid_search_finds_a_high_recall_config_on_the_settle_session() {
        let (frames, anns) = settle_session();
        let base = CodecConfig {
            grid: (1, 1),
            ..Default::default()
        };
        let grid = Grid {
            epsilon_noise: vec![0.02, 0.05],
            settle_window_ms: vec![400],
            salience_min: vec![0.3, 0.5],
            novelty_decay: vec![0.9],
        };
        let res = grid_search(
            &frames,
            &screen_surface(),
            (100, 100),
            &base,
            &grid,
            &anns,
            300,
            1.0,
        )
        .expect("a config should meet the (loose) ceiling");
        assert_eq!(res.trials, 4);
        assert!(res.score.recall >= 0.9, "recall {}", res.score.recall);
    }

    #[test]
    fn returns_none_when_no_config_meets_the_ceiling() {
        let (frames, anns) = settle_session();
        let base = CodecConfig {
            grid: (1, 1),
            ..Default::default()
        };
        let grid = Grid {
            epsilon_noise: vec![0.02, 0.05],
            settle_window_ms: vec![400],
            salience_min: vec![0.3, 0.5],
            novelty_decay: vec![0.9],
        };
        // the settle session emits ~0.29 of frames; a 0.1 ceiling rejects everything.
        let res = grid_search(
            &frames,
            &screen_surface(),
            (100, 100),
            &base,
            &grid,
            &anns,
            300,
            0.1,
        );
        assert!(res.is_none());
    }

    #[test]
    fn returns_none_for_an_empty_grid() {
        let (frames, anns) = settle_session();
        let base = CodecConfig {
            grid: (1, 1),
            ..Default::default()
        };
        let grid = Grid {
            epsilon_noise: vec![],
            settle_window_ms: vec![400],
            salience_min: vec![0.4],
            novelty_decay: vec![0.9],
        };
        assert!(grid.is_empty());
        let res = grid_search(
            &frames,
            &screen_surface(),
            (100, 100),
            &base,
            &grid,
            &anns,
            300,
            1.0,
        );
        assert!(res.is_none());
    }
}
