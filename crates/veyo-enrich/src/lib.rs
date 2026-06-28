//! `veyo-enrich` — the meaning layer `veyo-core` deliberately omits.
//!
//! `veyo-core` decides *which* moments are salient (when / where / how-salient) but
//! never retains pixels and never assigns meaning — its [`Delta`](veyo_core::Delta)
//! `summary` is a template string, and its `evidence` (thumbnail/phash) is
//! `#[serde(skip)]`, local-only. `veyo-enrich` turns those salient deltas — plus the
//! salient frames the *caller* retained and optional audio — into the three streams
//! that make a clip agent-legible:
//!
//! - **transcript** — time-aligned speech ([`TranscriptSegment`])
//! - **on-screen text** — OCR'd, located, timestamped ([`OcrSpan`])
//! - **visual timeline** — one captioned moment per salient delta ([`EnrichedMoment`])
//!
//! Every backend is a trait with a no-op default, so the pipeline runs **offline today**
//! (OCR via the system `tesseract`, captions grounded in the on-screen text) and accepts
//! heavier backends (whisper.cpp for transcript, a VLM for captions) by swapping one box.
//!
//! ```no_run
//! use veyo_enrich::{Enricher, EnrichInput};
//! let enricher = Enricher::with_local_defaults();
//! let out = enricher.enrich(&EnrichInput { deltas: &[], frames: &[], audio: None }).unwrap();
//! assert!(out.visual_timeline.is_empty());
//! ```

pub mod caption;
pub mod enrich;
pub mod ocr;
pub mod transcribe;
pub mod types;

pub use caption::{Captioner, HeuristicCaptioner, MoondreamCaptioner};
pub use enrich::{EnrichInput, Enricher};
pub use ocr::{NullOcr, Ocr, PaddleOcr, TesseractCliOcr};
pub use transcribe::{NullTranscriber, Transcriber, WhisperCppTranscriber};
pub use types::{
    CaptionContext, EnrichedMoment, Enrichment, OcrSpan, SalientFrame, TextSource,
    TranscriptSegment,
};

/// Returns true if `bin` looks runnable (probes `bin --version`). Used by backend
/// `detect()` constructors to pick the best locally-available engine without panicking
/// when an engine isn't installed.
pub fn detect_binary(bin: &str) -> bool {
    std::process::Command::new(bin)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
