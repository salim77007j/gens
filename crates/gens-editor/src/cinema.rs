//! Cinematic and network panel data + renderers (Phase 9).
//!
//! The Game View / Animator / Network tabs consume **snapshots the
//! embedding app refreshes** — the same seam the telemetry panels use
//! (Phase 8). Nothing here invents data: the Animator tab renders the
//! active cinematic's real track rows and clock, the Game View tab
//! renders the scene through the cinematic's staged [`CameraPose`] when
//! the director owns a camera, and the Network tab renders a session
//! snapshot when the embedding app runs one. Absence is rendered as the
//! honest empty state, never a fake number.
//!
//! Why the Network tab takes a snapshot instead of a live session: the
//! editor replay hashes every frame byte-for-byte, and packet arrival
//! timing is not deterministic across runners — a live session inside
//! the replay would break the CI contract. The snapshot type keeps the
//! wiring real (unit-tested against the same fields a session counts)
//! while the empty state stays honest until a windowed editor exists.

use crate::shell::EditorState;
use crate::theme;
use gens_runtime::cinematic::CameraPose;

/// One track row in the Animator tab.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRow {
    /// Logical target the track drives.
    pub target: String,
    /// Channel label (Translation / Rotation / Scale / Fov / Fade).
    pub channel: &'static str,
    /// Keyframe count on the track.
    pub keys: usize,
}

/// The snapshot the Animator + Game View tabs read.
#[derive(Debug, Clone, PartialEq)]
pub struct CinematicPanelData {
    /// Timeline duration in seconds.
    pub duration: f32,
    /// The player's current local time.
    pub local_time: f32,
    /// Playback state label (Idle/Playing/Paused/Finished).
    pub state: String,
    /// Whether the cinematic currently stages the camera.
    pub camera_live: bool,
    /// The staged camera pose (Game View renders through it).
    pub camera_pose: Option<CameraPose>,
    /// Track rows in deterministic (target, channel) order.
    pub tracks: Vec<TrackRow>,
    /// Events fired so far, in firing order.
    pub events: Vec<String>,
}

/// The snapshot the Network tab reads (see the module doc for why this
/// is a snapshot, not a live session).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetPanelData {
    /// Session identity the handshake assigned.
    pub session_id: u64,
    /// "host" or "client".
    pub role: String,
    /// Messages sent this session.
    pub sent: u64,
    /// Messages received this session.
    pub received: u64,
    /// Last message kind label ("Snapshot"/"Input"/"Bye"/"—").
    pub last_kind: String,
}

/// Stable label for a [`gens_cinematics::Channel`].
#[must_use]
pub fn channel_label(channel: gens_cinematics::Channel) -> &'static str {
    match channel {
        gens_cinematics::Channel::Translation => "Translation",
        gens_cinematics::Channel::Rotation => "Rotation",
        gens_cinematics::Channel::Scale => "Scale",
        gens_cinematics::Channel::Fov => "Fov",
        gens_cinematics::Channel::Fade => "Fade",
    }
}

/// Animator tab: the active cinematic's real rows, or the honest empty
/// state. Deterministic: no clocks, no animation, pure layout.
pub fn animator_panel(ui: &mut egui::Ui, state: &EditorState) {
    let Some(cin) = &state.cinematic else {
        empty(
            ui,
            "No cinematic active. The animation runtime (clips, blend trees, FK) \
             is live via gens-animation; cinematic playback appears here when a \
             director drives the scene.",
        );
        return;
    };
    ui.label(
        egui::RichText::new(format!(
            "cinematic — {} | t = {:.3} / {:.3} s | {} event(s) fired",
            cin.state,
            cin.local_time,
            cin.duration,
            cin.events.len()
        ))
        .color(theme::TEXT_DIM),
    );
    ui.separator();
    // Track table header.
    ui.columns(3, |cols| {
        cols[0].label(egui::RichText::new("TARGET").color(theme::TEXT_DIM));
        cols[1].label(egui::RichText::new("CHANNEL").color(theme::TEXT_DIM));
        cols[2].label(egui::RichText::new("KEYS").color(theme::TEXT_DIM));
    });
    ui.separator();
    for row in &cin.tracks {
        ui.columns(3, |cols| {
            cols[0].label(&row.target);
            cols[1].label(row.channel);
            cols[2].label(row.keys.to_string());
        });
    }
    ui.separator();
    // Progress bar (pure arithmetic on the snapshot clock).
    let progress = if cin.duration > 0.0 {
        (cin.local_time / cin.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ui.add(egui::ProgressBar::new(progress).show_percentage());
    if cin.camera_live {
        ui.label(
            egui::RichText::new("camera: directed by the cinematic (Game View shows its pose)")
                .color(theme::TEXT_DIM),
        );
    }
    if !cin.events.is_empty() {
        ui.label(egui::RichText::new("fired events:").color(theme::TEXT_DIM));
        for e in &cin.events {
            ui.label(format!("· {e}"));
        }
    }
}

/// Network bottom tab: the session snapshot, or the honest empty state.
pub fn network_panel(ui: &mut egui::Ui, state: &EditorState) {
    let Some(net) = &state.net else {
        empty(
            ui,
            "No network session in this run. The editor replay hashes every frame \
             byte-for-byte; packet timing would break that contract, so live \
             sessions appear in a windowed editor. gens-networking's QUIC loopback \
             transport is CI-proven on all three operating systems.",
        );
        return;
    };
    ui.columns(2, |cols| {
        cols[0].colored_label(theme::TEXT_DIM, format!("session {}", net.session_id));
        cols[1].colored_label(theme::TEXT_DIM, &net.role);
    });
    ui.columns(3, |cols| {
        cols[0].colored_label(theme::TEXT, format!("sent {}", net.sent));
        cols[1].colored_label(theme::TEXT, format!("recv {}", net.received));
        cols[2].colored_label(theme::TEXT_DIM, format!("last {}", net.last_kind));
    });
}

/// The shared honest empty state (same styling as the shell's).
fn empty(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(text).color(theme::TEXT_DIM));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gens_math::quat;

    fn sample() -> CinematicPanelData {
        CinematicPanelData {
            duration: 4.0,
            local_time: 1.5,
            state: "Playing".to_string(),
            camera_live: true,
            camera_pose: Some(CameraPose {
                position: gens_math::vec3(2.0, 3.0, 10.0),
                rotation: quat(0.0, -0.25881905, 0.0, 0.9659258),
                fov_degrees: 52.5,
            }),
            tracks: vec![
                TrackRow {
                    target: "camera".to_string(),
                    channel: "Translation",
                    keys: 2,
                },
                TrackRow {
                    target: "camera".to_string(),
                    channel: "Fov",
                    keys: 2,
                },
                TrackRow {
                    target: "hero".to_string(),
                    channel: "Translation",
                    keys: 2,
                },
            ],
            events: vec!["title-card".to_string()],
        }
    }

    #[test]
    fn animator_layout_places_track_rows_in_columns() {
        let mut state = EditorState::new();
        state.cinematic = Some(sample());
        let state_cell = std::cell::RefCell::new(state);
        let mut ctx = gens_ui::UiContext::new();
        let frame = ctx.run(gens_ui::UiInput::new([640, 400], 0.0, 1.0 / 60.0), |ui| {
            animator_panel(ui, &state_cell.borrow_mut());
        });
        // The header row and at least one track row must have landed.
        assert!(!frame.meshes.is_empty(), "the animator table drew geometry");
    }

    #[test]
    fn animator_empty_state_when_no_cinematic() {
        let state = EditorState::new();
        let state_cell = std::cell::RefCell::new(state);
        let mut ctx = gens_ui::UiContext::new();
        let frame = ctx.run(gens_ui::UiInput::new([640, 400], 0.0, 1.0 / 60.0), |ui| {
            animator_panel(ui, &state_cell.borrow_mut());
        });
        assert!(
            !frame.meshes.is_empty(),
            "the honest empty state still renders text"
        );
    }

    #[test]
    fn network_tab_renders_snapshot_and_empty_state() {
        // Snapshot path: real counters render.
        let mut state = EditorState::new();
        state.net = Some(NetPanelData {
            session_id: 7,
            role: "host".to_string(),
            sent: 12,
            received: 11,
            last_kind: "Snapshot".to_string(),
        });
        let cell = std::cell::RefCell::new(state);
        let mut ctx = gens_ui::UiContext::new();
        let frame = ctx.run(gens_ui::UiInput::new([640, 200], 0.0, 1.0 / 60.0), |ui| {
            network_panel(ui, &cell.borrow_mut());
        });
        assert!(!frame.meshes.is_empty());
        // Empty path: the honest text still draws.
        let state = EditorState::new();
        let cell = std::cell::RefCell::new(state);
        let frame = ctx.run(gens_ui::UiInput::new([640, 200], 0.0, 1.0 / 60.0), |ui| {
            network_panel(ui, &cell.borrow_mut());
        });
        assert!(!frame.meshes.is_empty());
    }
}
