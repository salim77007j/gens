//! Developer console: tracing ring buffer + profiler/performance tabs — implemented in P7-6; this placeholder keeps the shell
//! compiling while the phase lands panel by panel.

use crate::shell::EditorState;

pub(crate) fn panel(ui: &mut egui::Ui, _state: &mut EditorState) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new("(P7-6) not landed yet — see the phase 7 report")
                .color(crate::theme::TEXT_DIM),
        );
    });
}
