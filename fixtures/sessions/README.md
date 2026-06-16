# veyo eval sessions

Drop recorded sessions here, one directory per session. These are the fixtures the
Phase-0 [eval harness](../../docs/eval-harness.md) replays and the CI regression guards
run against. The go/no-go gate needs **≥3 real recorded sessions**.

## Layout

```
fixtures/sessions/<name>/
├── frames.jsonl        # one {"frame_idx": N, "t_ms": T} per line
├── annotations.jsonl   # one {"t_ms": T, "kind": "...", "surface": "...", "note": "..."} per line
└── frames/
    ├── 0.png           # 8-bit PNG (grayscale or RGB/RGBA); name = frame_idx
    ├── 1.png
    └── ...
```

- **`frames.jsonl`** — the frame timeline. Capture with any screen recorder, then dump
  frames + timestamps. `frame_idx` maps to `frames/<frame_idx>.png`.
- **`annotations.jsonl`** — the ground truth: one line per "thing that mattered" (a modal
  appeared, a build finished, an error showed, an app switched). Aim for **30–60
  annotations per recorded hour**. Only `t_ms` and `kind` are required; `surface` and
  `note` are optional.
- **`frames/`** — the images. 8-bit PNG only for now (grayscale, RGB, or RGBA). Indexed
  and 16-bit PNGs are rejected — re-export as RGB or grayscale. Video decode is out of
  Phase-0 scope.

## Running

```bash
# score a session with the default knobs
cargo run -p veyo-eval -- fixtures/sessions/<name>

# grid-search the four tuning knobs on a session
cargo run -p veyo-eval -- fixtures/sessions/<name> --tune

# no session yet? run the built-in synthetic demo
cargo run -p veyo-eval -- --demo
```

## A note on this directory

Recorded `frames/` are large, so they're git-ignored (see the repo `.gitignore`). Commit
the small `frames.jsonl` and `annotations.jsonl` if you want a session to act as a CI
fixture; keep the heavy PNGs out of version control (or store them via LFS / an external
artifact store).
