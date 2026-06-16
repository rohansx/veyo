# veyo

> A local-first **visual event codec**. It turns a live screen feed into a compact,
> diff-on-state, LLM-native event stream — continuous, affordable, private *sight* for an
> LLM or agent, **without raw imagery ever leaving the device**.

The thesis: *visual data can live as cheap text deltas at the edge, which fixes cost and
privacy with the same move.* Instead of streaming pixels to a vision model 30×/second,
`veyo` keeps a text **world-state** on-device and emits a small structured **delta** only
when the scene *meaningfully* changes. Most of the time the stream is silent.

```
~2.6B tokens/day   (naive: every 1080p frame → a VLM)
        ↓  veyo: emit-only-deltas, on CPU, no imagery leaves the box
~a few thousand tokens/day
```

## Documentation

Full design lives in [`docs/`](docs/README.md) (Markdown + generated HTML):
[product overview](docs/product-overview.md) · [architecture](docs/architecture.md) ·
[tech specs](docs/tech-specs.md) · [policy engine](docs/policy-engine.md) ·
[privacy model](docs/privacy-model.md) · [eval harness](docs/eval-harness.md) ·
[plan](docs/plan.md) · [phases](docs/phases.md) ·
[risks](docs/risks-and-open-questions.md) · [glossary](docs/glossary.md).

## Workspace

```
crates/
  veyo-core/   # pure, no-I/O: frozen delta schema, debounce FSM, salience, habituation,
               # Gate-1 diff, and the Codec engine that wires them together
  veyo-eval/   # Phase-0 offline harness: replay recorded sessions through veyo-core,
               # score recall/precision/emission, grid-search the tuning knobs
fixtures/
  sessions/    # recorded eval sessions + annotations (see its README)
docs/          # the design docs (md + html)
```

Planned (later phases): `veyo-capture`, `veyo-mcp`, `veyo-store`, `veyo-enrich`, and the
`veyod` daemon binary. See [phases](docs/phases.md).

## Status — Phase 0 (validate the codec offline)

The bet is whether the four tuning knobs can be set so the codec emits on what matters and
stays silent otherwise. We test that **before** building a daemon — the gate is *recall
≥ 0.9 at emission < 1% of frames on ≥3 real sessions, on CPU*. `veyo-core` and the
`veyo-eval` harness exist; the next dependency is **recording real sessions**.

```bash
cargo test --workspace          # unit + integration tests
cargo run -p veyo-eval -- --demo # run the built-in synthetic demo session
cargo run -p veyo-eval -- fixtures/sessions/<name> --tune   # tune on a real session
```

## License

Open-core: Apache-2.0 core daemon + schema; a commercial layer for managed enrichment,
CloakPipe integration, and compliance tooling. See [licensing](docs/product-overview.md#7-licensing).
