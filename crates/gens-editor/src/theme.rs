//! Editor visual theme — the dark, docked-panel look of the reference
//! design (`docs/design/REFERENCE.png`, archived from the project brief).
//!
//! Every color lives here so all panels draw from one source of truth.
//! All values are compile-time constants (no OS theming, no wall-clock
//! dependent styling) so a headless editor replay produces identical
//! pixels across runs — the Phase 7 determinism contract.

use egui::{Color32, Context, CornerRadius, Stroke, Vec2};

/// Editor chrome background (menu bar, toolbar, status bar).
pub const CHROME_BG: Color32 = Color32::from_rgb(12, 14, 19);
/// The same chrome color as an RGBA8 clear value (window-target clear).
pub const CHROME_CLEAR: [u8; 4] = [12, 14, 19, 255];
/// Dock background (hierarchy, inspector, asset browser, console).
pub const PANEL_BG: Color32 = Color32::from_rgb(17, 20, 26);
/// Raised surface (tiles, hovered rows, input fields).
pub const SURFACE_BG: Color32 = Color32::from_rgb(24, 28, 36);
/// Accent blue — selection stroke, active tab underline, toggles.
pub const ACCENT: Color32 = Color32::from_rgb(56, 132, 244);
/// Selected row fill in the hierarchy / active tab pill.
pub const ROW_SELECTED: Color32 = Color32::from_rgb(45, 84, 145);
/// Primary text.
pub const TEXT: Color32 = Color32::from_rgb(214, 219, 228);
/// Dimmed text (secondary labels, counts, unit suffixes).
pub const TEXT_DIM: Color32 = Color32::from_rgb(130, 140, 155);
/// Hairline separators between docks and sections.
pub const BORDER: Color32 = Color32::from_rgb(38, 44, 55);
/// Viewport clear color (the 3D scene behind the gizmo).
pub const VIEWPORT_CLEAR: [u8; 4] = [24, 28, 36, 255];

/// Console severity colors (`INFO`, `WARN`, `ERROR`, `DEBUG` in that
/// order) — mirrored by the reference console panel.
pub const SEVERITY: [Color32; 4] = [
    Color32::from_rgb(126, 231, 135), // info: green
    Color32::from_rgb(244, 202, 88),  // warn: amber
    Color32::from_rgb(244, 96, 96),   // error: red
    Color32::from_rgb(130, 140, 155), // debug: gray
];

/// Gizmo axis colors (X red, Y green, Z blue — matching the reference
/// viewport's transform gizmo).
pub const GIZMO_AXES: [Color32; 3] = [
    Color32::from_rgb(224, 74, 74),
    Color32::from_rgb(96, 200, 96),
    Color32::from_rgb(74, 124, 244),
];

/// Applies the dark editor theme to the egui context.
///
/// Called once per shell construction (idempotent per pass, but there is
/// no reason to re-apply: the style object is owned by the shell).
pub fn apply(ctx: &Context) {
    // The editor is a dark-theme app (reference look); pin the theme
    // explicitly so a default-light context can never leak in.
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = PANEL_BG;
    v.window_fill = SURFACE_BG;
    v.extreme_bg_color = CHROME_BG;
    v.override_text_color = Some(TEXT);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(4);
    style.spacing.item_spacing = Vec2::new(6.0, 4.0);
    style.spacing.button_padding = Vec2::new(8.0, 3.0);
    style.spacing.indent = 12.0;

    v.widgets.noninteractive.bg_fill = SURFACE_BG;
    v.widgets.noninteractive.weak_bg_fill = SURFACE_BG;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(3);

    v.widgets.inactive.bg_fill = SURFACE_BG;
    v.widgets.inactive.weak_bg_fill = SURFACE_BG;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.corner_radius = CornerRadius::same(3);

    v.widgets.hovered.bg_fill = SURFACE_BG;
    v.widgets.hovered.weak_bg_fill = SURFACE_BG;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.hovered.corner_radius = CornerRadius::same(3);

    v.widgets.active.bg_fill = ROW_SELECTED;
    v.widgets.active.weak_bg_fill = ROW_SELECTED;
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.active.corner_radius = CornerRadius::same(3);

    v.selection.bg_fill = ROW_SELECTED;
    v.selection.stroke = Stroke::new(1.0, ACCENT);

    ctx.set_style_of(egui::Theme::Dark, style);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The theme application must be deterministic: applying it twice to
    /// fresh contexts yields identical style snapshots (pinned so a
    /// future "load theme from file" feature cannot silently drift).
    #[test]
    fn theme_is_deterministic() {
        let a = Context::default();
        apply(&a);
        let b = Context::default();
        apply(&b);
        let sa = a.style_of(egui::Theme::Dark);
        let sb = b.style_of(egui::Theme::Dark);
        assert_eq!(sa.visuals.panel_fill, sb.visuals.panel_fill);
        assert_eq!(sa.visuals.selection.bg_fill, sb.visuals.selection.bg_fill);
        assert_eq!(
            sa.visuals.widgets.active.fg_stroke,
            sb.visuals.widgets.active.fg_stroke
        );
        // The look is dark with the reference chrome color.
        assert!(sa.visuals.dark_mode);
        assert_eq!(sa.visuals.panel_fill, PANEL_BG);
        assert_eq!(sa.visuals.selection.bg_fill, ROW_SELECTED);
    }
}
