//! The per-region debounce FSM: `Static → Changing → Settling → Static`.
//!
//! Turns a burst of per-frame change observations into exactly two events — a
//! `RegionChange` when a static region starts moving, and a `StateSettle` when it
//! holds still again past `settle_window_ms`. That collapse (60 frames → 2 events) is
//! the codec's headline compression. Time is logical (`t_ms`), so it's deterministic.

use serde::{Deserialize, Serialize};

use crate::schema::TimeMs;

/// FSM state for one region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Static,
    Changing,
    Settling,
}

/// What a single [`RegionFsm::observe`] step decided to emit, if anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsmEvent {
    /// A static region started changing.
    RegionChange,
    /// A changing region went static and held; `duration_ms` is how long it changed.
    StateSettle { duration_ms: u32 },
}

/// Per-region debounce state machine.
#[derive(Debug, Clone)]
pub struct RegionFsm {
    pub phase: Phase,
    changing_since: Option<TimeMs>,
    settle_deadline: Option<TimeMs>,
}

impl Default for RegionFsm {
    fn default() -> Self {
        Self {
            phase: Phase::Static,
            changing_since: None,
            settle_deadline: None,
        }
    }
}

impl RegionFsm {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance one frame. `changed` is `diff > epsilon_noise`, computed upstream.
    ///
    /// Returns the FSM-level event this frame produced, if any. Emission is still
    /// subject to the salience gate downstream — the FSM only decides *what kind* of
    /// transition happened, never *whether the agent should care*.
    pub fn observe(
        &mut self,
        changed: bool,
        t_ms: TimeMs,
        settle_window_ms: u64,
    ) -> Option<FsmEvent> {
        match self.phase {
            Phase::Static => {
                if changed {
                    self.phase = Phase::Changing;
                    self.changing_since = Some(t_ms);
                    Some(FsmEvent::RegionChange)
                } else {
                    None
                }
            }
            Phase::Changing => {
                if changed {
                    None // still changing — hold
                } else {
                    self.phase = Phase::Settling;
                    self.settle_deadline = Some(t_ms.saturating_add(settle_window_ms));
                    None
                }
            }
            Phase::Settling => {
                if changed {
                    // New change before the window elapsed — cancel the settle.
                    self.phase = Phase::Changing;
                    self.settle_deadline = None;
                    None
                } else if t_ms >= self.settle_deadline.unwrap_or(u64::MAX) {
                    let start = self.changing_since.unwrap_or(t_ms);
                    // Saturate rather than wrap: a multi-week episode shouldn't alias.
                    let duration_ms = t_ms.saturating_sub(start).min(u32::MAX as u64) as u32;
                    self.phase = Phase::Static;
                    self.changing_since = None;
                    self.settle_deadline = None;
                    Some(FsmEvent::StateSettle { duration_ms })
                } else {
                    None // still within the settle window — wait
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u64 = 400; // settle_window_ms used across the table tests

    #[test]
    fn static_plus_change_goes_changing_and_emits_region_change() {
        let mut fsm = RegionFsm::new();
        let ev = fsm.observe(true, 0, W);
        assert_eq!(fsm.phase, Phase::Changing);
        assert_eq!(ev, Some(FsmEvent::RegionChange));
    }

    #[test]
    fn static_without_change_stays_static_silent() {
        let mut fsm = RegionFsm::new();
        let ev = fsm.observe(false, 0, W);
        assert_eq!(fsm.phase, Phase::Static);
        assert_eq!(ev, None);
    }

    #[test]
    fn changing_while_still_changing_stays_changing_silent() {
        let mut fsm = RegionFsm::new();
        fsm.observe(true, 0, W); // -> Changing
        let ev = fsm.observe(true, 250, W);
        assert_eq!(fsm.phase, Phase::Changing);
        assert_eq!(ev, None);
    }

    #[test]
    fn changing_to_quiet_arms_settling_silent() {
        let mut fsm = RegionFsm::new();
        fsm.observe(true, 0, W); // -> Changing
        let ev = fsm.observe(false, 250, W);
        assert_eq!(fsm.phase, Phase::Settling);
        assert_eq!(ev, None);
    }

    #[test]
    fn settling_before_window_elapses_stays_silent() {
        let mut fsm = RegionFsm::new();
        fsm.observe(true, 0, W); // Changing
        fsm.observe(false, 250, W); // Settling, deadline = 650
        let ev = fsm.observe(false, 500, W); // 500 < 650
        assert_eq!(fsm.phase, Phase::Settling);
        assert_eq!(ev, None);
    }

    #[test]
    fn settling_after_window_emits_state_settle_with_duration() {
        let mut fsm = RegionFsm::new();
        fsm.observe(true, 0, W); // Changing, changing_since = 0
        fsm.observe(false, 250, W); // Settling, deadline = 650
        let ev = fsm.observe(false, 700, W); // 700 >= 650 -> settle
        assert_eq!(fsm.phase, Phase::Static);
        assert_eq!(ev, Some(FsmEvent::StateSettle { duration_ms: 700 }));
    }

    #[test]
    fn new_change_during_settling_cancels_back_to_changing() {
        let mut fsm = RegionFsm::new();
        fsm.observe(true, 0, W); // Changing
        fsm.observe(false, 250, W); // Settling, deadline = 650
        let ev = fsm.observe(true, 500, W); // change before deadline
        assert_eq!(fsm.phase, Phase::Changing);
        assert_eq!(ev, None);
    }

    /// The headline compression: a ~1.25s changing burst collapses to exactly two
    /// events (one RegionChange + one StateSettle), not one per frame.
    #[test]
    fn changing_burst_collapses_to_two_events() {
        let mut fsm = RegionFsm::new();
        let frames = [
            (0u64, true),
            (250, true),
            (500, true),
            (750, false),
            (1000, false),
            (1250, false),
        ];
        let events: Vec<FsmEvent> = frames
            .iter()
            .filter_map(|&(t, c)| fsm.observe(c, t, W))
            .collect();
        assert_eq!(
            events,
            vec![
                FsmEvent::RegionChange,
                FsmEvent::StateSettle { duration_ms: 1250 }
            ]
        );
        assert_eq!(fsm.phase, Phase::Static);
    }

    /// A pause-and-resume mid-burst is still one episode: changing_since persists
    /// across the cancel, so duration spans the whole episode.
    #[test]
    fn paused_burst_is_one_episode() {
        let mut fsm = RegionFsm::new();
        let frames = [
            (0u64, true),  // RegionChange, changing_since = 0
            (250, false),  // Settling, deadline 650
            (500, true),   // cancel -> Changing
            (750, false),  // Settling, deadline 1150
            (1000, false), // < 1150
            (1250, false), // settle, duration = 1250 - 0
        ];
        let events: Vec<FsmEvent> = frames
            .iter()
            .filter_map(|&(t, c)| fsm.observe(c, t, W))
            .collect();
        assert_eq!(
            events,
            vec![
                FsmEvent::RegionChange,
                FsmEvent::StateSettle { duration_ms: 1250 }
            ]
        );
    }
}
