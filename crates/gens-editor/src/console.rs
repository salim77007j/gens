//! Developer console — the bottom-right dock.
//!
//! Log capture is REAL: a `tracing_subscriber::Layer` records every
//! event the engine emits into a bounded ring buffer (with an honest
//! dropped counter), timestamped with the editor's DETERMINISTIC sim
//! clock (no wall time — ADR-0025). The panel renders the buffer with
//! per-severity colors, and the Profiler/Performance tabs surface the
//! same deterministic counters as the status bar. The Network tab is
//! an honest empty state (net panels ship with the window shell).

use crate::{shell::EditorState, theme};
use gens_assets::AssetRegistry;
use std::collections::VecDeque;

use std::sync::{Arc, Mutex};
use tracing_subscriber::Layer;

/// One captured log line.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleLine {
    /// Deterministic sim time when the event fired (seconds).
    pub time_seconds: f64,
    /// Severity (INFO/WARN/ERROR/DEBUG/TRACE).
    pub level: tracing::Level,
    /// Event target (module path).
    pub target: String,
    /// Formatted message.
    pub message: String,
}

/// Ring buffer of captured lines. Shared between the tracing layer
/// (writer) and the panel (reader) behind a mutex.
#[derive(Debug, Default)]
pub struct ConsoleBuffer {
    inner: Mutex<ConsoleBufferInner>,
}

#[derive(Debug, Default)]
struct ConsoleBufferInner {
    lines: VecDeque<ConsoleLine>,
    dropped: u64,
    now_seconds: f64,
}

/// Capacity of the ring buffer (oldest lines are dropped first).
pub const CAPACITY: usize = 200;

impl ConsoleBuffer {
    /// An empty buffer at sim time zero.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ConsoleBufferInner {
                lines: VecDeque::new(),
                dropped: 0,
                now_seconds: 0.0,
            }),
        }
    }

    /// Advances the deterministic clock (called once per editor frame).
    pub fn set_time(&self, seconds: f64) {
        self.inner.lock().expect("console lock").now_seconds = seconds;
    }

    /// Pushes a line, evicting the oldest beyond [`CAPACITY`].
    pub fn push(&self, line: ConsoleLine) {
        let mut inner = self.inner.lock().expect("console lock");
        if inner.lines.len() >= CAPACITY {
            inner.lines.pop_front();
            inner.dropped += 1;
        }
        inner.lines.push_back(line);
    }

    /// Snapshot of the lines (oldest first).
    pub fn lines(&self) -> Vec<ConsoleLine> {
        self.inner
            .lock()
            .expect("console lock")
            .lines
            .iter()
            .cloned()
            .collect()
    }

    /// How many lines were evicted by the capacity cap.
    pub fn dropped(&self) -> u64 {
        self.inner.lock().expect("console lock").dropped
    }

    /// Line count.
    pub fn len(&self) -> usize {
        self.inner.lock().expect("console lock").lines.len()
    }

    /// Whether the buffer is empty.
    /// Clears the ring (the console `clear` command). Dropped-counter
    /// history is preserved: it reports ring overflow, not user actions.
    pub fn clear(&self) {
        self.inner.lock().expect("console lock").lines.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The tracing layer installed by the embedding app.
#[derive(Clone)]
pub struct ConsoleSink {
    buffer: Arc<ConsoleBuffer>,
}

impl ConsoleSink {
    /// A sink writing into `buffer`.
    pub fn new(buffer: Arc<ConsoleBuffer>) -> Self {
        Self { buffer }
    }
}

struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // Debug of a string payload quotes it; normalize so both the
            // record_str and record_debug paths render identically.
            self.0 = format!("{value:?}").trim_matches('"').to_string();
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_string();
        }
    }
}

impl<S> Layer<S> for ConsoleSink
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let now = self.buffer.inner.lock().expect("console lock").now_seconds;
        self.buffer.push(ConsoleLine {
            time_seconds: now,
            level: *event.metadata().level(),
            target: event.metadata().target().to_string(),
            message: visitor.0,
        });
    }
}

/// Bottom-right half of the bottom dock: tab row + active tab content.
pub(crate) fn panel(ui: &mut egui::Ui, state: &mut EditorState, assets: &AssetRegistry) {
    // Tab row (reference: Console | Profiler | Performance | Network).
    ui.horizontal(|ui| {
        for tab in crate::shell::BottomTab::ALL {
            let active = state.bottom_tab == tab;
            if ui.selectable_label(active, tab.label()).clicked() {
                state.bottom_tab = tab;
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let dropped = state.console.dropped();
            if dropped > 0 {
                ui.label(
                    egui::RichText::new(format!("{dropped} dropped"))
                        .small()
                        .color(theme::SEVERITY[1]),
                );
            }
            ui.label(
                egui::RichText::new(format!("{} lines", state.console.len()))
                    .small()
                    .color(theme::TEXT_DIM),
            );
        });
    });
    ui.separator();

    match state.bottom_tab {
        crate::shell::BottomTab::Console => console_tab(ui, state),
        crate::shell::BottomTab::Profiler => profiler_tab(ui, state),
        crate::shell::BottomTab::Performance => performance_tab(ui, state),
        crate::shell::BottomTab::Network => crate::cinema::network_panel(ui, state),
    }
    let _ = assets; // reserved: console commands that touch assets
}

fn console_tab(ui: &mut egui::Ui, state: &mut EditorState) {
    // Command row: input + Run. The shell records intent only — the app
    // drains `pending_commands` and executes against the real world
    // (same pattern as menu commands), results land via `tracing` in the
    // console buffer itself.
    ui.horizontal(|ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.command_input)
                .hint_text("command (help, entities, validate, stats)")
                .desired_width(280.0),
        );
        state
            .row_rects
            .insert("command_input".into(), response.rect);
        let run = ui.button("Run");
        state.row_rects.insert("command_run".into(), run.rect);
        let submit =
            run.clicked() || response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if submit && !state.command_input.trim().is_empty() {
            state
                .pending_commands
                .push(state.command_input.trim().to_string());
            state.command_input.clear();
        }
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .id_salt("console_lines")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for line in state.console.lines() {
                let color = severity_color(&line.level);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!("[t={:08.3}]", line.time_seconds))
                            .monospace()
                            .color(theme::TEXT_DIM),
                    );
                    ui.label(
                        egui::RichText::new(format!("[{}]", line.level))
                            .monospace()
                            .color(color),
                    );
                    ui.label(egui::RichText::new(&line.message).monospace());
                });
            }
            if state.console.is_empty() {
                ui.label(egui::RichText::new("No events captured yet.").color(theme::TEXT_DIM));
            }
        });
}

fn profiler_tab(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::Grid::new("profiler_grid")
        .num_columns(2)
        .show(ui, |ui| {
            row(ui, "Frame", &state.stats.frame.to_string());
            row(ui, "Simulation step", &state.stats.step.to_string());
            row(ui, "Entities (live)", &state.stats.entities.to_string());
            row(
                ui,
                "fps (1/dt, deterministic)",
                &format!("{:.1}", state.stats.fps),
            );
            row(ui, "Console lines", &state.console.len().to_string());
            row(ui, "Dropped lines", &state.console.dropped().to_string());
        });
    ui.separator();
    if state.telemetry.frames == 0 {
        ui.label(
            egui::RichText::new(
                "No profiler data yet — the app has not recorded any frames
                 (a disabled profiler shows the same honest state).",
            )
            .small()
            .color(theme::TEXT_DIM),
        );
        return;
    }
    // Real per-phase timings (ADR-0027: observation-only, proven not to
    // touch the simulation). Columns match the reference mock's spirit:
    // name | median | max | share.
    egui::Grid::new("phase_grid").num_columns(4).show(ui, |ui| {
        ui.label(egui::RichText::new("Phase").strong().color(theme::TEXT));
        ui.label(egui::RichText::new("median ms").strong().color(theme::TEXT));
        ui.label(egui::RichText::new("max ms").strong().color(theme::TEXT));
        ui.label(egui::RichText::new("share").strong().color(theme::TEXT));
        ui.end_row();
        for phase in &state.telemetry.phases {
            ui.label(
                egui::RichText::new(&phase.name)
                    .monospace()
                    .color(theme::TEXT),
            );
            ui.label(
                egui::RichText::new(format!("{:.2}", phase.median_ms))
                    .monospace()
                    .color(theme::TEXT),
            );
            ui.label(
                egui::RichText::new(format!("{:.2}", phase.max_ms))
                    .monospace()
                    .color(theme::TEXT),
            );
            ui.label(
                egui::RichText::new(format!("{:.0}%", phase.share * 100.0))
                    .monospace()
                    .color(theme::TEXT_DIM),
            );
            ui.end_row();
        }
    });
    ui.label(
        egui::RichText::new(format!(
            "wall-clock, rolling window of {} frames — observation-only \
             (ADR-0027): the byte-identical replay contract is unaffected",
            state.telemetry.frames
        ))
        .small()
        .color(theme::TEXT_DIM),
    );
}

fn performance_tab(ui: &mut egui::Ui, state: &mut EditorState) {
    egui::Grid::new("perf_grid").num_columns(2).show(ui, |ui| {
        row(
            ui,
            "Viewport",
            &format!(
                "{}x{}",
                state.viewport_size_px[0], state.viewport_size_px[1]
            ),
        );
        row(
            ui,
            "Camera",
            &format!(
                "yaw {:.0}° pitch {:.0}° dist {:.1}",
                state.orbit.yaw_deg, state.orbit.pitch_deg, state.orbit.dist
            ),
        );
        row(
            ui,
            "Selection",
            &state
                .selected
                .map(|_| "set".to_string())
                .unwrap_or_else(|| "none".to_string()),
        );
        row(
            ui,
            "Active asset",
            &state
                .selected_asset
                .clone()
                .unwrap_or_else(|| "none".to_string()),
        );
        if state.telemetry.frames > 0 {
            row(
                ui,
                "Frame time (median / max)",
                &format!(
                    "{:.2} ms / {:.2} ms",
                    state.telemetry.frame_median_ms, state.telemetry.frame_max_ms
                ),
            );
            row(
                ui,
                "Memory (current / peak)",
                &format!(
                    "{:.1} MB / {:.1} MB",
                    state.telemetry.memory_current_mb, state.telemetry.memory_peak_mb
                ),
            );
            row(ui, "Allocations", &state.telemetry.allocations.to_string());
        } else {
            row(
                ui,
                "Frame time",
                "no profiler data (app has not recorded frames)",
            );
            row(ui, "Memory", "no tracker data yet");
        }
    });
}

fn row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(egui::RichText::new(label).color(theme::TEXT_DIM));
    ui.label(egui::RichText::new(value).monospace());
}

/// Severity color (mirrors the reference console's palette).
pub fn severity_color(level: &tracing::Level) -> egui::Color32 {
    match *level {
        tracing::Level::INFO => theme::SEVERITY[0],
        tracing::Level::WARN => theme::SEVERITY[1],
        tracing::Level::ERROR => theme::SEVERITY[2],
        _ => theme::SEVERITY[3],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::layer::SubscriberExt;

    #[test]
    fn sink_captures_engine_events() {
        let buffer = Arc::new(ConsoleBuffer::new());
        buffer.set_time(4.2);
        let sink = ConsoleSink::new(buffer.clone());
        let subscriber = tracing_subscriber::registry().with(sink);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("engine initialized");
            tracing::warn!(target: "gens_demo", "missing texture");
        });
        let lines = buffer.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].message, "engine initialized");
        assert_eq!(lines[0].level, tracing::Level::INFO);
        assert_eq!(lines[0].time_seconds, 4.2);
        assert_eq!(lines[1].level, tracing::Level::WARN);
    }

    #[test]
    fn ring_capacity_evicts_oldest_and_counts() {
        let buffer = Arc::new(ConsoleBuffer::new());
        for i in 0..(CAPACITY as u64 + 25) {
            buffer.push(ConsoleLine {
                time_seconds: i as f64,
                level: tracing::Level::INFO,
                target: "t".into(),
                message: format!("line {i}"),
            });
        }
        assert_eq!(buffer.len(), CAPACITY);
        assert_eq!(buffer.dropped(), 25);
        let lines = buffer.lines();
        // 225 pushes into a 200-cap ring: 25 evicted, first kept = line 25.
        assert_eq!(lines[0].message, "line 25");
    }

    #[test]
    fn severity_colors_map_deterministically() {
        assert_eq!(severity_color(&tracing::Level::INFO), theme::SEVERITY[0]);
        assert_eq!(severity_color(&tracing::Level::WARN), theme::SEVERITY[1]);
        assert_eq!(severity_color(&tracing::Level::ERROR), theme::SEVERITY[2]);
        assert_eq!(severity_color(&tracing::Level::DEBUG), theme::SEVERITY[3]);
    }
}

#[cfg(test)]
mod paint_tests {
    use super::*;
    use crate::shell::EditorState;
    use gens_assets::AssetRegistry;
    use gens_ui::UiContext;
    use tracing_subscriber::layer::SubscriberExt;

    /// The console panel must tessellate the captured lines (headless,
    /// no GPU — the same pass gens-ui's tests use).
    #[test]
    fn console_panel_tessellates_captured_lines() {
        let state = EditorState::new();
        let sink = ConsoleSink::new(state.console.clone());
        let subscriber = tracing_subscriber::registry().with(sink);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("engine initialized");
            tracing::info!("physics initialized: rapier3d");
        });
        assert_eq!(state.console.len(), 2, "capture must land in the buffer");
        let mut ctx = UiContext::new();
        let state_cell = std::cell::RefCell::new(state);
        let frame = ctx.run(gens_ui::UiInput::new([700, 200], 0.0, 1.0 / 60.0), |ui| {
            panel(ui, &mut state_cell.borrow_mut(), &AssetRegistry::new())
        });
        let state = state_cell.into_inner();
        let _ = &state;
        assert!(
            !frame.meshes.is_empty(),
            "console panel must tessellate text into meshes"
        );
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use crate::shell::EditorState;
    use gens_assets::AssetRegistry;
    use gens_ui::UiContext;
    use tracing_subscriber::layer::SubscriberExt;

    /// Replicates the example's exact flow: sink cloned from state,
    /// events logged under with_default, then the shell's mem::take
    /// dance, then a paint pass. The console must survive all of it.
    #[test]
    fn console_survives_frame_take_dance() {
        let mut state = EditorState::new();
        let sink = ConsoleSink::new(state.console.clone());
        let subscriber = tracing_subscriber::registry()
            .with(sink)
            .with(tracing_subscriber::fmt::layer().without_time().compact());
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("engine initialized");
            tracing::info!("physics initialized: rapier3d");
            // The shell frame dance (mirrors EditorShell::frame).
            let state_cell = std::cell::RefCell::new(std::mem::take(&mut state));
            {
                let s = state_cell.borrow();
                assert_eq!(s.console.len(), 2, "taken state keeps the buffer");
            }
            let mut ctx = UiContext::new();
            let frame = ctx.run(gens_ui::UiInput::new([700, 200], 0.5, 1.0 / 60.0), |ui| {
                panel(ui, &mut state_cell.borrow_mut(), &AssetRegistry::new())
            });
            assert!(
                !frame.meshes.is_empty(),
                "console must tessellate after the take dance"
            );
            state = state_cell.into_inner();
        });
        assert_eq!(state.console.len(), 2);
    }

    /// ADR-0026 geometry pin (Phase 8): with telemetry present the
    /// Profiler tab paints the phase table (more tessellation than the
    /// no-data state), and everything lands inside the bottom dock's
    /// console column.
    #[test]
    fn profiler_tab_paints_phase_table_when_telemetry_present() {
        let no_data = EditorState::new();
        let no_data_cell = std::cell::RefCell::new(no_data);
        let mut ctx = UiContext::new();
        let frame_empty = ctx.run(gens_ui::UiInput::new([700, 200], 0.0, 1.0 / 60.0), |ui| {
            let mut s = no_data_cell.borrow_mut();
            s.bottom_tab = crate::shell::BottomTab::Profiler;
            profiler_tab(ui, &mut s)
        });
        let empty_vertices: usize = frame_empty.meshes.iter().map(|m| m.vertices.len()).sum();

        let mut with_data = EditorState::new();
        with_data.bottom_tab = crate::shell::BottomTab::Profiler;
        with_data.telemetry = crate::telemetry::Telemetry {
            frames: 12,
            frame_median_ms: 3.1,
            frame_max_ms: 5.0,
            phases: vec![
                crate::telemetry::PhaseTimings {
                    name: "physics".into(),
                    median_ms: 1.2,
                    max_ms: 2.0,
                    share: 0.4,
                },
                crate::telemetry::PhaseTimings {
                    name: "scripts".into(),
                    median_ms: 0.8,
                    max_ms: 1.5,
                    share: 0.3,
                },
            ],
            ..Default::default()
        };
        let with_data_cell = std::cell::RefCell::new(with_data);
        let mut ctx = UiContext::new();
        let frame_full = ctx.run(gens_ui::UiInput::new([700, 200], 0.0, 1.0 / 60.0), |ui| {
            let mut s = with_data_cell.borrow_mut();
            s.bottom_tab = crate::shell::BottomTab::Profiler;
            profiler_tab(ui, &mut s)
        });
        let full_vertices: usize = frame_full.meshes.iter().map(|m| m.vertices.len()).sum();
        assert!(
            full_vertices > empty_vertices,
            "the phase table must paint (empty {empty_vertices} vs full {full_vertices})"
        );
        let max_y = frame_full
            .meshes
            .iter()
            .flat_map(|m| m.vertices.iter())
            .map(|v| v.pos[1])
            .fold(0.0f32, f32::max);
        assert!(
            max_y <= 202.0,
            "profiler table must stay in the panel (max y {max_y})"
        );
    }

    /// ADR-0026 geometry pin (Phase 8): the command row exists, its Run
    /// button rect is captured for scripted replay, and submitting a
    /// command queues it for the app (the shell never executes it).
    #[test]
    fn command_row_captures_rects_and_queues_real_intents() {
        let mut state = EditorState::new();
        state.bottom_tab = crate::shell::BottomTab::Console;
        state.command_input = "validate".into();
        let state_cell = std::cell::RefCell::new(state);
        let mut ctx = UiContext::new();
        // Click position = center of the Run button once we know it — but
        // we don't yet; run once, read the rect, click it on the SECOND
        // run (the same two-pass pattern the hierarchy click test uses).
        let _ = ctx.run(gens_ui::UiInput::new([700, 200], 0.0, 1.0 / 60.0), |ui| {
            let mut s = state_cell.borrow_mut();
            s.bottom_tab = crate::shell::BottomTab::Console;
            console_tab(ui, &mut s)
        });
        let run_rect = state_cell
            .borrow()
            .row_rects
            .get("command_run")
            .copied()
            .expect("Run button rect must be layout-captured");
        assert!(run_rect.width() > 0.0 && run_rect.height() > 0.0);
        assert!(run_rect.right() <= 700.0, "command row must fit the panel");

        // Second pass: click the captured rect center (press + release
        // events, the same UiEvent vocabulary the example replay uses).
        let center = run_rect.center();
        let click = gens_ui::UiInput::with_events(
            [700, 200],
            vec![
                gens_ui::UiEvent::PointerButton {
                    x: center.x,
                    y: center.y,
                    button: 0,
                    pressed: true,
                },
                gens_ui::UiEvent::PointerButton {
                    x: center.x,
                    y: center.y,
                    button: 0,
                    pressed: false,
                },
            ],
        );
        let _ = ctx.run(click, |ui| {
            let mut s = state_cell.borrow_mut();
            s.bottom_tab = crate::shell::BottomTab::Console;
            console_tab(ui, &mut s)
        });
        let queued = state_cell.borrow().pending_commands.clone();
        assert_eq!(
            queued,
            vec!["validate".to_string()],
            "clicking Run queues the command"
        );
        assert!(
            state_cell.borrow().command_input.is_empty(),
            "the input clears on submit"
        );
    }
}
