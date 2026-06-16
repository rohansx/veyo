//! Scoring: match emitted deltas against annotations, compute recall / precision /
//! emission. **Recall is the objective, emission-rate the constraint** — false
//! negatives hurt an agent more than false positives.

use crate::session::Annotation;
use veyo_core::Delta;

/// The score of one replay against one session's ground truth.
#[derive(Debug, Clone, PartialEq)]
pub struct Scored {
    pub annotated: usize,
    pub emitted: usize,
    pub matched: usize,
    pub frames: usize,
    pub duration_ms: u64,
    /// matched ÷ annotated — the number that matters most.
    pub recall: f32,
    /// matched ÷ emitted.
    pub precision: f32,
    /// emitted ÷ frames (target < ~0.01).
    pub emission_rate: f32,
    pub events_per_hour: f32,
}

impl Scored {
    /// The Phase-0 gate: recall ≥ 0.9 at emission < 1% of frames.
    ///
    /// A session with no annotations or no frames is **unscoreable**, not a pass —
    /// recall is only vacuously 1.0 there, so the gate requires real ground truth.
    pub fn passes_gate(&self) -> bool {
        self.annotated > 0 && self.frames > 0 && self.recall >= 0.9 && self.emission_rate < 0.01
    }
}

/// Match annotations to emitted deltas within `match_tolerance_ms` (by `t_observed`) using an
/// **optimal** max-cardinality bipartite matching, then compute the metrics.
///
/// Greedy (nearest-first) matching can strand an annotation when an emission falls
/// between clustered annotations, understating recall — the gate metric — so we use
/// augmenting paths (Kuhn's algorithm) to maximize the matched count. The data is
/// sparse, so the simple O(V·E) form is more than fast enough.
pub fn score(
    deltas: &[Delta],
    annotations: &[Annotation],
    frames: usize,
    duration_ms: u64,
    match_tolerance_ms: u64,
) -> Scored {
    // Bipartite edges: annotation -> deltas within tolerance.
    let adjacency: Vec<Vec<usize>> = annotations
        .iter()
        .map(|a| {
            deltas
                .iter()
                .enumerate()
                .filter(|(_, d)| a.t_ms.abs_diff(d.t_observed) <= match_tolerance_ms)
                .map(|(i, _)| i)
                .collect()
        })
        .collect();
    let matched = max_cardinality_matching(&adjacency, deltas.len());

    let annotated = annotations.len();
    let emitted = deltas.len();
    let recall = if annotated == 0 {
        1.0
    } else {
        matched as f32 / annotated as f32
    };
    let precision = if emitted == 0 {
        1.0
    } else {
        matched as f32 / emitted as f32
    };
    let emission_rate = if frames == 0 {
        0.0
    } else {
        emitted as f32 / frames as f32
    };
    let events_per_hour = if duration_ms == 0 {
        0.0
    } else {
        emitted as f32 / (duration_ms as f32 / 3_600_000.0)
    };

    Scored {
        annotated,
        emitted,
        matched,
        frames,
        duration_ms,
        recall,
        precision,
        emission_rate,
        events_per_hour,
    }
}

/// Maximum-cardinality bipartite matching via Kuhn's augmenting paths.
/// `adjacency[a]` lists the right-side (delta) indices annotation `a` may match.
fn max_cardinality_matching(adjacency: &[Vec<usize>], right_len: usize) -> usize {
    let mut match_right = vec![usize::MAX; right_len]; // delta index -> annotation index
    let mut matched = 0;
    for left in 0..adjacency.len() {
        let mut seen = vec![false; right_len];
        if augment(left, adjacency, &mut seen, &mut match_right) {
            matched += 1;
        }
    }
    matched
}

fn augment(
    left: usize,
    adjacency: &[Vec<usize>],
    seen: &mut [bool],
    match_right: &mut [usize],
) -> bool {
    for &right in &adjacency[left] {
        if !seen[right] {
            seen[right] = true;
            if match_right[right] == usize::MAX
                || augment(match_right[right], adjacency, seen, match_right)
            {
                match_right[right] = left;
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use veyo_core::{Delta, EventId, EventKind, Evidence, Rect, RegionRef, SurfaceRef, SCHEMA_V};

    fn delta_at(t: u64, kind: EventKind) -> Delta {
        Delta {
            v: SCHEMA_V,
            id: EventId(format!("ev_{t}")),
            t_event: t,
            t_observed: t,
            source: "screen:0".into(),
            kind,
            surface: SurfaceRef {
                id: "s".into(),
                app: "a".into(),
                title: "t".into(),
                focused: true,
            },
            region: RegionRef {
                id: "r_0".into(),
                grid: [0, 0],
                bounds: Rect {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                },
            },
            summary: "x".into(),
            salience: 1.0,
            novelty: 1.0,
            duration_ms: None,
            evidence: Evidence::default(),
        }
    }

    fn anno_at(t: u64) -> Annotation {
        Annotation {
            t_ms: t,
            kind: "k".into(),
            surface: "screen:0".into(),
            note: "n".into(),
        }
    }

    fn approx(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "expected ~{b}, got {a}");
    }

    #[test]
    fn perfect_recall_and_precision_when_every_annotation_has_a_close_delta() {
        let deltas = [
            delta_at(1000, EventKind::StateSettle),
            delta_at(2000, EventKind::StateSettle),
        ];
        let anns = [anno_at(1010), anno_at(1990)];
        let s = score(&deltas, &anns, 100, 25_000, 100);
        approx(s.recall, 1.0);
        approx(s.precision, 1.0);
        assert_eq!(s.matched, 2);
    }

    #[test]
    fn a_delta_outside_tolerance_does_not_match() {
        let deltas = [delta_at(1000, EventKind::RegionChange)];
        let anns = [anno_at(1500)]; // 500ms away, tol 100
        let s = score(&deltas, &anns, 10, 2000, 100);
        approx(s.recall, 0.0);
        assert_eq!(s.matched, 0);
    }

    #[test]
    fn missed_annotation_lowers_recall() {
        // one annotation matched, one missed -> recall 0.5
        let deltas = [delta_at(1000, EventKind::StateSettle)];
        let anns = [anno_at(1000), anno_at(9000)];
        let s = score(&deltas, &anns, 100, 10_000, 200);
        approx(s.recall, 0.5);
    }

    #[test]
    fn spurious_emissions_lower_precision_not_recall() {
        // 1 annotation, 4 emissions (1 matches) -> recall 1.0, precision 0.25
        let deltas = [
            delta_at(1000, EventKind::StateSettle),
            delta_at(3000, EventKind::RegionChange),
            delta_at(5000, EventKind::RegionChange),
            delta_at(7000, EventKind::RegionChange),
        ];
        let anns = [anno_at(1000)];
        let s = score(&deltas, &anns, 100, 8000, 100);
        approx(s.recall, 1.0);
        approx(s.precision, 0.25);
        approx(s.emission_rate, 0.04);
    }

    #[test]
    fn one_delta_cannot_match_two_annotations() {
        // a single delta near two annotations matches only one -> recall 0.5
        let deltas = [delta_at(1000, EventKind::StateSettle)];
        let anns = [anno_at(990), anno_at(1010)];
        let s = score(&deltas, &anns, 10, 2000, 100);
        approx(s.recall, 0.5);
        assert_eq!(s.matched, 1);
    }

    #[test]
    fn no_annotations_is_vacuously_perfect() {
        let s = score(&[], &[], 100, 1000, 100);
        approx(s.recall, 1.0);
        approx(s.precision, 1.0);
    }

    #[test]
    fn events_per_hour_scales_with_duration() {
        let deltas = [delta_at(1000, EventKind::RegionChange)];
        // 1 event over 1000ms -> 3600 events/hour
        let s = score(&deltas, &[], 10, 1000, 100);
        approx(s.events_per_hour, 3600.0);
    }

    #[test]
    fn optimal_matching_beats_greedy_on_clustered_annotations() {
        // annotations [18,24,27], deltas [13,20], tol 6.
        // greedy takes a18->d20 (nearer), stranding a24 -> matched 1, recall 1/3.
        // optimal assigns a18->d13, a24->d20 -> matched 2, recall 2/3.
        let deltas = [
            delta_at(13, EventKind::RegionChange),
            delta_at(20, EventKind::RegionChange),
        ];
        let anns = [anno_at(18), anno_at(24), anno_at(27)];
        let s = score(&deltas, &anns, 100, 30_000, 6);
        assert_eq!(s.matched, 2, "optimal matching should pair both deltas");
        approx(s.recall, 2.0 / 3.0);
    }

    #[test]
    fn an_empty_session_does_not_pass_the_gate() {
        // recall is vacuously 1.0 but an unscoreable session must not read as PASS.
        let s = score(&[], &[], 0, 0, 100);
        assert!(!s.passes_gate());
    }
}
