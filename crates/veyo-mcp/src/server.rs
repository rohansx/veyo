use crate::store::EventStore as MemStore;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::Deserialize;

#[cfg(feature = "persist")]
use veyo_store::{EventStore as SqlStore, QueryParams};

// ---------------------------------------------------------------------------
// Tool parameter types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetEventsParams {
    /// Only return events with t_event >= this value (epoch-ms). Default: 0.
    since_ms: Option<u64>,
    /// Cap the result count. Default: all matching events.
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetLatestParams {
    /// Number of most-recent events to return. Default: 20.
    count: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct QueryEventsParams {
    /// Only events with t_event >= this value (epoch-ms).
    since_ms: Option<u64>,
    /// Only events with t_event <= this value (epoch-ms).
    until_ms: Option<u64>,
    /// Filter by event kind: "RegionChange" or "StateSettle".
    kind: Option<String>,
    /// Filter by surface id (e.g. "win_42").
    surface_id: Option<String>,
    /// Max events to return (default 100).
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchEventsParams {
    /// FTS5 query string. Supports prefix queries (foo*), phrase queries ("foo bar"),
    /// and boolean operators (foo NOT bar).
    query: String,
    /// Max events to return (default 20).
    limit: Option<usize>,
}

// ---------------------------------------------------------------------------
// MCP handler
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct VeyoHandler {
    mem: MemStore,
    #[cfg(feature = "persist")]
    sql: Option<SqlStore>,
    #[allow(dead_code)]
    tool_router: ToolRouter<VeyoHandler>,
}

#[tool_router]
impl VeyoHandler {
    #[cfg(not(feature = "persist"))]
    pub fn new(mem: MemStore) -> Self {
        Self {
            mem,
            tool_router: Self::tool_router(),
        }
    }

    #[cfg(feature = "persist")]
    pub fn new(mem: MemStore, sql: Option<SqlStore>) -> Self {
        Self {
            mem,
            sql,
            tool_router: Self::tool_router(),
        }
    }

    /// Return screen-change deltas with t_event >= since_ms (epoch-ms).
    /// Deltas are semantic text, never raw pixels. Poll this with your last
    /// known timestamp to receive only new events.
    #[tool(description = "Return veyo screen-change events since a given epoch-ms timestamp.")]
    fn get_events(
        &self,
        Parameters(GetEventsParams { since_ms, limit }): Parameters<GetEventsParams>,
    ) -> String {
        let events = self.mem.since(since_ms.unwrap_or(0), limit);
        serde_json::to_string_pretty(&events).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// Return the N most recent screen-change deltas (from the in-memory buffer).
    #[tool(description = "Return the N most recent veyo screen-change events.")]
    fn get_latest_events(
        &self,
        Parameters(GetLatestParams { count }): Parameters<GetLatestParams>,
    ) -> String {
        let events = self.mem.latest(count.unwrap_or(20));
        serde_json::to_string_pretty(&events).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// Query historical events from the persistent SQLite store.
    /// Requires veyod to be started with --store-path. Falls back to the
    /// in-memory buffer when no store is attached.
    #[tool(
        description = "Query historical veyo events by time range, kind, or surface. Requires --store-path."
    )]
    fn query_events(
        &self,
        Parameters(QueryEventsParams {
            since_ms,
            until_ms,
            kind,
            surface_id,
            limit,
        }): Parameters<QueryEventsParams>,
    ) -> String {
        #[cfg(feature = "persist")]
        if let Some(ref sql) = self.sql {
            let params = QueryParams {
                since: since_ms,
                until: until_ms,
                kind,
                surface_id,
                limit: limit.or(Some(100)),
            };
            return match sql.query(&params) {
                Ok(events) => serde_json::to_string_pretty(&events)
                    .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")),
                Err(e) => format!("{{\"error\":\"{e}\"}}"),
            };
        }
        // Fallback: in-memory buffer, no time-range or kind filter.
        let events = self.mem.since(since_ms.unwrap_or(0), limit.or(Some(100)));
        serde_json::to_string_pretty(&events).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// Full-text search across event summaries, app names, and window titles.
    /// Requires veyod started with --store-path. Returns events matching the query
    /// ordered by timestamp ascending.
    #[tool(
        description = "Full-text search veyo events by summary, app name, or window title. Requires --store-path."
    )]
    fn search_events(
        &self,
        Parameters(SearchEventsParams { query, limit }): Parameters<SearchEventsParams>,
    ) -> String {
        #[cfg(feature = "persist")]
        if let Some(ref sql) = self.sql {
            return match sql.search(&query, limit.unwrap_or(20)) {
                Ok(events) => serde_json::to_string_pretty(&events)
                    .unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")),
                Err(e) => format!("{{\"error\":\"{e}\"}}"),
            };
        }
        "{\"error\":\"search_events requires --store-path\"}".into()
    }

    /// Return a one-liner health snapshot: buffer size, store count, and the most
    /// recent event summary. Call this first to orient yourself before querying.
    #[tool(description = "Return a concise health snapshot of the veyo event buffer and store.")]
    fn get_state(&self) -> String {
        let buf_len = self.mem.len();
        let latest = self.mem.latest(1);
        let last_summary = latest
            .first()
            .map(|d| d.summary.as_str())
            .unwrap_or("(none)");
        let last_t = latest
            .first()
            .map(|d| d.t_event.to_string())
            .unwrap_or_else(|| "—".into());

        #[cfg(feature = "persist")]
        {
            let store_count = self
                .sql
                .as_ref()
                .and_then(|s| s.count().ok())
                .map(|n| n.to_string())
                .unwrap_or_else(|| "—".into());
            return serde_json::json!({
                "buffer_events": buf_len,
                "store_events": store_count,
                "last_t_ms": last_t,
                "last_summary": last_summary,
            })
            .to_string();
        }

        #[cfg(not(feature = "persist"))]
        serde_json::json!({
            "buffer_events": buf_len,
            "store_events": "—",
            "last_t_ms": last_t,
            "last_summary": last_summary,
        })
        .to_string()
    }
}

#[tool_handler]
impl ServerHandler for VeyoHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}

// ---------------------------------------------------------------------------
// Public entry-point
// ---------------------------------------------------------------------------

pub struct VeyoMcpServer {
    mem: MemStore,
    #[cfg(feature = "persist")]
    sql: Option<SqlStore>,
}

impl VeyoMcpServer {
    #[cfg(not(feature = "persist"))]
    pub fn new(mem: MemStore) -> Self {
        Self { mem }
    }

    #[cfg(feature = "persist")]
    pub fn new(mem: MemStore, sql: Option<SqlStore>) -> Self {
        Self { mem, sql }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        #[cfg(feature = "persist")]
        let handler = VeyoHandler::new(self.mem, self.sql);
        #[cfg(not(feature = "persist"))]
        let handler = VeyoHandler::new(self.mem);
        let service = handler.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }
}
