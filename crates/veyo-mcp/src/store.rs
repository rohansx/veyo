use std::sync::{Arc, Mutex};
use veyo_core::{Delta, TimeMs};

/// Bounded circular buffer of recent deltas shared between the capture loop
/// and the MCP server handler.  The server reads from this; the daemon pushes
/// into it.
#[derive(Clone)]
pub struct EventStore(Arc<Mutex<StoreInner>>);

struct StoreInner {
    buf: Vec<Delta>,
    cap: usize,
    /// Monotonically increasing write-sequence number (not an index).
    seq: u64,
}

impl EventStore {
    pub fn new(cap: usize) -> Self {
        EventStore(Arc::new(Mutex::new(StoreInner {
            buf: Vec::with_capacity(cap),
            cap,
            seq: 0,
        })))
    }

    /// Append a delta; drops oldest when capacity is reached.
    pub fn push(&self, delta: Delta) {
        let mut inner = self.0.lock().unwrap();
        inner.seq += 1;
        if inner.buf.len() < inner.cap {
            inner.buf.push(delta);
        } else {
            let pos = ((inner.seq - 1) % inner.cap as u64) as usize;
            inner.buf[pos] = delta;
        }
    }

    /// All events with `t_event >= since_ms`, up to `limit` (default: all).
    pub fn since(&self, since_ms: TimeMs, limit: Option<usize>) -> Vec<Delta> {
        let inner = self.0.lock().unwrap();
        let mut out: Vec<Delta> = inner
            .buf
            .iter()
            .filter(|d| d.t_event >= since_ms)
            .cloned()
            .collect();
        out.sort_by_key(|d| d.t_event);
        if let Some(n) = limit {
            out.truncate(n);
        }
        out
    }

    /// Most recent N events regardless of time.
    pub fn latest(&self, n: usize) -> Vec<Delta> {
        let inner = self.0.lock().unwrap();
        let skip = inner.buf.len().saturating_sub(n);
        inner.buf[skip..].to_vec()
    }

    pub fn len(&self) -> usize {
        self.0.lock().unwrap().buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().buf.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use veyo_core::{Delta, EventId, EventKind, Evidence, Rect, RegionRef, SurfaceRef, SCHEMA_V};

    fn dummy(t: TimeMs) -> Delta {
        Delta {
            v: SCHEMA_V,
            id: EventId(format!("ev_{t}")),
            t_event: t,
            t_observed: t,
            source: "screen:0".into(),
            kind: EventKind::RegionChange,
            surface: SurfaceRef {
                id: "win_1".into(),
                app: "test".into(),
                title: "t".into(),
                focused: true,
            },
            region: RegionRef {
                id: "r_0".into(),
                grid: [0, 0],
                bounds: Rect {
                    x: 0,
                    y: 0,
                    w: 100,
                    h: 100,
                },
            },
            summary: "test".into(),
            salience: 0.9,
            novelty: 0.9,
            duration_ms: None,
            evidence: Evidence::default(),
        }
    }

    #[test]
    fn empty_store_returns_no_events() {
        let s = EventStore::new(100);
        assert_eq!(s.since(0, None).len(), 0);
    }

    #[test]
    fn since_filters_by_timestamp() {
        let s = EventStore::new(100);
        s.push(dummy(100));
        s.push(dummy(200));
        s.push(dummy(300));
        let out = s.since(200, None);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.t_event >= 200));
    }

    #[test]
    fn limit_caps_result_count() {
        let s = EventStore::new(100);
        for t in 0..10 {
            s.push(dummy(t * 100));
        }
        let out = s.since(0, Some(3));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn capacity_wraps_oldest_first() {
        let s = EventStore::new(3);
        s.push(dummy(100));
        s.push(dummy(200));
        s.push(dummy(300));
        s.push(dummy(400)); // should evict one old entry
        assert_eq!(s.len(), 3);
        let out = s.since(0, None);
        assert!(out.iter().any(|d| d.t_event == 400));
    }

    #[test]
    fn latest_returns_most_recent_n() {
        let s = EventStore::new(100);
        for t in 0..10u64 {
            s.push(dummy(t * 100));
        }
        let latest = s.latest(3);
        assert_eq!(latest.len(), 3);
    }
}
