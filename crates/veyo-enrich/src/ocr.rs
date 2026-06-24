//! OCR backends. The default shells out to the system `tesseract` CLI and parses its
//! TSV output (per-line text + bounding boxes + confidence) — robust, with no FFI or
//! build-time dependency. [`NullOcr`] keeps the pipeline working when no engine exists.

use crate::detect_binary;
use crate::types::{OcrSpan, TextSource};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use veyo_core::{Rect, TimeMs};

/// Extracts on-screen text from a frame image.
pub trait Ocr: Send + Sync {
    /// OCR the image at `path`, stamping every span with `t_ms`.
    fn extract(&self, path: &Path, t_ms: TimeMs) -> Result<Vec<OcrSpan>>;
    fn name(&self) -> &'static str;
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
}
