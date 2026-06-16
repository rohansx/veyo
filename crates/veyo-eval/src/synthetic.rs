//! Synthetic sessions with known ground truth — so the whole scoring/tuning pipeline
//! is deterministically testable before any real footage exists.

use crate::frame::SessionFrame;
use crate::session::Annotation;
use veyo_core::{Cell, CELL_LEN};

fn solid(v: u8) -> Vec<Cell> {
    vec![[v; CELL_LEN]]
}

/// A page-load-then-settle session on a 1×1 grid: static dark, a jump to bright at
/// 750 ms, then it holds and settles (~1500 ms). One annotation marks the settle —
/// the canonical "the thing finished" event an agent cares about.
pub fn settle_session() -> (Vec<SessionFrame>, Vec<Annotation>) {
    let script: [(u64, u8); 7] = [
        (0, 0),
        (250, 0),
        (500, 0),
        (750, 255),
        (1000, 255),
        (1250, 255),
        (1500, 255),
    ];
    let frames = script
        .iter()
        .map(|&(t, v)| SessionFrame {
            t_ms: t,
            cells: solid(v),
        })
        .collect();
    let annotations = vec![Annotation {
        t_ms: 1500,
        kind: "settle".into(),
        surface: "screen:0".into(),
        note: "content settled".into(),
    }];
    (frames, annotations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_codec, screen_surface};
    use veyo_core::{CodecConfig, EventKind};

    #[test]
    fn settle_session_emits_a_matchable_settle() {
        let (frames, _anns) = settle_session();
        let cfg = CodecConfig {
            grid: (1, 1),
            ..Default::default()
        };
        let deltas = run_codec(&frames, cfg, screen_surface(), (100, 100));
        assert!(
            deltas.iter().any(|d| d.kind == EventKind::StateSettle),
            "expected a StateSettle, got {deltas:?}"
        );
    }
}
