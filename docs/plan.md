# veyo — build plan

> The build *philosophy* and the testing strategy. For the concrete sequence of deliverables and gates, see [Phases & Roadmap](phases.md). For the artifact this plan front-loads, see the [Eval Harness](eval-harness.md).

---

## 1. The sequencing rule

One principle drives the whole plan:

> **Build the test for the riskiest assumption before building the thing.**

The daemon — capture → diff → FSM → MCP — is the **known** part. Every piece is a mature crate ([Tech Specs](tech-specs.md)); wiring them is engineering, not research. The **unknown** part — the only thing that can kill the product — is whether the four tuning knobs can be set so the codec emits on what matters and stays silent otherwise, **on real footage**.

So **Phase 0 is the eval harness running the core pipeline offline on recorded sessions**, with zero live capture and zero daemon. If the recall/emission numbers can't be hit there, you stop — having spent **days, not months**.

**Everything after Phase 0 is incremental hardening of a thing already proven to work in principle.**

---

## 2. Why this ordering

| If you build… | first | the risk is |
|---|---|---|
| the capture backend | first | you sink weeks into Wayland/PipeWire fiddliness before knowing the codec works at all |
| the daemon + MCP | first | you have a beautiful pipeline that emits the *wrong* events, and no way to measure it |
| **the eval harness** | **first** | **you learn in days whether the thesis is even true — the cheapest possible way to fail** |

The eval harness is cheap precisely because [`veyo-core`](architecture.md) does no I/O: you can drive the entire decision pipeline from recorded frames with no capture backend, no daemon, and no MCP server.

---

## 3. Build order (summary)

The full table with success gates is in [Phases & Roadmap](phases.md). In brief:

1. **Phase 0 — Validate.** `veyo-core` (schema, FSM, salience stub) + `veyo-eval`, offline on recorded sessions. *Gate: recall ≥0.9 @ <1% emission on ≥3 sessions, CPU-only.*
2. **Phase 1 — Live MVP.** `PollBackend` (xcap/scap) + core + `veyo-mcp` stream; the `veyod` binary. *Gate: a local LLM subscribes over MCP and reacts to real on-screen events live.*
3. **Phase 2 — Tune & habituate.** Full habituation/novelty wired; defaults locked from the harness.
4. **Phase 3 — Optimize & enrich.** `PipewireBackend` (damage/DMA-BUF) + `veyo-store` + optional Tier-2.
5. **Phase 4 — Expand.** CloakPipe routing, `blocks` mode, camera, Windows/macOS backends.

The decoupling that makes this order safe: the **capture backend is isolated behind a trait**, so starting with the trivial `PollBackend` and adding the optimized `PipewireBackend` later changes nothing downstream. The FSM never learns which OS it's on. See [Architecture §3](architecture.md).

---

## 4. Testing strategy {#testing-strategy}

Three layers, each with a clear job.

### Unit (`veyo-core`)

- FSM transition table — **table-driven tests** over every `(state, trigger) → (state, emission)` row from the [Policy Engine](policy-engine.md#per-region-finite-state-machine).
- Salience math.
- Habituation **decay** *and* **pattern-break spike** — the reversibility is a correctness property, not a nice-to-have.
- Coalescing.

### Integration (the eval harness *is* the integration test)

Replay fixtures, assert recall/emission stay within thresholds, and **wire it into CI so recall can't regress.** The recorded sessions become permanent fixtures. This is the connective tissue between "it works on my session" and "it keeps working." See the [Eval Harness](eval-harness.md).

### Bench (`criterion`)

- Per-region diff throughput — target *imperceptible* CPU at `capture_fps`.
- End-to-end frame→delta latency.
- Bounded RSS over a long **soak run** (the ring buffer must not leak).

---

## 5. Treat tuning as ML, not intuition

The recurring trap is to hand-tweak thresholds until a demo looks good. Don't. The four knobs are hyperparameters; the harness is the training/eval loop; the CI fixtures are the held-out set. This framing — *empirical, measured, regression-guarded* — is the difference between a codec that holds up across sessions and one that was overfit to a single screen recording. The detail of that loop is in the [Eval Harness](eval-harness.md#tuning-loop).

---

## 6. Rough timebox (solo)

| Phase | Estimate | Notes |
|---|---|---|
| 0 — Validate | ~1–2 weeks | |
| 1 — Live MVP | ~2 weeks | |
| 2 — Tune & habituate | ~2–3 weeks | **tuning is the long pole** |
| 3 — Optimize & enrich | ~3–4 weeks | |
| 4 — Expand | open-ended | |

These are guidance, not commitments; the Phase-0 gate is what actually governs whether Phase 1 begins.

---

## Related

- The deliverables and gates per phase: **[Phases & Roadmap](phases.md)**.
- The thing built first: **[Eval Harness](eval-harness.md)**.
- The decoupling that makes the order safe: **[Architecture](architecture.md)**.
- The risks this plan is structured to retire: **[Risks & Open Questions](risks-and-open-questions.md)**.
