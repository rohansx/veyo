//! Caption backends. The default [`HeuristicCaptioner`] grounds a caption in the
//! delta's own `summary` plus the OCR'd on-screen text near the moment — no VLM,
//! fully offline. The trait leaves room for a VLM backend later (swap one box).

use crate::detect_binary;
use crate::types::CaptionContext;
use anyhow::Result;
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
    # the `moondream` package is only for an explicit local .mf model; its bare vl() defaults
    # to the CLOUD API, which we never want here — so fall through to transformers otherwise.
    mp = os.environ.get("MOONDREAM_MODEL")
    if mp and mp.endswith(".mf"):
        try:
            import moondream as md
            return ("md", md.vl(model=mp), None)
        except Exception:
            pass
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer
    cuda = torch.cuda.is_available()
    model = AutoModelForCausalLM.from_pretrained(
        "vikhyatk/moondream2",
        trust_remote_code=True,
        torch_dtype=(torch.float16 if cuda else torch.float32),
    )
    if cuda:
        model = model.to("cuda")  # fp16 on the GPU (~4.5GB VRAM); else CPU fp32
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
        // auto-enable the transformers path when the model is ALREADY cached locally (no
        // surprise multi-GB download); `CLIPXD_MOONDREAM=1` forces it even before that.
        let cached = std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".cache/huggingface/hub/models--vikhyatk--moondream2").exists())
            .unwrap_or(false);
        if !(import_ok("moondream") || (import_ok("transformers") && (opted_in || cached))) {
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
        let req = serde_json::json!({ "prompt": self.prompt, "frames": frame_payload(ctxs) }).to_string();
        run_caption_sidecar(&self.python, &self.script, &req, ctxs)
    }
}

/// The `[{path, hint}]` array a caption sidecar consumes (hint = nearby OCR text).
fn frame_payload(ctxs: &[CaptionContext]) -> Vec<serde_json::Value> {
    ctxs.iter()
        .map(|c| {
            let hint = c.on_screen_text.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" / ");
            serde_json::json!({ "path": c.frame.map(|p| p.display().to_string()), "hint": hint })
        })
        .collect()
}

/// Spawn a caption sidecar, feed it `request` JSON on stdin, read its JSON `[caption,...]`,
/// and fall back to the heuristic caption for any frame that comes back empty (never worse).
fn run_caption_sidecar(python: &str, script: &std::path::Path, request: &str, ctxs: &[CaptionContext]) -> Result<Vec<String>> {
    let heuristic = HeuristicCaptioner::new();
    // Captioning must NEVER fail the whole pipeline — a dead/unreachable caption server or a
    // crashing sidecar should degrade to heuristic captions, not lose the recording. So every
    // failure path here falls back to heuristic captions for all frames instead of bailing.
    let all_heuristic = || -> Result<Vec<String>> { ctxs.iter().map(|c| heuristic.caption(c)).collect() };

    let caps: Vec<String> = (|| -> Option<Vec<String>> {
        let mut child = Command::new(python)
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(request.as_bytes()).ok()?;
        let out = child.wait_with_output().ok()?;
        if !out.status.success() {
            return None; // sidecar crashed → heuristic for all
        }
        serde_json::from_slice(&out.stdout).ok()
    })()
    .unwrap_or_default();

    if caps.is_empty() {
        return all_heuristic();
    }
    let mut result = Vec::with_capacity(ctxs.len());
    for (i, ctx) in ctxs.iter().enumerate() {
        let cap = caps.get(i).map(|s| s.trim().to_string()).unwrap_or_default();
        result.push(if cap.is_empty() { heuristic.caption(ctx)? } else { cap });
    }
    Ok(result)
}

/// Captions from a **self-hosted** caption server (e.g. Moondream2 on your VPS), enabled by
/// `CLIPXD_CAPTION_URL` (+ optional `CLIPXD_TOKEN`). Offloads the heavy model to your own
/// infrastructure — useful on weak local connections — while keeping it out of third parties'
/// hands. A stdlib-`urllib` sidecar base64-encodes the salient frames and POSTs them in one
/// batch to `<url>/caption`. Per-frame fallback to the heuristic caption on any failure.
#[derive(Debug, Clone)]
pub struct RemoteCaptioner {
    pub python: String,
    pub script: PathBuf,
    pub url: String,
    pub token: String,
    pub prompt: String,
}

const REMOTE_SIDECAR: &str = r#"
import sys, json, base64, time, urllib.request
from concurrent.futures import ThreadPoolExecutor

def main():
    req = json.load(sys.stdin)
    # Detect the wire format. Moondream's hosted API at https://api.moondream.ai/v1/
    # takes {"image_url": "data:image/jpeg;base64,..."} + length/stream and returns
    # {"caption": "..."}. A self-hosted captioner (CLIPXD_CAPTION_URL pointing at your
    # own /caption endpoint) takes {"prompt": "...", "frames": [{"b64": "..."}, ...]}.
    is_moondream_cloud = "moondream.ai" in req["url"]
    headers = {
        "content-type": "application/json",
        # Cloudflare (which fronts api.moondream.ai) refuses requests without a browser-like
        # User-Agent with error 1010 ("browser integrity check"). urllib's default
        # "Python-urllib/3.x" is rejected. Use a Chrome UA which is the standard workaround.
        "user-agent": "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
    }
    if req.get("token"):
        if is_moondream_cloud:
            headers["x-moondream-auth"] = req["token"]
        else:
            headers["authorization"] = "Bearer " + req["token"]

    def encode(f):
        p = f.get("path")
        if not p:
            return None
        try:
            with open(p, "rb") as fh:
                return base64.b64encode(fh.read()).decode("ascii")
        except Exception:
            return None

    def fetch(b64):
        # One caption per call; "" on any failure so the output array stays aligned with
        # the input frames (the Rust side substitutes the heuristic caption per empty slot).
        if b64 is None:
            return ""
        if is_moondream_cloud:
            payload = json.dumps({
                "image_url": "data:image/jpeg;base64," + b64,
                "length": "normal",
                "stream": False,
            }).encode()
        else:
            payload = json.dumps({
                "prompt": req.get("prompt") or "",
                "frames": [{"b64": b64}],
            }).encode()
        for attempt in (0, 1):
            try:
                req_obj = urllib.request.Request(req["url"], data=payload, headers=headers)
                with urllib.request.urlopen(req_obj, timeout=180) as resp:
                    body = json.load(resp)
                if is_moondream_cloud:
                    return body.get("caption") or ""
                if isinstance(body, list):
                    return (body[0] if body else "") or ""
                # self-hosted /caption may return {"captions": [...]} or {"text": "..."}
                return body.get("caption") or body.get("text") or ""
            except Exception as e:
                sys.stderr.write(str(e))
                if attempt == 0:
                    time.sleep(1.0)  # one retry: transient Cloudflare/network blips
        return ""

    # Wall-clock for a clip's captions used to be N_frames x round-trip because every frame
    # was a fresh, serial urllib call. Fan the calls out instead; ex.map preserves order.
    frames = [encode(f) for f in req.get("frames", [])]
    workers = max(1, min(int(req.get("concurrency") or 4), 16))
    with ThreadPoolExecutor(max_workers=workers) as ex:
        captions = list(ex.map(fetch, frames))
    print(json.dumps(captions))

main()
"#;

impl RemoteCaptioner {
    /// Enabled when `CLIPXD_CAPTION_URL` is set and a Python is available. Writes the sidecar.
    pub fn detect() -> Option<Self> {
        let url = std::env::var("CLIPXD_CAPTION_URL").ok().filter(|u| !u.is_empty())?;
        let python = ["python3", "python"].into_iter().find(|p| detect_binary(p))?;
        let script = std::env::temp_dir().join("veyo-remote-caption.py");
        std::fs::write(&script, REMOTE_SIDECAR).ok()?;
        Some(Self {
            python: python.into(),
            script,
            url,
            token: std::env::var("CLIPXD_TOKEN").unwrap_or_default(),
            prompt: "Describe what is happening on this screen in one concise sentence.".into(),
        })
    }
}

impl Captioner for RemoteCaptioner {
    fn caption(&self, ctx: &CaptionContext) -> Result<String> {
        Ok(self.caption_batch(std::slice::from_ref(ctx))?.into_iter().next().unwrap_or_default())
    }

    fn name(&self) -> &'static str {
        "remote"
    }

    fn caption_batch(&self, ctxs: &[CaptionContext]) -> Result<Vec<String>> {
        // Concurrent fan-out inside the sidecar. Default 4 in-flight requests: Moondream
        // Cloud's free tier is rate-limited to 2 req/s (10 req/s paid) and the Modal
        // self-hosted server allows 4 concurrent inputs — override with
        // CLIPXD_CAPTION_CONCURRENCY to match your tier.
        let concurrency = std::env::var("CLIPXD_CAPTION_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&c| c >= 1)
            .unwrap_or(4);
        let req = serde_json::json!({
            "url": self.url,
            "token": self.token,
            "prompt": self.prompt,
            "concurrency": concurrency,
            "frames": frame_payload(ctxs),
        })
        .to_string();
        run_caption_sidecar(&self.python, &self.script, &req, ctxs)
    }
}

/// Captions via **OpenRouter** (any hosted vision model — `gpt-4o-mini`, `gemini-flash`,
/// `qwen-2-vl`, …). The fastest way to *test* the caption feature end-to-end: no model
/// download, no GPU. Enabled by `OPENROUTER_API_KEY` (+ optional `OPENROUTER_VLM_MODEL`).
/// Frames go to a third party — a **testing/opt-in** path; local Moondream stays the private
/// default. A stdlib-`urllib` sidecar sends each frame to the chat-completions endpoint.
#[derive(Debug, Clone)]
pub struct OpenRouterCaptioner {
    pub python: String,
    pub script: PathBuf,
    pub api_key: String,
    pub model: String,
    pub prompt: String,
}

const OPENROUTER_SIDECAR: &str = r#"
import sys, json, base64, urllib.request

def main():
    req = json.load(sys.stdin)
    key = req["api_key"]
    model = req.get("model") or "openai/gpt-4o-mini"
    prompt = req.get("prompt") or "Describe what is happening on this screen in one concise sentence."
    out = []
    for f in req.get("frames", []):
        cap = ""
        p = f.get("path")
        if p:
            try:
                with open(p, "rb") as fh:
                    b64 = base64.b64encode(fh.read()).decode("ascii")
                payload = {
                    "model": model,
                    "max_tokens": 80,
                    "messages": [{"role": "user", "content": [
                        {"type": "text", "text": prompt},
                        {"type": "image_url", "image_url": {"url": "data:image/png;base64," + b64}},
                    ]}],
                }
                r = urllib.request.Request(
                    "https://openrouter.ai/api/v1/chat/completions",
                    data=json.dumps(payload).encode(),
                    headers={"content-type": "application/json",
                             "authorization": "Bearer " + key,
                             "x-title": "clipxd"},
                )
                with urllib.request.urlopen(r, timeout=90) as resp:
                    j = json.load(resp)
                cap = (j["choices"][0]["message"]["content"] or "").strip()
            except Exception as e:
                sys.stderr.write(str(e) + "\n")
                cap = ""
        out.append(cap)
    print(json.dumps(out))

main()
"#;

impl OpenRouterCaptioner {
    /// Enabled when `OPENROUTER_API_KEY` is set. Model from `OPENROUTER_VLM_MODEL` (default a
    /// cheap, strong vision model). Writes the sidecar.
    pub fn detect() -> Option<Self> {
        let api_key = std::env::var("OPENROUTER_API_KEY").ok().filter(|k| !k.is_empty())?;
        let python = ["python3", "python"].into_iter().find(|p| detect_binary(p))?;
        let script = std::env::temp_dir().join("veyo-openrouter-caption.py");
        std::fs::write(&script, OPENROUTER_SIDECAR).ok()?;
        Some(Self {
            python: python.into(),
            script,
            api_key,
            model: std::env::var("OPENROUTER_VLM_MODEL").unwrap_or_else(|_| "openai/gpt-4o-mini".into()),
            prompt: "Describe what is happening on this screen in one concise sentence.".into(),
        })
    }
}

impl Captioner for OpenRouterCaptioner {
    fn caption(&self, ctx: &CaptionContext) -> Result<String> {
        Ok(self.caption_batch(std::slice::from_ref(ctx))?.into_iter().next().unwrap_or_default())
    }

    fn name(&self) -> &'static str {
        "openrouter"
    }

    fn caption_batch(&self, ctxs: &[CaptionContext]) -> Result<Vec<String>> {
        let req = serde_json::json!({
            "api_key": self.api_key,
            "model": self.model,
            "prompt": self.prompt,
            "frames": frame_payload(ctxs),
        })
        .to_string();
        run_caption_sidecar(&self.python, &self.script, &req, ctxs)
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
