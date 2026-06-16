//! `veyo-core` — the pure heart of the visual event codec.
//!
//! This crate performs **no I/O**. It takes per-region change observations in and
//! emits typed [`Delta`](schema::Delta) events out, which is what lets the offline
//! [eval harness] drive the entire decision pipeline with no capture backend and no
//! daemon — and what makes every transition unit-testable.
//!
//! The pipeline, cheap → expensive (see the policy-engine doc):
//!
//! 1. **Gate 1 — cheap diff** (upstream): a region either `changed` or not, relative
//!    to `epsilon_noise`. `veyo-core` consumes that boolean; it does not hash pixels.
//! 2. **Debounce FSM** ([`fsm`]): `Static → Changing → Settling → Static`, turning a
//!    burst of frames into a `RegionChange` + a `StateSettle`.
//! 3. **Gate 2 — salience** ([`salience`]): `w_focus · magnitude · novelty`, gated by
//!    `salience_min`.
//! 4. **Habituation** ([`habituation`]): repetitive change decays novelty toward 0.
//!
//! [eval harness]: ../../../docs/eval-harness.md

pub mod diff;
pub mod engine;
pub mod fsm;
pub mod habituation;
pub mod salience;
pub mod schema;

pub use diff::{Cell, CELL_LEN, CELL_SIDE};
pub use engine::{Codec, CodecConfig, Frame};
pub use fsm::{FsmEvent, Phase, RegionFsm};
pub use habituation::NoveltyBaseline;
pub use salience::{focus_multiplier, salience, should_emit};
pub use schema::{
    Delta, EventId, EventKind, Evidence, Rect, RegionRef, SurfaceRef, TimeMs, SCHEMA_V,
};
