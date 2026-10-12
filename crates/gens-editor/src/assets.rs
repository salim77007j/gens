//! Asset browser: res:// tree + tile grid over the real registry — implemented in P7-5; this placeholder keeps the shell
//! compiling while the phase lands panel by panel.

use crate::shell::EditorState;

pub(crate) fn panel(ui: &mut egui::Ui, _state: &mut EditorState) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new("(P7-5) not landed yet — see the phase 7 report")
                .color(crate::theme::TEXT_DIM),
        );
    });
}
