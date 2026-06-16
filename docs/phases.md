# veyo — phases & roadmap

> The execution plan: the v1 MVP scope, the phased roadmap with success gates, and timeboxes. For *why* it's ordered this way, see the [Build Plan](plan.md).

---

## 1. v1 scope (MVP)

### In

- one screen/display source, one platform (**Wayland first**, via the trivial `PollBackend`)
- grid-region change detection (Tier-1: cheap diff + pHash)
- the settle/debounce FSM
- salience scoring + habituation
- event kinds: `focus_change`, `surface_open/close`, `region_change`, `state_settle`, `idle/active`
- MCP `subscribe_events` + `get_current_state` + `query_events`
- sqlite (or `ctxgraph`) bi-temporal log
- `veyo.toml` thresholds

### Out (later)

- camera source; presence/activity; **any safety/eldercare claim**
- Tier-2 semantic tagging / OCR enrichment (**stub the interface, ship off**)
- multi-display, `blocks` region mode, `anomaly` events
- bundled local-LLM reasoning; CloakPipe PII routing
- non-Wayland capture backends

The discipline here matters: v1 ships the **codec proven by the harness**, plus the thinnest live wiring that lets a real agent consume it. Everything tempting-but-heavier (Tier-2, blocks mode, camera) is deferred behind interfaces that are stubbed now.

---

## 2. The phased roadmap

| Phase | Deliverable | Success gate |
|---|---|---|
| **0 — Validate** | [`veyo-core`](tech-specs.md#core-data-model) (schema, FSM, salience stub) + [`veyo-eval`](eval-harness.md) running offline on recorded sessions | **recall ≥0.9 @ <1% emission on ≥3 sessions, CPU-only** |
| **1 — Live MVP** | `PollBackend` (xcap/scap) + core + [`veyo-mcp`](architecture.md#mcp-surface) stream; the `veyod` binary | a local LLM subscribes over MCP and reacts to real on-screen events **live** |
| **2 — Tune & habituate** | full habituation/novelty wired; defaults locked from the harness | survives a video/spinner-heavy session **without spamming**; anomaly (pattern-break) fires correctly |
| **3 — Optimize & enrich** | `PipewireBackend` (damage/DMA-BUF) + [`veyo-store`](tech-specs.md#storage) (sqlite/ctxgraph) + optional Tier-2 | <X% CPU on the damage path; `query_events` + `describe_region` work |
| **4 — Expand** | CloakPipe summary routing; `region_mode=blocks` (YOLO); camera source; Win/macOS backends | regulated-deploy story (text-only + PII-sanitized); cross-platform parity |

---

## 3. Phase detail

### Phase 0 — Validate <span class="pill next">next up</span>

The whole bet, tested cheaply. Build only [`veyo-core`](policy-engine.md) and the [eval harness](eval-harness.md); no capture, no daemon. Record ≥3 real sessions, annotate them, grid-search the four knobs, and see whether the gate is reachable. **If not, the thesis fails here — by design.**

### Phase 1 — Live MVP

Make it real-time. Add the trivial cross-platform `PollBackend` (grab a frame at `capture_fps`, no damage info) and the `veyo-mcp` streaming surface, wire them through `veyod`. Success is qualitative and concrete: **a local LLM subscribes over MCP and visibly reacts to on-screen events as they happen.**

### Phase 2 — Tune & habituate

The long pole. Wire the full habituation/novelty model, lock the defaults the harness produced, and stress it: a session full of video and spinners must not flood the stream, while a *pattern-break* inside a habituated region (the spinner stops, an error flashes) must still fire. See [Policy Engine §4](policy-engine.md#habituation).

### Phase 3 — Optimize & enrich

Now make it cheap and durable. Add the `PipewireBackend` for compositor **damage rects** (skip untouched regions for ~free) and DMA-BUF zero-copy; add `veyo-store` (sqlite/FTS5 + the ctxgraph adapter); and turn on the optional Tier-2 enrichment path so `describe_region` returns semantic text. The capture-trait decoupling means none of this touches the FSM.

### Phase 4 — Expand

Open-ended. CloakPipe routing for cloud consumers (the [regulated-deploy story](privacy-model.md#compliance)); `blocks` region mode (YOLO-detected UI blocks instead of a fixed grid) *if recall demands it*; the camera source; and the Windows DXGI / macOS ScreenCaptureKit backends for cross-platform parity.

---

## 4. Timebox (solo, rough)

| Phase | Estimate |
|---|---|
| 0 — Validate | ~1–2 weeks |
| 1 — Live MVP | ~2 weeks |
| 2 — Tune & habituate | ~2–3 weeks (**long pole**) |
| 3 — Optimize & enrich | ~3–4 weeks |
| 4 — Expand | open-ended |

> Estimates are guidance. The **Phase-0 gate** is the real governor: Phase 1 does not begin until the codec is proven on recorded sessions.

---

## Related

- Why this order (the sequencing rule): **[Build Plan](plan.md)**.
- The gate that governs Phase 0→1: **[Eval Harness](eval-harness.md)**.
- The decisions still open as these phases begin: **[Risks & Open Questions](risks-and-open-questions.md)**.
