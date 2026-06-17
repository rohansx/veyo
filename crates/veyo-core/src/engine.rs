//! The codec engine: the per-region pipeline glue.
//!
//! Wires the four pure pieces into one step — `diff` (Gate 1) → `RegionFsm` (debounce)
//! → `NoveltyBaseline` (habituation) → `salience` (Gate 2) → [`Delta`]. Both the daemon
//! and the offline [eval harness] drive this same engine; it owns no I/O and is fully
//! deterministic given a frame stream.
//!
//! ## Salience basis
//!
//! Salience is `w_focus · magnitude · novelty`, but *which* magnitude/novelty differs
//! by event:
//!
//! - **`RegionChange`** uses the triggering frame's magnitude and current novelty.
//! - **`StateSettle`** can't use the settle frame's magnitude (it's ~0 — that's *why*
//!   it settled). It uses the episode's **peak magnitude** and the novelty captured
//!   when the episode began. So a settle's importance reflects how much changed during
//!   the episode and how novel the episode was — not the silence at its end.
//!
//! [eval harness]: ../../../docs/eval-harness.md

use crate::diff::{self, Cell};
use crate::fsm::{FsmEvent, RegionFsm};
use crate::habituation::NoveltyBaseline;
use crate::salience::{focus_multiplier, salience, should_emit};
use crate::schema::{
    Delta, EventId, EventKind, Evidence, Rect, RegionRef, SurfaceRef, TimeMs, SCHEMA_V,
};

/// The tunable subset of `veyo.toml` the pure engine consumes.
#[derive(Debug, Clone)]
pub struct CodecConfig {
    /// Region grid as `(cols, rows)`.
    pub grid: (u8, u8),
    /// Gate-1 magnitude floor in `[0,1]` (mean abs diff).
    pub epsilon_noise: f32,
    /// Quiet hold (ms) before a `Changing` region emits `StateSettle`.
    pub settle_window_ms: u64,
    /// Gate-2 salience floor in `[0,1]`; deltas scoring below it are suppressed.
    pub salience_min: f32,
    /// Habituation EWMA decay in `(0,1)`; higher = slower to habituate and forget.
    pub novelty_decay: f32,
    /// Salience multiplier applied to regions in the focused surface (else `1.0`).
    pub focus_weight: f32,
    /// Spatial anti-spam: when at least this many regions emit the **same kind** in one
    /// frame (e.g. a modal or app switch lighting many grid cells), they're merged into
    /// a single coalesced macro-delta. Set high to disable.
    pub coalesce_min_regions: usize,
    /// Source descriptor stamped onto every delta, e.g. `"screen:0"`.
    pub source: String,
}

impl Default for CodecConfig {
    fn default() -> Self {
        Self {
            grid: (8, 8),
            epsilon_noise: 0.03,
            settle_window_ms: 400,
            salience_min: 0.4,
            novelty_decay: 0.9,
            focus_weight: 1.5,
            coalesce_min_regions: 4,
            source: "screen:0".to_string(),
        }
    }
}

/// One captured frame, already downscaled into per-region cells (`cells.len()` must
/// equal the grid's `cols × rows`).
pub struct Frame<'a> {
    pub t_ms: TimeMs,
    pub cells: &'a [Cell],
}

struct RegionRuntime {
    reference: RegionRef,
    fsm: RegionFsm,
    baseline: NoveltyBaseline,
    last: Option<Cell>,
    episode_peak: f32,
    episode_novelty: f32,
}

/// The stateful per-display codec. Feed it frames; it emits deltas.
pub struct Codec {
    cfg: CodecConfig,
    surface: SurfaceRef,
    regions: Vec<RegionRuntime>,
    seq: u64,
}

impl Codec {
    /// Build a codec for a `dims` (width, height) display under one focused surface.
    pub fn new(cfg: CodecConfig, surface: SurfaceRef, dims: (u32, u32)) -> Self {
        let cols = cfg.grid.0.max(1) as u32;
        let rows = cfg.grid.1.max(1) as u32;
        let cw = (dims.0 / cols) as i32;
        let ch = (dims.1 / rows) as i32;
        let mut regions = Vec::with_capacity((cols * rows) as usize);
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                regions.push(RegionRuntime {
                    reference: RegionRef {
                        id: format!("r_{i}"),
                        grid: [c as u8, r as u8],
                        bounds: Rect {
                            x: c as i32 * cw,
                            y: r as i32 * ch,
                            w: cw,
                            h: ch,
                        },
                    },
                    fsm: RegionFsm::new(),
                    baseline: NoveltyBaseline::new(cfg.novelty_decay),
                    last: None,
                    episode_peak: 0.0,
                    episode_novelty: 0.0,
                });
            }
        }
        Self {
            cfg,
            surface,
            regions,
            seq: 0,
        }
    }

    /// Number of regions (`cols × rows`).
    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    /// Advance one frame; return the deltas it produced (usually none).
    ///
    /// Per-region intents are collected first, then turned into deltas with spatial
    /// coalescing — when many regions emit the same kind in one frame (a modal, an app
    /// switch), they merge into one macro-delta instead of fragmenting across the grid.
    pub fn observe(&mut self, frame: Frame) -> Vec<Delta> {
        debug_assert_eq!(
            frame.cells.len(),
            self.regions.len(),
            "frame cell count must equal region count (cols × rows)"
        );
        let n = self.regions.len().min(frame.cells.len());
        let mut intents: Vec<Intent> = Vec::new();
        for i in 0..n {
            let cell = &frame.cells[i];
            let region = &mut self.regions[i];
            let mag = match region.last {
                Some(ref prev) => diff::magnitude(prev, cell),
                None => 0.0,
            };
            region.last = Some(*cell);
            let changed = mag >= self.cfg.epsilon_noise;
            region.baseline.observe(changed);
            let novelty = region.baseline.novelty();

            let event = region
                .fsm
                .observe(changed, frame.t_ms, self.cfg.settle_window_ms);

            // Maintain the episode's peak magnitude / opening novelty.
            match event {
                Some(FsmEvent::RegionChange) => {
                    region.episode_peak = mag;
                    region.episode_novelty = novelty;
                }
                _ if changed => region.episode_peak = region.episode_peak.max(mag),
                _ => {}
            }

            let Some(event) = event else { continue };
            let (basis_mag, basis_nov, kind, duration) = match event {
                FsmEvent::RegionChange => (mag, novelty, EventKind::RegionChange, None),
                FsmEvent::StateSettle { duration_ms } => {
                    let t = (
                        region.episode_peak,
                        region.episode_novelty,
                        EventKind::StateSettle,
                        Some(duration_ms),
                    );
                    region.episode_peak = 0.0;
                    t
                }
            };
            let w = focus_multiplier(self.surface.focused, self.cfg.focus_weight);
            let score = salience(w, basis_mag, basis_nov);
            if should_emit(score, self.cfg.salience_min) {
                intents.push(Intent {
                    region: i,
                    kind,
                    salience: score,
                    novelty: basis_nov,
                    duration,
                });
            }
        }
        self.build_deltas(intents, frame.t_ms)
    }

    /// Turn this frame's emit intents into deltas, coalescing dense same-kind bursts.
    fn build_deltas(&mut self, intents: Vec<Intent>, t: TimeMs) -> Vec<Delta> {
        let mut out = Vec::new();
        for kind in [EventKind::RegionChange, EventKind::StateSettle] {
            let group: Vec<&Intent> = intents.iter().filter(|x| x.kind == kind).collect();
            if group.is_empty() {
                continue;
            }
            if group.len() >= self.cfg.coalesce_min_regions.max(2) {
                out.push(self.coalesced_delta(&group, kind, t));
            } else {
                for it in &group {
                    out.push(self.single_delta(it, t));
                }
            }
        }
        out
    }

    fn single_delta(&mut self, it: &Intent, t: TimeMs) -> Delta {
        self.seq += 1;
        let reference = self.regions[it.region].reference.clone();
        let summary = summary_for(&it.kind, &reference.id, it.duration, &self.surface);
        Delta {
            v: SCHEMA_V,
            id: EventId(format!("ev_{:012}", self.seq)),
            t_event: t,
            t_observed: t,
            source: self.cfg.source.clone(),
            kind: it.kind,
            surface: self.surface.clone(),
            region: reference,
            summary,
            salience: it.salience,
            novelty: it.novelty,
            duration_ms: it.duration,
            evidence: Evidence::default(),
        }
    }

    fn coalesced_delta(&mut self, group: &[&Intent], kind: EventKind, t: TimeMs) -> Delta {
        self.seq += 1;
        let n = group.len();
        let salience = group.iter().map(|x| x.salience).fold(0.0_f32, f32::max);
        let novelty = group.iter().map(|x| x.novelty).fold(0.0_f32, f32::max);
        let bounds = group
            .iter()
            .map(|x| self.regions[x.region].reference.bounds)
            .reduce(union_rect)
            .unwrap_or(Rect {
                x: 0,
                y: 0,
                w: 0,
                h: 0,
            });
        let (verb, duration) = match kind {
            EventKind::StateSettle => ("settled", group.iter().filter_map(|x| x.duration).max()),
            _ => ("changed", None),
        };
        Delta {
            v: SCHEMA_V,
            id: EventId(format!("ev_{:012}", self.seq)),
            t_event: t,
            t_observed: t,
            source: self.cfg.source.clone(),
            kind,
            surface: self.surface.clone(),
            region: RegionRef {
                id: "r_multi".to_string(),
                grid: [255, 255],
                bounds,
            },
            summary: {
                let ctx = surface_ctx(&self.surface);
                format!("{n} regions {verb}{ctx}")
            },
            salience,
            novelty,
            duration_ms: duration,
            evidence: Evidence::default(),
        }
    }
}

/// One region's decision for a frame, before coalescing.
struct Intent {
    region: usize,
    kind: EventKind,
    salience: f32,
    novelty: f32,
    duration: Option<u32>,
}

/// Smallest rectangle covering both inputs.
fn union_rect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

/// Build a human-readable summary the LLM reads. Include surface context so
/// summaries are self-contained without looking up the surface field.
fn summary_for(
    kind: &EventKind,
    region_id: &str,
    duration: Option<u32>,
    surface: &SurfaceRef,
) -> String {
    let ctx = surface_ctx(surface);
    match kind {
        EventKind::RegionChange => format!("region {region_id} started changing{ctx}"),
        EventKind::StateSettle => format!(
            "region {region_id} settled after {}ms{ctx}",
            duration.unwrap_or(0)
        ),
        other => format!("region {region_id}: {other:?}{ctx}"),
    }
}

/// Returns " in App — Title" when the surface has a meaningful app/title.
fn surface_ctx(s: &SurfaceRef) -> String {
    let app = s.app.trim();
    let title = s.title.trim();
    match (app.is_empty(), title.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(" in {app}"),
        (true, false) => format!(" — {title}"),
        (false, false) => format!(" in {app} — {title}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focused_surface() -> SurfaceRef {
        SurfaceRef {
            id: "win_1".into(),
            app: "test".into(),
            title: "t".into(),
            focused: true,
        }
    }

    fn one_by_one() -> Codec {
        Codec::new(
            CodecConfig {
                grid: (1, 1),
                ..Default::default()
            },
            focused_surface(),
            (100, 100),
        )
    }

    fn solid(v: u8) -> Vec<Cell> {
        vec![[v; crate::diff::CELL_LEN]]
    }

    /// A change-then-settle episode emits exactly RegionChange + StateSettle, with the
    /// settle carrying the episode duration and both clearing the salience gate.
    #[test]
    fn change_then_settle_emits_two_salient_events() {
        let mut codec = one_by_one();
        let script = [(0u64, 0u8), (250, 255), (500, 255), (1000, 255)];
        let mut out = Vec::new();
        for (t, v) in script {
            out.extend(codec.observe(Frame {
                t_ms: t,
                cells: &solid(v),
            }));
        }
        assert_eq!(
            out.len(),
            2,
            "expected exactly change + settle, got {out:?}"
        );
        assert_eq!(out[0].kind, EventKind::RegionChange);
        assert_eq!(out[0].t_event, 250);
        assert_eq!(out[0].v, SCHEMA_V);
        assert!(out[0].summary.contains("r_0"));
        assert!(out[0].salience >= 0.4);
        assert_eq!(out[1].kind, EventKind::StateSettle);
        assert_eq!(out[1].duration_ms, Some(750));
        assert!(
            out[1].salience >= 0.4,
            "settle salience {}",
            out[1].salience
        );
    }

    /// A change big enough to trip the FSM but too small to clear `salience_min` fires
    /// no delta — the region transitions internally, silently.
    #[test]
    fn subthreshold_change_is_gated_by_salience() {
        let mut codec = one_by_one();
        codec.observe(Frame {
            t_ms: 0,
            cells: &solid(0),
        });
        // mean abs diff = 26/255 ≈ 0.10 > epsilon (0.03) but salience ≈ 1.5·0.10·0.9 < 0.4
        let out = codec.observe(Frame {
            t_ms: 250,
            cells: &solid(26),
        });
        assert!(out.is_empty(), "should be salience-gated, got {out:?}");
    }

    /// Continuous change collapses to a single RegionChange — the FSM never re-fires
    /// while a region stays in motion (this is the compression, at the codec level).
    #[test]
    fn continuous_change_emits_one_event() {
        let mut codec = one_by_one();
        let mut count = 0;
        for k in 0..50u64 {
            let v = if k % 2 == 0 { 0 } else { 255 }; // changed every frame
            count += codec
                .observe(Frame {
                    t_ms: k * 250,
                    cells: &solid(v),
                })
                .len();
        }
        assert_eq!(count, 1);
    }

    /// Habituation engages across repeated episodes: the novelty of a repetitive
    /// region's later changes is strictly lower than its first — the anti-spam moat.
    #[test]
    fn repeated_episodes_lose_novelty() {
        let mut codec = one_by_one();
        // Prime `last` so the first cycle's value registers as a change.
        codec.observe(Frame {
            t_ms: 0,
            cells: &solid(128),
        });

        let mut changes: Vec<f32> = Vec::new();
        // 12 cycles of [change, quiet, quiet, quiet]; the 3 quiet frames (750ms) clear
        // the 400ms settle window, so each cycle is a clean change→settle→static episode.
        for cycle in 0..12u64 {
            let v = if cycle % 2 == 0 { 255 } else { 0 };
            for step in 0..4u64 {
                let t = 250 + (cycle * 4 + step) * 250;
                for d in codec.observe(Frame {
                    t_ms: t,
                    cells: &solid(v),
                }) {
                    if d.kind == EventKind::RegionChange {
                        changes.push(d.novelty);
                    }
                }
            }
        }
        assert!(
            changes.len() >= 8,
            "need many episodes, got {}",
            changes.len()
        );
        assert!(
            *changes.last().unwrap() < changes[0] - 0.05,
            "novelty did not habituate: first {}, last {}",
            changes[0],
            changes.last().unwrap()
        );
    }

    fn grid_codec(cols: u8, rows: u8, dims: (u32, u32)) -> Codec {
        Codec::new(
            CodecConfig {
                grid: (cols, rows),
                ..Default::default()
            },
            focused_surface(),
            dims,
        )
    }

    #[test]
    fn grid_produces_one_region_per_cell() {
        assert_eq!(grid_codec(2, 2, (200, 200)).region_count(), 4);
        assert_eq!(grid_codec(8, 8, (1280, 720)).region_count(), 64);
    }

    /// In a 2×2 grid, driving only the bottom-right cell emits deltas for that region
    /// alone, with the correct grid coord and bounds — locking the i = r·cols + c
    /// mapping and proving per-region isolation (no cross-region state bleed).
    #[test]
    fn only_the_changing_region_emits_with_correct_ref() {
        const L: usize = crate::diff::CELL_LEN;
        let mut codec = grid_codec(2, 2, (200, 200));
        // 4 cells row-major; index 3 = (col 1, row 1) is the only one that moves.
        let frame = |v3: u8| -> Vec<Cell> { vec![[10; L], [10; L], [10; L], [v3; L]] };
        let script = [(0u64, 10u8), (250, 255), (500, 255), (1000, 255)];
        let mut out = Vec::new();
        for (t, v3) in script {
            out.extend(codec.observe(Frame {
                t_ms: t,
                cells: &frame(v3),
            }));
        }
        assert!(!out.is_empty(), "the moving region should emit");
        assert!(
            out.iter().all(|d| d.region.id == "r_3"),
            "only r_3 should emit, got {:?}",
            out.iter().map(|d| d.region.id.clone()).collect::<Vec<_>>()
        );
        let settle = out
            .iter()
            .find(|d| d.kind == EventKind::StateSettle)
            .unwrap();
        assert_eq!(settle.region.grid, [1, 1]);
        assert_eq!(
            settle.region.bounds,
            Rect {
                x: 100,
                y: 100,
                w: 100,
                h: 100
            }
        );
    }

    /// Two different regions driven independently each keep their own FSM — both emit,
    /// with distinct region ids.
    #[test]
    fn distinct_regions_track_independently() {
        const L: usize = crate::diff::CELL_LEN;
        let mut codec = grid_codec(2, 1, (200, 100));
        let frames: [(u64, [u8; 2]); 4] = [
            (0, [0, 0]),
            (250, [255, 0]),
            (500, [255, 255]),
            (1500, [255, 255]),
        ];
        let mut ids = std::collections::BTreeSet::new();
        for (t, [a, b]) in frames {
            for d in codec.observe(Frame {
                t_ms: t,
                cells: &[[a; L], [b; L]],
            }) {
                ids.insert(d.region.id.clone());
            }
        }
        assert!(
            ids.contains("r_0") && ids.contains("r_1"),
            "both regions should emit: {ids:?}"
        );
    }

    #[test]
    fn dense_simultaneous_changes_coalesce_into_one() {
        const L: usize = crate::diff::CELL_LEN;
        let mut codec = grid_codec(2, 2, (200, 200)); // 4 regions; default coalesce_min = 4
        codec.observe(Frame {
            t_ms: 0,
            cells: &vec![[0; L]; 4],
        });
        let out = codec.observe(Frame {
            t_ms: 250,
            cells: &vec![[255; L]; 4],
        });
        assert_eq!(
            out.len(),
            1,
            "4 simultaneous changes should coalesce, got {out:?}"
        );
        assert_eq!(out[0].kind, EventKind::RegionChange);
        assert_eq!(out[0].region.id, "r_multi");
        assert!(out[0].summary.contains("4 regions"));
        // the macro-delta's bounds cover the union of all four cells (the whole frame)
        assert_eq!(
            out[0].region.bounds,
            Rect {
                x: 0,
                y: 0,
                w: 200,
                h: 200
            }
        );
    }

    #[test]
    fn sparse_changes_stay_separate() {
        const L: usize = crate::diff::CELL_LEN;
        let mut codec = grid_codec(2, 2, (200, 200));
        codec.observe(Frame {
            t_ms: 0,
            cells: &vec![[0; L]; 4],
        });
        // only 2 of 4 regions change -> below the coalesce threshold -> individual
        let out = codec.observe(Frame {
            t_ms: 250,
            cells: &[[255; L], [255; L], [0; L], [0; L]],
        });
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.region.id != "r_multi"));
    }
}
