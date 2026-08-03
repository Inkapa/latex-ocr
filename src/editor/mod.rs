//! The LaTeX source editor: a highlighted, line-numbered text area.

mod highlight;
pub mod snippet;

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::text::{CCursor, CCursorRange, LayoutJob};
use eframe::egui::{self, Color32, FontId, RichText, ScrollArea, TextEdit, TextStyle, Ui};

use crate::theme;

pub struct Editor {
    pub text: String,
    cursor: Option<CCursorRange>,
    pending_cursor: Option<CCursorRange>,
    pub file_path: Option<PathBuf>,
    pub saved_text: String,
    pub tab_insert: bool,
    /// Display name of the document, editable from the toolbar.
    pub doc_name: String,
}

impl Default for Editor {
    fn default() -> Self {
        Self::with_text(include_str!("sample.tex").to_string())
    }
}

impl Editor {
    pub fn with_text(text: String) -> Self {
        let saved = text.clone();
        Self {
            text,
            cursor: None,
            pending_cursor: None,
            file_path: None,
            saved_text: saved,
            tab_insert: true,
            doc_name: "Untitled".to_string(),
        }
    }

    pub fn dirty(&self) -> bool {
        self.text != self.saved_text
    }

    pub fn mark_saved(&mut self) {
        self.saved_text = self.text.clone();
    }

    pub fn reset_cursor(&mut self) {
        self.cursor = None;
        self.pending_cursor = None;
    }

    /// Character-range selection, if the user has one.
    pub fn selected_char_range(&self) -> Option<Range<usize>> {
        self.cursor
            .as_ref()
            .map(|c| c.as_sorted_char_range())
            .map(|r| Range {
                start: r.start.into(),
                end: r.end.into(),
            })
    }

    /// Inserts a snippet template at the cursor (or around the selection).
    pub fn insert_snippet(&mut self, template: &str) {
        let selection = self.selected_char_range().unwrap_or(0..0);
        let (cursor, _) = snippet::apply(&mut self.text, Some(selection), template);
        self.pending_cursor = Some(CCursorRange::one(CCursor::new(cursor)));
    }

    /// Inserts a math snippet at the cursor, wrapped in `$...$` when the cursor
    /// is not already inside a math context so the result stays compilable.
    pub fn insert_math_snippet(&mut self, template: &str) {
        let selection = self.selected_char_range().unwrap_or(0..0);
        let before = text_before(&self.text, selection.start);
        let tpl = if snippet::in_math_context(&before) {
            template.to_string()
        } else {
            format!("${template}$")
        };
        self.insert_snippet(&tpl);
    }

    /// Replaces the selection (or inserts at the cursor) with `content`.
    pub fn replace_selection(&mut self, content: &str) {
        let selection = self.selected_char_range().unwrap_or(0..0);
        let start = selection.start;
        let end = selection.end;
        let start_byte = char_to_byte(&self.text, start);
        let end_byte = char_to_byte(&self.text, end);
        self.text.replace_range(start_byte..end_byte, content);
        let cursor = start + content.chars().count();
        self.pending_cursor = Some(CCursorRange::one(CCursor::new(cursor)));
    }

    /// A short title for the current document.
    pub fn title(&self) -> String {
        let name = self.doc_name.clone();
        if self.dirty() {
            format!("{name} •")
        } else {
            name
        }
    }

    /// Renders the editor; returns `true` when the text changed this frame.
    pub fn show(&mut self, ui: &mut Ui) -> bool {
        // Tab indentation: insert two spaces instead of moving focus. This must
        // run before the TextEdit processes the key event.
        let tab_pressed = ui.input(|i| i.key_pressed(egui::Key::Tab));
        if tab_pressed && self.tab_insert {
            self.insert_snippet("  ");
        }

        let line_count = self.text.lines().count().max(1);
        let digits = line_count.to_string().len().max(2);
        let gutter_color = theme::DIM_TEXT;

        ScrollArea::both()
            .id_salt("editor_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let changed = ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for n in 1..=line_count {
                            ui.label(
                                RichText::new(format!("{n:>digits$}"))
                                    .monospace()
                                    .color(gutter_color),
                            );
                        }
                    });

                    let available = ui.available_width();
                    let desired_width = available.max(12_000.0);

                    let mut layouter = |ui: &Ui,
                                        text: &dyn egui::TextBuffer,
                                        wrap_width: f32|
                     -> Arc<egui::Galley> {
                        let mut job = LayoutJob::default();
                        job.wrap.max_width = wrap_width;
                        let font_id = TextStyle::Monospace.resolve(ui.style());
                        apply_highlight(text.as_str(), &font_id, &mut job);
                        ui.fonts_mut(|f| f.layout_job(job))
                    };

                    let mut output = TextEdit::multiline(&mut self.text)
                        .id_salt("editor_text")
                        .font(TextStyle::Monospace)
                        .desired_width(desired_width)
                        .desired_rows(line_count.max(24))
                        .lock_focus(true)
                        .text_color(theme::TOKEN_DEFAULT)
                        .background_color(Color32::TRANSPARENT)
                        .layouter(&mut layouter)
                        .show(ui);

                    self.cursor = output.cursor_range;
                    if self.cursor.is_none() {
                        // The text edit only reports a cursor while focused;
                        // keep the last one so insertions land at the cursor
                        // even after clicking a toolbar button.
                        if let Some(state) =
                            egui::TextEdit::load_state(ui.ctx(), output.response.id)
                        {
                            self.cursor =
                                Some(state.cursor.range(&output.galley).unwrap_or_default());
                        }
                    }
                    let mut changed = output.response.changed();
                    if let Some(pending) = self.pending_cursor.take() {
                        output.state.cursor.set_char_range(Some(pending));
                        output.state.store(ui.ctx(), output.response.id);
                        self.cursor = Some(pending);
                        changed = true; // programmatic edit via snippet/tab
                    }

                    changed
                });
                changed.inner
            })
            .inner
    }
}

fn char_to_byte(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

fn text_before(text: &str, char_idx: usize) -> String {
    let byte = char_to_byte(text, char_idx);
    text[..byte].to_string()
}

/// Fills a layout job with syntax-colored LaTeX spans.
pub fn apply_highlight(text: &str, font_id: &FontId, job: &mut LayoutJob) {
    use eframe::egui::text::TextFormat;
    use highlight::TokenKind;

    for (range, kind) in highlight::tokenize(text) {
        let color = match kind {
            TokenKind::Comment => theme::TOKEN_COMMENT,
            TokenKind::Command => theme::TOKEN_COMMAND,
            TokenKind::Keyword => theme::TOKEN_KEYWORD,
            TokenKind::Math => theme::TOKEN_MATH,
            TokenKind::Number => theme::TOKEN_NUMBER,
            TokenKind::Bracket => theme::TOKEN_BRACKET,
            TokenKind::Default => theme::TOKEN_DEFAULT,
        };
        job.append(
            &text[range],
            0.0,
            TextFormat {
                font_id: font_id.clone(),
                color,
                ..Default::default()
            },
        );
    }
}

pub fn snippet_menu(ui: &mut Ui, editor: &mut Editor, snippets: &[snippet::Snippet], math: bool) {
    for snippet in snippets {
        if ui.button(snippet.label).clicked() {
            if math {
                editor.insert_math_snippet(snippet.template);
            } else {
                editor.insert_snippet(snippet.template);
            }
            ui.close();
        }
    }
}

pub fn snippet_submenu(
    ui: &mut Ui,
    title: &str,
    editor: &mut Editor,
    snippets: &[snippet::Snippet],
    math: bool,
) {
    ui.menu_button(title, |ui| {
        snippet_menu(ui, editor, snippets, math);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::text::CCursor;

    #[test]
    fn dirty_tracking() {
        let mut e = Editor::with_text("hello".into());
        assert!(!e.dirty());
        e.text.push_str(" world");
        assert!(e.dirty());
        e.mark_saved();
        assert!(!e.dirty());
    }

    #[test]
    fn insert_snippet_uses_selection() {
        let mut e = Editor::with_text("x".into());
        e.cursor = Some(CCursorRange::two(CCursor::new(0), CCursor::new(1)));
        e.insert_snippet("$@s$@|");
        assert_eq!(e.text, "$x$");
        assert!(e.pending_cursor.is_some());
    }

    #[test]
    fn replace_selection_replaces_only_selection() {
        let mut e = Editor::with_text("abc".into());
        e.cursor = Some(CCursorRange::two(CCursor::new(1), CCursor::new(2)));
        e.replace_selection("ZZ");
        assert_eq!(e.text, "aZZc");
    }

    #[test]
    fn title_shows_filename_and_dirty_marker() {
        let mut e = Editor::with_text("t".into());
        assert_eq!(e.title(), "Untitled");
        e.doc_name = "My Notes".into();
        assert_eq!(e.title(), "My Notes");
        e.text.push_str(" change");
        assert_eq!(e.title(), "My Notes •");
    }
}
