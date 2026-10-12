//! Asset browser — the bottom-left dock: a folder tree and tile grid
//! over the REAL `AssetRegistry` contents.
//!
//! Every tile is an actual registry entry (FNV-hashed, versioned); the
//! kind badge comes from `AssetKind::as_str`. There is no fake
//! filesystem listing: the browser shows what the engine has imported,
//! and the phase report documents that an on-disk browser (watching
//! `res/` directly) is future work.

use crate::{shell::EditorState, theme};
use gens_assets::{AssetKind, AssetRegistry};

/// Bottom-left half of the bottom dock.
pub(crate) fn panel(ui: &mut egui::Ui, state: &mut EditorState, assets: &AssetRegistry) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Asset Browser").strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!("res:// · {} assets", assets.len()))
                    .color(theme::TEXT_DIM),
            );
        });
    });
    ui.separator();

    let entries: Vec<(String, AssetKind, usize, u32)> = assets
        .iter()
        .map(|(_, e)| (e.key.clone(), e.kind, e.size, e.version))
        .collect();

    ui.horizontal(|ui| {
        // Folder tree (left column of the browser).
        let folders = folders_from_keys(&entries);
        egui::ScrollArea::vertical()
            .id_salt("asset_tree")
            .max_width(150.0)
            .show(ui, |ui| {
                folder_row(ui, state, None, "res://");
                for name in folders {
                    folder_row(ui, state, Some(name.clone()), &name);
                }
            });
        ui.separator();
        // Tile grid (right side of the browser).
        egui::ScrollArea::vertical()
            .id_salt("asset_grid")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let visible: Vec<&(String, AssetKind, usize, u32)> = entries
                    .iter()
                    .filter(|(key, _, _, _)| {
                        state
                            .asset_folder
                            .as_ref()
                            .is_none_or(|folder| key.starts_with(&format!("{folder}/")))
                    })
                    .collect();
                if visible.is_empty() {
                    ui.label(
                        egui::RichText::new(
                            "No assets here. Import via the registry (the demo scene ships a few).",
                        )
                        .color(theme::TEXT_DIM),
                    );
                }
                egui::Grid::new("asset_tiles")
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        let mut in_row = 0usize;
                        for (key, kind, size, version) in visible {
                            if in_row == 4 {
                                ui.end_row();
                                in_row = 0;
                            }
                            tile(ui, state, key, *kind, *size, *version);
                            in_row += 1;
                        }
                    });
            });
    });
}

fn folder_row(ui: &mut egui::Ui, state: &mut EditorState, folder: Option<String>, label: &str) {
    let selected = state.asset_folder == folder;
    // Bullet is Latin-1 — guaranteed by the embedded font set (no emoji
    // dependency; deterministic glyphs across CI OSes).
    let glyph = if folder.is_none() { "»" } else { "•" };
    let text = egui::RichText::new(format!("{glyph} {label}"));
    let text = if selected {
        text.color(theme::TEXT)
    } else {
        text.color(theme::TEXT_DIM)
    };
    if ui.selectable_label(selected, text).clicked() {
        state.asset_folder = folder;
    }
}

fn tile(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    key: &str,
    kind: AssetKind,
    size: usize,
    version: u32,
) {
    let selected = state.selected_asset.as_deref() == Some(key);
    ui.vertical(|ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(84.0, 62.0), egui::Sense::click());
        let fill = if selected {
            theme::ROW_SELECTED
        } else {
            theme::SURFACE_BG
        };
        ui.painter().rect_filled(rect, 4.0, fill);
        if selected {
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(1.5, theme::ACCENT),
                egui::StrokeKind::Middle,
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            kind.as_str().to_uppercase(),
            egui::FontId::monospace(11.0),
            kind_color(kind),
        );
        ui.label(
            egui::RichText::new(display_name(key))
                .small()
                .color(if selected {
                    theme::TEXT
                } else {
                    theme::TEXT_DIM
                }),
        );
        ui.label(
            egui::RichText::new(format!("v{version} · {}", format_size(size)))
                .small()
                .color(theme::TEXT_DIM),
        );
        if response.clicked() {
            state.selected_asset = Some(key.to_string());
        }
    });
}

/// Deterministic per-kind accent color for the tile glyph.
pub fn kind_color(kind: AssetKind) -> egui::Color32 {
    match kind {
        AssetKind::GlTF => theme::GIZMO_AXES[2],     // blue
        AssetKind::Data => theme::SEVERITY[0],       // green
        AssetKind::Script => theme::SEVERITY[1],     // amber
        AssetKind::UiLayout => theme::GIZMO_AXES[0], // red
        AssetKind::Raw => theme::TEXT_DIM,           // gray
        // AssetKind is #[non_exhaustive] — future kinds render gray.
        _ => theme::TEXT_DIM,
    }
}

/// Unique top-level folders among the registry keys (sorted).
pub fn folders_from_keys(entries: &[(String, AssetKind, usize, u32)]) -> Vec<String> {
    let mut folders = std::collections::BTreeSet::new();
    for (key, ..) in entries {
        if let Some((dir, _)) = key.split_once('/') {
            folders.insert(dir.to_string());
        }
    }
    folders.into_iter().collect()
}

/// File name after the last `/` (the whole key when there is none).
pub fn display_name(key: &str) -> &str {
    key.rsplit('/').next().unwrap_or(key)
}

/// Bytes to a short human string (deterministic, no locale).
pub fn format_size(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str) -> (String, AssetKind, usize, u32) {
        (key.to_string(), AssetKind::Data, 100, 1)
    }

    #[test]
    fn folders_are_sorted_and_unique() {
        let entries = vec![
            entry("assets/walker.gltf"),
            entry("assets/models/ship.json"),
            entry("scenes/patrol.json"),
            entry("assets/blocks.gltf"),
        ];
        let folders = folders_from_keys(&entries);
        assert_eq!(folders, vec!["assets".to_string(), "scenes".to_string()]);
    }

    #[test]
    fn display_name_takes_the_file_name() {
        assert_eq!(display_name("assets/walker.gltf"), "walker.gltf");
        assert_eq!(display_name("root.json"), "root.json");
    }

    #[test]
    fn size_formatting_is_deterministic() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KiB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MiB");
    }
}
