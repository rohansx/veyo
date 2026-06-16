//! The owned frame type the harness feeds to the codec.

use veyo_core::Cell;

/// One replayed frame: a timestamp plus the per-region downscaled cells (`cells.len()`
/// must equal the codec grid's `cols × rows`). Produced by [`crate::synthetic`] for
/// tests or by [`crate::decode`] from real PNG frames.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionFrame {
    pub t_ms: u64,
    pub cells: Vec<Cell>,
}
