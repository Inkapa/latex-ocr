//! Live preview pipeline: debounced rendering on a background thread.

pub mod engine;
pub mod raster;
pub mod tex;

pub use raster::RenderSetup;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};

enum PreviewMsg {
    Rendered {
        generation_id: u64,
        source: String,
        result: Result<(Vec<RgbaImage>, Vec<u8>), String>,
    },
}

use image::RgbaImage;

struct RenderedPage {
    texture: TextureHandle,
}

/// A rendered document: GPU textures for display plus the source images and
/// the compiled PDF, which the Save button writes to disk.
struct RenderedDoc {
    textures: Vec<RenderedPage>,
    images: Vec<RgbaImage>,
    pdf: Vec<u8>,
}

enum PreviewState {
    Empty,
    Rendering,
    Rendered(RenderedDoc),
    Failed(String),
}

pub struct Preview {
    pub enabled: bool,
    pub zoom: f32,
    pub fit_width: bool,
    setup: Arc<RenderSetup>,
    tectonic_override: Option<String>,
    generation: u64,
    in_flight_cancel: Option<Arc<AtomicBool>>,
    last_rendered_source: Option<String>,
    dirty: bool,
    last_edit: Option<Instant>,
    tx: Sender<PreviewMsg>,
    rx: Receiver<PreviewMsg>,
    state: PreviewState,
    save_message: Option<(String, Instant)>,
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
            in_flight_cancel: None,
            last_rendered_source: None,
            dirty: false,
            last_edit: None,
            tx,
            rx,
            state: PreviewState::Empty,
            save_message: None,
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
        self.setup.is_ready()
    }

    /// Called whenever the document changes.
    pub fn mark_edited(&mut self) {
        self.dirty = true;
        self.last_edit = Some(Instant::now());
    }

    /// Forces an immediate render on the next frame, bypassing the skip for
    /// unchanged documents.
    pub fn request_render(&mut self) {
        self.dirty = true;
        self.last_edit = None;
        self.last_rendered_source = None;
    }

    /// Re-reads completed render results from the worker thread.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut result = None;
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                PreviewMsg::Rendered {
                    generation_id,
                    source,
                    result: r,
                } => {
                    if generation_id == self.generation {
                        result = Some((source, r));
                    }
                }
            }
        }

        if let Some((source, result)) = result {
            self.in_flight_cancel = None;
            match result {
                Ok((images, pdf)) => {
                    self.last_rendered_source = Some(source);
                    let textures = images
                        .iter()
                        .filter_map(|img| {
                            let (w, h) = (img.width() as usize, img.height() as usize);
                            if w == 0 || h == 0 {
                                return None;
                            }
                            let image = ColorImage::from_rgba_unmultiplied([w, h], img.as_raw());
                            let name = format!("preview-page-{}", w * h);
                            Some(RenderedPage {
                                texture: ctx.load_texture(name, image, TextureOptions::LINEAR),
                            })
                        })
                        .collect();
                    self.state = PreviewState::Rendered(RenderedDoc {
                        textures,
                        images,
                        pdf,
                    });
                }
                Err(message) if message == engine::CANCELLED => {
                    // Dropped because newer edits arrived; keep the previous
                    // state instead of surfacing a spurious error.
                }
                Err(message) => {
                    self.state = PreviewState::Failed(message);
                }
            }
        }
    }

    /// Kicks off a render when the debounce window has elapsed. A render that
    /// is already running gets cancelled when newer edits are waiting, so the
    /// preview never queues behind a stale compile.
    pub fn maybe_render(&mut self, source: &str, debounce: Duration) {
        if !self.enabled || !self.dirty {
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
            // Still typing: drop any stale render instead of letting it run
            // to completion.
            self.cancel_in_flight();
            return;
        }
        // The document is identical to the preview on screen; nothing to do.
        if matches!(self.state, PreviewState::Rendered(_))
            && self.last_rendered_source.as_deref() == Some(source)
        {
            self.dirty = false;
            return;
        }

        self.cancel_in_flight();
        self.dirty = false;
        self.generation += 1;
        let cancel = Arc::new(AtomicBool::new(false));
        self.in_flight_cancel = Some(cancel.clone());
        self.state = PreviewState::Rendering;

        let generation_id = self.generation;
        let source = source.to_string();
        let setup = self.setup.clone();
        let override_path = self.tectonic_override.clone();
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name("preview-render".to_string())
            .spawn(move || {
                let result =
                    raster::render_source(&setup, &source, override_path.as_deref(), Some(&cancel));
                let _ = tx.send(PreviewMsg::Rendered {
                    generation_id,
                    source,
                    result,
                });
            })
            .expect("spawn preview render thread");
    }

    fn cancel_in_flight(&mut self) {
        if let Some(cancel) = self.in_flight_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Render now").clicked() {
                self.request_render();
            }
            if ui
                .button("Save…")
                .on_hover_text("Save the rendered output as PNG, JPEG or PDF")
                .clicked()
            {
                self.save_rendered();
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
            checkbox(ui, &mut self.fit_width, "Fit width");
            checkbox(ui, &mut self.enabled, "Auto");
            if let Some((message, since)) = &self.save_message
                && since.elapsed() < Duration::from_secs(5)
            {
                ui.label(egui::RichText::new(message.as_str()).weak());
            } else {
                self.save_message = None;
            }
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
                    PreviewState::Rendered(doc) => {
                        let avail_width = ui.available_width();
                        for page in &doc.textures {
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

    /// Opens a save dialog and writes the rendered output in the format implied
    /// by the chosen file extension. PNG and JPEG write the first page image;
    /// PDF writes the compiled document.
    fn save_rendered(&mut self) {
        let Some(doc) = (match &self.state {
            PreviewState::Rendered(doc) => Some(doc),
            _ => None,
        }) else {
            self.save_message = Some(("Nothing to save yet.".to_string(), Instant::now()));
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("PNG", &["png"])
            .add_filter("JPEG", &["jpg", "jpeg"])
            .add_filter("PDF", &["pdf"])
            .set_file_name("render.png")
            .save_file()
        else {
            return; // cancelled
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .unwrap_or_default();
        let result: Result<(), String> = match ext.as_str() {
            "pdf" => std::fs::write(&path, &doc.pdf).map_err(|e| e.to_string()),
            "jpg" | "jpeg" => doc
                .images
                .first()
                .map(|img| {
                    image::DynamicImage::ImageRgba8(img.clone())
                        .to_rgb8()
                        .save(&path)
                        .map_err(|e| e.to_string())
                })
                .unwrap_or(Ok(())),
            "png" => doc
                .images
                .first()
                .map(|img| img.save(&path).map_err(|e| e.to_string()))
                .unwrap_or(Ok(())),
            _ => {
                self.save_message = Some((
                    "Choose a .png, .jpg or .pdf file name.".to_string(),
                    Instant::now(),
                ));
                return;
            }
        };
        let message = match result {
            Ok(()) => format!("Saved to {}", path.display()),
            Err(e) => format!("Save failed: {e}"),
        };
        self.save_message = Some((message, Instant::now()));
    }
}

/// A checkbox whose box and label are laid out together and centered as one
/// unit, so every checkbox in a toolbar row lines up vertically.
fn checkbox(ui: &mut egui::Ui, checked: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let response = ui.add(egui::Checkbox::without_text(checked));
        ui.label(egui::RichText::new(label));
        response
    })
    .inner
    .on_hover_cursor(egui::CursorIcon::PointingHand);
}
