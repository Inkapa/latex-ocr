#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

fn main() -> eframe::Result {
    env_logger::init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("LaTeX OCR")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([780.0, 500.0])
            .with_transparent(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    eframe::run_native(
        "latex-ocr",
        options,
        Box::new(|cc| Ok(Box::new(latex_ocr::app::LatexOcrApp::new(cc)))),
    )
}
