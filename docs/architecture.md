# veyo — architecture

> The structural *how*: components, data flow, trait boundaries, and the workspace that keeps them decoupled. For the concrete types and crate versions see [Tech Specs](tech-specs.md); for the decision logic inside the pipeline see the [Policy Engine](policy-engine.md).

`veyo` ships as a single Rust binary, **`veyod`**, built from a Cargo workspace. It reuses existing muscles: `ort` (ONNX), the MCP server pattern, sqlite/FTS5, and single-binary distribution.

---

## 1. System shape

```
 capture backend ─► ring buffer ─► diff/pHash ─► region FSM ─► salience+habituation
                                                                      │
                                                          (pass) ─────┤
                                                                      ▼
                                                        Tier-2 enrich (optional, ort)
                                                                      ▼
                                              world-state  ──►  delta emitter
                                                                      ├─► MCP server (stream + query)
                                                                      └─► ctxgraph / sqlite (bi-temporal log)
```

The flow is **cheap → expensive, short-circuiting early**: ~99% of frames are dropped at the first cheap diff gate and never reach the FSM, let alone the optional ML enrichment. That ordering is the whole performance story — see the [Policy Engine](policy-engine.md) for the gate-by-gate breakdown.

---

## 2. Components

| Component | Crate | Responsibility |
|---|---|---|
| **Capture** | `veyo-capture` | Platform backends behind one trait; produce frames (+ optional OS damage rects). |
| **Ring buffer** | `veyo-capture` | Bounded in-memory frame store. Raw pixels live **here only** — never on disk by default. |
| **Diff / pHash** | `veyo-core` | Per-region downscaled perceptual hash / SAD — the cheap Tier-1 change signal. |
| **Region FSM** | `veyo-core` | Per-region debounce state machine: `STATIC → CHANGING → SETTLING → STATIC`. |
| **Salience + habituation** | `veyo-core` | Scores each candidate change; decays repetitive change toward silence. |
| **Tier-2 enrich** | `veyo-enrich` | Optional, off by default. UI tagger / OCR via `ort`, run only on gate-passing events. |
| **World-state** | `veyo-core` | The compact, always-current text model of the scene. |
| **Delta emitter** | `veyo-core` | Builds the typed, bi-temporal records and fans them out. |
| **MCP server** | `veyo-mcp` | Live stream + query surface via `rmcp`. |
| **Store** | `veyo-store` | Append-only bi-temporal log (rusqlite/FTS5) and a `ctxgraph` adapter. |
| **Daemon** | `veyod` (bin) | Wires capture → core → mcp → store; owns config and lifecycle. |

### Capture backends {#capture-backends}

`veyo` targets several platforms behind one trait, built in this order:

1. **`PollBackend`** (`xcap`/`scap`) — grab a full frame at `capture_fps`, no damage info. **Phase 1.** Trivial, cross-platform, gets the whole pipeline alive on every OS.
2. **`PipewireBackend`** (`ashpd` portal + `pipewire`, DMA-BUF) — **Phase 3** optimization on Linux/Wayland. Feeds compositor **damage rects** so the differ skips untouched regions for ~free, and zero-copies via DMA-BUF.
3. **`DxgiBackend`** (Windows, DXGI Desktop Duplication) and **`ScreenCaptureKitBackend`** (macOS) — later. Note: macOS exposes **no native dirty-rects**, so it leans entirely on Tier-1 pHash.

> **The decoupling is the point.** The FSM and everything downstream never change regardless of which backend is active. The capture layer is the *only* place that knows which OS it's on. See the [`CaptureBackend` trait](tech-specs.md#capture-backend-trait).

### Tier-2 enrichment (optional)

A small UI-element detector (YOLO-class) and/or a Rust OCR crate, all via `ort`. It runs **only on gate-passing events, on a single region's thumbnail** — dozens of times per hour, not 30×/second. Off by default to preserve the cheap/private posture. It turns a templated summary (*"region r_3 settled after 1.4s"*) into a semantic one (*"a confirmation dialog 'Overwrite file?' appeared"*). Details in [Tech Specs](tech-specs.md#tier-2-enrichment).

---

## 3. Pipeline & trait boundaries

```
CaptureBackend ──frames──► RegionDiffer ──changed regions──► Fsm ──transitions──►
    SalienceScorer ──(pass)──► [Enricher (opt)] ──► WorldState.update + DeltaEmitter
                                                          ├──► veyo-mcp  (stream/query)
                                                          └──► veyo-store (bi-temporal log)
```

**Each arrow is a channel** (`tokio::sync`), so stages run concurrently and a slow Tier-2 inference never blocks Tier-1 diffing. The pipeline is back-pressure-friendly: if enrichment falls behind, Tier-1 diffing and the FSM keep running; only enriched summaries lag, and the templated fallback is always available.

Boundaries that matter:

- **`veyo-core` does no I/O.** It takes frames/regions in and emits deltas out. That makes it trivially unit-testable and lets the [eval harness](eval-harness.md) drive the entire decision pipeline with *no capture backend and no daemon at all*.
- **Capture is isolated behind a trait** so platform churn (Wayland fiddliness, macOS's missing damage rects) never reaches the FSM.
- **Enrichment is a feature-flagged crate** so the default build is cheap and private, and the ML dependency is opt-in.

---

## 4. Workspace layout

A Cargo workspace decouples the capture backend from the core pipeline — a hard requirement from the cross-platform risk analysis: **the FSM must not know which OS it's on.**

```
veyo/
├── Cargo.toml                 # workspace
├── crates/
│   ├── veyo-core/             # schema, world-state, FSM, salience, habituation — NO I/O
│   ├── veyo-capture/          # CaptureBackend trait + xcap / pipewire / x11 impls
│   ├── veyo-enrich/           # Tier-2: ort/rten OCR + UI tagging (optional feature)
│   ├── veyo-store/            # rusqlite bi-temporal log + query; ctxgraph adapter
│   ├── veyo-mcp/              # rmcp server exposing the tool surface
│   └── veyo-eval/             # Phase-0 harness: replay, annotate, score, tune
├── bins/
│   └── veyod/                 # the daemon: wires capture → core → mcp → store
└── fixtures/
    └── sessions/              # recorded sessions + annotations for eval/CI
```

`veyo-core` is pure logic with no I/O — frames/regions in, deltas out — which is what makes both unit testing and offline replay possible.

---

## 5. MCP surface {#mcp-surface}

The daemon exposes its world-state and event stream over MCP (`rmcp`). Transport is `stdio` for a single-consumer agent, or `streamable-http`/SSE for the daemon-with-multiple-subscribers model.

| Tool | Returns |
|---|---|
| `subscribe_events(filter)` | live stream of deltas — filter by kind, surface, `min_salience` |
| `get_current_state()` | the world-state snapshot, as compact text |
| `query_events(since, until, kind?, surface?)` | historical deltas from the store |
| `describe_region(region_id)` | **lazy, on-demand** Tier-2 enrichment of a region — returns **text, never pixels** |

`describe_region` is the privacy-preserving escape hatch: a remote consumer that wants more detail asks for it and gets *text back*, never the underlying image. The exact tool signatures are in [Tech Specs](tech-specs.md#mcp-surface); the invariant that makes it safe is in the [Privacy Model](privacy-model.md).

---

## 6. Storage

`veyo-store` uses `rusqlite` with FTS5 on the `summary` field. The event table is **append-only**, keyed by `id`, indexed on `t_event`, `t_observed`, `kind`, and `surface`. Thumbnails (only if Tier-2 is on) live in a local cache dir referenced by `evidence.thumb_ref` and **never leave disk**. A `ctxgraph` adapter lets the same deltas flow into the bi-temporal graph as the long-term memory layer. Schema detail in [Tech Specs](tech-specs.md#storage).

---

## Related

- The types and crate versions behind every box above: **[Tech Specs](tech-specs.md)**.
- The logic *inside* the FSM / salience boxes: **[Policy Engine](policy-engine.md)**.
- Why the capture/enrich isolation is also a privacy boundary: **[Privacy Model](privacy-model.md)**.
- The build order for these components: **[Phases & Roadmap](phases.md)**.
