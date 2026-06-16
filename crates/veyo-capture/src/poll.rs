use crate::backend::{CaptureBackend, CapturedFrame};
use std::time::{SystemTime, UNIX_EPOCH};
use xcap::Monitor;

/// Grab-and-poll backend: captures a full-monitor screenshot on every call to
/// `next_frame()`.  Works on X11 and Wayland (via `xcap`); no damage rects.
pub struct PollBackend {
    monitor: Monitor,
}

impl PollBackend {
    /// Open the primary monitor (falls back to the first available).
    pub fn primary() -> anyhow::Result<Self> {
        let monitors = Monitor::all().map_err(|e| anyhow::anyhow!("xcap Monitor::all: {e}"))?;
        let monitor = monitors
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .or_else(|| Monitor::all().ok()?.into_iter().next())
            .ok_or_else(|| anyhow::anyhow!("no monitor found"))?;
        tracing::debug!(
            name = monitor.name().unwrap_or_default(),
            "PollBackend opened"
        );
        Ok(Self { monitor })
    }

    /// Open a specific monitor by 0-based index.
    pub fn from_index(idx: usize) -> anyhow::Result<Self> {
        let monitors = Monitor::all().map_err(|e| anyhow::anyhow!("xcap Monitor::all: {e}"))?;
        let monitor = monitors
            .into_iter()
            .nth(idx)
            .ok_or_else(|| anyhow::anyhow!("monitor index {idx} out of range"))?;
        Ok(Self { monitor })
    }
}

impl CaptureBackend for PollBackend {
    fn next_frame(&mut self) -> anyhow::Result<CapturedFrame> {
        let t_ms = epoch_ms();
        let img = self
            .monitor
            .capture_image()
            .map_err(|e| anyhow::anyhow!("xcap capture_image: {e}"))?;
        Ok(CapturedFrame {
            width: img.width(),
            height: img.height(),
            rgba: img.as_raw().clone(),
            t_ms,
        })
    }
}

fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This test requires a live display — skip in headless CI.
    #[test]
    #[ignore = "requires display"]
    fn primary_backend_captures_a_frame() {
        let mut backend = PollBackend::primary().expect("primary backend");
        let frame = backend.next_frame().expect("next_frame");
        assert!(frame.width > 0 && frame.height > 0);
        assert_eq!(frame.rgba.len(), (frame.width * frame.height * 4) as usize);
        assert!(frame.t_ms > 0);
    }

    #[test]
    #[ignore = "requires display"]
    fn frame_width_and_height_are_consistent() {
        let mut b = PollBackend::primary().unwrap();
        let f1 = b.next_frame().unwrap();
        let f2 = b.next_frame().unwrap();
        assert_eq!(f1.width, f2.width);
        assert_eq!(f1.height, f2.height);
    }
}
