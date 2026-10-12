//! Scene hierarchy — live ECS tree with search, visibility and
//! selection; implemented in P7-2.

use crate::shell::EditorState;
use gens_ecs::prelude::World;

pub(crate) fn panel(ui: &mut egui::Ui, _state: &mut EditorState, _world: &mut World) {
    ui.centered_and_justified(|ui| {
        ui.label(
            egui::RichText::new("(P7-2) hierarchy not landed yet — see the phase 7 report")
                .color(crate::theme::TEXT_DIM),
        );
    });
}
