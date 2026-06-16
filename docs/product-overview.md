# veyo — product overview

> The *what* and *why*. For the *how*, see [Architecture](architecture.md), [Tech Specs](tech-specs.md), and the [Policy Engine](policy-engine.md). This document distills the product thesis; the companion docs make it buildable.

`veyo` is a local-first **visual event codec**. It turns a live screen (later, camera) feed into a compact, diff-on-state, LLM-native event stream — so an LLM or agent can have continuous, affordable, private "sight" of what's happening, without raw imagery ever leaving the device.

**The one-sentence thesis:** *visual data can live as cheap text deltas at the edge, which fixes cost and privacy with the same move.*

---

## 1. Problem

Continuous visual awareness for an LLM is currently gated by three things **at once**:

1. **Cost.** A single 1080p frame is ~1k vision tokens. 30 fps × 24 h ≈ **2.6B tokens/day** for one source. Nobody runs an LLM watching a live feed; it's economically dead.
2. **Privacy.** The tools that *do* watch continuously (screen-memory recorders, computer-use agents) capture and often transmit raw frames — every visible name, email, account number, message.
3. **Bandwidth.** Raw or even H.264 video chokes weak networks, edge devices, and per-call agent budgets.

`veyo` collapses all three: reduce each frame to structured text **on-device**, maintain a persistent world-state, and emit only a delta when the scene **meaningfully changes**. The reduction is simultaneously the cost fix and the privacy guarantee — what crosses any boundary is text describing change, never pixels.

---

## 2. Who it's for (v1)

The lowest-stakes beachhead that still proves the codec: **continuous screen perception for AI agents and copilots.**

- **Computer-use / coding agents** that need cheap, ongoing awareness of a screen or terminal without re-screenshotting into a VLM every step.
- **Local copilots** that want to react to events — "build finished", "a modal appeared", "the long-running job errored" — without a 24/7 vision bill.

**Later tiers (explicitly out of v1):** camera-based presence/activity monitoring, then high-trust safety/eldercare use cases once the reduction is battle-tested. **Do not lead with life-safety claims.** The beachhead is chosen precisely because a missed event costs an agent a re-screenshot, not a life.

---

## 3. Core concepts

### 3.1 World-state

A compact, always-current text representation of the scene. For a screen source:

- `surfaces` — windows/apps (id, app, title, focused?, bounds)
- `regions` — spatial zones within the focused surface (a grid in v1; detected UI blocks in v2)
- per-region `activity_state` — `STATIC | CHANGING | SETTLING`
- per-region `baseline` — rolling novelty/change-frequency estimate (for habituation)
- optional `summary` — a short semantic description of a region (populated lazily, only via Tier-2)

Queryable at any time via `get_current_state()`. This is what an agent reads to orient itself. The data model behind it is specified in [Tech Specs](tech-specs.md#core-data-model).

### 3.2 Deltas (events)

The stream is a sequence of typed, bi-temporal, compact records. **Most of the time the stream is silent.**

Event types (screen source, v1):

| Event | Meaning |
|---|---|
| `focus_change` | active surface changed (app/window switch) |
| `surface_open` / `surface_close` | a window/app appeared or disappeared |
| `region_change` | a region started changing past the noise floor |
| `state_settle` | a previously-changing region went static (page/load finished) — **often the most useful event** |
| `idle` / `active` | overall session activity transition |
| `anomaly` | change inconsistent with recent baseline (v2, optional) |

The full wire schema for a delta is in [Tech Specs](tech-specs.md#delta-schema). The logic that *decides* when to emit one is the [Policy Engine](policy-engine.md).

### 3.3 The bi-temporal axis

Every delta carries **two** timestamps:

- `t_event` — when it happened on screen
- `t_observed` — when the codec emitted it

This is exactly the shape `ctxgraph` ingests. `veyo` writes into `ctxgraph` as its event log, making "the sensor that feeds the knowledge graph" a real integration rather than a slogan. The same bi-temporal log is also the [privacy/compliance audit trail](privacy-model.md#compliance).

---

## 4. Token economics (the proof)

| approach | tokens/day (1 source) |
|---|---|
| naive: every frame to a VLM (1080p, 30 fps) | **~2.6B** |
| OmniParser-style: full parse per sampled frame | still 100s of millions, **+ GPU** |
| **veyo:** emit-only-deltas, ~dozens of events/hr × ~50 tok | **a few thousand** |

**Five-to-six orders of magnitude, on CPU, with no imagery leaving the box.** This number is what the [Eval Harness](eval-harness.md) exists to prove on real footage before any daemon is built.

---

## 5. Competitive landscape

The pieces and two adjacent tools exist; the specific *combination* does not.

| tool | what it is | why it's not this |
|---|---|---|
| **OmniParser v2** (Microsoft) | screenshot → structured UI elements for LLM agents | stateless per-frame, full re-parse every call, GPU-heavy (~0.6–0.8s/frame), built for action-grounding not change; no diff, no event stream, no temporal compression, needs the frame |
| **peepshow** | video file → scene-detect + pHash dedup → LLM narrates changes → timeline | batch over recorded files, camera/CCTV focus, sends selected *frames* to the model (imagery leaves), no persistent world-state, not screen/agent |
| **ChainStream** (MobiSys '24) | academic stream-based LLM context-sensing framework | research-grade WIP, queries LLM per frame (no cheap-diff-first), mobile-context focus |
| **Screenpipe** | continuous local screen+audio capture → OCR/index → search/memory | records & stores *everything* raw; opposite of emit-only-deltas; personal memory, not agent perception |
| **Recall / Rewind** | screen-history search | raw capture (Recall local, Rewind cloud); no diff-codec, no agent stream |

**Differentiation — all of these, together, is the wedge:**

- live (not batch)
- screen-source for **agent perception**
- diff-on-semantic-state with a **persistent world-model**
- **emit-only-deltas**
- **no imagery leaves the device**
- cheap / CPU (no A100)
- **MCP-native**
- bi-temporal log (ctxgraph-ready)

**Positioning line:** *OmniParser parses a frame; veyo watches a session.*

---

## 6. Portfolio fit

`veyo` is not a standalone bet — it slots into an existing local-agent stack and reuses proven muscles.

- **`ctxgraph`** — the bi-temporal store `veyo` writes into.
- **`homn`** — supervises an agent's *actions* (hands); `veyo` gives it *perception* (eyes). Eyes + hands = a coherent local agent runtime, both halves yours.
- **`cloakpipe`** — the optional text-layer PII gate for cloud-bound summaries; extends its DPDP evidence story into the visual modality. See [Privacy Model](privacy-model.md).
- **Reused infrastructure** — `ort` (ONNX), MCP, sqlite/FTS5, single-binary Rust. No new muscles required.

---

## 7. Licensing

Open-core, consistent with CloakPipe:

- **Core** (permissive, or AGPL): the daemon + the delta schema.
- **Commercial layer:** managed enrichment models, CloakPipe integration, compliance/audit tooling, and support.

---

## Where to go next

- The structure that realizes this: **[Architecture](architecture.md)**.
- The thing that decides whether it works at all: the **[Policy Engine](policy-engine.md)** and the **[Eval Harness](eval-harness.md)**.
- Why "no imagery leaves the device" is enforceable, not just promised: the **[Privacy Model](privacy-model.md)**.
