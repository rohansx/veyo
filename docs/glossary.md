# veyo — glossary

> Every term of art in one place. Where a term has a home doc, it links there.

---

**Anomaly** — an `EventKind` (v2, optional): a change inconsistent with a region's recent baseline. A pattern-break that habituation surfaces rather than suppresses. See [Policy Engine](policy-engine.md#habituation).

**Baseline (`NoveltyBaseline`)** — a region's rolling estimate of how often it changes. Drives habituation: a region that changes constantly has a high baseline and low novelty. See [Policy Engine](policy-engine.md#habituation).

**Bi-temporal** — carrying two timestamps: `t_event` (when it happened on screen) and `t_observed` (when the codec emitted it). The shape `ctxgraph` ingests and the basis of the compliance audit log. See [Product Overview](product-overview.md#33-the-bi-temporal-axis).

**Coalescing** — merging concurrent low-salience changes (across adjacent grid cells, or globally per `coalesce_window_ms`) into one macro-event, so a modal spanning 4 cells doesn't become 4 events. See [Policy Engine](policy-engine.md#5-anti-spam-coalescing).

**`ctxgraph`** — the external bi-temporal knowledge store `veyo` writes deltas into as its event log / long-term memory layer.

**CloakPipe** — the text-layer PII gate. Optional; routes summaries through redaction before they reach a *cloud* LLM consumer. See [Privacy Model](privacy-model.md#5-cloud-consumers-cloakpipe).

**Damage / dirty-rects** — OS/compositor-provided rectangles marking which parts of the screen changed. Free change detection on Wayland (PipeWire) and Windows (DXGI); absent on macOS. Consumed via the [`CaptureBackend` trait](tech-specs.md#capture-backend-trait).

**Delta** — a single typed, bi-temporal, compact event record — the unit of the output stream. Schema in [Tech Specs](tech-specs.md#delta-schema).

**`describe_region(region_id)`** — the MCP tool that lazily runs Tier-2 enrichment on one region and returns **text only, never the image**. The privacy-preserving detail escape hatch. See [Privacy Model](privacy-model.md#4-the-describe_region-escape-hatch).

**`epsilon_noise`** — Gate-1 per-region diff floor: below it, a region counts as "not changing." A tuning knob. See [Policy Engine](policy-engine.md).

**Evidence** — the **local-only** part of a delta: a perceptual hash (`phash`) + a thumbnail reference (`thumb_ref`). Type-fenced so it can never be serialized onto the wire. See [Privacy Model](privacy-model.md#3-enforced-by-the-type-system-not-by-discipline).

**Emission rate** — emitted events ÷ frames captured. The constraint in tuning (target < ~1% of frames); recall is the objective. See [Eval Harness](eval-harness.md#4-scoring).

**Focus weight (`focus_weight`)** — salience multiplier applied to regions in the focused surface, so foreground always outranks background.

**FSM (debounce / settle)** — the per-region finite state machine `STATIC → CHANGING → SETTLING → STATIC` that turns a burst of frames into a start event and a settle event. See [Policy Engine](policy-engine.md#per-region-finite-state-machine).

**Gate 1 / Gate 2** — the two cost-ordered emission filters: Gate 1 is the cheap per-region diff (`epsilon_noise`); Gate 2 is the salience threshold (`salience_min`). See [Policy Engine](policy-engine.md#1-the-pipeline).

**Habituation** — the rising suppression of repetitive change, driving `novelty → 0` for spinners/video/blinking cursors. Reversible: a broken pattern spikes novelty back to ~1. The hard-to-copy differentiator. See [Policy Engine](policy-engine.md#habituation).

**Magnitude** — the normalized diff (pHash distance / SAD) for a region; a factor in salience.

**MCP** — Model Context Protocol; the transport over which `veyo` exposes its stream and query tools (via `rmcp`). See [Architecture](architecture.md#mcp-surface).

**Novelty** — `1 − habituation` ∈ [0,1]; the inverse of how expected a region's change is. A factor in salience and a field on every delta.

**`PollBackend`** — the Phase-1 capture backend: grab a full frame at `capture_fps`, no damage info. Trivial and cross-platform. See [Architecture](architecture.md#capture-backends).

**`PipewireBackend`** — the Phase-3 Linux/Wayland capture backend: XDG ScreenCast portal + DMA-BUF zero-copy + compositor damage rects.

**Recall** — annotated "things that mattered" that `veyo` caught ÷ total annotations. **The primary success metric** (false negatives hurt agents most). See [Eval Harness](eval-harness.md#4-scoring).

**Region** — a spatial zone within the focused surface; a grid cell in v1 (`blocks` = detected UI blocks in v2). Each region runs its own FSM and baseline.

**Ring buffer** — the bounded **in-memory** store where raw frames live transiently and are overwritten — never persisted to disk by default. The root of the privacy posture. See [Privacy Model](privacy-model.md#1-privacy-invariants-non-negotiable).

**Salience** — `w_focus · magnitude · novelty` ∈ [0,1]; how much a downstream consumer should care. Emission requires `salience ≥ salience_min`. See [Policy Engine](policy-engine.md#salience).

**`state_settle`** — the event fired when a region holds static past `settle_window_ms`. The **key compression**: a 2-second scroll/load becomes 2 events, not 60 frames. Often the most useful event. See [Policy Engine](policy-engine.md#per-region-finite-state-machine).

**Surface** — a window/app in the world-state (id, app, title, focused?, bounds).

**Settle window (`settle_window_ms`)** — how long a region must hold static before `state_settle` fires. A tuning knob.

**Tier-1** — the cheap, always-on path: downscale + perceptual hash/SAD diff. Runs on every candidate frame, on CPU.

**Tier-2** — the optional, gated enrichment path: UI tagging / OCR via `ort`, run only on events that pass both gates, on a single region's thumbnail. Off by default. See [Tech Specs](tech-specs.md#tier-2-enrichment).

**`veyod`** — the daemon binary that wires capture → core → MCP → store.

**World-state** — the compact, always-current text model of the scene (surfaces, regions, per-region activity + baseline). Queryable via `get_current_state()`. See [Product Overview](product-overview.md#31-world-state).

---

## Related

- The data shapes behind these terms: **[Tech Specs](tech-specs.md)**.
- The logic that uses them: **[Policy Engine](policy-engine.md)**.
- The index: **[Overview](README.md)**.
