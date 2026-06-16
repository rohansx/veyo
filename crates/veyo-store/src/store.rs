use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use rusqlite::{params, Connection};
use veyo_core::{Delta, TimeMs};

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
    id            TEXT    PRIMARY KEY,
    t_event       INTEGER NOT NULL,
    t_observed    INTEGER NOT NULL,
    source        TEXT    NOT NULL,
    kind          TEXT    NOT NULL,
    surface_id    TEXT    NOT NULL,
    surface_app   TEXT    NOT NULL,
    surface_title TEXT    NOT NULL,
    surface_focused INTEGER NOT NULL,
    region_id     TEXT    NOT NULL,
    summary       TEXT    NOT NULL,
    salience      REAL    NOT NULL,
    novelty       REAL    NOT NULL,
    duration_ms   INTEGER,
    payload       TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_t_event  ON events(t_event);
CREATE INDEX IF NOT EXISTS idx_kind     ON events(kind);
CREATE INDEX IF NOT EXISTS idx_surface  ON events(surface_id);
CREATE INDEX IF NOT EXISTS idx_summary_fts ON events(summary);
";

// ---------------------------------------------------------------------------
// Query params
// ---------------------------------------------------------------------------

/// Filters for [`EventStore::query`].
#[derive(Debug, Default, Clone)]
pub struct QueryParams {
    /// Only events with `t_event >= since`.
    pub since: Option<TimeMs>,
    /// Only events with `t_event <= until`.
    pub until: Option<TimeMs>,
    /// Filter to one event kind by name (e.g. `"state_settle"`).
    pub kind: Option<String>,
    /// Filter to one surface by id.
    pub surface_id: Option<String>,
    /// Cap the result count (default: 200).
    pub limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// EventStore
// ---------------------------------------------------------------------------

/// Thread-safe SQLite-backed append-only event store.
///
/// Use `:memory:` as the path for in-process testing.
#[derive(Clone)]
pub struct EventStore(Arc<Mutex<Connection>>);

impl EventStore {
    /// Open (or create) the store at `path`. Use `":memory:"` for tests.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening SQLite store at {}", path.display()))?;
        conn.execute_batch(SCHEMA)
            .context("applying veyo-store schema")?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .context("setting WAL mode")?;
        tracing::info!(path = %path.display(), "store opened");
        Ok(Self(Arc::new(Mutex::new(conn))))
    }

    /// In-memory store (tests / demo mode).
    pub fn in_memory() -> anyhow::Result<Self> {
        Self::open(Path::new(":memory:"))
    }

    /// Append one delta.  Ignores duplicates (same `id`).
    pub fn insert(&self, delta: &Delta) -> anyhow::Result<()> {
        let payload =
            serde_json::to_string(delta).context("serializing delta to JSON for store")?;
        let conn = self.0.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO events
             (id, t_event, t_observed, source, kind,
              surface_id, surface_app, surface_title, surface_focused,
              region_id, summary, salience, novelty, duration_ms, payload)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            params![
                delta.id.0,
                delta.t_event as i64,
                delta.t_observed as i64,
                delta.source,
                format!("{:?}", delta.kind),
                delta.surface.id,
                delta.surface.app,
                delta.surface.title,
                delta.surface.focused as i64,
                delta.region.id,
                delta.summary,
                delta.salience as f64,
                delta.novelty as f64,
                delta.duration_ms,
                payload,
            ],
        )
        .context("inserting delta")?;
        Ok(())
    }

    /// Query events with optional filters. Results ordered by `t_event ASC`.
    pub fn query(&self, p: &QueryParams) -> anyhow::Result<Vec<Delta>> {
        let conn = self.0.lock().unwrap();
        let mut sql = String::from("SELECT payload FROM events WHERE 1=1");
        // Build params alongside SQL so positional indices always match.
        let mut bind: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(v) = p.since {
            bind.push(Box::new(v as i64));
            sql.push_str(&format!(" AND t_event >= ?{}", bind.len()));
        }
        if let Some(v) = p.until {
            bind.push(Box::new(v as i64));
            sql.push_str(&format!(" AND t_event <= ?{}", bind.len()));
        }
        if let Some(ref k) = p.kind {
            bind.push(Box::new(k.clone()));
            sql.push_str(&format!(" AND kind = ?{}", bind.len()));
        }
        if let Some(ref sid) = p.surface_id {
            bind.push(Box::new(sid.clone()));
            sql.push_str(&format!(" AND surface_id = ?{}", bind.len()));
        }
        let limit = p.limit.unwrap_or(200);
        sql.push_str(&format!(" ORDER BY t_event ASC LIMIT {limit}"));

        let refs: Vec<&dyn rusqlite::ToSql> = bind.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).context("preparing query")?;
        let rows = stmt
            .query_map(refs.as_slice(), |row| row.get::<_, String>(0))
            .context("executing query")?;

        let mut out = Vec::new();
        for row in rows {
            let json = row.context("reading row")?;
            let delta: Delta =
                serde_json::from_str(&json).context("deserializing delta from store")?;
            out.push(delta);
        }
        Ok(out)
    }

    /// Most recent N events, ordered newest-first.
    pub fn latest(&self, n: usize) -> anyhow::Result<Vec<Delta>> {
        let conn = self.0.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT payload FROM events ORDER BY t_event DESC LIMIT ?1")
            .context("preparing latest query")?;
        let rows = stmt
            .query_map(params![n as i64], |row| row.get::<_, String>(0))
            .context("executing latest query")?;
        let mut out = Vec::new();
        for row in rows {
            let json = row.context("reading row")?;
            let delta: Delta = serde_json::from_str(&json).context("deserializing")?;
            out.push(delta);
        }
        Ok(out)
    }

    /// Count of stored events.
    pub fn count(&self) -> anyhow::Result<u64> {
        let conn = self.0.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .context("counting events")?;
        Ok(n as u64)
    }
}

// ---------------------------------------------------------------------------
// Tests (all in-memory)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use veyo_core::{Delta, EventId, EventKind, Evidence, Rect, RegionRef, SurfaceRef, SCHEMA_V};

    fn dummy(id: &str, t: TimeMs, kind: EventKind) -> Delta {
        Delta {
            v: SCHEMA_V,
            id: EventId(id.into()),
            t_event: t,
            t_observed: t,
            source: "screen:0".into(),
            kind,
            surface: SurfaceRef {
                id: "win_1".into(),
                app: "firefox".into(),
                title: "GitHub".into(),
                focused: true,
            },
            region: RegionRef {
                id: "r_0".into(),
                grid: [0, 0],
                bounds: Rect {
                    x: 0,
                    y: 0,
                    w: 640,
                    h: 400,
                },
            },
            summary: format!("region r_0 event at t={t}"),
            salience: 0.8,
            novelty: 0.9,
            duration_ms: None,
            evidence: Evidence::default(),
        }
    }

    fn store() -> EventStore {
        EventStore::in_memory().unwrap()
    }

    #[test]
    fn empty_store_has_zero_count() {
        assert_eq!(store().count().unwrap(), 0);
    }

    #[test]
    fn insert_and_count() {
        let s = store();
        s.insert(&dummy("ev_1", 1000, EventKind::RegionChange))
            .unwrap();
        s.insert(&dummy("ev_2", 2000, EventKind::StateSettle))
            .unwrap();
        assert_eq!(s.count().unwrap(), 2);
    }

    #[test]
    fn duplicate_insert_is_ignored() {
        let s = store();
        s.insert(&dummy("ev_1", 1000, EventKind::RegionChange))
            .unwrap();
        s.insert(&dummy("ev_1", 1000, EventKind::RegionChange))
            .unwrap();
        assert_eq!(s.count().unwrap(), 1);
    }

    #[test]
    fn query_since_filters_by_timestamp() {
        let s = store();
        for (id, t) in [("ev_1", 1000u64), ("ev_2", 2000), ("ev_3", 3000)] {
            s.insert(&dummy(id, t, EventKind::RegionChange)).unwrap();
        }
        let out = s
            .query(&QueryParams {
                since: Some(2000),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.t_event >= 2000));
    }

    #[test]
    fn query_until_filters_by_timestamp() {
        let s = store();
        for (id, t) in [("ev_1", 1000u64), ("ev_2", 2000), ("ev_3", 3000)] {
            s.insert(&dummy(id, t, EventKind::RegionChange)).unwrap();
        }
        let out = s
            .query(&QueryParams {
                until: Some(2000),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.t_event <= 2000));
    }

    #[test]
    fn query_since_and_until_is_inclusive_range() {
        let s = store();
        for (id, t) in [("ev_1", 1000u64), ("ev_2", 2000), ("ev_3", 3000)] {
            s.insert(&dummy(id, t, EventKind::RegionChange)).unwrap();
        }
        let out = s
            .query(&QueryParams {
                since: Some(1000),
                until: Some(2000),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn query_kind_filter() {
        let s = store();
        s.insert(&dummy("ev_1", 1000, EventKind::RegionChange))
            .unwrap();
        s.insert(&dummy("ev_2", 2000, EventKind::StateSettle))
            .unwrap();
        s.insert(&dummy("ev_3", 3000, EventKind::StateSettle))
            .unwrap();
        let out = s
            .query(&QueryParams {
                kind: Some("StateSettle".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|d| d.kind == EventKind::StateSettle));
    }

    #[test]
    fn query_results_are_ordered_by_t_event_asc() {
        let s = store();
        for (id, t) in [("ev_3", 3000u64), ("ev_1", 1000), ("ev_2", 2000)] {
            s.insert(&dummy(id, t, EventKind::RegionChange)).unwrap();
        }
        let out = s.query(&QueryParams::default()).unwrap();
        let times: Vec<u64> = out.iter().map(|d| d.t_event).collect();
        assert_eq!(times, vec![1000, 2000, 3000]);
    }

    #[test]
    fn query_limit_caps_results() {
        let s = store();
        for i in 0..10u64 {
            s.insert(&dummy(
                &format!("ev_{i}"),
                i * 1000,
                EventKind::RegionChange,
            ))
            .unwrap();
        }
        let out = s
            .query(&QueryParams {
                limit: Some(3),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn latest_returns_newest_n() {
        let s = store();
        for i in 0..5u64 {
            s.insert(&dummy(
                &format!("ev_{i}"),
                i * 1000,
                EventKind::RegionChange,
            ))
            .unwrap();
        }
        let out = s.latest(2).unwrap();
        assert_eq!(out.len(), 2);
        // latest() orders newest-first
        assert!(out[0].t_event > out[1].t_event);
    }

    #[test]
    fn delta_round_trips_through_json_payload() {
        let s = store();
        let original = dummy("ev_rt", 9999, EventKind::StateSettle);
        s.insert(&original).unwrap();
        let out = s.query(&QueryParams::default()).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id.0, "ev_rt");
        assert_eq!(out[0].t_event, 9999);
        assert_eq!(out[0].kind, EventKind::StateSettle);
        assert!((out[0].salience - 0.8).abs() < 1e-4);
    }

    #[test]
    fn evidence_is_not_in_payload_json() {
        let s = store();
        s.insert(&dummy("ev_priv", 1000, EventKind::RegionChange))
            .unwrap();
        let conn = s.0.lock().unwrap();
        let payload: String = conn
            .query_row("SELECT payload FROM events WHERE id='ev_priv'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            !payload.contains("phash"),
            "evidence must not reach the store"
        );
        assert!(
            !payload.contains("thumb_ref"),
            "evidence must not reach the store"
        );
    }
}
