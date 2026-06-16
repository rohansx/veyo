//! Session model + the on-disk recording format.
//!
//! A session lives at `fixtures/sessions/<name>/`:
//! - `frames.jsonl`      — one `{frame_idx, t_ms}` per line
//! - `annotations.jsonl` — one `{t_ms, kind, surface, note}` per line (the ground truth)
//! - `frames/<idx>.png`  — the frame images (loaded by [`crate::decode`])

use serde::{Deserialize, Serialize};

/// One "thing that mattered" — the ground truth a session is scored against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub t_ms: u64,
    pub kind: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub note: String,
}

/// One recorded frame's metadata (the image itself is loaded separately).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameMeta {
    pub frame_idx: u64,
    pub t_ms: u64,
}

/// A loaded session: its frame timeline and annotations.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    pub name: String,
    pub frames: Vec<FrameMeta>,
    pub annotations: Vec<Annotation>,
}

/// Parse a `frames.jsonl` document (one JSON object per non-blank line).
pub fn parse_frames_jsonl(text: &str) -> Result<Vec<FrameMeta>, serde_json::Error> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

/// Parse an `annotations.jsonl` document (one JSON object per non-blank line).
pub fn parse_annotations_jsonl(text: &str) -> Result<Vec<Annotation>, serde_json::Error> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

/// The session's wall-clock span (last frame timestamp), used for events/hour.
pub fn duration_ms(frames: &[FrameMeta]) -> u64 {
    frames.last().map(|f| f.t_ms).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frames_jsonl_ignoring_blank_lines() {
        let doc = "{\"frame_idx\":0,\"t_ms\":0}\n\n{\"frame_idx\":1,\"t_ms\":250}\n";
        let frames = parse_frames_jsonl(doc).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(
            frames[1],
            FrameMeta {
                frame_idx: 1,
                t_ms: 250
            }
        );
        assert_eq!(duration_ms(&frames), 250);
    }

    #[test]
    fn annotation_surface_and_note_default_when_absent() {
        let a: Annotation =
            serde_json::from_str("{\"t_ms\":100,\"kind\":\"build_finished\"}").unwrap();
        assert_eq!(a.kind, "build_finished");
        assert_eq!(a.surface, "");
        assert_eq!(a.note, "");
    }

    #[test]
    fn parses_annotations_jsonl() {
        let doc = "{\"t_ms\":252100,\"kind\":\"build_finished\",\"surface\":\"terminal\",\"note\":\"cargo build done\"}";
        let anns = parse_annotations_jsonl(doc).unwrap();
        assert_eq!(anns.len(), 1);
        assert_eq!(anns[0].surface, "terminal");
    }
}
