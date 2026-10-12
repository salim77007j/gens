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

use crate::{model::EditorCommand, theme, viewport::EditorViewport};
use gens_assets::AssetRegistry;
use gens_ecs::prelude::*;
use gens_renderer::{GpuContext, Renderer};
use gens_ui::{UiInput, UiRenderer};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

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
            ..Default::default()
        }
    }

    /// Drains queued menu commands (the app executes them).
    pub fn take_commands(&mut self) -> Vec<EditorCommand> {
        std::mem::take(&mut self.pending)
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
        // The 3D viewport renders FIRST (own target); its pixels become
        // an egui texture; then the whole editor UI paints over them.
        viewport.render(ui.core_mut().ctx(), state, world)?;
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
    status_bar(ui, state);
    if state.show_bottom_dock {
        bottom_dock(ui, state);
    }
    if state.show_hierarchy {
        crate::hierarchy::panel(ui, state, world);
    }
    if state.show_inspector {
        crate::inspector::panel(ui, state, world);
    }
    center_dock(ui, state, viewport, world);
    let _ = assets; // consumed by the bottom dock from P7-5 on
}

/// Top menu bar (File / Edit / View / Help) — items queue real
/// commands; decoration-only items are deliberately absent.
fn menu_bar(ui: &mut egui::Ui, state: &mut EditorState) {
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
                    if ui.button("New Scene").clicked() {
                        state.pending.push(EditorCommand::NewScene);
                        ui.close();
                    }
                    if ui.button("Save Scene").clicked() {
                        gens_core::logging::info!("editor: save scene requested (via app)");
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    let has_selection = state.selected.is_some();
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Duplicate Selected"))
                        .clicked()
                    {
                        state.pending.push(EditorCommand::DuplicateSelected);
                        ui.close();
                    }
                    if ui
                        .add_enabled(has_selection, egui::Button::new("Delete Selected"))
                        .clicked()
                    {
                        state.pending.push(EditorCommand::DeleteSelected);
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.toggle_value(&mut state.show_hierarchy, "Scene Hierarchy");
                    ui.toggle_value(&mut state.show_inspector, "Inspector");
                    ui.toggle_value(&mut state.show_bottom_dock, "Bottom Dock");
                });
                ui.menu_button("Help", |ui| {
                    ui.label(egui::RichText::new("Gens Engine — visual editor").strong());
                    ui.label(
                        egui::RichText::new("offscreen-first build; see docs/design/REFERENCE.png")
                            .color(theme::TEXT_DIM),
                    );
                });
            });
        });
}

/// Bottom status bar: readiness dot + deterministic counters.
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
fn bottom_dock(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::Panel::bottom("bottom_dock")
        .exact_size(230.0)
        .resizable(false)
        .frame(
            egui::Frame::default()
                .fill(theme::PANEL_BG)
                .inner_margin(egui::Margin::same(4)),
        )
        .show(ui, |ui| {
            let half = ui.available_width() * 0.5;
            let height = ui.available_height();
            ui.horizontal(|ui| {
                ui.allocate_ui(egui::vec2(half, height), |ui| {
                    crate::assets::panel(ui, state);
                });
                ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
                    crate::console::panel(ui, state);
                });
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
        .frame(egui::Frame::default().fill(theme::PANEL_BG).inner_margin(egui::Margin::same(4)))
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
                Some(ViewTab::GameView) => empty_state(
                    ui,
                    "Game View renders the player camera — Phase 8 scope (profiling + build pipeline ships it alongside the window shell).",
                ),
                Some(ViewTab::Animator) => empty_state(
                    ui,
                    "The animator graph editor is Phase 9 scope; the animation runtime (clips, blend trees, FK) is already live via gens-animation.",
                ),
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
