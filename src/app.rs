//! Application state and top-level layout.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use image::RgbaImage;

use crate::editor;
use crate::editor::Editor;
use crate::editor::snippet;
use crate::ocr::beautify;
use crate::ocr::local::LocalOcrBackend;
use crate::ocr::provision;
use crate::ocr::{HttpOcrBackend, OcrBackend, OcrError};
use crate::preview::{Preview, RenderSetup};
use crate::settings::OcrEngine;
use crate::settings::SettingsStore;
use crate::snapshot;
use crate::snip;
use crate::theme;

/// Pending destructive action that needs the user's blessing.
#[derive(Clone, Copy, PartialEq)]
enum DiscardAction {
    Close,
    New,
    Open,
}

impl DiscardAction {
    fn prompt(&self) -> &'static str {
        match self {
            Self::Close => "Close the current document?",
            Self::New => "Start a new document?",
            Self::Open => "Open another file?",
        }
    }
}

enum SnipState {
    Idle,
    /// Capturing the desktop snapshot before the overlay opens.
    Preparing(mpsc::Receiver<Result<Arc<Mutex<snip::OverlayState>>, String>>),
    Overlay(Arc<Mutex<snip::OverlayState>>),
    /// Capturing the selected region off the UI thread.
    Capturing(mpsc::Receiver<Result<RgbaImage, String>>),
}

struct OcrDialog {
    image: RgbaImage,
    texture: Option<TextureHandle>,
    latex: String,
    error: Option<String>,
    busy: bool,
    rx: Receiver<Result<String, OcrError>>,
}

pub struct LatexOcrApp {
    editor: Editor,
    preview: Preview,
    settings: SettingsStore,
    ocr: Arc<dyn OcrBackend>,
    snip: SnipState,
    ocr_dialog: Option<OcrDialog>,
    show_settings: bool,
    discard_pending: Option<DiscardAction>,
    status: String,
    status_since: Option<Instant>,
    engine_message: String,
    engine_message_slot: Option<Arc<std::sync::Mutex<String>>>,
    current_ctx: Option<egui::Context>,
    ocr_url_edit: String,
    ocr_backend_edit: OcrEngine,
    ocr_model_dir_edit: String,
    ocr_beautify_edit: bool,
    debounce_ms_edit: u32,
    tectonic_edit: String,
    renaming: bool,
    rename_before: String,
    focus_rename: bool,
    ocr_test_result: Option<String>,
    prepare_started: Option<Instant>,
}

impl LatexOcrApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(cc);

        let settings = SettingsStore::load();
        let setup = Arc::new(RenderSetup::new());
        let ocr_url = settings.get().ocr_url.clone();
        let ocr = Self::build_ocr(settings.get());

        let mut app = Self {
            editor: Editor::default(),
            preview: Preview::new(setup),
            settings,
            ocr,
            snip: SnipState::Idle,
            ocr_dialog: None,
            show_settings: false,
            discard_pending: None,
            status: String::new(),
            status_since: None,
            engine_message: String::new(),
            engine_message_slot: None,
            current_ctx: None,
            ocr_url_edit: ocr_url,
            ocr_backend_edit: OcrEngine::Local,
            ocr_model_dir_edit: String::new(),
            ocr_beautify_edit: true,
            debounce_ms_edit: 600,
            tectonic_edit: String::new(),
            renaming: false,
            rename_before: String::new(),
            focus_rename: false,
            ocr_test_result: None,
            prepare_started: None,
        };
        app.sync_edits_from_settings();
        app.prewarm_engine();
        app.preview.request_render();
        app.set_status("Welcome. Type LaTeX on the left, or use \"Snip & OCR\" to capture math from the screen.");
        // Screen capture initializes a Direct3D device on first use; warm it
        // up in the background so the first snip opens without that delay.
        std::thread::Builder::new()
            .name("capture-warmup".into())
            .spawn(|| {
                let _ = snapshot::capture_virtual_desktop();
            })
            .expect("spawn capture warmup thread");
        app
    }

    /// Builds the OCR backend selected in the settings. The local backend
    /// provisions its models lazily on the first recognition.
    fn build_ocr(settings: &crate::settings::Settings) -> Arc<dyn OcrBackend> {
        if settings.ocr_backend == OcrEngine::Local {
            let dir = settings
                .ocr_model_dir
                .as_deref()
                .map(PathBuf::from)
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| {
                    // Portable builds carry the models next to the executable;
                    // development builds use the app data directory.
                    if provision::bundled_models_dir().is_dir() {
                        provision::bundled_models_dir()
                    } else {
                        provision::ocr_dir()
                    }
                });
            Arc::new(LocalOcrBackend::new(dir))
        } else {
            Arc::new(HttpOcrBackend::new(settings.ocr_url.clone()))
        }
    }

    fn sync_edits_from_settings(&mut self) {
        let s = self.settings.get();
        self.debounce_ms_edit = s.preview_debounce_ms as u32;
        self.tectonic_edit = s.tectonic_path.clone().unwrap_or_default();
        self.ocr_backend_edit = s.ocr_backend;
        self.ocr_model_dir_edit = s.ocr_model_dir.clone().unwrap_or_default();
        self.ocr_beautify_edit = s.ocr_beautify;
    }

    fn prewarm_engine(&mut self) {
        self.engine_message = "Preparing LaTeX engine (first run downloads Tectonic)…".into();
        let setup = self.preview.engine_handle();
        let override_path = self.tectonic_edit.trim().to_string();
        let override_path = if override_path.is_empty() {
            None
        } else {
            Some(override_path)
        };
        let message = Arc::new(std::sync::Mutex::new(String::new()));
        let message_shared = message.clone();
        self.engine_message_slot = Some(message);
        std::thread::Builder::new()
            .name("engine-prewarm".into())
            .spawn(move || {
                let result = setup.get(override_path.as_deref());
                let text = match result {
                    Ok(_) => "engine ready".to_string(),
                    Err(e) => format!("engine setup failed: {e}"),
                };
                *message_shared.lock().unwrap() = text;
            })
            .expect("spawn engine prewarm thread");
    }

    fn poll_engine(&mut self) {
        if let Some(slot) = &self.engine_message_slot {
            let text = slot.lock().unwrap().clone();
            if !text.is_empty() {
                self.engine_message = text;
                self.engine_message_slot = None;
            }
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_since = Some(Instant::now());
    }

    fn status_expired(&self) -> bool {
        self.status_since
            .map(|t| t.elapsed() > Duration::from_secs(8))
            .unwrap_or(false)
    }

    fn handle_snip(&mut self, ctx: &egui::Context) {
        match &self.snip {
            SnipState::Overlay(overlay) => {
                let mut guard = overlay.lock().unwrap();
                // Safety net: if the overlay window went away without an
                // outcome (closed by the OS), cancel so the app never stays
                // stuck in a snip.
                if guard.rendered_once
                    && guard.outcome.is_none()
                    && !ctx.input(|i| i.raw.viewports.contains_key(&guard.viewport_id))
                {
                    guard.outcome = Some(snip::OverlayOutcome::Cancelled);
                }
                let outcome = guard.outcome;
                drop(guard);

                match outcome {
                    Some(snip::OverlayOutcome::Captured(region)) => {
                        self.start_capture(region);
                    }
                    Some(snip::OverlayOutcome::Cancelled) => {
                        self.snip = SnipState::Idle;
                        self.set_status("Capture cancelled.");
                    }
                    None => {
                        // The overlay only lives while we keep showing it, so
                        // keep waking the root until the selection finishes.
                        ctx.request_repaint_after(Duration::from_millis(16));
                    }
                }
            }
            SnipState::Capturing(rx) => {
                // Poll for a finished background capture, treating a dead
                // capture thread (it panicked without sending) as a failure so
                // the app does not stay stuck in the "Capturing" state.
                let ready = match rx.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("screen capture aborted".to_string()))
                    }
                };
                if let Some(result) = ready {
                    match result {
                        Ok(image) => {
                            self.snip = SnipState::Idle;
                            self.open_ocr_dialog(ctx, image);
                        }
                        Err(message) => {
                            self.snip = SnipState::Idle;
                            self.set_status(format!("Screen capture failed: {message}"));
                        }
                    }
                }
            }
            SnipState::Preparing(rx) => match rx.try_recv() {
                Ok(Ok(state)) => {
                    self.prepare_started = None;
                    self.snip = SnipState::Overlay(state);
                    self.set_status("Drag over the math to select it.  Esc to cancel.");
                }
                Ok(Err(message)) => {
                    self.prepare_started = None;
                    self.snip = SnipState::Idle;
                    self.set_status(format!("Screen capture failed: {message}"));
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(16));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.prepare_started = None;
                    self.snip = SnipState::Idle;
                    self.set_status("Screen capture failed.");
                }
            },
            SnipState::Idle => {
                // Safety net: close any overlay windows left over from an
                // earlier snip that failed to tear down.
                let strays: Vec<egui::ViewportId> = ctx.input(|i| {
                    i.raw
                        .viewports
                        .keys()
                        .copied()
                        .filter(|id| *id != egui::ViewportId::ROOT)
                        .collect()
                });
                if !strays.is_empty() {
                    log::debug!("closing stale viewports: {strays:?}");
                    for id in strays {
                        ctx.send_viewport_cmd_to(id, egui::ViewportCommand::Close);
                    }
                }
            }
        }
    }

    /// Captures the selected region off the UI thread and polls for the result.
    fn start_capture(&mut self, region: snapshot::Region) {
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("snip-capture".into())
            .spawn(move || {
                let result = snapshot::capture_region(region);
                let _ = tx.send(result);
            })
            .expect("spawn snip capture thread");
        self.snip = SnipState::Capturing(rx);
        self.set_status("Capturing screen…");
    }

    fn open_ocr_dialog(&mut self, ctx: &egui::Context, image: RgbaImage) {
        let (w, h) = (image.width() as usize, image.height() as usize);
        let texture = ctx.load_texture(
            "ocr-capture",
            ColorImage::from_rgba_unmultiplied([w, h], &image.clone().into_raw()),
            TextureOptions::LINEAR,
        );
        let mut dialog = OcrDialog {
            image,
            texture: Some(texture),
            latex: String::new(),
            error: None,
            busy: true,
            rx: mpsc::channel().1,
        };
        start_ocr(&self.ocr, &mut dialog);
        self.ocr_dialog = Some(dialog);
    }

    fn poll_ocr(&mut self) {
        let Some(dialog) = &mut self.ocr_dialog else {
            return;
        };
        if !dialog.busy {
            return;
        }
        match dialog.rx.try_recv() {
            Ok(Ok(latex)) => {
                dialog.busy = false;
                dialog.latex = if self.settings.get().ocr_beautify {
                    beautify::beautify(&latex)
                } else {
                    latex
                };
            }
            Ok(Err(e)) => {
                dialog.busy = false;
                dialog.error = Some(e.message);
            }
            Err(_) => {}
        }
    }

    // ----- file operations -----

    fn open_file_dialog(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("LaTeX", &["tex", "ltx", "latex"])
            .add_filter("Text", &["txt", "md"])
            .pick_file();
        if let Some(path) = picked {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    self.editor.file_path = Some(path.clone());
                    self.editor.doc_name = path
                        .file_stem()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or("Untitled".to_string());
                    self.editor.text = content;
                    self.editor.mark_saved();
                    self.preview.request_render();
                    self.set_status(format!("Opened {}", path.display()));
                }
                Err(e) => self.set_status(format!("Cannot open file: {e}")),
            }
        }
    }

    fn save_file(&mut self) -> bool {
        if let Some(path) = self.editor.file_path.clone() {
            match std::fs::write(&path, &self.editor.text) {
                Ok(()) => {
                    self.editor.mark_saved();
                    self.set_status(format!("Saved {}", path.display()));
                    true
                }
                Err(e) => {
                    self.set_status(format!("Cannot save file: {e}"));
                    false
                }
            }
        } else {
            self.save_file_as()
        }
    }

    fn save_file_as(&mut self) -> bool {
        // The save dialog is pre-filled with the document name.
        let mut default_name = self.editor.doc_name.trim().to_string();
        if default_name.is_empty() {
            default_name = "Untitled".to_string();
        }
        if !default_name.to_lowercase().ends_with(".tex") {
            default_name.push_str(".tex");
        }
        let path = rfd::FileDialog::new()
            .add_filter("LaTeX", &["tex"])
            .set_file_name(&default_name)
            .save_file();
        if let Some(path) = path {
            match std::fs::write(&path, &self.editor.text) {
                Ok(()) => {
                    self.editor.file_path = Some(path.clone());
                    self.editor.doc_name = path
                        .file_stem()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or(self.editor.doc_name.clone());
                    self.editor.mark_saved();
                    self.set_status(format!("Saved {}", path.display()));
                    true
                }
                Err(e) => {
                    self.set_status(format!("Cannot save file: {e}"));
                    false
                }
            }
        } else {
            false
        }
    }

    // ----- UI sections -----

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("toolbar").show(ui, |ui| {
            // Buttons sit flush against the toolbar background and only light
            // up on hover or press, so the strip reads as a single element.
            {
                let visuals = ui.visuals_mut();
                visuals.widgets.inactive.bg_fill = egui::Color32::TRANSPARENT;
                visuals.widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
            }
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                if ui.button("New").clicked() {
                    if self.editor.dirty() {
                        self.discard_pending = Some(DiscardAction::New);
                    } else {
                        self.new_document();
                    }
                }
                if ui.button("Open…").clicked() {
                    if self.editor.dirty() {
                        self.discard_pending = Some(DiscardAction::Open);
                    } else {
                        self.open_file_dialog();
                    }
                }
                if ui.button("Save").clicked() {
                    self.save_file();
                }
                if ui.button("Save as…").clicked() {
                    self.save_file_as();
                }

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                ui.menu_button("Insert", |ui| {
                    if ui.button("Inline math").clicked() {
                        self.editor.insert_snippet(snippet::INLINE_MATH.template);
                        ui.close();
                    }
                    if ui.button("Display math").clicked() {
                        self.editor.insert_snippet(snippet::DISPLAY_MATH.template);
                        ui.close();
                    }
                    ui.separator();
                    editor::snippet_submenu(ui, "Math", &mut self.editor, snippet::MATH, true);
                    editor::snippet_submenu(
                        ui,
                        "Structure",
                        &mut self.editor,
                        snippet::STRUCTURE,
                        false,
                    );
                    editor::snippet_submenu(
                        ui,
                        "Text",
                        &mut self.editor,
                        snippet::TEXT_TOOLS,
                        false,
                    );
                    editor::snippet_submenu(ui, "Greek", &mut self.editor, snippet::GREEK, true);
                });

                ui.add_space(6.0);
                ui.separator();
                ui.add_space(6.0);

                let snip_button = egui::Button::new(
                    egui::RichText::new("Snip & OCR")
                        .strong()
                        .color(egui::Color32::from_rgb(0x06, 0x1A, 0x18)),
                )
                .fill(theme::ACCENT);
                if ui
                    .add(snip_button)
                    .on_hover_text("Capture math from the screen and turn it into LaTeX")
                    .clicked()
                {
                    self.start_snip(ui.ctx());
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if ui.button("Settings").clicked() {
                        self.show_settings = true;
                    }
                    ui.separator();
                    self.rename_widget(ui);
                });
            });
        });
    }

    /// The editable document name shown in the toolbar.
    fn rename_widget(&mut self, ui: &mut egui::Ui) {
        if self.renaming {
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.editor.doc_name)
                    .desired_width(180.0)
                    .hint_text("Document name"),
            );
            if self.focus_rename {
                response.request_focus();
                self.focus_rename = false;
            }
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            let commit = response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter));
            if escape {
                self.editor.doc_name = std::mem::take(&mut self.rename_before);
                self.renaming = false;
            } else if commit {
                self.renaming = false;
            }
        } else {
            let title = self.editor.title();
            let response = ui
                .add(egui::Button::new(egui::RichText::new(title).weak().monospace()).frame(true))
                .on_hover_cursor(egui::CursorIcon::Text);
            if response.clicked() {
                self.rename_before = self.editor.doc_name.clone();
                self.renaming = true;
                self.focus_rename = true;
            }
        }
    }

    fn start_snip(&mut self, ctx: &egui::Context) {
        if !matches!(self.snip, SnipState::Idle) {
            self.set_status("A capture is already in progress.");
            return;
        }
        let alive: Vec<_> = ctx.input(|i| {
            i.raw
                .viewports
                .keys()
                .copied()
                .filter(|id| *id != egui::ViewportId::ROOT)
                .collect()
        });
        if !alive.is_empty() {
            log::debug!("snip start: {alive:?} still alive");
        }
        let monitor = match snapshot::monitor_at_cursor() {
            Ok(monitor) => monitor,
            Err(message) => {
                self.set_status(format!("Screen capture unavailable: {message}"));
                return;
            }
        };
        // The desktop snapshot is captured and uploaded on a background thread
        // so the overlay opens already fully painted. The status message gives
        // feedback during the brief capture.
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("snip-background".into())
            .spawn(move || {
                let started = std::time::Instant::now();
                let result = snapshot::capture_virtual_desktop().map(|desktop| {
                    log::debug!("desktop snapshot captured in {:?}", started.elapsed());
                    snip::OverlayState::begin(monitor, Some(desktop), &ctx)
                });
                let _ = tx.send(result);
            })
            .expect("spawn snip background thread");
        self.snip = SnipState::Preparing(rx);
        self.prepare_started = Some(Instant::now());
        self.set_status("Preparing screen capture…");
    }

    fn editor_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("editor_panel")
            .resizable(true)
            .default_size(480.0)
            .min_size(300.0)
            .max_size(1100.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::EDITOR_BG)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| {
                if self.editor.show(ui) {
                    self.preview.mark_edited();
                }
            });
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::PANEL_BG)
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ui, |ui| {
                self.preview.show(ui);
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let show_status = !self.status.is_empty() && !self.status_expired();
                let text = if show_status {
                    self.status.clone()
                } else {
                    self.engine_message.clone()
                };
                ui.label(egui::RichText::new(text).weak().small());
                if matches!(self.snip, SnipState::Preparing(_)) {
                    let elapsed = self
                        .prepare_started
                        .map(|t| t.elapsed())
                        .unwrap_or_default();
                    let progress = (elapsed.as_secs_f32() / 0.7).clamp(0.0, 1.0);
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .desired_width(140.0)
                            .fill(theme::ACCENT),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.engine_message_slot.is_some() {
                        ui.add(egui::Spinner::new().size(12.0));
                    } else if !self.preview.engine_ready()
                        && ui
                            .button(egui::RichText::new("Retry engine setup").small())
                            .clicked()
                    {
                        self.preview.reset_engine();
                        self.prewarm_engine();
                    }
                });
            });
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_settings;
        let mut saved = false;
        let mut cancelled = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.heading("OCR");
                ui.label("Engine:");
                ui.horizontal(|ui| {
                    ui.selectable_value(
                        &mut self.ocr_backend_edit,
                        OcrEngine::Local,
                        "Local (ONNX)",
                    );
                    ui.selectable_value(
                        &mut self.ocr_backend_edit,
                        OcrEngine::Http,
                        "Server (pix2tex)",
                    );
                });
                ui.add_space(4.0);
                if self.ocr_backend_edit == OcrEngine::Local {
                    ui.label("Model directory (leave empty for default):");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ocr_model_dir_edit)
                            .desired_width(340.0),
                    );
                    let help = if provision::bundled_models_dir().is_dir() {
                        "The OCR models ship with this build; ONNX Runtime is loaded from the app folder."
                    } else {
                        "Runs fully offline once the models are prepared. Run `python tools/export_onnx.py` to create them; the ONNX Runtime library downloads automatically on first use."
                    };
                    ui.label(egui::RichText::new(help).weak());
                } else {
                    ui.label("pix2tex (LaTeX-OCR) server URL:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ocr_url_edit).desired_width(340.0),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Test connection").clicked() {
                            let backend = HttpOcrBackend::new(self.ocr_url_edit.trim());
                            let ok = backend.is_reachable();
                            self.ocr_test_result = Some(if ok {
                                "Server is reachable".to_string()
                            } else {
                                "Server did not respond".to_string()
                            });
                        }
                        if let Some(result) = &self.ocr_test_result {
                            let color = if result.contains("reachable") {
                                egui::Color32::from_rgb(0x7F, 0xA8, 0x6F)
                            } else {
                                egui::Color32::from_rgb(0xE0, 0x6C, 0x75)
                            };
                            ui.colored_label(color, result);
                        }
                    });
                }
                ui.add_space(8.0);

                ui.checkbox(&mut self.ocr_beautify_edit, "Beautify recognized LaTeX")
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                ui.add_space(8.0);

                ui.heading("Preview");
                ui.horizontal(|ui| {
                    ui.label("Auto-render delay:");
                    ui.add(egui::Slider::new(&mut self.debounce_ms_edit, 100..=2000).suffix(" ms"));
                });
                ui.add_space(8.0);

                ui.heading("LaTeX engine");
                ui.label("tectonic executable (leave empty for auto):");
                ui.add(egui::TextEdit::singleline(&mut self.tectonic_edit).desired_width(340.0));
                ui.add_space(12.0);

                ui.heading("About");
                ui.label(format!("LaTeX OCR {}", env!("CARGO_PKG_VERSION")));
                ui.label("Created by Liam CORNU");
                ui.hyperlink("https://github.com/Inkapa/latex-ocr");
                ui.add_space(12.0);

                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        saved = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if saved {
            self.apply_settings();
            self.show_settings = false;
            self.ocr_test_result = None;
        } else if cancelled {
            self.sync_edits_from_settings();
            self.show_settings = false;
            self.ocr_test_result = None;
        } else {
            self.show_settings = open;
            if !open {
                self.ocr_test_result = None;
            }
        }
    }

    fn apply_settings(&mut self) {
        {
            let s = self.settings.get_mut();
            s.ocr_backend = self.ocr_backend_edit;
            s.ocr_url = self.ocr_url_edit.trim().to_string();
            let model_dir = self.ocr_model_dir_edit.trim().to_string();
            s.ocr_model_dir = if model_dir.is_empty() {
                None
            } else {
                Some(model_dir)
            };
            s.ocr_beautify = self.ocr_beautify_edit;
            s.preview_debounce_ms = self.debounce_ms_edit as u64;
            let tectonic = self.tectonic_edit.trim().to_string();
            s.tectonic_path = if tectonic.is_empty() {
                None
            } else {
                Some(tectonic)
            };
        }

        if let Err(e) = self.settings.save() {
            self.set_status(format!("Settings not saved: {e}"));
            return;
        }

        let s = self.settings.get();
        self.ocr = Self::build_ocr(s);
        self.preview.set_tectonic_override(s.tectonic_path.clone());
        self.preview.reset_engine();
        self.prewarm_engine();
        self.set_status("Settings saved.");
    }

    fn new_document(&mut self) {
        self.editor.file_path = None;
        self.editor.doc_name = "Untitled".to_string();
        self.editor.text = "% LaTeX OCR\n\n".to_string();
        self.editor.mark_saved();
        self.editor.reset_cursor();
        self.preview.request_render();
        self.set_status("New document.");
    }

    fn discard_dialog(&mut self, ctx: &egui::Context) {
        let Some(action) = self.discard_pending else {
            return;
        };
        let mut resolved = None;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(action.prompt());
                ui.label("Save your changes first?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        resolved = Some(if self.save_file() {
                            DiscardChoice::Proceed
                        } else {
                            DiscardChoice::Stay
                        });
                    }
                    if ui.button("Discard").clicked() {
                        resolved = Some(DiscardChoice::Proceed);
                    }
                    if ui.button("Cancel").clicked() {
                        resolved = Some(DiscardChoice::Stay);
                    }
                });
            });

        if let Some(choice) = resolved {
            self.discard_pending = None;
            if choice == DiscardChoice::Proceed {
                match action {
                    DiscardAction::Close => {
                        // Mark the document as not-dirty so the close is not
                        // re-intercepted and cancelled again.
                        self.editor.mark_saved();
                        self.ctx_close();
                    }
                    DiscardAction::New => self.new_document(),
                    DiscardAction::Open => self.open_file_dialog(),
                }
            }
        }
    }

    /// Sends a close request to the window manager.
    fn ctx_close(&self) {
        if let Some(ctx) = self.current_ctx.clone() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ocr_window(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.ocr_dialog else {
            return;
        };

        let mut open = true;
        let mut close = false;
        let mut action: Option<OcrAction> = None;
        egui::Window::new("OCR result")
            .open(&mut open)
            .collapsible(false)
            .resize(|r| {
                r.resizable([true, false]) // width only; height follows content
                    .min_size(egui::vec2(300.0, 120.0))
                    .max_size(egui::vec2(1200.0, 1600.0))
            })
            .default_width(460.0)
            .show(ctx, |ui| {
                if let Some(texture) = &dialog.texture {
                    let native = texture.size_vec2();
                    let max_w = 420.0;
                    let scale = (max_w / native.x).min(1.0);
                    ui.add(egui::Image::new(texture).fit_to_exact_size(native * scale));
                }
                ui.add_space(8.0);

                if dialog.busy {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(16.0));
                        ui.label("Recognizing…");
                    });
                } else {
                    if let Some(error) = &dialog.error {
                        ui.colored_label(egui::Color32::from_rgb(0xE0, 0x6C, 0x75), error);
                        ui.add_space(4.0);
                    }
                    let mut layouter = |ui: &egui::Ui,
                                        text: &dyn egui::TextBuffer,
                                        wrap_width: f32|
                     -> std::sync::Arc<egui::Galley> {
                        let mut job = egui::text::LayoutJob::default();
                        job.wrap.max_width = wrap_width;
                        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
                        editor::apply_highlight(text.as_str(), &font_id, &mut job);
                        ui.fonts_mut(|f| f.layout_job(job))
                    };
                    ui.add(
                        egui::TextEdit::multiline(&mut dialog.latex)
                            .font(egui::TextStyle::Monospace)
                            .desired_rows(5)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Insert at cursor").clicked() {
                            action = Some(OcrAction::Insert(dialog.latex.clone()));
                            close = true;
                        }
                        if ui.button("Copy").clicked() {
                            action = Some(OcrAction::Copy(dialog.latex.clone()));
                        }
                        if ui.button("OCR again").clicked() {
                            action = Some(OcrAction::Retry);
                        }
                    });
                }
            });

        if !open {
            close = true;
        }

        match action {
            Some(OcrAction::Insert(latex)) => {
                let before = self.text_before_cursor();
                self.editor
                    .replace_selection(&wrap_ocr_result(&latex, &before));
                self.set_status("OCR result inserted.");
            }
            Some(OcrAction::Copy(latex)) => {
                ctx.copy_text(latex);
                self.set_status("OCR result copied to clipboard.");
            }
            Some(OcrAction::Retry) => {
                if let Some(dialog) = &mut self.ocr_dialog {
                    start_ocr(&self.ocr, dialog);
                }
            }
            None => {}
        }

        if close {
            self.ocr_dialog = None;
        }
    }

    /// Document text up to the cursor, used to detect the surrounding math
    /// context before inserting an OCR result.
    fn text_before_cursor(&self) -> String {
        let cursor = self
            .editor
            .selected_char_range()
            .map(|r| r.start)
            .unwrap_or(0);
        let byte = self
            .editor
            .text
            .char_indices()
            .nth(cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.editor.text.len());
        self.editor.text[..byte].to_string()
    }
}

/// Renders the OCR result ready for insertion: wrapped in inline or display
/// math when the cursor is not already inside a math context.
fn wrap_ocr_result(latex: &str, before_cursor: &str) -> String {
    if snippet::in_math_context(before_cursor) {
        latex.to_string()
    } else if latex.contains("\\begin{") || latex.contains('\n') {
        format!("\\[\n{latex}\n\\]")
    } else {
        format!("${latex}$")
    }
}

fn start_ocr(backend: &Arc<dyn OcrBackend>, dialog: &mut OcrDialog) {
    let (tx, rx) = mpsc::channel();
    let backend = backend.clone();
    let image = dialog.image.clone();
    dialog.rx = rx;
    dialog.busy = true;
    dialog.error = None;
    std::thread::Builder::new()
        .name("ocr-worker".into())
        .spawn(move || {
            let result = backend.recognize(&image);
            let _ = tx.send(result);
        })
        .expect("spawn ocr thread");
}

#[derive(Clone, Copy, PartialEq)]
enum DiscardChoice {
    Proceed,
    Stay,
}

enum OcrAction {
    Insert(String),
    Copy(String),
    Retry,
}

impl eframe::App for LatexOcrApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.current_ctx = Some(ctx.clone());
        self.poll_engine();
        self.handle_snip(ctx);
        self.poll_ocr();
        self.preview.poll(ctx);
        if self.preview.enabled {
            let debounce = Duration::from_millis(self.settings.get().preview_debounce_ms);
            self.preview.maybe_render(&self.editor.text, debounce);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Intercept window close so a dirty document is not lost silently.
        if ctx.input(|i| i.viewport().close_requested()) && self.editor.dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.discard_pending = Some(DiscardAction::Close);
        }

        self.toolbar(ui);
        self.editor_panel(ui);
        self.status_bar(ui);
        self.preview_panel(ui);

        if self.show_settings {
            self.settings_window(&ctx);
        }
        self.discard_dialog(&ctx);
        self.ocr_window(&ctx);

        // The snip overlay is its own transparent window on top of the desktop.
        if let SnipState::Overlay(state) = &self.snip {
            snip::show_viewport(&ctx, state);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Fully transparent so the snip overlay shows only what it paints.
        // The main window is covered entirely by opaque panels.
        [0.0, 0.0, 0.0, 0.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_inline_result_in_dollar_signs() {
        assert_eq!(wrap_ocr_result("x^{2}", ""), "$x^{2}$");
        assert_eq!(
            wrap_ocr_result("E=mc^{2}", "\\section{Physics}\n"),
            "$E=mc^{2}$"
        );
    }

    #[test]
    fn wraps_multi_line_results_in_display_math() {
        assert_eq!(wrap_ocr_result("a+b\nc+d", ""), "\\[\na+b\nc+d\n\\]");
        assert!(wrap_ocr_result("\\begin{array}{l}a\\\\b\\end{array}", "").starts_with("\\["));
    }

    #[test]
    fn does_not_wrap_inside_math_context() {
        assert_eq!(wrap_ocr_result("y", "$x"), "y");
        assert_eq!(wrap_ocr_result("y", "\\begin{equation}\nx"), "y");
    }

    #[test]
    fn detects_open_math_context() {
        assert!(snippet::in_math_context("$x"));
        assert!(snippet::in_math_context("\\begin{align}\nx &="));
        assert!(!snippet::in_math_context(""));
        assert!(!snippet::in_math_context("$x$"));
        assert!(!snippet::in_math_context(
            "\\begin{align}\nx &= y \\end{align}\n"
        ));
        assert!(!snippet::in_math_context(r"the \$ price"));
    }
}
