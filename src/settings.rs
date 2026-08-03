use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "latex-ocr";

/// The OCR engine used to turn captured images into LaTeX.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OcrEngine {
    /// On-device inference through ONNX Runtime, no network or server needed.
    #[default]
    Local,
    /// A pix2tex-compatible HTTP server.
    Http,
}

fn default_true() -> bool {
    true
}

/// Persistent user configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    /// OCR engine.
    #[serde(default)]
    pub ocr_backend: OcrEngine,
    /// Beautify recognized LaTeX (line breaks, alignment, deprecated commands).
    #[serde(default = "default_true")]
    pub ocr_beautify: bool,
    /// Base URL of the OCR backend (pix2tex-compatible server).
    pub ocr_url: String,
    /// Directory holding the ONNX models and ONNX Runtime, or empty for the default.
    #[serde(default)]
    pub ocr_model_dir: Option<String>,
    /// Delay in milliseconds before the preview re-renders after editing.
    pub preview_debounce_ms: u64,
    /// Initial preview zoom factor.
    pub preview_zoom: f32,
    /// Path to a tectonic executable, or empty to auto-discover.
    pub tectonic_path: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ocr_backend: OcrEngine::Local,
            ocr_beautify: true,
            ocr_url: "http://127.0.0.1:8502".to_string(),
            ocr_model_dir: None,
            preview_debounce_ms: 600,
            preview_zoom: 1.0,
            tectonic_path: None,
        }
    }
}

/// Loads and stores `Settings` as TOML in the platform config directory.
pub struct SettingsStore {
    path: PathBuf,
    settings: Settings,
}

impl SettingsStore {
    /// Config file location for the current user.
    pub fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(APP_NAME)
            .join("config.toml")
    }

    /// Load settings from the user config directory, falling back to defaults
    /// when the file is missing or corrupt.
    pub fn load() -> Self {
        Self::load_from(Self::config_path())
    }

    pub fn load_from(path: PathBuf) -> Self {
        let settings = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| toml::from_str(&raw).ok())
            .unwrap_or_default();
        Self { path, settings }
    }

    pub fn get(&self) -> &Settings {
        &self.settings
    }

    pub fn get_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    /// Persist the current settings. A read-only config directory or a disk
    /// error is reported but never fatal.
    pub fn save(&mut self) -> Result<(), String> {
        let parent = self.path.parent().ok_or("config path has no parent")?;
        fs::create_dir_all(parent).map_err(|e| format!("cannot create config dir: {e}"))?;
        let raw = toml::to_string(&self.settings).map_err(|e| e.to_string())?;
        fs::write(&self.path, raw).map_err(|e| format!("cannot write config: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("latex-ocr-test-{}-{name}", std::process::id()))
    }

    #[test]
    fn default_settings_are_sane() {
        let s = Settings::default();
        assert_eq!(s.ocr_backend, OcrEngine::Local);
        assert!(s.ocr_beautify);
        assert!(s.ocr_url.starts_with("http://"));
        assert!(s.preview_debounce_ms > 0);
        assert!(s.preview_zoom > 0.0);
    }

    #[test]
    fn ocr_backend_serializes_as_lowercase_strings() {
        let settings = Settings {
            ocr_backend: OcrEngine::Http,
            ..Settings::default()
        };
        let raw = toml::to_string(&settings).unwrap();
        assert!(raw.contains("ocr_backend = \"http\""), "{raw}");

        let parsed: Settings = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.ocr_backend, OcrEngine::Http);
        // Existing lowercase config values keep working.
        let legacy = "ocr_backend = \"local\"\nocr_beautify = true\nocr_url = \"http://x\"\npreview_debounce_ms = 600\npreview_zoom = 1.0\n";
        let parsed: Settings = toml::from_str(legacy).unwrap();
        assert_eq!(parsed.ocr_backend, OcrEngine::Local);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let path = tmp_dir("roundtrip").join("config.toml");
        let mut store = SettingsStore::load_from(path.clone());
        store.get_mut().ocr_url = "http://localhost:9999".into();
        store.get_mut().preview_debounce_ms = 250;
        store.save().unwrap();

        let loaded = SettingsStore::load_from(path);
        assert_eq!(loaded.get().ocr_url, "http://localhost:9999");
        assert_eq!(loaded.get().preview_debounce_ms, 250);
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let path = tmp_dir("missing").join("does-not-exist.toml");
        let loaded = SettingsStore::load_from(path).get().clone();
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let dir = tmp_dir("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, "not = [valid toml").unwrap();
        let loaded = SettingsStore::load_from(path).get().clone();
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn old_config_without_new_fields_still_loads() {
        let dir = tmp_dir("old-config");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(
            &path,
            "ocr_url = \"http://localhost:9999\"\npreview_debounce_ms = 250\npreview_zoom = 1.0\n",
        )
        .unwrap();
        let loaded = SettingsStore::load_from(path).get().clone();
        assert_eq!(loaded.ocr_backend, OcrEngine::Local);
        assert_eq!(loaded.ocr_url, "http://localhost:9999");
        assert_eq!(loaded.preview_debounce_ms, 250);
    }
}
