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
        crate::shell::BottomTab::Network => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(
                        "Network stats ship with the window shell (Phase 8 scope); \
                         the session layer's counters live in gens-networking today.",
                    )
                    .color(theme::TEXT_DIM),
                );
            });
        }
    }
    let _ = assets; // reserved: console commands that touch assets
}

fn console_tab(ui: &mut egui::Ui, state: &mut EditorState) {
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
    ui.label(
        egui::RichText::new(
            "Per-system timings are wall-clock and would break the \
             byte-identical replay contract; they are omitted (ADR-0025).",
        )
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
}
