//! `veyo-eval` — replay a recorded session offline and print a scored report.
//!
//! Usage:
//!   veyo-eval                       # run the built-in synthetic demo session
//!   veyo-eval --demo                # same
//!   veyo-eval <session-dir>         # score a recorded session
//!   veyo-eval <session-dir> --tune  # grid-search the knobs on that session
//!
//! A session dir holds `frames.jsonl`, `annotations.jsonl`, and `frames/<idx>.png`.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use veyo_core::CodecConfig;
use veyo_eval::decode::load_cells;
use veyo_eval::frame::SessionFrame;
use veyo_eval::report::render;
use veyo_eval::score::score;
use veyo_eval::session::{duration_ms, parse_annotations_jsonl, parse_frames_jsonl};
use veyo_eval::tune::{grid_search, Grid};
use veyo_eval::{run_codec, screen_surface, synthetic};

const GRID: (u8, u8) = (8, 8);
const MATCH_TOLERANCE_MS: u64 = 500;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    if positional.is_empty() || args.iter().any(|a| a == "--demo") {
        return demo();
    }
    let dir = PathBuf::from(positional[0]);
    let tune = args.iter().any(|a| a == "--tune");
    run_session(&dir, tune)
}

/// Run the built-in synthetic settle session — works with no footage on disk.
fn demo() -> Result<()> {
    let (frames, annotations) = synthetic::settle_session();
    let cfg = CodecConfig {
        grid: (1, 1),
        ..Default::default()
    };
    let dur = frames.last().map(|f| f.t_ms).unwrap_or(0);
    let deltas = run_codec(&frames, cfg, screen_surface(), (100, 100));
    let scored = score(&deltas, &annotations, frames.len(), dur, MATCH_TOLERANCE_MS);
    print!("{}", render("demo (synthetic settle)", &scored));
    eprintln!("\n(no session dir given — ran the synthetic demo; pass a dir to score real frames)");
    Ok(())
}

fn run_session(dir: &Path, tune: bool) -> Result<()> {
    let name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    let frames_meta = parse_frames_jsonl(
        &std::fs::read_to_string(dir.join("frames.jsonl")).context("read frames.jsonl")?,
    )?;
    let annotations = parse_annotations_jsonl(
        &std::fs::read_to_string(dir.join("annotations.jsonl"))
            .context("read annotations.jsonl")?,
    )?;

    let mut frames = Vec::with_capacity(frames_meta.len());
    let mut dims = (0u32, 0u32);
    for m in &frames_meta {
        let png = dir.join("frames").join(format!("{}.png", m.frame_idx));
        let (cells, d) = load_cells(&png, GRID.0 as usize, GRID.1 as usize)?;
        dims = d;
        frames.push(SessionFrame {
            t_ms: m.t_ms,
            cells,
        });
    }

    let dur = duration_ms(&frames_meta);
    let base = CodecConfig {
        grid: GRID,
        ..Default::default()
    };

    if tune {
        let grid = Grid {
            epsilon_noise: vec![0.02, 0.03, 0.05, 0.08],
            settle_window_ms: vec![300, 400, 600],
            salience_min: vec![0.3, 0.4, 0.5],
            novelty_decay: vec![0.85, 0.9, 0.95],
        };
        match grid_search(
            &frames,
            &screen_surface(),
            dims,
            &base,
            &grid,
            &annotations,
            MATCH_TOLERANCE_MS,
            0.01,
        ) {
            Some(r) => {
                println!("best knobs over {} trials: {:?}", r.trials, r.best);
                print!("{}", render(name, &r.score));
            }
            None => println!("no knob combination met the 1% emission ceiling on '{name}'"),
        }
    } else {
        let deltas = run_codec(&frames, base, screen_surface(), dims);
        let scored = score(&deltas, &annotations, frames.len(), dur, MATCH_TOLERANCE_MS);
        print!("{}", render(name, &scored));
    }
    Ok(())
}
