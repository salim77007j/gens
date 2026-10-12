//! 3D viewport: offscreen scene render, orbit camera, gizmo overlay — implemented in P7-4; this placeholder keeps the shell
//! compiling while the phase lands panel by panel.

use crate::shell::EditorState;

pub(crate) fn panel(ui: &mut egui::Ui, _state: &mut EditorState) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new("(P7-4) not landed yet — see the phase 7 report")
                .color(crate::theme::TEXT_DIM),
        );
    });
}
