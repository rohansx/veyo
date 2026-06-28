//! Caption backends. The default [`HeuristicCaptioner`] grounds a caption in the
//! delta's own `summary` plus the OCR'd on-screen text near the moment — no VLM,
//! fully offline. The trait leaves room for a VLM backend later (swap one box).

use crate::detect_binary;
use crate::types::CaptionContext;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Produces a human/agent-readable caption for one salient moment.
pub trait Captioner: Send + Sync {
    fn caption(&self, ctx: &CaptionContext) -> Result<String>;
    fn name(&self) -> &'static str;

    /// Caption many moments in one call. A VLM loads its model **once** here; the default
    /// loops [`Self::caption`] (fine for cheap captioners like the heuristic one).
    fn caption_batch(&self, ctxs: &[CaptionContext]) -> Result<Vec<String>> {
        ctxs.iter().map(|c| self.caption(c)).collect()
    }
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

/// On-device VLM captions via **Moondream2** (~1.8B vision-language model), through a bundled
/// Python sidecar that loads the model **once** and captions every salient frame — a real
/// description of the scene/action, not a templated summary. Empty results per frame fall back
/// to the heuristic caption, so output is never worse than before. Nothing leaves the machine.
#[derive(Debug, Clone)]
pub struct MoondreamCaptioner {
    pub python: String,
    pub script: PathBuf,
    pub prompt: String,
}

/// The Python sidecar: reads `{"prompt", "frames":[{"path","hint"}]}` on stdin, loads
/// Moondream2 once (the `moondream` package if present, else transformers), prints a JSON
/// list of captions (one per frame; "" on per-frame failure).
const MOONDREAM_SIDECAR: &str = r#"
import sys, json, os

def load_model():
    try:
        import moondream as md
        mp = os.environ.get("MOONDREAM_MODEL")
        return ("md", md.vl(model=mp) if mp else md.vl(), None)
    except Exception:
        pass
    from transformers import AutoModelForCausalLM, AutoTokenizer
    model = AutoModelForCausalLM.from_pretrained("vikhyatk/moondream2", trust_remote_code=True)
    try:
        tok = AutoTokenizer.from_pretrained("vikhyatk/moondream2")
    except Exception:
        tok = None
    return ("hf", model, tok)

def cap_one(backend, model, tok, img, prompt):
    if hasattr(model, "caption"):
        try:
            return model.caption(img, length="short")["caption"]
        except Exception:
            pass
    if hasattr(model, "query"):
        try:
            return model.query(img, prompt)["answer"]
        except Exception:
            pass
    enc = model.encode_image(img)
    return model.answer_question(enc, prompt, tok)

def main():
    req = json.load(sys.stdin)
    frames = req.get("frames", [])
    prompt = req.get("prompt") or "Describe what is happening on this screen in one concise sentence."
    out = []
    try:
        from PIL import Image
        backend, model, tok = load_model()
        for f in frames:
            cap = ""
            p = f.get("path")
            if p:
                try:
                    img = Image.open(p).convert("RGB")
                    cap = (cap_one(backend, model, tok, img, prompt) or "").strip()
                except Exception:
                    cap = ""
            out.append(cap)
    except Exception as e:
        sys.stderr.write(str(e))
        out = ["" for _ in frames]
    print(json.dumps(out))

main()
"#;

impl Default for MoondreamCaptioner {
    fn default() -> Self {
        Self {
            python: "python3".into(),
            script: std::env::temp_dir().join("veyo-moondream.py"),
            prompt: "Describe what is happening on this screen in one concise sentence.".into(),
        }
    }
}

impl MoondreamCaptioner {
    /// Available when `moondream` is importable, OR when `CLIPXD_MOONDREAM` is set and
    /// `transformers` is importable. The opt-in env guards against a surprise multi-GB model
    /// download on a random ingest; returns `None` (→ heuristic fallback) otherwise.
    pub fn detect() -> Option<Self> {
        let python = ["python3", "python"].into_iter().find(|p| detect_binary(p))?;
        let import_ok = |m: &str| {
            Command::new(python)
                .args(["-c", &format!("import {m}")])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        let opted_in = std::env::var("CLIPXD_MOONDREAM").map(|v| !v.is_empty() && v != "0").unwrap_or(false);
        if !(import_ok("moondream") || (opted_in && import_ok("transformers"))) {
            return None;
        }
        let d = Self::default();
        std::fs::write(&d.script, MOONDREAM_SIDECAR).ok()?;
        Some(Self { python: python.into(), ..d })
    }
}

impl Captioner for MoondreamCaptioner {
    fn caption(&self, ctx: &CaptionContext) -> Result<String> {
        Ok(self.caption_batch(std::slice::from_ref(ctx))?.into_iter().next().unwrap_or_default())
    }

    fn name(&self) -> &'static str {
        "moondream2"
    }

    fn caption_batch(&self, ctxs: &[CaptionContext]) -> Result<Vec<String>> {
        let frames: Vec<serde_json::Value> = ctxs
            .iter()
            .map(|c| {
                let hint = c.on_screen_text.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" / ");
                serde_json::json!({ "path": c.frame.map(|p| p.display().to_string()), "hint": hint })
            })
            .collect();
        let req = serde_json::json!({ "prompt": self.prompt, "frames": frames }).to_string();

        let mut child = Command::new(&self.python)
            .arg(&self.script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to spawn moondream sidecar via `{}`", self.python))?;
        child.stdin.take().context("no stdin")?.write_all(req.as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            anyhow::bail!("moondream sidecar failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        let caps: Vec<String> = serde_json::from_slice(&out.stdout).unwrap_or_default();

        // per-frame: use the VLM caption, else fall back to the heuristic (never worse).
        let heuristic = HeuristicCaptioner::new();
        let mut result = Vec::with_capacity(ctxs.len());
        for (i, ctx) in ctxs.iter().enumerate() {
            let cap = caps.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
            result.push(if cap.is_empty() { heuristic.caption(ctx)? } else { cap });
        }
        Ok(result)
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
            frame: None,
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
            frame: None,
        };
        let cap = HeuristicCaptioner::new().caption(&ctx).unwrap();
        assert!(cap.contains("region_change"), "{cap}");
        assert!(cap.contains("10,20"), "{cap}");
    }
}
