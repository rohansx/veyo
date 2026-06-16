use crate::store::EventStore;
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};
use serde::Deserialize;

// ---------------------------------------------------------------------------
// Tool parameter types (JSON Schema derived for rmcp)
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

// ---------------------------------------------------------------------------
// MCP handler
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct VeyoHandler {
    store: EventStore,
    #[allow(dead_code)]
    tool_router: ToolRouter<VeyoHandler>,
}

#[tool_router]
impl VeyoHandler {
    pub fn new(store: EventStore) -> Self {
        Self {
            store,
            tool_router: Self::tool_router(),
        }
    }

    /// Return screen-change deltas with t_event >= since_ms (epoch-ms).
    ///
    /// Deltas are semantic descriptions of screen activity — never raw pixels.
    /// Filter by `since_ms` to poll only new events since your last call.
    #[tool(
        description = "Return veyo screen-change events since a given epoch-ms timestamp. Deltas are semantic text, not images."
    )]
    fn get_events(
        &self,
        Parameters(GetEventsParams { since_ms, limit }): Parameters<GetEventsParams>,
    ) -> String {
        let events = self.store.since(since_ms.unwrap_or(0), limit);
        serde_json::to_string_pretty(&events).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }

    /// Return the N most recent screen-change deltas regardless of time.
    #[tool(description = "Return the N most recent veyo screen-change events.")]
    fn get_latest_events(
        &self,
        Parameters(GetLatestParams { count }): Parameters<GetLatestParams>,
    ) -> String {
        let events = self.store.latest(count.unwrap_or(20));
        serde_json::to_string_pretty(&events).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
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
    store: EventStore,
}

impl VeyoMcpServer {
    pub fn new(store: EventStore) -> Self {
        Self { store }
    }

    /// Run the MCP server over stdio until the transport closes.
    pub async fn run(self) -> anyhow::Result<()> {
        let handler = VeyoHandler::new(self.store);
        let service = handler.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }
}
