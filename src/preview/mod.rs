//! Live preview pipeline: debounced rendering on a background thread.

pub mod engine;
pub mod raster;
pub mod tex;

pub use raster::RenderSetup;

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

enum PreviewMsg {
    Rendered {
        generation_id: u64,
        result: Result<Vec<RgbaImage>, String>,
    },
}

use image::RgbaImage;

struct RenderedPage {
    texture: TextureHandle,
}

enum PreviewState {
    Empty,
    Rendering,
    Rendered(Vec<RenderedPage>),
    Failed(String),
}

pub struct Preview {
    pub enabled: bool,
    pub zoom: f32,
    pub fit_width: bool,
    setup: Arc<RenderSetup>,
    tectonic_override: Option<String>,
    generation: u64,
    in_flight: bool,
    dirty: bool,
    last_edit: Option<Instant>,
    tx: Sender<PreviewMsg>,
    rx: Receiver<PreviewMsg>,
    state: PreviewState,
}

impl Preview {
    pub fn new(setup: Arc<RenderSetup>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            enabled: true,
            zoom: 1.0,
            fit_width: true,
            setup,
            tectonic_override: None,
            generation: 0,
            in_flight: false,
            dirty: false,
            last_edit: None,
            tx,
            rx,
            state: PreviewState::Empty,
        }
    }

    pub fn set_tectonic_override(&mut self, override_path: Option<String>) {
        self.tectonic_override = override_path;
    }

    /// Invalidates a cached engine setup (e.g. after settings changed).
    pub fn reset_engine(&mut self) {
        self.setup.reset();
    }

    /// Shared handle to the engine setup, used for background prewarming.
    pub fn engine_handle(&self) -> Arc<RenderSetup> {
        self.setup.clone()
    }

    pub fn engine_ready(&self) -> bool {
        self.setup.get(self.tectonic_override.as_deref()).is_ok()
    }

    /// Called whenever the document changes.
    pub fn mark_edited(&mut self) {
        self.dirty = true;
        self.last_edit = Some(Instant::now());
    }

    /// Forces an immediate render on the next frame.
    pub fn request_render(&mut self) {
        self.dirty = true;
        self.last_edit = None;
    }

    /// Re-reads completed render results from the worker thread.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut result = None;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                PreviewMsg::Rendered {
                    generation_id,
                    result: r,
                } => {
                    if generation_id == self.generation {
                        result = Some(r);
                    }
                }
            }
        }

        if let Some(result) = result {
            self.in_flight = false;
            match result {
                Ok(pages) => {
                    self.state = PreviewState::Rendered(
                        pages
                            .into_iter()
                            .filter_map(|img| {
                                let (w, h) = (img.width() as usize, img.height() as usize);
                                if w == 0 || h == 0 {
                                    return None;
                                }
                                let image =
                                    ColorImage::from_rgba_unmultiplied([w, h], &img.into_raw());
                                let name = format!("preview-page-{}", w * h);
                                Some(RenderedPage {
                                    texture: ctx.load_texture(name, image, TextureOptions::LINEAR),
                                })
                            })
                            .collect(),
                    );
                }
                Err(message) => {
                    self.state = PreviewState::Failed(message);
                }
            }
        }
    }

    /// Kicks off a render when the debounce window has elapsed.
    pub fn maybe_render(&mut self, source: &str, debounce: Duration) {
        if !self.enabled || !self.dirty || self.in_flight {
            return;
        }
        // An empty document has nothing to show.
        if !tex::has_content(source) {
            self.dirty = false;
            self.state = PreviewState::Empty;
            return;
        }
        let due = self
            .last_edit
            .map(|t| t.elapsed() >= debounce)
            .unwrap_or(true);
        if !due {
            return;
        }

        self.dirty = false;
        self.generation += 1;
        self.in_flight = true;
        self.state = PreviewState::Rendering;

        let generation_id = self.generation;
        let source = source.to_string();
        let setup = self.setup.clone();
        let override_path = self.tectonic_override.clone();
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("preview-render".to_string())
            .spawn(move || {
                let result = raster::render_source(&setup, &source, override_path.as_deref());
                let _ = tx.send(PreviewMsg::Rendered {
                    generation_id,
                    result,
                });
            })
            .expect("spawn preview render thread");
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.enabled, "Auto")
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if ui.button("Render now").clicked() {
                self.request_render();
            }
            ui.separator();
            if ui.button("−").on_hover_text("Zoom out").clicked() {
                self.zoom = (self.zoom / 1.25).max(0.1);
            }
            ui.add(
                egui::Slider::new(&mut self.zoom, 0.1..=4.0)
                    .logarithmic(true)
                    .show_value(false),
            );
            if ui.button("+").on_hover_text("Zoom in").clicked() {
                self.zoom = (self.zoom * 1.25).min(4.0);
            }
            ui.checkbox(&mut self.fit_width, "Fit width")
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        });

        ui.separator();

        egui::ScrollArea::both()
            .id_salt("preview_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut retry = false;
                match &self.state {
                    PreviewState::Empty => {
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                egui::RichText::new("Your rendered LaTeX will appear here.").weak(),
                            );
                        });
                    }
                    PreviewState::Rendering => {
                        ui.centered_and_justified(|ui| {
                            ui.add(egui::Spinner::new().size(32.0));
                            ui.label("Rendering…");
                        });
                    }
                    PreviewState::Failed(message) => {
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new("LaTeX error")
                                .strong()
                                .color(egui::Color32::from_rgb(0xE0, 0x6C, 0x75)),
                        );
                        ui.add_space(6.0);
                        egui::Frame::new()
                            .fill(ui.visuals().extreme_bg_color)
                            .corner_radius(6.0)
                            .inner_margin(egui::Margin::same(10))
                            .show(ui, |ui| {
                                ui.set_min_height(ui.available_height().max(120.0));
                                egui::ScrollArea::vertical()
                                    .id_salt("preview_error_scroll")
                                    .auto_shrink([false, true])
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(message.as_str()).monospace(),
                                            )
                                            .wrap()
                                            .selectable(true),
                                        );
                                    });
                            });
                        ui.add_space(8.0);
                        if ui.button("Retry").clicked() {
                            retry = true;
                        }
                    }
                    PreviewState::Rendered(pages) => {
                        let avail_width = ui.available_width();
                        for page in pages {
                            let native = page.texture.size_vec2();
                            let scale = if self.fit_width && native.x > 0.0 {
                                (avail_width / native.x).min(self.zoom)
                            } else {
                                self.zoom
                            };
                            let size = native * scale;
                            ui.add(
                                egui::Image::new(&page.texture)
                                    .fit_to_exact_size(size)
                                    .sense(egui::Sense::hover()),
                            );
                            ui.add_space(12.0);
                        }
                    }
                }
                if retry {
                    self.setup.reset();
                    self.request_render();
                }
            });
    }
}
