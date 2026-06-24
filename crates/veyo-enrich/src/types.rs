//! Enrichment data types — the meaning layer.
//!
//! These are veyo-enrich's *own* output types; they intentionally do **not** depend on
//! any downstream product (e.g. clipxd), so the dependency arrow points one way:
//! clipxd → veyo-enrich → veyo-core. A consumer maps these into its own index shape.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use veyo_core::{Rect, TimeMs};

/// A time-aligned span of transcribed speech.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub start_ms: TimeMs,
    pub end_ms: TimeMs,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

/// Where a piece of on-screen text came from. OCR for the screen backend; the DOM/a11y
/// tree (verbatim) for the browser backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSource {
    Ocr,
    Dom,
}

/// A timestamped, located piece of on-screen text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OcrSpan {
    pub t_ms: TimeMs,
    pub text: String,
    pub source: TextSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bbox: Option<Rect>,
    /// Mean per-line OCR confidence in `0..=100`, when the engine reports it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// One enriched salient moment — a veyo [`Delta`](veyo_core::Delta) turned into meaning.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnrichedMoment {
    pub t_ms: TimeMs,
    pub salience: f32,
    pub caption: String,
    pub delta_kind: String,
    pub region: Rect,
    /// A reference (path/URL) to the retained, redacted salient frame, if one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_ref: Option<String>,
}

/// A salient frame the caller retained for enrichment.
///
/// veyo-core discards pixels by design, so the **caller** (clipxd) is responsible for
/// writing salient frames to disk and pointing enrichment at them. `region` optionally
/// scopes OCR to the changed area; when `None`, the whole frame is read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SalientFrame {
    pub t_ms: TimeMs,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<Rect>,
}

/// The full enrichment output: the three streams that make a clip agent-legible.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Enrichment {
    pub transcript: Vec<TranscriptSegment>,
    pub on_screen_text: Vec<OcrSpan>,
    pub visual_timeline: Vec<EnrichedMoment>,
}

/// Context handed to a [`Captioner`](crate::Captioner) to describe a salient moment.
pub struct CaptionContext<'a> {
    pub delta_kind: &'a str,
    pub summary: &'a str,
    pub salience: f32,
    pub region: Rect,
    /// On-screen text near this moment in time, already OCR'd — lets a caption be
    /// grounded in what was actually visible.
    pub on_screen_text: &'a [OcrSpan],
}
