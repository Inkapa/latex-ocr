//! Visual styling and fonts for the whole application.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, TextStyle, Vec2};

pub const ACCENT: Color32 = Color32::from_rgb(0x56, 0xC2, 0xB6);
pub const ACCENT_STRONG: Color32 = Color32::from_rgb(0x3A, 0x9D, 0x90);
pub const BG: Color32 = Color32::from_rgb(0x1E, 0x1E, 0x24);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x25, 0x25, 0x2C);
pub const EDITOR_BG: Color32 = Color32::from_rgb(0x1B, 0x1B, 0x20);
pub const DIM_TEXT: Color32 = Color32::from_rgb(0x8A, 0x8A, 0x94);

// Syntax colors.
pub const TOKEN_DEFAULT: Color32 = Color32::from_rgb(0xE4, 0xE4, 0xE8);
pub const TOKEN_COMMENT: Color32 = Color32::from_rgb(0x7F, 0xA8, 0x6F);
pub const TOKEN_COMMAND: Color32 = Color32::from_rgb(0xC6, 0x78, 0xDD);
pub const TOKEN_MATH: Color32 = Color32::from_rgb(0x56, 0xB6, 0xC2);
pub const TOKEN_NUMBER: Color32 = Color32::from_rgb(0xD1, 0x9A, 0x66);
pub const TOKEN_KEYWORD: Color32 = Color32::from_rgb(0xE5, 0xC0, 0x7B);
pub const TOKEN_BRACKET: Color32 = Color32::from_rgb(0x9D, 0xCB, 0xF5);

const DEJA_VU_SANS: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
const DEJA_VU_MONO: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");

pub fn install(cc: &eframe::CreationContext<'_>) {
    install_fonts(&cc.egui_ctx);
    set_style(&cc.egui_ctx);
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "dejavu".to_owned(),
        FontData::from_static(DEJA_VU_SANS).into(),
    );
    fonts.font_data.insert(
        "dejavu_mono".to_owned(),
        FontData::from_static(DEJA_VU_MONO).into(),
    );

    fonts.families.insert(
        FontFamily::Proportional,
        vec![
            "dejavu".to_owned(),
            "Ubuntu-Light".to_owned(),
            "NotoEmoji-Regular".to_owned(),
        ],
    );
    fonts.families.insert(
        FontFamily::Monospace,
        vec!["dejavu_mono".to_owned(), "Ubuntu-Light".to_owned()],
    );
    ctx.set_fonts(fonts);
}

fn set_style(ctx: &egui::Context) {
    let theme = egui::Theme::Dark;
    let base = ctx.style_of(theme);
    let mut style = base.as_ref().clone();

    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.window_margin = egui::Margin::same(10);
    style.spacing.menu_margin = egui::Margin::same(6);

    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
    style.text_styles.insert(
        TextStyle::Monospace,
        FontId::new(14.0, FontFamily::Monospace),
    );
    style.text_styles.insert(
        TextStyle::Small,
        FontId::new(12.0, FontFamily::Proportional),
    );

    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL_BG;
    visuals.faint_bg_color = Color32::from_rgb(0x1A, 0x1A, 0x20);
    visuals.extreme_bg_color = Color32::from_rgb(0x12, 0x12, 0x16);
    visuals.interact_cursor = Some(egui::CursorIcon::PointingHand);
    visuals.widgets.noninteractive.bg_fill = Color32::TRANSPARENT;
    visuals.widgets.noninteractive.weak_bg_fill = Color32::from_rgb(0x2C, 0x2C, 0x34);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x30, 0x30, 0x38);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x30, 0x30, 0x38);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(0x3A, 0x3A, 0x44);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x38, 0x38, 0x42);
    visuals.widgets.active.bg_fill = ACCENT_STRONG;
    visuals.widgets.active.weak_bg_fill = ACCENT_STRONG;
    // Flat, rectangular buttons so toolbar items read as a single strip.
    visuals.widgets.noninteractive.corner_radius = 0.into();
    visuals.widgets.inactive.corner_radius = 0.into();
    visuals.widgets.hovered.corner_radius = 0.into();
    visuals.widgets.active.corner_radius = 0.into();
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(0x56, 0xC2, 0xB6, 80);
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.window_stroke = egui::Stroke::new(1.0, Color32::from_rgb(0x3A, 0x3A, 0x44));
    visuals.window_corner_radius = 6.into();

    ctx.set_style_of(theme, style);
    ctx.set_visuals_of(theme, visuals);
}
