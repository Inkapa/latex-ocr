//! PDF rasterization and engine provisioning.
//!
//! The LaTeX engine (Tectonic) and the PDF rasterizer are external programs
//! that are downloaded on first use and cached in the app data directory.
//! Poppler's `pdftoppm` or MuPDF's `mutool` are preferred when already
//! installed; otherwise the app provisions the pdfium library itself.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use image::RgbaImage;
use log::{debug, info};

use crate::download::{self, ArchiveKind};
use crate::settings::APP_NAME;

pub const TECTONIC_VERSION: &str = "0.17.0";
pub const PDFIUM_CHROMIUM: &str = "7881";
pub const PREVIEW_DPI: f32 = 150.0;

/// Failure during PDF rasterization.
#[derive(Debug)]
pub struct RasterError {
    pub message: String,
}

impl std::fmt::Display for RasterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RasterError {}

/// Converts a PDF file into a list of page images.
pub trait Rasterizer: Send + Sync {
    fn rasterize(&self, pdf: &Path) -> Result<Vec<RgbaImage>, RasterError>;
}

/// Ready-to-use engines shared between the app and render workers.
pub struct EngineBundle {
    pub tectonic: PathBuf,
    pub rasterizer: std::sync::Arc<dyn Rasterizer>,
}

/// Lazily builds and caches an [`EngineBundle`]. A failed setup is remembered
/// so downloads are not retried on every render; call [`RenderSetup::reset`]
/// to force a retry.
pub struct RenderSetup {
    inner: Mutex<Option<Result<std::sync::Arc<EngineBundle>, String>>>,
    engine_dir: PathBuf,
}

impl Default for RenderSetup {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderSetup {
    pub fn new() -> Self {
        Self::with_engine_dir(default_engine_dir())
    }

    pub fn with_engine_dir(engine_dir: PathBuf) -> Self {
        Self {
            inner: Mutex::new(None),
            engine_dir,
        }
    }

    pub fn get(
        &self,
        tectonic_override: Option<&str>,
    ) -> Result<std::sync::Arc<EngineBundle>, String> {
        let mut slot = self.inner.lock().unwrap();
        if slot.is_none() {
            *slot = Some(build_bundle(&self.engine_dir, tectonic_override));
        }
        slot.as_ref().unwrap().clone()
    }

    pub fn reset(&self) {
        *self.inner.lock().unwrap() = None;
    }
}

fn default_engine_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
        .join("engine")
}

fn build_bundle(
    engine_dir: &Path,
    tectonic_override: Option<&str>,
) -> Result<std::sync::Arc<EngineBundle>, String> {
    let tectonic = resolve_tectonic(engine_dir, tectonic_override)?;
    let rasterizer = resolve_rasterizer(engine_dir)?;
    Ok(std::sync::Arc::new(EngineBundle {
        tectonic,
        rasterizer,
    }))
}

fn resolve_tectonic(engine_dir: &Path, override_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = override_path.map(str::trim).filter(|p| !p.is_empty()) {
        let path = PathBuf::from(p);
        if verify_tectonic(&path) {
            return Ok(path);
        }
        return Err(format!("configured tectonic executable not found at {p}"));
    }

    if let Some(path) = find_in_path("tectonic") {
        info!("using tectonic from PATH: {}", path.display());
        return Ok(path);
    }

    let cached = cached_tectonic(engine_dir);
    if verify_tectonic(&cached) {
        return Ok(cached);
    }

    download_tectonic(engine_dir)?;
    Ok(cached_tectonic(engine_dir))
}

/// pdfium-render can only initialize its library once per process, so the
/// pdfium-backed rasterizer is cached for the lifetime of the app.
static PDFIUM_CACHE: std::sync::OnceLock<Result<std::sync::Arc<dyn Rasterizer>, String>> =
    std::sync::OnceLock::new();

fn resolve_rasterizer(engine_dir: &Path) -> Result<std::sync::Arc<dyn Rasterizer>, String> {
    if let Some(path) = find_in_path("pdftoppm") {
        info!("using rasterizer: pdftoppm");
        return Ok(std::sync::Arc::new(CommandRasterizer {
            command: path,
            kind: CommandKind::Poppler,
        }));
    }
    if let Some(path) = find_in_path("mutool") {
        info!("using rasterizer: mutool");
        return Ok(std::sync::Arc::new(CommandRasterizer {
            command: path,
            kind: CommandKind::Mutool,
        }));
    }

    PDFIUM_CACHE
        .get_or_init(|| {
            let lib = cached_pdfium_lib(engine_dir);
            if !lib.exists() {
                download_pdfium(engine_dir)?;
            }
            let dir = lib
                .parent()
                .ok_or("pdfium cache dir missing")?
                .to_path_buf();
            let bindings = pdfium_render::prelude::Pdfium::bind_to_library(
                pdfium_render::prelude::Pdfium::pdfium_platform_library_name_at_path(&dir),
            )
            .map_err(|e| format!("cannot load pdfium library: {e}"))?;
            info!(
                "using rasterizer: pdfium ({} bytes)",
                lib.metadata().map(|m| m.len()).unwrap_or(0)
            );
            Ok(std::sync::Arc::new(PdfiumRasterizer {
                pdfium: pdfium_render::prelude::Pdfium::new(bindings),
            }))
        })
        .clone()
}

/// Compiles `source` to a PDF and rasterizes every page. Runs entirely on the
/// calling thread; used from background render workers.
pub fn render_source(
    setup: &RenderSetup,
    source: &str,
    tectonic_override: Option<&str>,
) -> Result<Vec<RgbaImage>, String> {
    let bundle = setup.get(tectonic_override)?;
    let wrapped = crate::preview::tex::wrap_source(source);
    let tmp = tempfile::tempdir().map_err(|e| format!("cannot create temp dir: {e}"))?;
    use crate::preview::engine::TexEngine;
    let engine = crate::preview::engine::TectonicCli {
        binary: bundle.tectonic.clone(),
    };
    let pdfs = engine.compile(&wrapped, tmp.path()).map_err(|e| {
        let log = e.log.unwrap_or_default();
        if log.trim().is_empty() {
            e.message
        } else {
            format!("{}\n\n{}", e.message, log.trim_end())
        }
    })?;
    let pdf = &pdfs[0];
    let pages: Vec<RgbaImage> = bundle
        .rasterizer
        .rasterize(pdf)
        .map_err(|e| e.message)?
        .into_iter()
        .map(trim_white)
        .collect();
    debug!("rendered {} page(s)", pages.len());
    Ok(pages)
}

/// Threshold below which a pixel counts as "content" (not page white).
const CONTENT_THRESHOLD: u8 = 250;

/// Crops a rendered page to its content, leaving a small padding, so that
/// math snippets are not surrounded by a page of white.
fn trim_white(image: RgbaImage) -> RgbaImage {
    let (width, height) = (image.width(), image.height());
    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;

    for y in 0..height {
        for x in 0..width {
            let p = image.get_pixel(x, y).0;
            let is_content =
                p[0] < CONTENT_THRESHOLD || p[1] < CONTENT_THRESHOLD || p[2] < CONTENT_THRESHOLD;
            if is_content {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }

    if max_x < min_x {
        return image; // fully blank page
    }

    let padding = ((width.min(height) as f32) * 0.01).round().max(4.0) as u32;
    let x0 = min_x.saturating_sub(padding);
    let y0 = min_y.saturating_sub(padding);
    let x1 = (max_x + padding).min(width - 1);
    let y1 = (max_y + padding).min(height - 1);
    image::imageops::crop_imm(&image, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image()
}

#[derive(Clone, Copy)]
enum CommandKind {
    Poppler,
    Mutool,
}

/// Rasterizer that shells out to `pdftoppm` or `mutool draw`.
struct CommandRasterizer {
    command: PathBuf,
    kind: CommandKind,
}

impl Rasterizer for CommandRasterizer {
    fn rasterize(&self, pdf: &Path) -> Result<Vec<RgbaImage>, RasterError> {
        let tmp = tempfile::tempdir().map_err(|e| RasterError {
            message: format!("cannot create temp dir: {e}"),
        })?;
        let prefix = tmp.path().join("page");

        let status = match self.kind {
            CommandKind::Poppler => std::process::Command::new(&self.command)
                .args(["-png", "-r", &PREVIEW_DPI.to_string(), "-f", "1"])
                .arg(pdf)
                .arg(&prefix)
                .output(),
            CommandKind::Mutool => std::process::Command::new(&self.command)
                .args(["draw", "-r", &PREVIEW_DPI.to_string(), "-o"])
                .arg(format!("{}-%d.png", prefix.display()))
                .arg(pdf)
                .output(),
        }
        .map_err(|e| RasterError {
            message: format!("failed to run {}: {e}", self.command.display()),
        })?;

        if !status.status.success() {
            let stderr = String::from_utf8_lossy(&status.stderr);
            return Err(RasterError {
                message: format!("{} failed: {}", self.command.display(), stderr.trim()),
            });
        }

        let mut pages = Vec::new();
        for i in 1.. {
            let path = prefix.with_file_name(format!("page-{i}.png"));
            if !path.exists() {
                break;
            }
            let img = image::open(&path)
                .map_err(|e| RasterError {
                    message: format!("cannot read page {i}: {e}"),
                })?
                .into_rgba8();
            pages.push(img);
        }
        if pages.is_empty() {
            return Err(RasterError {
                message: "rasterizer produced no pages".to_string(),
            });
        }
        Ok(pages)
    }
}

/// Rasterizer backed by the pdfium library (Google Chromium's PDF engine).
struct PdfiumRasterizer {
    pdfium: pdfium_render::prelude::Pdfium,
}

/// Below this pixel width pdfium produces blank pages for small documents.
const MIN_RENDER_WIDTH: i32 = 700;

impl Rasterizer for PdfiumRasterizer {
    fn rasterize(&self, pdf: &Path) -> Result<Vec<RgbaImage>, RasterError> {
        use pdfium_render::prelude::*;
        let doc = self
            .pdfium
            .load_pdf_from_file(pdf, None)
            .map_err(|e| RasterError {
                message: format!("cannot open PDF: {e}"),
            })?;

        let mut pages = Vec::new();
        for page in doc.pages().iter() {
            let target_width = (page.width().value * PREVIEW_DPI / 72.0).round() as i32;
            let config =
                PdfRenderConfig::new().set_target_width(target_width.max(MIN_RENDER_WIDTH));
            let rendered = page.render_with_config(&config).map_err(|e| RasterError {
                message: format!("cannot render page: {e}"),
            })?;
            let image = rendered.as_image().map_err(|e| RasterError {
                message: format!("cannot convert rendered page: {e}"),
            })?;
            pages.push(image.into_rgba8());
        }

        if pages.is_empty() {
            return Err(RasterError {
                message: "PDF has no pages".to_string(),
            });
        }
        Ok(pages)
    }
}

// ---------------------------------------------------------------------------
// Provisioning
// ---------------------------------------------------------------------------

fn cached_tectonic(engine_dir: &Path) -> PathBuf {
    engine_dir.join("tectonic").join(if cfg!(windows) {
        "tectonic.exe"
    } else {
        "tectonic"
    })
}

fn cached_pdfium_lib(engine_dir: &Path) -> PathBuf {
    engine_dir.join("pdfium").join(if cfg!(windows) {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    })
}

fn verify_tectonic(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let output = match std::process::Command::new(path).arg("--version").output() {
        Ok(o) => o,
        Err(_) => return false,
    };
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text.trim().starts_with("Tectonic")
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    for dir in env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        if cfg!(windows) {
            let with_ext = dir.join(format!("{name}.exe"));
            if with_ext.is_file() {
                return Some(with_ext);
            }
        }
    }
    None
}

fn download_tectonic(engine_dir: &Path) -> Result<(), String> {
    let (file, kind) = tectonic_asset();
    let url = format!(
        "https://github.com/tectonic-typesetting/tectonic/releases/download/tectonic%40{TECTONIC_VERSION}/{file}"
    );
    let dest = engine_dir.join("tectonic");
    download::download_and_extract(&url, &dest, kind)?;
    let cached = cached_tectonic(engine_dir);
    if !verify_tectonic(&cached) {
        return Err(format!(
            "downloaded tectonic binary failed verification at {}",
            cached.display()
        ));
    }
    Ok(())
}

fn download_pdfium(engine_dir: &Path) -> Result<(), String> {
    let (file, kind) = pdfium_asset();
    let url = format!(
        "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/{PDFIUM_CHROMIUM}/{file}"
    );
    let dest = engine_dir.join("pdfium");
    let lib = cached_pdfium_lib(engine_dir);
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    download::download_and_extract(&url, tmp.path(), kind)?;
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    // The library lives under bin/ in the archive; flatten it for easy loading.
    let found = find_library_file(tmp.path())?;
    fs::copy(&found, &lib).map_err(|e| format!("cannot store pdfium library: {e}"))?;
    Ok(())
}

fn find_library_file(root: &Path) -> Result<PathBuf, String> {
    let expected = if cfg!(windows) {
        "pdfium.dll"
    } else if cfg!(target_os = "macos") {
        "libpdfium.dylib"
    } else {
        "libpdfium.so"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy() == expected)
            {
                return Ok(path);
            }
        }
    }
    Err(format!("pdfium archive did not contain {expected}"))
}

fn tectonic_asset() -> (&'static str, ArchiveKind) {
    if cfg!(target_os = "windows") {
        (
            "tectonic-0.17.0-x86_64-pc-windows-msvc.zip",
            ArchiveKind::Zip,
        )
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            (
                "tectonic-0.17.0-aarch64-apple-darwin.tar.gz",
                ArchiveKind::TarGz,
            )
        } else {
            (
                "tectonic-0.17.0-x86_64-apple-darwin.tar.gz",
                ArchiveKind::TarGz,
            )
        }
    } else {
        (
            "tectonic-0.17.0-x86_64-unknown-linux-gnu.tar.gz",
            ArchiveKind::TarGz,
        )
    }
}

fn pdfium_asset() -> (&'static str, ArchiveKind) {
    if cfg!(target_os = "windows") {
        ("pdfium-win-x64.tgz", ArchiveKind::TarGz)
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") {
            ("pdfium-mac-arm64.tgz", ArchiveKind::TarGz)
        } else {
            ("pdfium-mac-x64.tgz", ArchiveKind::TarGz)
        }
    } else {
        ("pdfium-linux-x64.tgz", ArchiveKind::TarGz)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names_are_valid() {
        let (name, kind) = tectonic_asset();
        assert!(name.contains("tectonic-0.17.0"));
        assert!(matches!(kind, ArchiveKind::Zip | ArchiveKind::TarGz));
        let (name, _) = pdfium_asset();
        assert!(name.contains("pdfium"));
    }

    #[test]
    fn cached_paths_use_expected_names() {
        let dir = PathBuf::from("engine");
        assert!(
            cached_tectonic(&dir)
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains("tectonic"))
        );
        assert!(
            cached_pdfium_lib(&dir)
                .file_name()
                .is_some_and(|n| n.to_string_lossy().contains("pdfium"))
        );
    }

    #[test]
    fn verify_tectonic_rejects_missing_file() {
        assert!(!verify_tectonic(Path::new("/does/not/exist")));
    }

    #[test]
    fn trim_white_removes_blank_margins() {
        let mut img = RgbaImage::from_pixel(100, 80, image::Rgba([255, 255, 255, 255]));
        // Draw a black rectangle in the middle.
        for y in 30..50 {
            for x in 40..60 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let trimmed = trim_white(img);
        assert!(trimmed.width() <= 60);
        assert!(trimmed.height() <= 40);
        assert!(trimmed.width() >= 20);
        assert!(trimmed.height() >= 20);
    }

    #[test]
    fn trim_white_keeps_blank_pages_unchanged() {
        let img = RgbaImage::from_pixel(50, 50, image::Rgba([255, 255, 255, 255]));
        let trimmed = trim_white(img);
        assert_eq!(trimmed.width(), 50);
        assert_eq!(trimmed.height(), 50);
    }

    #[test]
    fn trim_white_handles_content_at_edges() {
        let mut img = RgbaImage::from_pixel(10, 10, image::Rgba([255, 255, 255, 255]));
        img.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        img.put_pixel(9, 9, image::Rgba([0, 0, 0, 255]));
        let trimmed = trim_white(img);
        assert_eq!(trimmed.width(), 10);
        assert_eq!(trimmed.height(), 10);
    }
}
