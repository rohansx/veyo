# veyo — policy engine (the "meaningful change" codec)

> **This is the real IP.** Everything else — capture, storage, MCP — is a mature, known quantity. The thing that can *make or kill* the product is whether this policy can be tuned to emit on what matters and stay silent otherwise, on real footage. That is exactly what the [Eval Harness](eval-harness.md) is built first to prove.

The policy is a per-region pipeline, ordered **cheap → expensive, short-circuiting early**. Most frames die at the first gate having done almost no work.

---

## 1. The pipeline

```
              ┌─────────── frame (low fps / OS damage event) ───────────┐
              ▼
   [Capture]  low-fps grab OR compositor dirty-rect/damage event
              ▼
   [Gate 1: cheap diff]   per-region downscaled pHash / SAD
              │  diff < epsilon_noise  ──► DROP (no work, no event)   ~99% of frames
              ▼
   [Settle/debounce FSM]  STATIC → CHANGING → SETTLING → STATIC
              │  emit region_change on STATIC→CHANGING
              │  emit state_settle  on SETTLING→STATIC (held > settle_window)
              ▼
   [Gate 2: salience]     score = f(region_weight, magnitude, novelty)
              │  salience < salience_min  ──► log internally, do NOT emit upstream
              ▼
   [Tier 2 enrichment]    OPTIONAL, only on passing events:
                          run UI tagger / OCR on THIS region only → semantic summary
              ▼
   [State update + emit]  update world-state, emit delta over MCP, append to ctxgraph
```

Two gates, in order of cost:

- **Gate 1 — cheap diff.** A per-region downscaled perceptual hash / SAD. `diff < epsilon_noise` ⇒ **drop**, no work, no event. This catches ~99% of frames.
- **Gate 2 — salience.** Only changes that survive the FSM are scored. `salience < salience_min` ⇒ logged internally but **not emitted upstream**.

Tier-2 ML enrichment, when enabled at all, runs **only** on events that pass *both* gates — so the expensive model sees a region a few dozen times an hour, never per-frame.

---

## 2. Per-region finite state machine {#per-region-finite-state-machine}

States: `STATIC`, `CHANGING`, `SETTLING`. Evaluated per region, on each candidate frame.

| From | Trigger | Action |
|---|---|---|
| `Static` | diff > `epsilon_noise` | → `Changing`; record `changing_since`; emit `region_change` *(if salience passes)* |
| `Changing` | diff < `epsilon_noise` | → `Settling`; arm `settle_deadline = now + settle_window_ms` |
| `Settling` | `settle_deadline` elapsed | → `Static`; emit `state_settle { duration_ms }` *(if salience passes)* |
| `Settling` | diff > `epsilon_noise` | → `Changing`; cancel deadline; **no emission** |

### Why `state_settle` is the workhorse

`state_settle` is the **key compression**: a 2-second scroll / animation / page-load collapses to **two events** (start + settle), not 60 frames. It is also frequently the *most useful* event to an agent — "the page finished loading", "the build is done", "the dialog stopped animating" are all settles.

Implement the deadlines with `tokio::time` timers keyed by `RegionId`. The `state_settle` shape is **frozen before v0.1** because downstream agents bind to it — see [Risks & Open Questions](risks-and-open-questions.md).

---

## 3. Salience {#salience}

The "too sensitive vs too lax" control surface. Salience gates emission:

```
salience = w_focus(region) * magnitude * novelty
emit if salience >= salience_min
```

- **`magnitude`** = normalized diff — mean absolute difference over the downscaled cell, in `[0,1]` (see `veyo-core::diff`).
- **`w_focus`** = `focus_weight` if the region is in the focused surface, else `1.0`. Focused-surface events are always weighted above background.
- **`novelty`** = `1 - habituation` (below).

The output lands in `Delta.salience ∈ [0,1]`; consumers filter on it via `min_salience` in [`subscribe_events`](architecture.md#mcp-surface).

---

## 4. Habituation / novelty {#habituation}

> **The differentiator — and the part that's genuinely hard to copy.**

Each region keeps a **rolling estimate of how often it changes** (`NoveltyBaseline`). Repetitive, periodic change — a video playing, a blinking cursor, a spinner — drives `novelty → 0`, so its events get low salience, fall below `salience_min`, and **the stream stops spamming**. A *new* kind of change (a modal, an app switch, a content region that's been static suddenly moving) scores high novelty.

The decay follows a stretched-exponential-style curve — but **treat the exact law as a tunable, not gospel** (`novelty_decay`).

### Dynamic, not just decaying

The crucial property: habituation is **reversible**. If a habituated pattern *breaks* — the blinking cursor stops, a static editor suddenly flashes a red error — `novelty` spikes back to ~1 and the event fires. This pattern-break spike is what lets `veyo` habituate to noise without going deaf to the *meaningful* repeated event.

This is the anti-spam moat, and it must be **exercised hard in the eval harness** — specifically: a video/spinner-heavy session must not spam, *and* a pattern-break inside a habituated region must still fire.

> Conceptually adjacent to a successor-representation / predictive-novelty model — cf. `primd`.

---

## 5. Anti-spam / coalescing {#5-anti-spam-coalescing}

> **Status: spatial coalescing is implemented; temporal is planned.** `veyo-core` now
> merges a same-frame, same-kind burst across ≥ `coalesce_min_regions` cells into one
> macro-delta (region id `r_multi`, bounds = the union) — so a modal or app switch that
> lights the whole grid emits *one* event, not sixty. Measured on a recorded session
> this cut emissions ~16× with **no** recall loss. The **temporal** `coalesce_window_ms`
> rate cap (merging across adjacent *frames*) is still future work.

- A **global rate cap** per `coalesce_window_ms`. Excess low-salience events merge into one coalesced summary — *"3 background regions changed"*.
- **Adjacent-cell coalescing.** A modal that spans 4 grid cells would naively produce 4 `region_change` events; `coalesce_window_ms` merges concurrent low-salience changes across adjacent grid cells into one macro-event. (This is also the main mitigation for grid mode "fracturing" a UI element — see [Risks](risks-and-open-questions.md).)
- **Focus always wins.** Focused-surface events are weighted above background, so coalescing never swallows the thing the user is actually looking at.

---

## 6. The tunable knobs

These four are **hyperparameters**, not constants. Their values come from the [Eval Harness](eval-harness.md), not from intuition.

| knob | role in the policy | starting default |
|---|---|---|
| `epsilon_noise` | Gate-1 diff floor — below this, a region is "not changing" | 0.03 (mean abs diff, `[0,1]`) |
| `settle_window_ms` | how long a region must hold static before `state_settle` fires | 400 |
| `salience_min` | Gate-2 emit threshold | 0.4 |
| `novelty_decay` | how fast a repetitive region habituates toward silence | 0.9 / window |

Supporting knobs: `focus_weight` (1.5), `coalesce_window_ms` (1000), `capture_fps` (4), `region_mode` (`grid`). Full list in [Tech Specs](tech-specs.md#knob-reference).

**The engineering work is finding the settings** where recall stays high (the agent doesn't miss what matters) while emission stays under ~1% of frames (the stream isn't noise). There is no closed form; it is treated as an ML tuning problem. See the [tuning loop](eval-harness.md#tuning-loop).

---

## Related

- The types this logic operates on (`Phase`, `RegionState`, `Delta`, `NoveltyBaseline`): **[Tech Specs](tech-specs.md#core-data-model)**.
- Where this sits in the running system: **[Architecture](architecture.md)**.
- How we prove it actually works before building a daemon: **[Eval Harness](eval-harness.md)**.
- The risks specific to tuning and to grid regions: **[Risks & Open Questions](risks-and-open-questions.md)**.
