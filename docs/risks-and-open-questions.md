# veyo — risks & open questions

> The honest list: what could go wrong, how the plan mitigates it, and which decisions are deliberately deferred to Phase 0–1. The whole [Build Plan](plan.md) is structured to retire the top risk first.

---

## 1. The headline risk

> **Salience tuning is empirical.** There is no closed form for "meaningful change." It needs the [eval harness](eval-harness.md) and real sessions. **This is the main risk to the timeline** — and the reason Phase 0 exists before anything else.

Everything below is secondary to this one.

---

## 2. Product-level risks

| Risk | Mitigation |
|---|---|
| **Region definition** — a fixed grid is simple but dumb; detected UI blocks are smarter but heavier. | Start with `grid`. Measure recall in the harness; escalate to `blocks` **only if recall demands it**. |
| **Salience tuning is empirical** — no closed form. | Phase 0 harness + CI fixtures; treat it as ML, not intuition. The dominant timeline risk. |
| **Settle window varies by content** — a spinner, a page load, and typing settle very differently. | May need per-content-class windows. Start with one `settle_window_ms`; let the harness reveal whether one value suffices. |
| **Animation / video regions** | Habituation should handle these (novelty → 0). **Verify it doesn't also habituate to legitimately important repeated events** — exercise pattern-break hard in the harness. |
| **Capture portability** — Wayland capture is fiddly. | Budget for it. Don't start there: `PollBackend` first, PipeWire as a Phase-3 optimization. |
| **Schema versioning** — consumers bind to the wire shape. | Lock `v` semantics (and the `StateSettle` shape) **early**, before v0.1 ships. |

---

## 3. Engineering-level risks

| Risk | Mitigation |
|---|---|
| **Tuning is empirical** | Phase 0 harness + CI fixtures; treat as ML, not intuition. |
| **macOS has no dirty-rects** | The [`CaptureBackend` trait](tech-specs.md#capture-backend-trait) isolates this; macOS leans on Tier-1 pHash; the FSM is unchanged. |
| **Grid fractures a modal across cells** — one UI element spanning 4 grid cells → 4 events. | `coalesce_window_ms` merges adjacent-cell changes; escalate to `blocks` mode only if recall demands it. See [Policy Engine §5](policy-engine.md#5-anti-spam-coalescing). |
| **False negatives > false positives for agents** — a missed "build finished" costs more than a spurious "region changed." | Optimize the harness for **recall first, emission-rate second**. |
| **Wayland capture is fiddly** | Don't start there; `PollBackend` first, PipeWire as a Phase-3 optimization. |
| **Schema churn breaks consumers** | Freeze `v` / `StateSettle` before v0.1. |

---

## 4. Why these two risk lists overlap

The product and engineering lists repeat "tuning is empirical," "macOS/Wayland capture," and "schema freeze" on purpose — these are the points where a *product* concern and an *engineering* concern are the same fact seen from two sides. The repetition is a signal of where to spend attention, not redundancy to trim.

---

## 5. Open decisions (resolve during Phase 0–1)

These are deliberately **not** decided yet; the harness and the first live sessions are expected to settle them.

1. **Region model** — fixed `grid` (start) vs semantic `blocks` (defer). *Decided by:* whether grid recall is good enough in the harness.
2. **Capture priority beyond Linux** — Windows DXGI (has damage, easy win) vs macOS (no damage, more work). *Decided by:* target users + effort.
3. **Store** — `rusqlite` standalone vs `ctxgraph` as the primary sink from day one. *Decided by:* how tightly the bi-temporal memory story couples in early.
4. **MCP transport default** — `stdio` (single agent) vs SSE (daemon with subscribers). *Decided by:* the dominant consumer shape in Phase 1.
5. **Final name + crate publish identity** — **resolved: `veyo`** (the retired alternative was `veyl`; `fovea` was taken on crates.io). The daemon is `veyod`, the config is `veyo.toml`.

---

## 6. The freeze list (lock before v0.1)

Two things must be frozen before the first release because downstream agents bind to them:

- the schema version field **`v`** and its semantics
- the **`state_settle`** event shape (`duration_ms`, the surface/region refs)

Freezing these is itself a [privacy/compliance guarantee](privacy-model.md#compliance): a stable, versioned, text-only contract is what makes the bi-temporal audit log meaningful over time.

---

## Related

- The plan built around retiring risk #1 first: **[Build Plan](plan.md)**.
- The instrument that retires it: **[Eval Harness](eval-harness.md)**.
- The trait that isolates the capture risks: **[Architecture](architecture.md#capture-backends)** / **[Tech Specs](tech-specs.md#capture-backend-trait)**.
