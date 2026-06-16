//! The frozen delta schema (`v = 1`).
//!
//! **Freeze discipline:** downstream agents bind to the wire shape, so the schema
//! version [`SCHEMA_V`] and the [`Delta`] / [`EventKind`] shapes are frozen before
//! v0.1 ships. Changing them is a breaking, version-bumping event.
//!
//! **Privacy invariant (type-enforced):** [`Delta::evidence`] is local-only and
//! carries `#[serde(skip)]`, so it *physically cannot* be serialized onto the MCP
//! wire. Pixels never cross the process boundary by construction, not by discipline.

use serde::{Deserialize, Serialize};

/// Wire schema version. Bump only on a breaking change to [`Delta`].
pub const SCHEMA_V: u8 = 1;

/// Time in milliseconds as a monotonic `u64`. The **epoch is source-defined**:
/// wall-clock epoch-ms for the live daemon, relative-to-recording-start when the eval
/// harness replays a session. The core is driven by these frame timestamps (not
/// `Instant`s), which keeps every transition deterministic and replayable offline.
pub type TimeMs = u64;

/// An axis-aligned pixel rectangle: top-left `(x, y)` plus `w`×`h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A ULID-style event id (e.g. `"ev_01H..."`). Minted by the emitter, not by the pure
/// core, so the core stays deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventId(pub String);

/// The kinds of delta the codec emits. `snake_case` on the wire (`"state_settle"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    FocusChange,
    SurfaceOpen,
    SurfaceClose,
    RegionChange,
    StateSettle,
    Idle,
    Active,
    Anomaly,
}

/// The surface (window/app) a delta refers to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceRef {
    pub id: String,
    pub app: String,
    pub title: String,
    pub focused: bool,
}

/// The region (grid cell in v1) a delta refers to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionRef {
    pub id: String,
    pub grid: [u8; 2],
    pub bounds: Rect,
}

/// **Local-only** corroborating data: a perceptual hash and an optional thumbnail
/// reference into the local cache. Never transmitted — see [`Delta::evidence`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Evidence {
    pub phash: String,
    pub thumb_ref: Option<String>,
}

/// A single typed, bi-temporal, compact event — the unit of the output stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Delta {
    /// Schema version. Always [`SCHEMA_V`] for emitted deltas.
    pub v: u8,
    pub id: EventId,
    /// When it happened on screen.
    pub t_event: TimeMs,
    /// When the codec emitted it (the bi-temporal second axis). In the pure offline
    /// core this equals `t_event`; the live daemon stamps them separately.
    pub t_observed: TimeMs,
    /// Source descriptor, e.g. `"screen:0"`.
    pub source: String,
    #[serde(rename = "type")]
    pub kind: EventKind,
    pub surface: SurfaceRef,
    pub region: RegionRef,
    /// The text payload the LLM reads (templated for Tier-1, semantic for Tier-2).
    pub summary: String,
    pub salience: f32,
    pub novelty: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u32>,
    /// **LOCAL ONLY — type-enforced.** `#[serde(skip)]` makes it impossible to
    /// serialize evidence (perceptual hash + thumbnail ref) onto the wire. The
    /// privacy invariant lives in the type system, not in reviewer discipline.
    #[serde(skip)]
    pub evidence: Evidence,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_delta() -> Delta {
        Delta {
            v: SCHEMA_V,
            id: EventId("ev_01HZZTEST".into()),
            t_event: 1_000,
            t_observed: 1_040,
            source: "screen:0".into(),
            kind: EventKind::StateSettle,
            surface: SurfaceRef {
                id: "win_42".into(),
                app: "firefox".into(),
                title: "PR #1182 · github".into(),
                focused: true,
            },
            region: RegionRef {
                id: "r_3".into(),
                grid: [2, 1],
                bounds: Rect {
                    x: 640,
                    y: 80,
                    w: 640,
                    h: 680,
                },
            },
            summary: "content in main region stopped changing after ~1.4s".into(),
            salience: 0.71,
            novelty: 0.83,
            duration_ms: Some(1420),
            evidence: Evidence {
                phash: "f3a1deadbeef".into(),
                thumb_ref: Some("local://cache/ev_01HZZTEST.webp".into()),
            },
        }
    }

    /// PRIVACY INVARIANT: evidence (phash + thumbnail) must never reach the wire.
    #[test]
    fn evidence_never_serialized_to_wire() {
        let json = serde_json::to_string(&sample_delta()).unwrap();
        assert!(!json.contains("evidence"), "evidence key leaked: {json}");
        assert!(!json.contains("phash"), "phash leaked: {json}");
        assert!(!json.contains("f3a1deadbeef"), "phash value leaked: {json}");
        assert!(!json.contains("thumb_ref"), "thumb_ref leaked: {json}");
        assert!(!json.contains(".webp"), "thumbnail path leaked: {json}");
    }

    /// The non-private fields the agent actually consumes must still be present.
    #[test]
    fn wire_carries_the_agent_payload() {
        let json = serde_json::to_string(&sample_delta()).unwrap();
        assert!(json.contains("\"type\":\"state_settle\""), "{json}");
        assert!(json.contains("\"summary\""));
        assert!(json.contains("\"t_event\":1000"));
        assert!(json.contains("\"t_observed\":1040"));
        assert!(json.contains("\"duration_ms\":1420"));
    }

    /// `duration_ms` is omitted entirely when absent (most events have no duration).
    #[test]
    fn duration_omitted_when_none() {
        let mut d = sample_delta();
        d.kind = EventKind::RegionChange;
        d.duration_ms = None;
        let json = serde_json::to_string(&d).unwrap();
        assert!(!json.contains("duration_ms"), "{json}");
    }
}
