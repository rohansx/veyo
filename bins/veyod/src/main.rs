use std::time::{Duration, Instant};
use veyo_capture::{rgba_to_cells, CaptureBackend, PollBackend};
use veyo_core::{Codec, CodecConfig, Frame as CoreFrame, SurfaceRef};
use veyo_mcp::{EventStore, VeyoMcpServer};

const CAPTURE_FPS: u64 = 4;
const STORE_CAP: usize = 2000;

fn main_sync() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("veyod=info".parse().unwrap()),
        )
        .init();

    let store = EventStore::new(STORE_CAP);
    let store_thread = store.clone();

    std::thread::Builder::new()
        .name("capture".into())
        .spawn(move || {
            if let Err(e) = run_capture(store_thread) {
                tracing::error!("capture thread: {e:#}");
            }
        })?;

    // Drive the MCP server on the main thread (async runtime).
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async { VeyoMcpServer::new(store).run().await })
}

fn main() {
    if let Err(e) = main_sync() {
        eprintln!("veyod error: {e:#}");
        std::process::exit(1);
    }
}

fn run_capture(store: EventStore) -> anyhow::Result<()> {
    let mut backend = PollBackend::primary()?;
    let cfg = CodecConfig::default();
    let surface = SurfaceRef {
        id: "screen:0".into(),
        app: "desktop".into(),
        title: String::new(),
        focused: true,
    };

    // First frame initialises the codec dimensions.
    let first = backend.next_frame()?;
    tracing::info!(w = first.width, h = first.height, "capture started");
    let mut codec = Codec::new(cfg.clone(), surface, (first.width, first.height));
    let first_cells = rgba_to_cells(
        &first.rgba,
        first.width,
        first.height,
        cfg.grid.0,
        cfg.grid.1,
    );
    push_deltas(
        &store,
        codec.observe(CoreFrame {
            t_ms: first.t_ms,
            cells: &first_cells,
        }),
    );

    let interval = Duration::from_millis(1000 / CAPTURE_FPS);
    loop {
        let tick = Instant::now();
        match backend.next_frame() {
            Ok(f) => {
                let cells = rgba_to_cells(&f.rgba, f.width, f.height, cfg.grid.0, cfg.grid.1);
                push_deltas(
                    &store,
                    codec.observe(CoreFrame {
                        t_ms: f.t_ms,
                        cells: &cells,
                    }),
                );
            }
            Err(e) => tracing::warn!("capture error: {e:#}"),
        }
        if let Some(rem) = interval.checked_sub(tick.elapsed()) {
            std::thread::sleep(rem);
        }
    }
}

fn push_deltas(store: &EventStore, deltas: Vec<veyo_core::Delta>) {
    for d in deltas {
        tracing::info!(
            id = %d.id.0,
            kind = ?d.kind,
            salience = %format!("{:.2}", d.salience),
            summary = %d.summary,
        );
        store.push(d);
    }
}
