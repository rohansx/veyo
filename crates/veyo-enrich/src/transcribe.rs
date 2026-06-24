//! Transcription backends. The default [`NullTranscriber`] needs no audio model;
//! [`WhisperCppTranscriber`] shells out to a whisper.cpp-style binary and parses its
//! JSON. Audio never leaves the device — transcription runs locally.

use crate::detect_binary;
use crate::types::TranscriptSegment;
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Turns an audio file into time-aligned transcript segments.
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, audio: &Path) -> Result<Vec<TranscriptSegment>>;
    fn name(&self) -> &'static str;
}

/// No-op transcriber — returns nothing. The default when no whisper binary is wired.
#[derive(Debug, Default, Clone, Copy)]
pub struct NullTranscriber;

impl Transcriber for NullTranscriber {
    fn transcribe(&self, _audio: &Path) -> Result<Vec<TranscriptSegment>> {
        Ok(Vec::new())
    }
    fn name(&self) -> &'static str {
        "null"
    }
}

/// A whisper.cpp-style CLI (e.g. `whisper-cli`) invoked with `-oj` to emit JSON.
#[derive(Debug, Clone)]
pub struct WhisperCppTranscriber {
    pub bin: String,
    /// Path to a ggml whisper model (e.g. `ggml-base.en.bin`).
    pub model: String,
}

impl WhisperCppTranscriber {
    pub fn new(bin: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            bin: bin.into(),
            model: model.into(),
        }
    }

    /// Pick the first whisper.cpp-style binary on `PATH`, if any, with the given model.
    pub fn detect(model: impl Into<String>) -> Option<Self> {
        for bin in ["whisper-cli", "whisper-cpp", "whisper"] {
            if detect_binary(bin) {
                return Some(Self::new(bin, model));
            }
        }
        None
    }
}

impl Transcriber for WhisperCppTranscriber {
    fn transcribe(&self, audio: &Path) -> Result<Vec<TranscriptSegment>> {
        // whisper.cpp writes `<of>.json`; we point `-of` at the audio path's stem.
        let out = Command::new(&self.bin)
            .arg("-m")
            .arg(&self.model)
            .arg("-f")
            .arg(audio)
            .arg("-oj")
            .arg("-of")
            .arg(audio)
            .output()
            .with_context(|| format!("failed to run `{}`", self.bin))?;
        if !out.status.success() {
            anyhow::bail!(
                "`{}` failed: {}",
                self.bin,
                String::from_utf8_lossy(&out.stderr)
            );
        }
        let json_path = format!("{}.json", audio.display());
        let text = std::fs::read_to_string(&json_path)
            .with_context(|| format!("reading whisper output `{json_path}`"))?;
        parse_whisper_json(&text)
    }
    fn name(&self) -> &'static str {
        "whisper.cpp"
    }
}

/// Parse whisper.cpp JSON — the `transcription` array of
/// `{ offsets: { from, to }, text }` (offsets are milliseconds). Pure + unit-tested.
pub fn parse_whisper_json(text: &str) -> Result<Vec<TranscriptSegment>> {
    let v: serde_json::Value = serde_json::from_str(text).context("parsing whisper JSON")?;
    let arr = v
        .get("transcription")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    let mut segs = Vec::new();
    for item in arr {
        let text = item
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        let offsets = item.get("offsets");
        let from = offsets
            .and_then(|o| o.get("from"))
            .and_then(|x| x.as_u64())
            .unwrap_or(0);
        let to = offsets
            .and_then(|o| o.get("to"))
            .and_then(|x| x.as_u64())
            .unwrap_or(from);
        segs.push(TranscriptSegment {
            start_ms: from,
            end_ms: to,
            text,
            speaker: None,
        });
    }
    Ok(segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_whisper_cpp_json_offsets() {
        let json = r#"{
            "transcription": [
                {"offsets": {"from": 1200, "to": 4800}, "text": " Okay, so I click submit"},
                {"offsets": {"from": 5000, "to": 7100}, "text": " and it throws an error"},
                {"offsets": {"from": 7200, "to": 7300}, "text": "   "}
            ]
        }"#;
        let segs = parse_whisper_json(json).unwrap();
        assert_eq!(segs.len(), 2, "blank segment skipped");
        assert_eq!(segs[0].start_ms, 1200);
        assert_eq!(segs[0].end_ms, 4800);
        assert_eq!(segs[0].text, "Okay, so I click submit");
        assert_eq!(segs[1].start_ms, 5000);
    }

    #[test]
    fn empty_or_missing_transcription_is_ok() {
        assert!(parse_whisper_json("{}").unwrap().is_empty());
        assert!(parse_whisper_json(r#"{"transcription":[]}"#).unwrap().is_empty());
    }

    #[test]
    fn null_transcriber_returns_nothing() {
        assert!(NullTranscriber.transcribe(Path::new("/x.wav")).unwrap().is_empty());
    }
}
