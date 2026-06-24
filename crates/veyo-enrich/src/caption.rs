//! Caption backends. The default [`HeuristicCaptioner`] grounds a caption in the
//! delta's own `summary` plus the OCR'd on-screen text near the moment — no VLM,
//! fully offline. The trait leaves room for a VLM backend later (swap one box).

use crate::types::CaptionContext;
use anyhow::Result;

/// Produces a human/agent-readable caption for one salient moment.
pub trait Captioner: Send + Sync {
    fn caption(&self, ctx: &CaptionContext) -> Result<String>;
    fn name(&self) -> &'static str;
}

/// Builds a caption from the delta summary + nearby on-screen text. Deterministic and
/// offline — the honest Phase-1 captioner until a VLM is wired.
#[derive(Debug, Clone, Copy)]
pub struct HeuristicCaptioner {
    /// Max characters of on-screen text to fold into the caption.
    pub max_text: usize,
}

impl Default for HeuristicCaptioner {
    fn default() -> Self {
        Self { max_text: 160 }
    }
}

impl HeuristicCaptioner {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Captioner for HeuristicCaptioner {
    fn caption(&self, ctx: &CaptionContext) -> Result<String> {
        let max = if self.max_text == 0 { 160 } else { self.max_text };

        // Fold nearby on-screen text into a single quoted snippet, de-duplicating
        // repeated lines (multiple frames near the same moment often repeat text).
        let mut seen = String::new();
        let mut added: Vec<&str> = Vec::new();
        for span in ctx.on_screen_text {
            let t = span.text.trim();
            if t.is_empty() || added.contains(&t) {
                continue;
            }
            added.push(t);
            if !seen.is_empty() {
                seen.push_str(" / ");
            }
            seen.push_str(t);
            if seen.len() >= max {
                break;
            }
        }
        if seen.chars().count() > max {
            seen = seen.chars().take(max).collect::<String>();
            seen.push('…');
        }

        let base = if ctx.summary.trim().is_empty() {
            format!(
                "Salient {} in region [{},{} {}×{}]",
                ctx.delta_kind, ctx.region.x, ctx.region.y, ctx.region.w, ctx.region.h
            )
        } else {
            ctx.summary.trim().to_string()
        };

        Ok(if seen.is_empty() {
            base
        } else {
            format!("{base}. On screen: \"{seen}\"")
        })
    }
    fn name(&self) -> &'static str {
        "heuristic"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OcrSpan, TextSource};
    use veyo_core::Rect;

    fn span(text: &str) -> OcrSpan {
        OcrSpan {
            t_ms: 0,
            text: text.into(),
            source: TextSource::Ocr,
            bbox: None,
            confidence: Some(99.0),
        }
    }

    #[test]
    fn grounds_caption_in_on_screen_text() {
        let spans = vec![span("Payment failed (500)")];
        let ctx = CaptionContext {
            delta_kind: "state_settle",
            summary: "content in main region stopped changing",
            salience: 0.9,
            region: Rect { x: 0, y: 0, w: 100, h: 40 },
            on_screen_text: &spans,
        };
        let cap = HeuristicCaptioner::new().caption(&ctx).unwrap();
        assert!(cap.contains("stopped changing"), "{cap}");
        assert!(cap.contains("Payment failed (500)"), "{cap}");
    }

    #[test]
    fn falls_back_to_region_when_no_summary_or_text() {
        let ctx = CaptionContext {
            delta_kind: "region_change",
            summary: "  ",
            salience: 0.5,
            region: Rect { x: 10, y: 20, w: 30, h: 40 },
            on_screen_text: &[],
        };
        let cap = HeuristicCaptioner::new().caption(&ctx).unwrap();
        assert!(cap.contains("region_change"), "{cap}");
        assert!(cap.contains("10,20"), "{cap}");
    }
}
