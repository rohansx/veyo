# veyo — eval harness (Phase 0, build this first)

> The single most important artifact in the whole project. The daemon is the *known* part — every piece is a mature crate. The *unknown* part, the only thing that can kill the product, is whether the four tuning knobs can be set so the codec emits on what matters and stays silent otherwise, on real footage. **So we test that before building the thing.** See the [Build Plan](plan.md) for the sequencing rule this embodies.

The harness runs `veyo-core` **offline on recorded sessions** — zero live capture, zero daemon. If the recall/emission numbers can't be hit here, you stop, having spent **days, not months**.

---

## 1. What it is

`veyo-eval` replays recorded screen sessions through the pure [`veyo-core`](tech-specs.md#core-data-model) pipeline (diff → FSM → salience → habituation), collects the deltas it *would* emit, and scores them against a human annotation of "things that mattered." It is also the **integration test** that runs in CI so recall can never silently regress.

Because `veyo-core` does no I/O, the harness needs no capture backend and no MCP server — it feeds frames straight in and reads deltas straight out.

---

## 2. Recording format

A **session** = a video file (or a PNG frame dump) + a sidecar `frames.jsonl`:

```json
{ "frame_idx": 0, "t_ms": 0 }
{ "frame_idx": 1, "t_ms": 250 }
```

Capture with **any** screen recorder; the harness only needs frames + timestamps. Store under:

```
fixtures/sessions/<name>/
├── frames/                # or a single video file
├── frames.jsonl          # {frame_idx, t_ms}
└── annotations.jsonl     # the ground truth (below)
```

---

## 3. Annotation schema

`<name>/annotations.jsonl`, one line per "thing that mattered":

```json
{ "t_ms": 252100, "kind": "build_finished", "surface": "terminal", "note": "cargo build done" }
```

Aim for **30–60 annotations per recorded hour**. Annotate the events an agent would care about: a modal appeared, a build finished, an error was shown, an app switch happened, a long job errored.

---

## 4. Scoring

Run `veyo-core` over the frames, collect emitted deltas, then compute:

| metric | definition | why |
|---|---|---|
| **recall** | annotated events with a matching emission within `±match_tolerance_ms` ÷ total annotations | **the number that matters most** — false negatives hurt agents more than false positives |
| **precision / emission rate** | matched emissions ÷ total emissions; plus raw events/hour | is the stream noisy? |
| **cost** | CPU time, peak RSS, total summary tokens | is it actually cheap, on CPU, bounded RSS? |

Output a **one-page report per session** plus an aggregate across sessions.

> **Recall is the primary objective; emission-rate is the constraint.** For an agent, missing "the build finished" is worse than one spurious "a region changed." Optimize accordingly — see [Risks](risks-and-open-questions.md).

---

## 5. The tuning loop {#tuning-loop}

Treat the four knobs — `epsilon_noise`, `settle_window_ms`, `salience_min`, `novelty_decay` — as **hyperparameters**, not constants.

1. **Coarse grid search** over `epsilon_noise × settle_window_ms × salience_min × novelty_decay`, maximizing recall **subject to an emission-rate ceiling**.
2. **Refine with Bayesian optimization** if the grid is too slow.
3. **Lock the winning defaults** into [`veyo.toml`](tech-specs.md#config).
4. **Keep the sessions as CI fixtures** so future changes can't silently regress recall.

The habituation behavior must be **exercised hard** here: a video/spinner-heavy session must not spam (novelty decays to silence), *and* a pattern-break inside a habituated region must still fire (novelty spikes back). See the [Policy Engine](policy-engine.md#habituation).

---

## 6. The Phase-0 success gate

> On **≥3 real recorded sessions**, hit **recall ≥ ~0.9** at **emission < ~1% of frames**, on **CPU**.

If that is reachable, the thesis holds and everything after is incremental hardening of a thing already proven to work in principle. **If it is unreachable, the thesis fails cheaply — and that is the entire point of doing this first.**

---

## 7. Why this de-risks everything

- It tests the **riskiest assumption** (tuning) with the **cheapest possible build** (no capture, no daemon, no MCP).
- It produces the **real config defaults** that the [Tech Specs](tech-specs.md#config) ship with.
- It becomes the **permanent regression test**: the same fixtures run in CI, so a refactor that quietly drops recall fails the build (see [Testing in the Build Plan](plan.md#testing-strategy)).

---

## Related

- The pipeline being scored: **[Policy Engine](policy-engine.md)**.
- The philosophy behind building this first: **[Build Plan](plan.md)**.
- Where Phase 0 sits and what unblocks once it passes: **[Phases & Roadmap](phases.md)**.
- The types it drives: **[Tech Specs](tech-specs.md#core-data-model)**.
