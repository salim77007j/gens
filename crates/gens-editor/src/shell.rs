//! The editor shell: docked panel layout, menu bar, view tabs and the
//! status bar — the chrome that surrounds every other panel.
//!
//! Layout follows `docs/design/REFERENCE.png`:
//!
//! ```text
//! ┌─ menu bar ────────────────────────────────────────────────┐
//! ├─ hierarchy ──┬─ view tabs / toolbar / viewport ───────────┤
//! │             │                                             │
//! ├─────────────┴───────────────┬─────────────────────────────┤
//! │ asset browser               │ developer console            │
//! ├─────────────────────────────┴─────────────────────────────┤
//! └─ status bar ──────────────────────────────────────────────┘
//! ```
//!
//! The shell owns intent, not execution: menu commands queue in
//! [`EditorState::pending`] for the embedding app to drain and act on
//! (the headless example executes them; a future window shell would
//! too). Docks can be toggled from the View menu — that is real state,
//! persisted nowhere (the editor is session-scoped, like the reference).

use crate::{
    console::{ConsoleBuffer, ConsoleSink},
    model::{CreateKind, EditorCommand, UndoOp},
    theme,
    viewport::{EditorViewport, ViewportDrag},
};
use gens_assets::AssetRegistry;
use gens_ecs::prelude::*;
use gens_renderer::{GpuContext, Renderer};
use gens_ui::{UiInput, UiRenderer};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Which center-dock tab is active (reference: 3D Viewport / Game View /
/// Animator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewTab {
    /// The live 3D viewport (P7-4).
    Viewport3d,
    /// The game camera view — Phase 8 scope, honest empty state.
    GameView,
    /// The animation graph editor — Phase 9 scope, honest empty state.
    Animator,
}

impl ViewTab {
    /// Tab label exactly as in the reference design.
    pub fn label(self) -> &'static str {
        match self {
            ViewTab::Viewport3d => "3D Viewport",
            ViewTab::GameView => "Game View",
            ViewTab::Animator => "Animator",
        }
    }

    /// Tabs in reference order.
    pub const ALL: [ViewTab; 3] = [ViewTab::Viewport3d, ViewTab::GameView, ViewTab::Animator];
}

/// Which bottom-dock tab is active (reference: Console / Profiler /
/// Performance / Network).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BottomTab {
    /// Tracing-captured log lines (P7-6).
    #[default]
    Console,
    /// Engine frame profile (P7-6).
    Profiler,
    /// Live counters (P7-6).
    Performance,
    /// Net session stats — future work, honest empty state.
    Network,
}

impl BottomTab {
    /// Tab label exactly as in the reference design.
    pub fn label(self) -> &'static str {
        match self {
            BottomTab::Console => "Console",
            BottomTab::Profiler => "Profiler",
            BottomTab::Performance => "Performance",
            BottomTab::Network => "Network",
        }
    }

    /// Tabs in reference order.
    pub const ALL: [BottomTab; 4] = [
        BottomTab::Console,
        BottomTab::Profiler,
        BottomTab::Performance,
        BottomTab::Network,
    ];
}

/// Deterministic status-bar counters. Wall-clock timings are
/// deliberately absent: they would differ between runs and break the
/// byte-identical replay contract (ADR-0025).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FrameStats {
    /// Editor frames rendered so far.
    pub frame: u64,
    /// Simulation steps advanced so far.
    pub step: u64,
    /// fps derived from the deterministic input clock (1/dt), not
    /// measured wall time.
    pub fps: f32,
    /// Live entity count (real query).
    pub entities: usize,
}

/// Simulation playback state (v1.1 play mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayMode {
    /// Editing: simulation idle (propagation still runs so edits show).
    #[default]
    Idle,
    /// Simulation steps at the fixed rate, wall-clock driven.
    Playing,
    /// Stepping suspended; the live state is kept (resume with Play).
    Paused,
}

impl PlayMode {
    /// Status-bar label.
    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Idle => "Edit",
            PlayMode::Playing => "Playing",
            PlayMode::Paused => "Paused",
        }
    }
}

/// All session state the panels read and write.
#[derive(Debug, Clone, Default)]
pub struct EditorState {
    /// Current selection (drives hierarchy highlight + inspector).
    pub selected: Option<Entity>,
    /// Hierarchy search filter text.
    pub search: String,
    /// Entities the user explicitly collapsed in the hierarchy (trees
    /// open by default — the reference editor behavior).
    pub collapsed: BTreeSet<Entity>,
    /// Active center-dock tab.
    pub view_tab: Option<ViewTab>,
    /// Active bottom-dock tab.
    pub bottom_tab: BottomTab,
    /// Dock visibility (View menu toggles).
    pub show_hierarchy: bool,
    pub show_inspector: bool,
    pub show_bottom_dock: bool,
    /// Deterministic counters for the status bar.
    pub stats: FrameStats,
    /// Orbit camera parameters (scripted-replayable camera state).
    pub orbit: crate::viewport::OrbitCam,
    /// Viewport render size in pixels (fed back from the widget rect).
    pub viewport_size_px: [u32; 2],
    /// Hierarchy row rects captured from layout, by node name — the
    /// scripted-replay seam for real UI clicks (P4-6 lesson: probe
    /// layout-captured rects, never hardcoded pixel guesses).
    pub row_rects: BTreeMap<String, egui::Rect>,
    /// Commands queued by the menu bar awaiting execution.
    pub pending: Vec<EditorCommand>,
    /// Asset browser: the folder filter (None = all).
    pub asset_folder: Option<String>,
    /// Asset browser: the selected asset key.
    pub selected_asset: Option<String>,
    /// Console ring buffer (tracing events land here).
    pub console: Arc<ConsoleBuffer>,
    /// Rolling profiler telemetry (app-updated, panels read).
    pub telemetry: crate::telemetry::Telemetry,
    /// Console command input (the command row's text field).
    pub command_input: String,
    /// Console commands queued by the command row, drained by the app —
    /// the same intent-queue pattern as [`EditorCommand`] (the shell
    /// never executes world-level work itself).
    pub pending_commands: Vec<String>,
    /// Play-mode state (v1.1): Edit / Playing / Paused.
    pub play_mode: PlayMode,
    /// Simulation clock accumulated while playing (seconds).
    pub play_time: f64,
    /// The pre-play world snapshot (captured on Play, consumed on Stop).
    pub play_snapshot: Option<crate::playmode::WorldSnapshot>,
    /// Undo history (most recent last), applied by the app via
    /// [`crate::commands::apply_undo`]. Cleared on scene load and on
    /// play-mode Stop (entity ids die in both).
    pub undo_stack: Vec<UndoOp>,
    /// Live viewport drag state (object move vs camera orbit), written
    /// by the viewport panel, consumed by the app after the drag ends
    /// (the undo op is recorded there).
    pub viewport_drag: Option<ViewportDrag>,
    /// Live cinematic snapshot the Animator + Game View tabs read
    /// (None = no cinematic; the tabs render honest empty states).
    pub cinematic: Option<crate::cinema::CinematicPanelData>,
    /// Network session snapshot the Network tab reads (None = no live
    /// session — see [`crate::cinema`] for why it is a snapshot).
    pub net: Option<crate::cinema::NetPanelData>,
}

impl EditorState {
    /// Default dock layout: everything visible, viewport active.
    pub fn new() -> Self {
        Self {
            view_tab: Some(ViewTab::Viewport3d),
            bottom_tab: BottomTab::Console,
            show_hierarchy: true,
            show_inspector: true,
            show_bottom_dock: true,
            orbit: crate::viewport::OrbitCam::default(),
            viewport_size_px: crate::viewport::DEFAULT_SIZE,
            console: Arc::new(ConsoleBuffer::new()),
            ..Default::default()
        }
    }

    /// Drains queued menu commands (the app executes them).
    pub fn take_commands(&mut self) -> Vec<EditorCommand> {
        std::mem::take(&mut self.pending)
    }

    /// Drains queued console commands (the app executes them against the
    /// world — see [`crate::commands`]).
    pub fn take_command_inputs(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending_commands)
    }
}

/// The full editor chrome: UI renderer + window target + state.
pub struct EditorShell {
    /// egui core + wgpu UI backend over the window target.
    pub ui: UiRenderer,
    /// The full-window offscreen target the UI paints onto.
    window_renderer: Renderer,
    /// The dedicated 3D viewport renderer (ADR-0025: two targets).
    pub viewport: EditorViewport,
    /// All panel/session state.
    pub state: EditorState,
}

impl EditorShell {
    /// Builds the shell around a fresh window renderer.
    ///
    /// # Errors
    /// Propagates wgpu device/target creation failures.
    pub fn new(window_ctx: GpuContext, width: u32, height: u32) -> gens_core::anyhow::Result<Self> {
        let window_renderer = Renderer::new(window_ctx, width, height)?;
        let viewport = EditorViewport::new()?;
        let mut ui = UiRenderer::new();
        theme::apply(ui.core_mut().ctx());
        Ok(Self {
            ui,
            window_renderer,
            viewport,
            state: EditorState::new(),
        })
    }

    /// The console tracing sink — install it as a subscriber layer
    /// BEFORE the first engine event so the console captures everything.
    pub fn console_sink(&self) -> ConsoleSink {
        ConsoleSink::new(self.state.console.clone())
    }

    /// Resizes the window target (viewport resize is independent).
    ///
    /// # Errors
    /// Propagates target re-creation failures.
    pub fn resize_window(&mut self, width: u32, height: u32) -> gens_core::anyhow::Result<()> {
        Ok(self.window_renderer.resize(width, height)?)
    }

    /// One full editor pass: paints every panel over the window target
    /// and returns the final RGBA pixels (the artifact of record).
    ///
    /// # Errors
    /// Propagates UI render failures (texture upload / readback).
    pub fn frame(
        &mut self,
        input: UiInput,
        world: &mut World,
        assets: &AssetRegistry,
    ) -> gens_core::anyhow::Result<Vec<u8>> {
        let clear = theme::CHROME_CLEAR;
        let EditorShell {
            ui,
            window_renderer,
            state,
            viewport,
        } = self;
        // Deterministic clock into the console + status-bar counters.
        state.console.set_time(input.time_seconds);
        state.stats.fps = if input.dt_seconds > 0.0 {
            1.0 / input.dt_seconds
        } else {
            0.0
        };
        state.stats.entities = world.iter_entities().count();
        // The 3D viewport renders FIRST (own target); its pixels become
        // an egui texture; then the whole editor UI paints over them.
        // Game View renders the SAME scene through the cinematic camera
        // when a directed camera pose exists (ADR-0030's staging rule);
        // otherwise it shows the orbit view like the 3D tab.
        let game_view_pose = (state.view_tab == Some(ViewTab::GameView))
            .then(|| state.cinematic.as_ref().and_then(|c| c.camera_pose))
            .flatten();
        match game_view_pose {
            Some(pose) => viewport.render_game_view(ui.core_mut().ctx(), state, world, &pose)?,
            None => viewport.render(ui.core_mut().ctx(), state, world)?,
        }
        // UiRenderer::render takes `impl Fn` (one paint call per pass, no
        // re-entrant borrows) — the same constraint the Phase 4 HUD met.
        // State and world travel through RefCells so panels can mutate
        // the REAL world mid-pass; both come back out afterwards.
        let state_cell = RefCell::new(std::mem::take(state));
        let world_cell = RefCell::new(std::mem::take(world));
        let viewport_ref: &EditorViewport = viewport;
        let render = ui.render(window_renderer, clear, input, |ui| {
            paint(
                ui,
                &mut state_cell.borrow_mut(),
                &mut world_cell.borrow_mut(),
                assets,
                viewport_ref,
            );
        })?;
        *state = state_cell.into_inner();
        *world = world_cell.into_inner();
        state.stats.frame += 1;
        Ok(render.pixels)
    }
}

/// Paints one whole editor frame into the root UI.
fn paint(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    world: &mut World,
    assets: &AssetRegistry,
    viewport: &EditorViewport,
) {
    menu_bar(ui, state);
    play_toolbar(ui, state);
    status_bar(ui, state);
    if state.show_bottom_dock {
        bottom_dock(ui, state, assets);
    }
    if state.show_hierarchy {
        crate::hierarchy::panel(ui, state, world);
    }
    if state.show_inspector {
        crate::inspector::panel(ui, state, world);
    }
    center_dock(ui, state, viewport, world);
}

/// Top menu bar (File / Edit / GameObject / View / Help) with real
/// keyboard shortcuts. Items queue real commands; decoration-only
/// items are deliberately absent.
fn menu_bar(ui: &mut egui::Ui, state: &mut EditorState) {
    // Keyboard shortcuts run BEFORE the menus: they work whether or not
    // a menu is open. Typing-focused keys (Delete, undo, duplicate) are
    // suppressed while a text field has focus; save/load stay global.
    let typing = ui.ctx().egui_wants_keyboard_input();
    ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
            state
                .pending
                .push(EditorCommand::SaveScene(default_scene_path()));
        }
        if input.consume_key(egui::Modifiers::COMMAND, egui::Key::O) {
            state
                .pending
                .push(EditorCommand::LoadScene(default_scene_path()));
        }
        if !typing {
            if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
                state.pending.push(EditorCommand::Undo);
            }
            if input.consume_key(egui::Modifiers::COMMAND, egui::Key::D) {
                state.pending.push(EditorCommand::DuplicateSelected);
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Delete) {
                state.pending.push(EditorCommand::DeleteSelected);
            }
        }
    });

    egui::Panel::top("menu_bar")
        .exact_size(24.0)
        .frame(
            egui::Frame::default()
                .fill(theme::CHROME_BG)
                .inner_margin(egui::Margin::symmetric(6, 2)),
        )
        .show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    let save = ui
                        .add(egui::Button::new("Save Scene").shortcut_text("Ctrl+S"))
                        .clicked();
                    let load = ui
                        .add(egui::Button::new("Load Scene").shortcut_text("Ctrl+O"))
                        .clicked();
                    if save {
                        state
                            .pending
                            .push(EditorCommand::SaveScene(default_scene_path()));
                        ui.close();
                    }
                    if load {
                        state
                            .pending
                            .push(EditorCommand::LoadScene(default_scene_path()));
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let has_selection = state.selected.is_some();
                    if ui
                        .add_enabled(true, egui::Button::new("Undo").shortcut_text("Ctrl+Z"))
                        .clicked()
                    {
                        state.pending.push(EditorCommand::Undo);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new("Duplicate Selected").shortcut_text("Ctrl+D"),
                        )
                        .clicked()
                    {
                        state.pending.push(EditorCommand::DuplicateSelected);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            has_selection,
                            egui::Button::new("Delete Selected").shortcut_text("Del"),
                        )
                        .clicked()
                    {
                        state.pending.push(EditorCommand::DeleteSelected);
                        ui.close();
                    }
                });
                ui.menu_button("GameObject", |ui| {
                    let mut created: Option<CreateKind> = None;
                    if ui.button("Create Empty").clicked() {
                        created = Some(CreateKind::Empty);
                    }
                    if ui.button("Create Cube").clicked() {
                        created = Some(CreateKind::Cube);
                    }
                    if ui.button("Create Marker").clicked() {
                        created = Some(CreateKind::Marker);
                    }
                    ui.separator();
                    let has_selection = state.selected.is_some();
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Delete Selected"))
                        .clicked()
                    {
                        created = None;
                        state.pending.push(EditorCommand::DeleteSelected);
                        ui.close();
                    }
                    if let Some(kind) = created {
                        state.pending.push(EditorCommand::CreateObject(kind));
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.toggle_value(&mut state.show_hierarchy, "Scene Hierarchy");
                    ui.toggle_value(&mut state.show_inspector, "Inspector");
                    ui.toggle_value(&mut state.show_bottom_dock, "Bottom Dock");
                });
                ui.menu_button("Help", |ui| {
                    ui.label(egui::RichText::new("Gens Engine — visual editor (v1.1)").strong());
                    ui.separator();
                    ui.label(egui::RichText::new(
                        "Click an object to select; drag it to move.\nDrag empty space to orbit; Ctrl+S save; Ctrl+O load;\nCtrl+Z undo; Ctrl+D duplicate; Del delete.",
                    ));
                    ui.label(
                        egui::RichText::new("see docs/design/REFERENCE.png").color(theme::TEXT_DIM),
                    );
                });
            });
        });
}

/// The deterministic default path the save/load items use (the editor
/// is session-scoped; no file dialogs in v1.1 — ADR-0036 documents the
/// boundary).
fn default_scene_path() -> String {
    "scene.gensscene".to_string()
}

/// The play toolbar (v1.1): Play / Pause / Stop / Save / Load buttons
/// plus the live mode + simulation clock. Buttons queue real commands
/// — execution stays with the app (the shell owns intent).
fn play_toolbar(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::Panel::top("play_toolbar")
        .exact_size(30.0)
        .frame(
            egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(egui::Margin::symmetric(8, 4)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let playing = state.play_mode == PlayMode::Playing;
                let paused = state.play_mode == PlayMode::Paused;
                if ui
                    .add_enabled(!playing, egui::Button::new("Play"))
                    .clicked()
                {
                    state.pending.push(EditorCommand::Play);
                }
                if ui
                    .add_enabled(playing, egui::Button::new("Pause"))
                    .clicked()
                {
                    state.pending.push(EditorCommand::Pause);
                }
                if ui
                    .add_enabled(playing || paused, egui::Button::new("Stop"))
                    .clicked()
                {
                    state.pending.push(EditorCommand::Stop);
                }
                ui.separator();
                if ui.button("Save").clicked() {
                    state
                        .pending
                        .push(EditorCommand::SaveScene(default_scene_path()));
                }
                if ui.button("Load").clicked() {
                    state
                        .pending
                        .push(EditorCommand::LoadScene(default_scene_path()));
                }
                ui.separator();
                let mode_color = match state.play_mode {
                    PlayMode::Idle => theme::TEXT_DIM,
                    PlayMode::Playing => theme::SEVERITY[0],
                    PlayMode::Paused => theme::GIZMO_AXES[2],
                };
                ui.label(
                    egui::RichText::new(format!(
                        "{}  t={:.2}s",
                        state.play_mode.label(),
                        state.play_time
                    ))
                    .color(mode_color),
                );
            });
        });
}

/// Bottom status bar: readiness dot + deterministic counters + the
/// observation-only CPU/Memory summary (Phase 8 telemetry; GPU stays an
/// honest omission, ADR-0027).
fn status_bar(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::Panel::bottom("status_bar")
        .exact_size(22.0)
        .frame(
            egui::Frame::default()
                .fill(theme::CHROME_BG)
                .inner_margin(egui::Margin::symmetric(8, 3)),
        )
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("●").color(theme::SEVERITY[0]));
                ui.label(egui::RichText::new("Ready").color(theme::TEXT));
                ui.separator();
                ui.label(
                    egui::RichText::new(format!(
                        "Frame {} · Step {} · {:.1} fps · {} entities",
                        state.stats.frame, state.stats.step, state.stats.fps, state.stats.entities
                    ))
                    .color(theme::TEXT_DIM),
                );
                if let Some(summary) = state.telemetry.status_summary() {
                    ui.separator();
                    ui.label(egui::RichText::new(summary).color(theme::TEXT_DIM));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new("gens editor · offscreen-first").color(theme::TEXT_DIM),
                    );
                });
            });
        });
}

/// Bottom dock: asset browser (left) + developer console (right),
/// split like the reference.
fn bottom_dock(ui: &mut egui::Ui, state: &mut EditorState, assets: &AssetRegistry) {
    egui::Panel::bottom("bottom_dock")
        .exact_size(230.0)
        .resizable(false)
        .frame(
            egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(egui::Margin::same(4)),
        )
        .show(ui, |ui| {
            // Two equal columns: asset browser left, console right.
            // (The previous manual allocate_ui split mis-placed both
            // panels — content landed in the wrong half and ~110 px too
            // low, pinned by the headless geometry test below.)
            ui.columns(2, |columns| {
                crate::assets::panel(&mut columns[0], state, assets);
                crate::console::panel(&mut columns[1], state, assets);
            });
        });
}

/// Center dock: view tabs row, then the active tab's content.
fn center_dock(
    ui: &mut egui::Ui,
    state: &mut EditorState,
    viewport: &EditorViewport,
    world: &mut World,
) {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(egui::Margin::same(4)),
        )
        .show(ui, |ui| {
            // Tab row (the reference's "3D Viewport | Game View | Animator").
            ui.horizontal(|ui| {
                for tab in ViewTab::ALL {
                    let active = state.view_tab == Some(tab);
                    if ui.selectable_label(active, tab.label()).clicked() {
                        state.view_tab = Some(tab);
                    }
                }
            });
            ui.separator();
            match state.view_tab {
                Some(ViewTab::Viewport3d) => {
                    viewport.panel(ui, state, world);
                }
                Some(ViewTab::GameView) => viewport.game_view_panel(ui, state),
                Some(ViewTab::Animator) => crate::cinema::animator_panel(ui, state),
                None => empty_state(ui, "No view tab active — pick one above."),
            }
        });
}

/// An honest empty-state placeholder for out-of-scope tabs.
fn empty_state(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(egui::RichText::new(text).color(theme::TEXT_DIM));
    });
}

#[cfg(test)]
mod shell_tests {
    use super::*;
    use gens_ui::UiContext;

    /// The bottom dock's two columns must place the console text in the
    /// RIGHT half near the dock top, inside the dock bounds. The first
    /// CI snapshot (run #90) exposed the manual allocate_ui split
    /// mis-placing content — this test pins the fix.
    #[test]
    fn bottom_dock_columns_place_console_content_correctly() {
        let state = EditorState::new();
        state.console.push(crate::console::ConsoleLine {
            time_seconds: 0.0,
            level: tracing::Level::INFO,
            target: "t".into(),
            message: "engine initialized".into(),
        });
        let assets = AssetRegistry::new();
        let state_cell = std::cell::RefCell::new(state);
        let mut ctx = UiContext::new();
        let frame = ctx.run(gens_ui::UiInput::new([1600, 230], 0.0, 1.0 / 60.0), |ui| {
            bottom_dock(ui, &mut state_cell.borrow_mut(), &assets)
        });
        let mut console_text_right_top = false;
        let mut right_half_max_y: f32 = 0.0;
        for mesh in &frame.meshes {
            for v in &mesh.vertices {
                if v.pos[0] > 810.0 {
                    right_half_max_y = right_half_max_y.max(v.pos[1]);
                    if v.pos[1] < 120.0 {
                        console_text_right_top = true;
                    }
                }
            }
        }
        assert!(
            console_text_right_top,
            "console content must tessellate in the right column near the dock top"
        );
        assert!(
            right_half_max_y <= 232.0,
            "right-column content must stay inside the dock (max y {right_half_max_y})"
        );
    }

    /// ADR-0026 geometry pin (Phase 8): the status bar gains the
    /// observation-only CPU/Memory summary — it must tessellate and sit in
    /// the bar's band. Without telemetry frames the summary is honestly
    /// absent (no fake CPU numbers).
    #[test]
    fn status_bar_shows_telemetry_summary_only_when_present() {
        // Without telemetry: no summary line.
        let state = EditorState::new();
        let state_cell = std::cell::RefCell::new(state);
        let mut ctx = UiContext::new();
        let frame = ctx.run(gens_ui::UiInput::new([800, 22], 0.0, 1.0 / 60.0), |ui| {
            status_bar(ui, &mut state_cell.borrow_mut())
        });
        let empty_bar_vertices: usize = frame.meshes.iter().map(|m| m.vertices.len()).sum();

        // With telemetry: the bar must paint MORE (the summary joined).
        let mut state = EditorState::new();
        state.telemetry = crate::telemetry::Telemetry {
            frames: 5,
            frame_median_ms: 2.5,
            memory_current_mb: 42.0,
            ..Default::default()
        };
        let state_cell = std::cell::RefCell::new(state);
        let mut ctx = UiContext::new();
        let frame = ctx.run(gens_ui::UiInput::new([800, 22], 0.0, 1.0 / 60.0), |ui| {
            status_bar(ui, &mut state_cell.borrow_mut())
        });
        let full_bar_vertices: usize = frame.meshes.iter().map(|m| m.vertices.len()).sum();
        assert!(
            full_bar_vertices > empty_bar_vertices,
            "status bar must paint the telemetry summary (empty {empty_bar_vertices} vs full {full_bar_vertices})"
        );
        // Everything stays near the 22px band (glyph descenders + frame
        // padding overhang ~3px; the dock test uses the same slack
        // convention: 230 dock -> 232 bound).
        let max_y = frame
            .meshes
            .iter()
            .flat_map(|m| m.vertices.iter())
            .map(|v| v.pos[1])
            .fold(0.0f32, f32::max);
        assert!(
            max_y <= 28.0,
            "status bar content must stay in its band (max y {max_y})"
        );
    }
}
