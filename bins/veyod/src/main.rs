mod config;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::Parser;
use veyo_capture::{rgba_to_cells, CaptureBackend, PollBackend};
use veyo_core::{
    Codec, CodecConfig, Delta, EventId, EventKind, Evidence, Frame as CoreFrame, Rect, RegionRef,
    SurfaceRef, TimeMs, SCHEMA_V,
};
use veyo_mcp::{EventStore, VeyoMcpServer};

use config::VeyoToml;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "veyod",
    about = "veyo daemon — local-first screen → semantic event stream over MCP",
    version
)]
struct Args {
    /// Path to veyo.toml config (default: ./veyo.toml).
    #[arg(long, default_value = "veyo.toml")]
    config: PathBuf,

    /// Skip capture and feed synthetic events (useful for testing MCP connectivity).
    #[arg(long)]
    demo: bool,

    /// Override: monitor index to capture (0 = primary).
    #[arg(long)]
    monitor: Option<usize>,

    /// Override: capture FPS (1–60).
    #[arg(long)]
    fps: Option<u64>,

    /// Override: Gate-1 noise floor, mean abs diff [0,1].
    #[arg(long)]
    epsilon_noise: Option<f32>,

    /// Print codec config at startup and exit.
    #[arg(long)]
    print_config: bool,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("veyod=info".parse().unwrap()),
        )
        .init();

    if let Err(e) = run(args) {
        eprintln!("veyod: {e:#}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> anyhow::Result<()> {
    // Load config, apply CLI overrides.
    let mut toml = VeyoToml::load_or_default(&args.config);
    if let Some(m) = args.monitor {
        toml.monitor = Some(m);
    }
    if let Some(f) = args.fps {
        toml.capture_fps = Some(f);
    }
    if let Some(e) = args.epsilon_noise {
        toml.epsilon_noise = Some(e);
    }

    let (codec_cfg, fps, monitor_idx, store_cap) = toml.into_codec_config();

    if args.print_config {
        println!("{codec_cfg:#?}");
        println!("fps={fps} monitor={monitor_idx} store_cap={store_cap}");
        return Ok(());
    }

    let store = EventStore::new(store_cap);

    if args.demo {
        let store_t = store.clone();
        std::thread::Builder::new()
            .name("demo".into())
            .spawn(move || demo_thread(store_t, fps))?;
        tracing::info!("running in demo mode — synthetic events only");
    } else {
        let store_t = store.clone();
        std::thread::Builder::new()
            .name("capture".into())
            .spawn(move || {
                if let Err(e) = capture_thread(store_t, codec_cfg, fps, monitor_idx) {
                    tracing::error!("capture thread: {e:#}");
                }
            })?;
    }

    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async { VeyoMcpServer::new(store).run().await })
}

// ---------------------------------------------------------------------------
// Capture thread (live, blocking I/O)
// ---------------------------------------------------------------------------

fn capture_thread(
    store: EventStore,
    cfg: CodecConfig,
    fps: u64,
    monitor_idx: usize,
) -> anyhow::Result<()> {
    let mut backend = if monitor_idx == 0 {
        PollBackend::primary()?
    } else {
        PollBackend::from_index(monitor_idx)?
    };

    let surface = SurfaceRef {
        id: "screen:0".into(),
        app: "desktop".into(),
        title: String::new(),
        focused: true,
    };

    let first = backend.next_frame()?;
    tracing::info!(w = first.width, h = first.height, fps, "capture started");

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

    let interval = Duration::from_millis(1000 / fps.max(1));
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

// ---------------------------------------------------------------------------
// Demo thread (synthetic events, no display required)
// ---------------------------------------------------------------------------

fn demo_thread(store: EventStore, fps: u64) {
    let interval = Duration::from_millis(1000 / fps.max(1));
    let events: &[(&str, EventKind, &str)] = &[
        (
            "r_3",
            EventKind::RegionChange,
            "content area started changing",
        ),
        (
            "r_3",
            EventKind::StateSettle,
            "content area settled after 1200ms",
        ),
        (
            "r_multi",
            EventKind::RegionChange,
            "4 regions changed (modal appeared)",
        ),
        (
            "r_multi",
            EventKind::StateSettle,
            "4 regions settled (modal dismissed)",
        ),
        ("r_7", EventKind::RegionChange, "sidebar updated"),
    ];
    let mut seq: u64 = 0;
    let surface = SurfaceRef {
        id: "win_demo".into(),
        app: "demo".into(),
        title: "veyo demo".into(),
        focused: true,
    };
    loop {
        for (region_id, kind, summary) in events.iter().copied() {
            std::thread::sleep(interval * 20); // one event every ~5s at 4fps
            seq += 1;
            let t_ms = epoch_ms();
            let delta = Delta {
                v: SCHEMA_V,
                id: EventId(format!("ev_{seq:012}")),
                t_event: t_ms,
                t_observed: t_ms,
                source: "demo:0".into(),
                kind,
                surface: surface.clone(),
                region: RegionRef {
                    id: region_id.into(),
                    grid: [0, 0],
                    bounds: Rect {
                        x: 0,
                        y: 0,
                        w: 640,
                        h: 400,
                    },
                },
                summary: summary.into(),
                salience: 0.8,
                novelty: 0.9,
                duration_ms: if kind == EventKind::StateSettle {
                    Some(1200)
                } else {
                    None
                },
                evidence: Evidence::default(),
            };
            tracing::info!(id = %delta.id.0, kind = ?delta.kind, "{}", delta.summary);
            store.push(delta);
        }
    }
}

fn push_deltas(store: &EventStore, deltas: Vec<Delta>) {
    for d in deltas {
        tracing::info!(
            id = %d.id.0,
            kind = ?d.kind,
            salience = %format!("{:.2}", d.salience),
            "{}",
            d.summary,
        );
        store.push(d);
    }
}

fn epoch_ms() -> TimeMs {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
