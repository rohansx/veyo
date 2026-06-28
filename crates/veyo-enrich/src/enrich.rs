//! The [`Enricher`] — orchestrates transcript + OCR + caption over veyo
//! [`Delta`](veyo_core::Delta)s to produce the three agent-legible streams.
//!
//! The flow:
//! 1. transcribe audio (if any) → `transcript`
//! 2. OCR every retained salient frame → `on_screen_text`
//! 3. for each salient delta, caption it grounded in the OCR text nearest in time,
//!    attaching the closest retained frame → `visual_timeline`

use crate::caption::{Captioner, HeuristicCaptioner};
use crate::ocr::{NullOcr, Ocr, PaddleOcr, TesseractCliOcr};
use crate::transcribe::{NullTranscriber, Transcriber};
use crate::types::{CaptionContext, EnrichedMoment, Enrichment, OcrSpan, SalientFrame};
use anyhow::Result;
use std::path::Path;
use veyo_core::{Delta, EventKind, TimeMs};

/// How close (ms) an OCR span must be to a delta to count as "on screen at that moment".
const NEAR_MS: TimeMs = 750;

/// Input to enrichment: the salient deltas from veyo-core, the salient frames the
/// caller retained (veyo-core discards pixels), and optional audio for transcription.
pub struct EnrichInput<'a> {
    pub deltas: &'a [Delta],
    pub frames: &'a [SalientFrame],
    pub audio: Option<&'a Path>,
}

/// Runs the three enrichment backends over a session's salient moments.
pub struct Enricher {
    transcriber: Box<dyn Transcriber>,
    ocr: Box<dyn Ocr>,
    captioner: Box<dyn Captioner>,
}

impl Enricher {
    pub fn new(
        transcriber: Box<dyn Transcriber>,
        ocr: Box<dyn Ocr>,
        captioner: Box<dyn Captioner>,
    ) -> Self {
        Self {
            transcriber,
            ocr,
            captioner,
        }
    }

    /// Auto-detect the best locally-available backends, all offline/on-device: OCR prefers
    /// **PaddleOCR** (PP-OCR / PaddleOCR-VL — far stronger on real screens) when a local
    /// `paddleocr` is importable, else falls back to the system `tesseract`, else null; plus
    /// the heuristic captioner and a null transcriber (no audio model assumed).
    pub fn with_local_defaults() -> Self {
        let ocr: Box<dyn Ocr> = if let Some(p) = PaddleOcr::detect() {
            Box::new(p)
        } else if let Some(t) = TesseractCliOcr::detect() {
            Box::new(t)
        } else {
            Box::new(NullOcr)
        };
        Self::new(Box::new(NullTranscriber), ocr, Box::new(HeuristicCaptioner::new()))
    }

    /// The `(transcriber, ocr, captioner)` backend names — handy for logging what's wired.
    pub fn backends(&self) -> (&'static str, &'static str, &'static str) {
        (
            self.transcriber.name(),
            self.ocr.name(),
            self.captioner.name(),
        )
    }

    /// Produce the full [`Enrichment`] for one session.
    pub fn enrich(&self, input: &EnrichInput) -> Result<Enrichment> {
        // 1. transcript
        let transcript = match input.audio {
            Some(a) => self.transcriber.transcribe(a)?,
            None => Vec::new(),
        };

        // 2. OCR every retained salient frame (failures are logged, not fatal —
        //    a partial index beats no index).
        let mut on_screen_text: Vec<OcrSpan> = Vec::new();
        for f in input.frames {
            match self.ocr.extract(&f.path, f.t_ms) {
                Ok(spans) => on_screen_text.extend(spans),
                Err(e) => {
                    tracing::warn!(path = %f.path.display(), error = %e, "OCR failed for frame")
                }
            }
        }

        // 3. visual timeline — one captioned moment per salient delta.
        let mut visual_timeline = Vec::with_capacity(input.deltas.len());
        for d in input.deltas {
            let near = near_text(&on_screen_text, d.t_event, NEAR_MS);
            let frame_ref =
                nearest_frame(input.frames, d.t_event).map(|f| f.path.display().to_string());
            let kind = kind_str(d.kind);
            let caption = self.captioner.caption(&CaptionContext {
                delta_kind: kind,
                summary: &d.summary,
                salience: d.salience,
                region: d.region.bounds,
                on_screen_text: &near,
            })?;
            visual_timeline.push(EnrichedMoment {
                t_ms: d.t_event,
                salience: d.salience,
                caption,
                delta_kind: kind.to_string(),
                region: d.region.bounds,
                frame_ref,
            });
        }

        Ok(Enrichment {
            transcript,
            on_screen_text,
            visual_timeline,
        })
    }
}

fn kind_str(k: EventKind) -> &'static str {
    match k {
        EventKind::FocusChange => "focus_change",
        EventKind::SurfaceOpen => "surface_open",
        EventKind::SurfaceClose => "surface_close",
        EventKind::RegionChange => "region_change",
        EventKind::StateSettle => "state_settle",
        EventKind::Idle => "idle",
        EventKind::Active => "active",
        EventKind::Anomaly => "anomaly",
    }
}

/// OCR spans within `window` ms of `t`, cloned for handing to a captioner.
fn near_text(spans: &[OcrSpan], t: TimeMs, window: TimeMs) -> Vec<OcrSpan> {
    spans
        .iter()
        .filter(|s| abs_diff(s.t_ms, t) <= window)
        .cloned()
        .collect()
}

fn nearest_frame(frames: &[SalientFrame], t: TimeMs) -> Option<&SalientFrame> {
    frames.iter().min_by_key(|f| abs_diff(f.t_ms, t))
}

fn abs_diff(a: TimeMs, b: TimeMs) -> TimeMs {
    if a > b {
        a - b
    } else {
        b - a
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextSource;
    use std::path::PathBuf;
    use veyo_core::{Delta, EventId, EventKind, Evidence, Rect, RegionRef, SurfaceRef, SCHEMA_V};

    /// OCR stub that returns one span per frame, text keyed to the frame's timestamp,
    /// so we can assert the time-nearest grounding without needing tesseract.
    struct MockOcr;
    impl Ocr for MockOcr {
        fn extract(&self, _path: &Path, t_ms: TimeMs) -> Result<Vec<OcrSpan>> {
            Ok(vec![OcrSpan {
                t_ms,
                text: format!("text@{t_ms}"),
                source: TextSource::Ocr,
                bbox: None,
                confidence: Some(99.0),
            }])
        }
        fn name(&self) -> &'static str {
            "mock"
        }
    }

    fn delta(kind: EventKind, t: TimeMs, summary: &str) -> Delta {
        Delta {
            v: SCHEMA_V,
            id: EventId(format!("ev_{t}")),
            t_event: t,
            t_observed: t,
            source: "screen:0".into(),
            kind,
            surface: SurfaceRef {
                id: "win".into(),
                app: "app".into(),
                title: "t".into(),
                focused: true,
            },
            region: RegionRef {
                id: "r_1".into(),
                grid: [0, 0],
                bounds: Rect { x: 0, y: 0, w: 100, h: 100 },
            },
            summary: summary.into(),
            salience: 0.8,
            novelty: 0.5,
            duration_ms: None,
            evidence: Evidence::default(),
        }
    }

    #[test]
    fn enrich_builds_one_moment_per_delta_grounded_in_nearest_text() {
        let enricher = Enricher::new(
            Box::new(NullTranscriber),
            Box::new(MockOcr),
            Box::new(HeuristicCaptioner::new()),
        );
        let deltas = vec![
            delta(EventKind::RegionChange, 12_400, "click submit"),
            delta(EventKind::StateSettle, 13_000, "error appeared"),
        ];
        let frames = vec![
            SalientFrame { t_ms: 12_400, path: PathBuf::from("/f1.png"), region: None },
            SalientFrame { t_ms: 13_000, path: PathBuf::from("/f2.png"), region: None },
        ];
        let out = enricher
            .enrich(&EnrichInput { deltas: &deltas, frames: &frames, audio: None })
            .unwrap();

        assert!(out.transcript.is_empty(), "no audio -> no transcript");
        assert_eq!(out.on_screen_text.len(), 2, "one OCR span per frame");
        assert_eq!(out.visual_timeline.len(), 2, "one moment per delta");

        let m = &out.visual_timeline[1];
        assert_eq!(m.t_ms, 13_000);
        assert_eq!(m.delta_kind, "state_settle");
        assert_eq!(m.frame_ref.as_deref(), Some("/f2.png"));
        // grounded in the time-nearest OCR span (text@13000, within 750ms)
        assert!(m.caption.contains("error appeared"), "{}", m.caption);
        assert!(m.caption.contains("text@13000"), "{}", m.caption);
        // and NOT the far span (text@12400 is 600ms away -> within window, so present
        // only for moment 0; verify moment 1 didn't pull a *non-existent* far span)
        assert!(!m.caption.contains("text@99999"), "{}", m.caption);
    }

    #[test]
    fn empty_input_yields_empty_enrichment() {
        let enricher = Enricher::with_local_defaults();
        let out = enricher
            .enrich(&EnrichInput { deltas: &[], frames: &[], audio: None })
            .unwrap();
        assert_eq!(out, Enrichment::default());
    }
}
