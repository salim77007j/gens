//! Inspector — reflected components (Name, Active, Transform, MeshSlot,
//! AuthoredBody, ScriptComponent); implemented in P7-3.

use crate::shell::EditorState;
use gens_ecs::prelude::World;

pub(crate) fn panel(ui: &mut egui::Ui, _state: &mut EditorState, _world: &mut World) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new("(P7-3) inspector not landed yet — see the phase 7 report")
                .color(crate::theme::TEXT_DIM),
        );
    });
}
