use veyo_core::{Rect, TimeMs};

/// One raw frame from the OS capture layer: RGBA bytes, not yet downscaled.
pub struct CapturedFrame {
    /// RGBA interleaved, row-major, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub t_ms: TimeMs,
}

/// Abstraction over platform-specific screen capture.
///
/// The daemon holds one `Box<dyn CaptureBackend>`. The FSM downstream never
/// changes regardless of which backend is active — that decoupling is the
/// whole point.
pub trait CaptureBackend: Send {
    /// Block until the next frame is ready; return it.
    fn next_frame(&mut self) -> anyhow::Result<CapturedFrame>;

    /// OS-provided dirty rects when available (Wayland/DXGI). `None` for poll
    /// backends — they treat the whole frame as potentially dirty.
    fn damage(&self) -> Option<&[Rect]> {
        None
    }

    /// Current visible surfaces (windows). Returns empty vec if not supported.
    fn surfaces(&self) -> Vec<SurfaceInfo> {
        Vec::new()
    }
}

/// Minimal window descriptor from the OS.
#[derive(Debug, Clone)]
pub struct SurfaceInfo {
    pub id: String,
    pub app: String,
    pub title: String,
    pub bounds: Rect,
    pub focused: bool,
}
