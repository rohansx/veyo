//! `veyo-eval` — the Phase-0 offline eval harness.
//!
//! Replays a recorded session (frames + timestamps) through the pure [`veyo_core`]
//! pipeline with **no capture backend and no daemon**, collects the deltas it *would*
//! emit, and scores them against a human annotation of "things that mattered." It is
//! both the tuning instrument and the permanent regression test for the codec.
//!
//! The go/no-go gate it exists to answer:
//! > on ≥3 real recorded sessions, **recall ≥ ~0.9** at **emission < ~1% of frames**,
//! > on CPU.
//!
//! See `docs/eval-harness.md` for the methodology.

pub mod downscale;
pub mod frame;
pub mod report;
pub mod score;
pub mod session;
pub mod synthetic;
pub mod tune;

#[cfg(feature = "decode")]
pub mod decode;

pub use frame::SessionFrame;
pub use score::{score, Scored};
pub use session::{Annotation, FrameMeta, Session};
pub use tune::{grid_search, Grid, Knobs, TuneResult};

use veyo_core::{Codec, CodecConfig, Delta, Frame, SurfaceRef};

/// Replay a session's frames through a freshly-built [`Codec`] and collect every delta.
///
/// This is the heart of the harness: a pure function from (frames, config) to deltas,
/// so scoring and tuning can run it thousands of times deterministically.
pub fn run_codec(
    frames: &[SessionFrame],
    cfg: CodecConfig,
    surface: SurfaceRef,
    dims: (u32, u32),
) -> Vec<Delta> {
    let mut codec = Codec::new(cfg, surface, dims);
    let mut out = Vec::new();
    for f in frames {
        out.extend(codec.observe(Frame {
            t_ms: f.t_ms,
            cells: &f.cells,
        }));
    }
    out
}

/// The default focused single-surface descriptor used for a one-display screen source.
pub fn screen_surface() -> SurfaceRef {
    SurfaceRef {
        id: "screen:0".to_string(),
        app: "screen".to_string(),
        title: "display 0".to_string(),
        focused: true,
    }
}
