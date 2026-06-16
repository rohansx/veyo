# veyo — documentation

> **veyo** *(/ˈveɪ.oʊ/, from Latin **veo / video**, "I see")* — a local-first **visual event codec**. It turns a live screen (later, camera) feed into a compact, diff-on-state, LLM-native event stream, so an LLM or agent can have continuous, affordable, private *sight* of what's happening — **without raw imagery ever leaving the device**.

The one-sentence thesis: **visual data can live as cheap text deltas at the edge, which fixes cost and privacy with the same move.**

Daemon binary: `veyod`. Config: `veyo.toml`. Published crate identity: `veyo`.

---

## What this is

`veyo` watches a screen the way the eye's fovea works: high-acuity attention on what matters, everything else ignored. Instead of streaming pixels to a vision model 30×/second, it maintains a persistent text **world-state** on-device and emits a small structured **delta** only when the scene *meaningfully* changes. Most of the time the stream is silent.

The reduction is simultaneously the **cost fix** and the **privacy guarantee** — what crosses any process or network boundary is text describing change, never pixels.

```
~2.6B tokens/day   (naive: every 1080p frame → a VLM)
        ↓  veyo: emit-only-deltas, on CPU, no imagery leaves the box
~a few thousand tokens/day
```

*OmniParser parses a frame; **veyo watches a session**.*

---

## Documentation map

Read in this order for the full picture. Each doc exists as both Markdown (`.md`) and HTML (`.html`).

### The What & Why
- **[Product Overview](product-overview.md)** — the problem (cost · privacy · bandwidth), the thesis, who it's for, core concepts (world-state, deltas, the bi-temporal axis), token economics, competitive landscape, portfolio fit, and licensing.

### The How (design)
- **[Architecture](architecture.md)** — system shape, the capture→diff→FSM→emit pipeline, trait boundaries, the Cargo workspace layout, capture backends, and the MCP surface.
- **[Tech Specs](tech-specs.md)** — verified crate stack, the core Rust data model, the on-the-wire delta schema, `veyo.toml`, and storage.
- **[Policy Engine](policy-engine.md)** — *the real IP.* The "meaningful change" policy: the per-region debounce FSM, salience scoring, habituation / novelty, and anti-spam coalescing.
- **[Privacy Model](privacy-model.md)** — the moat: the non-negotiable privacy invariants, how the type system enforces them, and the compliance/audit story.

### Execution
- **[Eval Harness](eval-harness.md)** — Phase 0, built first: how we test the riskiest assumption offline on recorded sessions before writing a daemon. Recording format, annotation schema, scoring, the tuning loop, and the go/no-go gate.
- **[Build Plan](plan.md)** — the build philosophy ("build the test for the riskiest assumption before building the thing"), the sequencing rule, and the testing strategy.
- **[Phases & Roadmap](phases.md)** — the phased roadmap (0→4), the v1 MVP scope (in / out), per-phase success gates, and timeboxes.
- **[Risks & Open Questions](risks-and-open-questions.md)** — product- and engineering-level risks with mitigations, plus the open decisions to resolve during Phase 0–1.

### Reference
- **[Glossary](glossary.md)** — every term of art in one place.

---

## Status at a glance

This is a **v0.1 draft spec**. Nothing is built yet; the next concrete deliverable is the Phase 0 eval harness.

| Phase | Focus | State |
|---|---|---|
| **0 — Validate** | Core pipeline + eval harness, offline on recorded sessions | <span class="pill next">next up</span> |
| 1 — Live MVP | `PollBackend` + core + MCP stream; the `veyod` binary | planned |
| 2 — Tune & habituate | Full habituation/novelty; defaults locked from the harness | planned |
| 3 — Optimize & enrich | PipeWire damage path + storage + optional Tier-2 | planned |
| 4 — Expand | CloakPipe routing, `blocks` mode, camera, Win/macOS backends | planned |

The single gate that decides whether the thesis holds: **on ≥3 real recorded sessions, recall ≥ ~0.9 at emission < ~1% of frames, on CPU.** See [Eval Harness](eval-harness.md).

---

## Working with these docs

The HTML is **generated** from the Markdown — never hand-edit a `.html` file.

```bash
# from docs/
python3 build.py            # render every *.md -> *.html (+ index.html)
python3 build.py --check    # CI: fail if any HTML is stale vs its Markdown
```

`build.py` renders tables, fenced code, and a per-page table of contents, rewrites intra-doc `*.md` links to `*.html`, and wraps each page in the shared sidebar template at [`assets/style.css`](assets/style.css).

---

## Project naming note

The working name was `fovea` (the high-acuity center of the retina). `fovea` is taken on crates.io, so the published identity is **`veyo`** — short, ownable, and phonetically "I see." The daemon is `veyod`; the config file is `veyo.toml`; workspace crates are `veyo-core`, `veyo-capture`, `veyo-enrich`, `veyo-store`, `veyo-mcp`, and `veyo-eval`. The retired alternative `veyl` is mentioned only in historical context.
