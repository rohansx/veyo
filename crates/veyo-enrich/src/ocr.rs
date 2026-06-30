//! OCR backends. The default shells out to the system `tesseract` CLI and parses its
//! TSV output (per-line text + bounding boxes + confidence) — robust, with no FFI or
//! build-time dependency. [`NullOcr`] keeps the pipeline working when no engine exists.

use crate::detect_binary;
use crate::types::{OcrSpan, TextSource};
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use veyo_core::{Rect, TimeMs};

/// Extracts on-screen text from a frame image.
pub trait Ocr: Send + Sync {
    /// OCR the image at `path`, stamping every span with `t_ms`.
    fn extract(&self, path: &Path, t_ms: TimeMs) -> Result<Vec<OcrSpan>>;
    fn name(&self) -> &'static str;

    /// OCR many frames at once, returning all spans (each stamped with its frame's `t_ms`).
    /// The default loops [`Self::extract`]; engines with heavy per-call startup (e.g. PaddleOCR
    /// reloading its model) override this to load once and process the whole batch — the single
    /// biggest win on indexing latency. Per-frame failures are skipped, not fatal.
    fn extract_batch(&self, frames: &[(std::path::PathBuf, TimeMs)]) -> Vec<OcrSpan> {
        frames.iter().flat_map(|(p, t)| self.extract(p, *t).unwrap_or_default()).collect()
    }
}

/// No-op OCR — returns nothing. The default when no engine is available.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullOcr;

impl Ocr for NullOcr {
    fn extract(&self, _path: &Path, _t_ms: TimeMs) -> Result<Vec<OcrSpan>> {
        Ok(Vec::new())
    }
    fn name(&self) -> &'static str {
        "null"
    }
}

/// OCR via the system `tesseract` CLI (v4/v5). We ask for TSV so we get per-word
/// bounding boxes + confidence, then aggregate words into lines.
#[derive(Debug, Clone)]
pub struct TesseractCliOcr {
    pub bin: String,
    pub lang: String,
    /// Minimum per-line mean confidence to keep a span (`0..=100`).
    pub min_confidence: f32,
}

impl Default for TesseractCliOcr {
    fn default() -> Self {
        Self {
            bin: "tesseract".into(),
            lang: "eng".into(),
            min_confidence: 40.0,
        }
    }
}

impl TesseractCliOcr {
    /// The default config, but only if a `tesseract` binary is on `PATH`.
    pub fn detect() -> Option<Self> {
        let d = Self::default();
        if detect_binary(&d.bin) {
            Some(d)
        } else {
            None
        }
    }
}

impl Ocr for TesseractCliOcr {
    fn extract(&self, path: &Path, t_ms: TimeMs) -> Result<Vec<OcrSpan>> {
        let out = Command::new(&self.bin)
            .arg(path)
            .arg("stdout")
            .arg("-l")
            .arg(&self.lang)
            .arg("--psm")
            .arg("6")
            .arg("tsv")
            .output()
            .with_context(|| format!("failed to run `{}`", self.bin))?;
        if !out.status.success() {
            anyhow::bail!(
                "`{}` failed: {}",
                self.bin,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let tsv = String::from_utf8_lossy(&out.stdout);
        Ok(parse_tsv(&tsv, t_ms, self.min_confidence))
    }
    fn name(&self) -> &'static str {
        "tesseract"
    }
}

/// OCR via a local **PaddleOCR** install (PP-OCR / PaddleOCR-VL), through a bundled Python
/// sidecar that normalizes every PaddleOCR version's output to one stable JSON contract:
/// `[{"text": str, "conf": float(0..1), "bbox": [x,y,w,h]}]`. Fully **on-device** — nothing
/// leaves the machine. Much stronger than tesseract on real screens (UI, code, tables, mixed
/// layout). Selected over tesseract by [`Enricher::with_local_defaults`] when available.
#[derive(Debug, Clone)]
pub struct PaddleOcr {
    pub python: String,
    pub script: std::path::PathBuf,
    pub lang: String,
    /// Minimum confidence (`0..=100`) to keep a span.
    pub min_confidence: f32,
}

/// The Python sidecar. Written to a temp file by [`PaddleOcr::detect`]; invoked as
/// `python <script> <image> <lang>` and prints the JSON contract on stdout. Defensive across
/// PaddleOCR 2.x (`.ocr`) and 3.x (`.predict`).
const PADDLE_SIDECAR: &str = r#"
import sys, json

def to_py(x):
    # numpy arrays/scalars -> python lists/scalars (PaddleOCR 3.x returns np.ndarray)
    return x.tolist() if hasattr(x, "tolist") else x

def build_ocr(lang):
    from paddleocr import PaddleOCR
    # Screen recordings are flat & upright — disable PP-OCRv5's doc-orientation, doc-unwarping,
    # and textline-orientation preprocessors (3.x). That skips loading 3 extra models and a lot
    # of per-frame work, the biggest OCR speedup. Fall back through older/safer kwarg sets.
    # enable_mkldnn=False avoids a paddle 3.x oneDNN CPU crash (ConvertPirAttribute2RuntimeAttribute)
    for kw in ({"lang": lang, "use_doc_orientation_classify": False, "use_doc_unwarping": False, "use_textline_orientation": False, "enable_mkldnn": False},
               {"lang": lang, "use_doc_orientation_classify": False, "use_doc_unwarping": False, "use_textline_orientation": False},
               {"lang": lang, "enable_mkldnn": False},
               {"use_angle_cls": True, "lang": lang, "enable_mkldnn": False},
               {"use_angle_cls": True, "lang": lang, "show_log": False},
               {"use_angle_cls": True, "lang": lang},
               {"lang": lang}, {}):
        try:
            return PaddleOCR(**kw)
        except Exception:
            continue
    return None

def run(ocr, path):
    out = []
    if ocr is None:
        return out
    res = None
    for call in (lambda: ocr.ocr(path, cls=True), lambda: ocr.ocr(path), lambda: ocr.predict(path)):
        try:
            r = call()
            if r is not None:
                res = r; break
        except Exception:
            res = None

    def add(text, conf, box):
        try:
            text = (text or "").strip()
        except Exception:
            text = str(text)
        if not text:
            return
        try:
            c = float(conf)
        except Exception:
            c = 1.0
        bb = [0, 0, 0, 0]
        try:
            box = to_py(box)
            if len(box) == 4 and all(isinstance(v, (int, float)) for v in box):
                x1, y1, x2, y2 = box            # axis-aligned [x1,y1,x2,y2] (3.x rec_boxes)
                bb = [int(min(x1, x2)), int(min(y1, y2)), int(abs(x2 - x1)), int(abs(y2 - y1))]
            else:
                xs = [float(p[0]) for p in box]; ys = [float(p[1]) for p in box]   # 4 [x,y] pts
                bb = [int(min(xs)), int(min(ys)), int(max(xs) - min(xs)), int(max(ys) - min(ys))]
        except Exception:
            bb = [0, 0, 0, 0]
        out.append({"text": text, "conf": c, "bbox": bb})

    def is_classic(r):
        # 2.x: [ per-image [ [box4pts, (text,score)], ... ] ]
        return (isinstance(r, list) and r and isinstance(r[0], list) and r[0]
                and isinstance(r[0][0], (list, tuple)) and len(r[0][0]) == 2
                and not isinstance(r[0][0][0], (int, float)))

    if is_classic(res):
        for page in res:
            for ln in (page or []):
                try:
                    add(ln[1][0], ln[1][1], ln[0])
                except Exception:
                    pass
    elif isinstance(res, list):
        # 3.x: list of OCRResult (dict-like; or .json -> {"res": {...}})
        def field(r, key):
            try:
                v = r[key]
                if v is not None:
                    return v
            except Exception:
                pass
            j = getattr(r, "json", None)
            cands = [j, (j.get("res") if hasattr(j, "get") else None), (r.get("res") if hasattr(r, "get") else None), (r if hasattr(r, "get") else None)]
            for c in cands:
                if hasattr(c, "get") and c.get(key) is not None:
                    return c.get(key)
            return None
        for r in res:
            texts = to_py(field(r, "rec_texts")) or to_py(field(r, "texts")) or []
            scores = to_py(field(r, "rec_scores")) or to_py(field(r, "scores")) or []
            polys = to_py(field(r, "rec_polys")) or to_py(field(r, "dt_polys")) or to_py(field(r, "rec_boxes")) or []
            for i, t in enumerate(texts):
                c = scores[i] if i < len(scores) else 1.0
                b = polys[i] if i < len(polys) else [0, 0, 0, 0]
                add(t, c, b)
    return out

try:
    if len(sys.argv) > 1 and sys.argv[1] == "--batch":
        # Batch mode: read {"paths":[...], "lang":"en"} on stdin, load the model ONCE, and
        # OCR every frame — avoids the multi-second model reload per frame. Output: [[span,...], ...]
        req = json.load(sys.stdin)
        lang = req.get("lang") or "en"
        ocr = build_ocr(lang)
        print(json.dumps([run(ocr, p) for p in req.get("paths", [])]))
    else:
        path = sys.argv[1]
        lang = sys.argv[2] if len(sys.argv) > 2 else "en"
        print(json.dumps(run(build_ocr(lang), path)))
except Exception as e:
    sys.stderr.write(str(e)); print("[]")
"#;

impl Default for PaddleOcr {
    fn default() -> Self {
        Self {
            python: "python3".into(),
            script: std::env::temp_dir().join("veyo-paddleocr.py"),
            lang: "en".into(),
            min_confidence: 50.0,
        }
    }
}

impl PaddleOcr {
    /// Available iff a Python with `paddleocr` importable is on `PATH`. Writes the sidecar to
    /// a temp file. Returns `None` (so the caller falls back to tesseract) otherwise.
    pub fn detect() -> Option<Self> {
        let python = ["python3", "python"].into_iter().find(|p| detect_binary(p))?;
        // import the class (not just the package) so a missing paddlepaddle backend fails here
        // and we fall back to tesseract, rather than silently producing empty OCR per frame.
        let importable = Command::new(python)
            .args(["-c", "from paddleocr import PaddleOCR"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !importable {
            return None;
        }
        let script = std::env::temp_dir().join("veyo-paddleocr.py");
        std::fs::write(&script, PADDLE_SIDECAR).ok()?;
        Some(Self { python: python.into(), script, ..Default::default() })
    }
}

impl Ocr for PaddleOcr {
    fn extract(&self, path: &Path, t_ms: TimeMs) -> Result<Vec<OcrSpan>> {
        let out = Command::new(&self.python)
            .arg(&self.script)
            .arg(path)
            .arg(&self.lang)
            .output()
            .with_context(|| format!("failed to run paddleocr sidecar via `{}`", self.python))?;
        if !out.status.success() {
            anyhow::bail!("paddleocr sidecar failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(parse_paddle_json(&String::from_utf8_lossy(&out.stdout), t_ms, self.min_confidence))
    }
    fn name(&self) -> &'static str {
        "paddleocr"
    }

    /// One sidecar process for the whole batch — PaddleOCR's model loads once instead of per
    /// frame (the dominant cost). Falls back to the per-frame default on any sidecar failure.
    fn extract_batch(&self, frames: &[(std::path::PathBuf, TimeMs)]) -> Vec<OcrSpan> {
        if frames.is_empty() {
            return Vec::new();
        }
        let req = serde_json::json!({
            "lang": self.lang,
            "paths": frames.iter().map(|(p, _)| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
        })
        .to_string();

        let parsed: Option<Vec<serde_json::Value>> = (|| {
            let mut child = Command::new(&self.python)
                .arg(&self.script)
                .arg("--batch")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .ok()?;
            child.stdin.take()?.write_all(req.as_bytes()).ok()?;
            let out = child.wait_with_output().ok()?;
            if !out.status.success() {
                return None;
            }
            serde_json::from_slice::<Vec<serde_json::Value>>(&out.stdout).ok()
        })();

        match parsed {
            Some(per_frame) if per_frame.len() == frames.len() => per_frame
                .into_iter()
                .zip(frames.iter())
                .flat_map(|(spans, (_, t))| parse_paddle_json(&spans.to_string(), *t, self.min_confidence))
                .collect(),
            // sidecar unavailable or shape mismatch → safe per-frame fallback
            _ => frames.iter().flat_map(|(p, t)| self.extract(p, *t).unwrap_or_default()).collect(),
        }
    }
}

/// Parse the sidecar's JSON contract into [`OcrSpan`]s. Confidence is normalized to `0..=100`
/// (PaddleOCR reports `0..1`). Pure and unit-tested.
pub fn parse_paddle_json(json: &str, t_ms: TimeMs, min_conf: f32) -> Vec<OcrSpan> {
    let v: serde_json::Value = match serde_json::from_str(json.trim()) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let mut spans = Vec::new();
    for it in v.as_array().into_iter().flatten() {
        let text = it.get("text").and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
        if text.is_empty() {
            continue;
        }
        let mut conf = it.get("conf").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32;
        if conf <= 1.0 {
            conf *= 100.0; // paddle reports 0..1 → normalize to 0..100
        }
        if conf < min_conf {
            continue;
        }
        let bbox = it.get("bbox").and_then(|b| b.as_array()).filter(|a| a.len() == 4).map(|a| Rect {
            x: a[0].as_i64().unwrap_or(0) as i32,
            y: a[1].as_i64().unwrap_or(0) as i32,
            w: a[2].as_i64().unwrap_or(0) as i32,
            h: a[3].as_i64().unwrap_or(0) as i32,
        });
        spans.push(OcrSpan { t_ms, text, source: TextSource::Ocr, bbox, confidence: Some(conf) });
    }
    spans
}

/// Parse tesseract TSV output into line-level [`OcrSpan`]s. Pure and unit-tested.
///
/// TSV columns: `level page block par line word left top width height conf text`.
/// Only word rows (`level == 5`) carry text; we group them by `(block, par, line)`,
/// union their boxes, and average confidence.
pub fn parse_tsv(tsv: &str, t_ms: TimeMs, min_conf: f32) -> Vec<OcrSpan> {
    let mut spans: Vec<OcrSpan> = Vec::new();
    let mut words: Vec<(String, Rect, f32)> = Vec::new();
    let mut cur_key: Option<(u32, u32, u32)> = None;

    for line in tsv.lines() {
        let c: Vec<&str> = line.split('\t').collect();
        if c.len() < 12 || c[0] == "level" {
            continue;
        }
        let level: u32 = c[0].parse().unwrap_or(0);
        if level != 5 {
            continue; // structural rows carry no text
        }
        let conf: f32 = c[10].parse().unwrap_or(-1.0);
        let text = c[11].trim();
        if conf < min_conf || text.is_empty() {
            continue;
        }
        let key = (
            c[2].parse().unwrap_or(0),
            c[3].parse().unwrap_or(0),
            c[4].parse().unwrap_or(0),
        );
        if cur_key != Some(key) {
            flush_line(&mut words, t_ms, &mut spans);
            cur_key = Some(key);
        }
        let rect = Rect {
            x: c[6].parse().unwrap_or(0),
            y: c[7].parse().unwrap_or(0),
            w: c[8].parse().unwrap_or(0),
            h: c[9].parse().unwrap_or(0),
        };
        words.push((text.to_string(), rect, conf));
    }
    flush_line(&mut words, t_ms, &mut spans);
    spans
}

/// Collapse the accumulated words of one line into a single [`OcrSpan`].
fn flush_line(words: &mut Vec<(String, Rect, f32)>, t_ms: TimeMs, spans: &mut Vec<OcrSpan>) {
    if words.is_empty() {
        return;
    }
    let text = words
        .iter()
        .map(|w| w.0.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let conf = words.iter().map(|w| w.2).sum::<f32>() / words.len() as f32;

    let mut bbox = words[0].1;
    let mut maxx = bbox.x + bbox.w;
    let mut maxy = bbox.y + bbox.h;
    for (_, r, _) in words.iter().skip(1) {
        bbox.x = bbox.x.min(r.x);
        bbox.y = bbox.y.min(r.y);
        maxx = maxx.max(r.x + r.w);
        maxy = maxy.max(r.y + r.h);
    }
    bbox.w = maxx - bbox.x;
    bbox.h = maxy - bbox.y;

    let trimmed = text.trim();
    if !trimmed.is_empty() {
        spans.push(OcrSpan {
            t_ms,
            text: trimmed.to_string(),
            source: TextSource::Ocr,
            bbox: Some(bbox),
            confidence: Some(conf),
        });
    }
    words.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsv_aggregates_words_into_a_line_with_union_bbox() {
        // header + three word rows on one line (block=1, par=1, line=1)
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
            5\t1\t1\t1\t1\t1\t320\t210\t200\t30\t95\tPayment\n\
            5\t1\t1\t1\t1\t2\t530\t210\t110\t30\t93\tfailed\n\
            5\t1\t1\t1\t1\t3\t650\t210\t130\t30\t90\t(500)\n";
        let spans = parse_tsv(tsv, 13_000, 40.0);
        assert_eq!(spans.len(), 1, "three words on one line -> one span");
        let s = &spans[0];
        assert_eq!(s.text, "Payment failed (500)");
        assert_eq!(s.t_ms, 13_000);
        let b = s.bbox.unwrap();
        assert_eq!(b.x, 320);
        assert_eq!(b.y, 210);
        assert_eq!(b.x + b.w, 780, "right edge = max(left+width)");
        assert!((s.confidence.unwrap() - 92.6667).abs() < 0.01);
    }

    #[test]
    fn tsv_splits_distinct_lines_and_drops_low_confidence() {
        let tsv = "5\t1\t1\t1\t1\t1\t0\t0\t10\t10\t95\thello\n\
            5\t1\t1\t1\t2\t1\t0\t20\t10\t10\t95\tworld\n\
            5\t1\t1\t1\t3\t1\t0\t40\t10\t10\t10\tnoise\n";
        let spans = parse_tsv(tsv, 0, 40.0);
        assert_eq!(spans.len(), 2, "two confident lines; 'noise' dropped at conf 10");
        assert_eq!(spans[0].text, "hello");
        assert_eq!(spans[1].text, "world");
    }

    #[test]
    fn null_ocr_returns_nothing() {
        let p = Path::new("/nonexistent.png");
        assert!(NullOcr.extract(p, 0).unwrap().is_empty());
    }

    #[test]
    fn paddle_json_parses_normalizes_conf_and_drops_blank_and_lowconf() {
        let j = r#"[
            {"text":"Payment failed (500)","conf":0.97,"bbox":[320,210,460,30]},
            {"text":"   ","conf":0.99,"bbox":[0,0,0,0]},
            {"text":"noise","conf":0.20,"bbox":[1,1,2,2]}
        ]"#;
        let spans = parse_paddle_json(j, 13_000, 50.0);
        assert_eq!(spans.len(), 1, "blank dropped, 0.20→20 dropped under min 50");
        let s = &spans[0];
        assert_eq!(s.text, "Payment failed (500)");
        assert_eq!(s.t_ms, 13_000);
        assert!((s.confidence.unwrap() - 97.0).abs() < 0.01, "0.97 normalized to 97");
        let b = s.bbox.unwrap();
        assert_eq!((b.x, b.y, b.w, b.h), (320, 210, 460, 30));
    }

    #[test]
    fn paddle_json_handles_garbage() {
        assert!(parse_paddle_json("not json", 0, 50.0).is_empty());
        assert!(parse_paddle_json("[]", 0, 50.0).is_empty());
    }
}
