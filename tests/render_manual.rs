//! End-to-end check of the real rendering pipeline. Ignored by default
//! because it downloads the pdfium library and needs a tectonic binary.
//!
//! Run manually with:
//!   TECTONIC_OVERRIDE=<path/to/tectonic> cargo test --test render_manual -- --ignored

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use latex_ocr::preview::engine::TectonicCli;
use latex_ocr::preview::raster::{self, RenderSetup};
use latex_ocr::preview::tex;

fn dark_pixels(rendered: &[image::RgbaImage]) -> usize {
    rendered
        .iter()
        .flat_map(|img| img.pixels())
        .filter(|p| p.0[0] < 128)
        .count()
}

#[test]
#[ignore = "downloads pdfium and needs a tectonic binary"]
fn full_pipeline_renders_latex() {
    let engine_dir = tempfile::tempdir().expect("temp dir");
    let setup = RenderSetup::with_engine_dir(engine_dir.path().to_path_buf());
    let override_path = std::env::var("TECTONIC_OVERRIDE").ok();

    let source = r"$\int_0^1 x^2\,dx = \frac{1}{3}$";
    let (pages, _) = raster::render_source(&setup, source, override_path.as_deref(), None)
        .expect("render_source should succeed");

    assert!(!pages.is_empty(), "expected at least one rendered page");
    assert!(
        dark_pixels(&pages) > 10,
        "expected visible content, found {} dark pixels",
        dark_pixels(&pages)
    );
}

#[test]
#[ignore = "downloads pdfium and needs a tectonic binary"]
fn renders_default_sample_document() {
    let engine_dir = tempfile::tempdir().expect("temp dir");
    let setup = RenderSetup::with_engine_dir(engine_dir.path().to_path_buf());
    let override_path = std::env::var("TECTONIC_OVERRIDE").ok();

    let source = include_str!("../src/editor/sample.tex");
    let (pages, _) = raster::render_source(&setup, source, override_path.as_deref(), None)
        .expect("sample document should compile");

    assert!(!pages.is_empty());
    assert!(dark_pixels(&pages) > 10, "sample document rendered blank");
}

#[test]
#[ignore = "downloads pdfium and needs a tectonic binary"]
fn renders_full_document_unchanged() {
    let engine_dir = tempfile::tempdir().expect("temp dir");
    let setup = RenderSetup::with_engine_dir(engine_dir.path().to_path_buf());
    let override_path = std::env::var("TECTONIC_OVERRIDE").ok();

    let source = r"\documentclass{article}
\usepackage{amsmath}
\begin{document}
\section{Title}
Inline $a^2 + b^2 = c^2$ and a display:
\begin{equation}
    E = mc^2
\end{equation}
\end{document}";
    let (pages, _) = raster::render_source(&setup, source, override_path.as_deref(), None)
        .expect("full document should compile");

    assert!(!pages.is_empty());
    assert!(dark_pixels(&pages) > 10, "full document rendered blank");
}

#[test]
#[ignore = "needs a tectonic binary"]
fn cancelled_compile_kills_the_engine() {
    let override_path = std::env::var("TECTONIC_OVERRIDE")
        .expect("TECTONIC_OVERRIDE must point to a tectonic binary");
    let engine = TectonicCli {
        binary: override_path.into(),
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let source = tex::wrap_source(r"$\int_0^1 x^2\,dx$");

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_handle = cancel.clone();
    let started = Instant::now();
    let handle =
        std::thread::spawn(move || engine.compile_cancellable(&source, dir.path(), &cancel_handle));
    std::thread::sleep(Duration::from_millis(100));
    cancel.store(true, Ordering::Relaxed);

    let result = handle.join().expect("compile thread panicked");
    assert!(result.is_err(), "a cancelled compile should fail");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the engine should be killed quickly, took {:?}",
        started.elapsed()
    );
}
