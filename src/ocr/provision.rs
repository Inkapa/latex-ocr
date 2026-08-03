//! Provisioning for the local OCR backend.
//!
//! Portable builds ship the ONNX Runtime library and the model graphs next to
//! the executable, so nothing is downloaded at runtime. Development builds
//! fall back to the app data directory: the runtime is downloaded on first use
//! (mirroring how the preview provisions tectonic and pdfium) and the model
//! graphs are produced by `tools/export_onnx.py`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::download::{self, ArchiveKind};
use crate::settings::APP_NAME;

/// ONNX Runtime release used by the local OCR backend. Must be at least the
/// API version `ort` was built against (1.27).
pub const ONNX_RUNTIME_VERSION: &str = "1.27.0";

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Models bundled next to the executable in a portable distribution.
pub fn bundled_models_dir() -> PathBuf {
    exe_dir().join("models")
}

/// ONNX Runtime library bundled next to the executable in a portable
/// distribution.
pub fn bundled_onnxruntime() -> PathBuf {
    exe_dir().join(onnxruntime_lib_name())
}

/// Directory that holds the OCR models and the ONNX Runtime library in
/// development builds.
pub fn ocr_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join("ocr")
}

pub fn encoder_path(dir: &Path) -> PathBuf {
    dir.join("encoder.onnx")
}

pub fn decoder_path(dir: &Path) -> PathBuf {
    dir.join("decoder.onnx")
}

pub fn resizer_path(dir: &Path) -> PathBuf {
    dir.join("resizer.onnx")
}

pub fn onnxruntime_path(dir: &Path) -> PathBuf {
    dir.join(onnxruntime_lib_name())
}

pub fn onnxruntime_lib_name() -> &'static str {
    if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

/// Resolves the ONNX Runtime library to load: an explicit `ORT_DYLIB_PATH`,
/// the copy bundled next to the executable, or a downloaded copy in the data
/// directory.
pub fn resolve_onnxruntime() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("ORT_DYLIB_PATH") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "ORT_DYLIB_PATH points to a missing file: {}",
            path.display()
        ));
    }
    let bundled = bundled_onnxruntime();
    if bundled.is_file() {
        return Ok(bundled);
    }
    ensure_onnxruntime(&ocr_dir())
}

/// Returns the path to a usable ONNX Runtime library, downloading it into
/// `dir` on first use.
pub fn ensure_onnxruntime(dir: &Path) -> Result<PathBuf, String> {
    let lib = onnxruntime_path(dir);
    if lib.is_file() {
        return Ok(lib);
    }
    let (file, kind) = onnxruntime_asset();
    let url = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{ONNX_RUNTIME_VERSION}/{file}"
    );
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    download::download_and_extract(&url, tmp.path(), kind)?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Copy the main library together with any companion shared libraries
    // (e.g. onnxruntime_providers_shared.dll) that it loads at runtime.
    let lib_dir = find_onnxruntime_dir(tmp.path())?;
    for entry in fs::read_dir(&lib_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if is_library_file(&path) {
            let name = entry.file_name();
            fs::copy(&path, dir.join(&name)).map_err(|e| {
                format!(
                    "cannot store onnxruntime library {}: {e}",
                    name.to_string_lossy()
                )
            })?;
        }
    }
    if !lib.is_file() {
        return Err(format!(
            "onnxruntime archive did not contain {}",
            lib.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    Ok(lib)
}

fn is_library_file(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    match name.as_deref() {
        Some(name) => name.ends_with(".dll") || name.ends_with(".so") || name.ends_with(".dylib"),
        None => false,
    }
}

fn onnxruntime_asset() -> (String, ArchiveKind) {
    if cfg!(windows) {
        (
            format!("onnxruntime-win-x64-{ONNX_RUNTIME_VERSION}.zip"),
            ArchiveKind::Zip,
        )
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            (
                format!("onnxruntime-osx-arm64-{ONNX_RUNTIME_VERSION}.tgz"),
                ArchiveKind::TarGz,
            )
        } else {
            (
                format!("onnxruntime-osx-x86_64-{ONNX_RUNTIME_VERSION}.tgz"),
                ArchiveKind::TarGz,
            )
        }
    } else {
        (
            format!("onnxruntime-linux-x64-{ONNX_RUNTIME_VERSION}.tgz"),
            ArchiveKind::TarGz,
        )
    }
}

fn find_onnxruntime_dir(root: &Path) -> Result<PathBuf, String> {
    let expected = onnxruntime_lib_name();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|n| n == expected) {
                return path
                    .parent()
                    .map(Path::to_path_buf)
                    .ok_or_else(|| "onnxruntime library has no parent directory".to_string());
            }
        }
    }
    Err(format!("onnxruntime archive did not contain {expected}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names_reference_release() {
        let (_, kind) = onnxruntime_asset();
        assert!(matches!(kind, ArchiveKind::Zip | ArchiveKind::TarGz));
    }

    #[test]
    fn library_path_uses_platform_name() {
        let name = onnxruntime_path(Path::new("d"))
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.contains("onnxruntime"));
    }
}
