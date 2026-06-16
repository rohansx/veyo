use serde::Deserialize;
use std::path::Path;
use veyo_core::CodecConfig;

/// `veyo.toml` — all fields optional (fall back to CodecConfig defaults).
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct VeyoToml {
    pub capture_fps: Option<u64>,
    pub grid: Option<[u8; 2]>,
    pub epsilon_noise: Option<f32>,
    pub settle_window_ms: Option<u64>,
    pub salience_min: Option<f32>,
    pub novelty_decay: Option<f32>,
    pub focus_weight: Option<f32>,
    pub coalesce_min_regions: Option<usize>,
    pub source: Option<String>,
    pub monitor: Option<usize>,
    pub store_cap: Option<usize>,
}

impl VeyoToml {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn load_or_default(path: &Path) -> Self {
        if path.exists() {
            match Self::load(path) {
                Ok(cfg) => {
                    tracing::info!(path = %path.display(), "loaded veyo.toml");
                    cfg
                }
                Err(e) => {
                    tracing::warn!(path = %path.display(), "veyo.toml parse error: {e:#}; using defaults");
                    Self::default()
                }
            }
        } else {
            Self::default()
        }
    }

    pub fn into_codec_config(self) -> (CodecConfig, u64, usize, usize) {
        let d = CodecConfig::default();
        let codec = CodecConfig {
            grid: self.grid.map(|[c, r]| (c, r)).unwrap_or(d.grid),
            epsilon_noise: self.epsilon_noise.unwrap_or(d.epsilon_noise),
            settle_window_ms: self.settle_window_ms.unwrap_or(d.settle_window_ms),
            salience_min: self.salience_min.unwrap_or(d.salience_min),
            novelty_decay: self.novelty_decay.unwrap_or(d.novelty_decay),
            focus_weight: self.focus_weight.unwrap_or(d.focus_weight),
            coalesce_min_regions: self.coalesce_min_regions.unwrap_or(d.coalesce_min_regions),
            source: self.source.unwrap_or(d.source),
        };
        let fps = self.capture_fps.unwrap_or(4).clamp(1, 60);
        let monitor = self.monitor.unwrap_or(0);
        let store_cap = self.store_cap.unwrap_or(2000);
        (codec, fps, monitor, store_cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn with_toml(content: &str) -> (tempfile_shim::TempDir, std::path::PathBuf) {
        let dir = tempfile_shim::TempDir::new();
        let path = dir.path().join("veyo.toml");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(content.as_bytes())
            .unwrap();
        (dir, path)
    }

    // Minimal "tempdir" shim so the test has no extra dep.
    mod tempfile_shim {
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);

        pub struct TempDir(std::path::PathBuf);
        impl TempDir {
            pub fn new() -> Self {
                let n = CTR.fetch_add(1, Ordering::Relaxed);
                let p =
                    std::env::temp_dir().join(format!("veyo_cfg_test_{}_{n}", std::process::id()));
                std::fs::create_dir_all(&p).unwrap();
                Self(p)
            }
            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn empty_toml_uses_all_defaults() {
        let (_dir, path) = with_toml("");
        let cfg = VeyoToml::load(&path).unwrap();
        let (codec, fps, monitor, cap) = cfg.into_codec_config();
        assert_eq!(codec.grid, (8, 8));
        assert_eq!(fps, 4);
        assert_eq!(monitor, 0);
        assert_eq!(cap, 2000);
        assert!((codec.epsilon_noise - 0.03).abs() < 1e-6);
    }

    #[test]
    fn partial_toml_overrides_only_set_fields() {
        let (_dir, path) = with_toml("epsilon_noise = 0.05\ncapture_fps = 8\n");
        let cfg = VeyoToml::load(&path).unwrap();
        let (codec, fps, _monitor, _cap) = cfg.into_codec_config();
        assert!((codec.epsilon_noise - 0.05).abs() < 1e-6);
        assert_eq!(fps, 8);
        assert_eq!(codec.grid, (8, 8)); // unchanged
    }

    #[test]
    fn grid_array_parsed_correctly() {
        let (_dir, path) = with_toml("grid = [4, 4]\n");
        let cfg = VeyoToml::load(&path).unwrap();
        let (codec, _, _, _) = cfg.into_codec_config();
        assert_eq!(codec.grid, (4, 4));
    }

    #[test]
    fn load_or_default_is_safe_on_nonexistent_path() {
        let cfg = VeyoToml::load_or_default(Path::new("/nonexistent/veyo.toml"));
        let (codec, _, _, _) = cfg.into_codec_config();
        assert_eq!(codec.grid, (8, 8));
    }
}
