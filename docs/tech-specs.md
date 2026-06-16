# veyo — tech specs

> The concrete *how*: verified crate stack, the core Rust data model, the on-the-wire delta schema, `veyo.toml`, and storage. Pairs with [Architecture](architecture.md) (structure) and the [Policy Engine](policy-engine.md) (decision logic).

> **Crate note.** Every crate named below was checked to exist on crates.io at time of writing. **Pin exact versions at `cargo init`.**

---

## 1. Tech stack (verified crates)

| Concern | Crate | Role / notes |
|---|---|---|
| Async runtime | `tokio` | daemon event loop, timers (settle windows), task spawning |
| Capture (simple, x-platform) | `xcap` or `scap` | Phase-1 grab-and-poll on Linux/Win/macOS — fastest path to a working pipeline |
| Capture (optimized, Linux) | `ashpd` + `pipewire` (or `lamco-pipewire`) | Phase-3 zero-copy via XDG ScreenCast portal + DMA-BUF + compositor damage |
| Capture (Wayland alt) | `libwayshot`, `waycap-rs` | fallback / reference implementations |
| Downscale | `fast_image_resize` | SIMD region downscale to 32×32 before hashing (hot path) |
| Perceptual hash / diff | `img_hash`; `dssim` (perceptual distance); `scenesdetect` (reference SIMD pHash/SAD) | Tier-1 cheap change detection |
| Tier-2 ML (optional) | `ort` (ONNX Runtime, already used in ctxgraph) or `rten` (pure-Rust) | OCR / UI-element tagging, gated, lazy |
| OCR (optional) | `oar-ocr` / `ocr-rs` | Tier-2 text enrichment |
| MCP server | `rmcp` (official MCP Rust SDK) | `subscribe_events`, `get_current_state`, `query_events`, `describe_region` |
| Storage | `rusqlite` (FTS5) | bi-temporal event log; or write into `ctxgraph` |
| Serialization | `serde`, `serde_json` | delta schema on the wire + on disk |
| CLI / config | `clap`, `toml`, `serde` | `veyod` flags + `veyo.toml` |
| Logging | `tracing`, `tracing-subscriber` | structured daemon logs |
| Errors | `anyhow` (bins), `thiserror` (libs) | |

---

## 2. Core data model {#core-data-model}

Lives in `veyo-core`, which performs **no I/O** — these types are driven directly by the daemon and by the [eval harness](eval-harness.md). The shapes below are the frozen v0.1 schema; the ones marked *(implemented)* already exist in `crates/veyo-core`.

> **Time is logical, not wall-clock.** The core is driven by frame timestamps (`TimeMs`, a `u64` of milliseconds) rather than `SystemTime`/`Instant`. The **epoch is source-defined**: wall-clock epoch-ms in the live daemon, relative-to-recording-start when the harness replays offline. That keeps every FSM transition deterministic and replayable — the single most important property for tuning. Event ids are a `String` newtype (`EventId`) minted by the emitter, not generated inside the pure core.

```rust
pub type TimeMs = u64;                  // ms; epoch is source-defined (implemented)
pub struct EventId(pub String);         // "ev_01H..."; minted by the emitter (implemented)

pub struct WorldState {
    pub surfaces: Vec<Surface>,         // windows/apps
    pub focused: Option<SurfaceId>,
    pub regions: HashMap<RegionId, RegionState>,
    pub t: TimeMs,                      // last update
}

pub struct Surface {
    pub id: SurfaceId,
    pub app: String,
    pub title: String,
    pub bounds: Rect,
}

pub struct RegionState {
    pub id: RegionId,
    pub grid: (u8, u8),
    pub bounds: Rect,
    pub fsm: RegionFsm,                 // Static → Changing → Settling (implemented)
    pub last_cell: [u8; 64],           // last 8×8 downscaled cell, see `veyo-core::diff`
    pub baseline: NoveltyBaseline,     // rolling change-frequency estimate (implemented)
}

pub enum Phase { Static, Changing, Settling }   // (implemented)

pub struct Delta {                      // (implemented)
    pub v: u8,                         // schema version — LOCKED at 1
    pub id: EventId,
    pub t_event: TimeMs,               // when it happened on screen
    pub t_observed: TimeMs,            // emitted-at; offline this == t_event
    pub source: String,                // "screen:0"
    pub kind: EventKind,
    pub surface: SurfaceRef,
    pub region: RegionRef,
    pub summary: String,               // LLM payload (templated T1, semantic T2)
    pub salience: f32,                 // [0,1]
    pub novelty: f32,                  // [0,1]
    pub duration_ms: Option<u32>,
    pub evidence: Evidence,            // LOCAL ONLY — #[serde(skip)], never on the wire
}

pub enum EventKind {                    // frozen; Phase-0 core emits only the two below
    FocusChange, SurfaceOpen, SurfaceClose,
    RegionChange, StateSettle,          // <- the only kinds veyo-core emits today
    Idle, Active, Anomaly,              // reserved for the daemon / later phases
}
```

**Two non-negotiables baked into the types:**

1. **`evidence` carries `#[serde(skip)]`** so it physically *cannot* be serialized onto the MCP wire — the [privacy invariant](privacy-model.md) enforced by the type system, not by discipline. A regression test (`evidence_never_serialized_to_wire`) asserts no `phash`/thumbnail ever appears in the JSON.
2. **The schema `v` and the `StateSettle` shape are frozen** — downstream agents bind to them. See the freeze rationale in [Risks & Open Questions](risks-and-open-questions.md).

---

## 3. Delta schema {#delta-schema}

JSON on the wire (MCP), stored in sqlite/ctxgraph. The schema is **versioned** (`v`).

```json
{
  "v": 1,
  "id": "ev_01H....",
  "t_event": 1781605862140,
  "t_observed": 1781605862180,
  "source": "screen:0",
  "type": "state_settle",
  "surface": { "id": "win_42", "app": "firefox", "title": "PR #1182 · github", "focused": true },
  "region": { "id": "r_3", "grid": [2, 1], "bounds": [640, 80, 1280, 760] },
  "summary": "content in main region stopped changing after ~1.4s",
  "salience": 0.71,
  "novelty": 0.83,
  "duration_ms": 1420,
  "evidence": { "phash": "f3a1...", "thumb_ref": "local://cache/ev_01H....webp" }
}
```

### Field notes

- **`t_event` / `t_observed`** — **milliseconds** as a `u64` (source-defined epoch), not RFC3339 strings. This is the canonical, frozen wire representation; an MCP edge *may* additionally render RFC3339 for human display, but the stored/transmitted type is the integer. Both axes are always present (bi-temporal); in the pure offline core they coincide, while the live daemon stamps them separately.
- **`summary`** — short text the LLM reads. For Tier-1-only events it's templated (*"region X started/stopped changing"*); for Tier-2-enriched events it's semantic (*"a confirmation dialog appeared"*, *"terminal shows a non-zero exit"*).
- **`salience` ∈ [0,1]** — how much downstream should care; consumers filter on it. Computed by the [salience model](policy-engine.md#salience).
- **`novelty` ∈ [0,1]** — inverse of habituation; repetitive changes decay toward 0. See [habituation](policy-engine.md#habituation).
- **`evidence`** — **local-only.** `phash` is a perceptual hash; `thumb_ref` points at a thumbnail in the local cache. **Neither is ever transmitted.** It exists for local audit/replay and lazy on-demand enrichment, not for export. This field is the one with the serialization boundary in §2.

---

## 4. Capture backend trait {#capture-backend-trait}

The trait that isolates every platform difference (see [Architecture §2](architecture.md#capture-backends)):

```rust
pub trait CaptureBackend: Send {
    /// Next frame, plus optional OS-provided dirty rects (damage) when available.
    fn next_frame(&mut self) -> Result<Frame>;
    fn damage(&self) -> Option<&[Rect]>;   // Some on Wayland/DXGI, None on macOS poll
    fn surfaces(&self) -> Result<Vec<Surface>>;
}
```

| Backend | Crate(s) | Damage rects | Phase |
|---|---|---|---|
| `PollBackend` | `xcap` / `scap` | none | 1 (first) |
| `PipewireBackend` | `ashpd` + `pipewire` | yes (compositor) | 3 (optimization) |
| `DxgiBackend` | Windows DXGI | yes (dirty-rects) | 4 |
| `ScreenCaptureKitBackend` | macOS | **none** → leans on Tier-1 pHash | 4 |

The FSM downstream never changes regardless of backend — that decoupling is the whole point.

---

## 5. Tier-2 enrichment {#tier-2-enrichment}

`veyo-enrich`, behind an **optional feature flag**. Runs **only** on a gate-passing event, on **only** that region's thumbnail — dozens of times/hour, not 30×/sec. `ort` (or `rten`) loads a small UI detector and/or OCR (`oar-ocr`). It promotes a templated summary (*"region r_3 settled after 1.4s"*) to a semantic one (*"a confirmation dialog 'Overwrite file?' appeared"*). **Off by default** to preserve the cheap/private posture.

---

## 6. MCP surface {#mcp-surface}

Exposed by `veyo-mcp` via `rmcp`. Transport: `stdio` (single-consumer agent) or `streamable-http`/SSE (daemon with multiple subscribers).

| Tool | Signature intent | Returns |
|---|---|---|
| `subscribe_events(filter)` | filter by `kind`, `surface`, `min_salience` | live notification stream of deltas |
| `get_current_state()` | — | compact text snapshot of `WorldState` |
| `query_events(since, until, kind?, surface?)` | time range + optional filters | historical deltas from the store |
| `describe_region(region_id)` | one region | lazy Tier-2 enrichment — **text only, never the image** |

---

## 7. Storage {#storage}

`rusqlite` with FTS5 on `summary`. An **append-only** event table keyed by `id`, indexed on `t_event`, `t_observed`, `kind`, and `surface`. Thumbnails (if Tier-2 on) live in a local cache dir referenced by `evidence.thumb_ref` and **never leave disk**. A `ctxgraph` adapter lets the same deltas flow into the bi-temporal graph as the long-term memory layer.

---

## 8. Config — `veyo.toml` {#config}

```toml
capture_fps        = 4
region_mode        = "grid"   # grid | blocks(v2)
grid               = [8, 8]
epsilon_noise      = 0.03     # Gate-1 mean-abs-diff floor, [0,1]
settle_window_ms   = 400
salience_min       = 0.4
novelty_decay      = 0.9
focus_weight       = 1.5
coalesce_window_ms = 1000
tier2              = "off"    # off | ocr | ui | both
mcp_transport      = "stdio"  # stdio | sse
persist_thumbs     = false
```

### Knob reference {#knob-reference}

| knob | meaning | starting default |
|---|---|---|
| `capture_fps` | poll rate when not damage-driven | 4 |
| `epsilon_noise` | per-region diff floor to ignore (mean abs diff, `[0,1]`) | 0.03 (tuned per source) |
| `settle_window_ms` | static-hold before declaring settled | 400 |
| `salience_min` | emit threshold to upstream | 0.4 |
| `novelty_decay` | habituation rate | 0.9 / window |
| `focus_weight` | multiplier for focused surface | 1.5 |
| `coalesce_window_ms` | anti-spam merge window (planned, Phase-1+) | 1000 |
| `region_mode` | `grid` (v1) or `blocks` (v2) | grid |
| `tier2` | enrichment on/off + model | off |
| `mcp_transport` | `stdio` (single agent) or `sse` (subscribers) | stdio |
| `persist_thumbs` | write thumbnails to local cache (Tier-2) | false |

> **These defaults are guesses.** The four tuning knobs (`epsilon_noise`, `settle_window_ms`, `salience_min`, `novelty_decay`) are *hyperparameters* — the [Eval Harness](eval-harness.md) sets their real values against recorded sessions, and that tuning **is** the core engineering work.

---

## Related

- How these types flow through the system: **[Architecture](architecture.md)**.
- The logic that computes `salience` / `novelty` and drives `Phase`: **[Policy Engine](policy-engine.md)**.
- Why `evidence` is serialization-fenced: **[Privacy Model](privacy-model.md)**.
- Terms used here: **[Glossary](glossary.md)**.
