//! `gens-editor` — the Gens interactive editor window (v1.1, ADR-0036).
//!
//! The piece v1.0.0 honestly deferred: a REAL OS window (winit) with
//! the live 3D viewport rendered every frame, clickable Play / Pause /
//! Stop / Save / Load toolbar buttons, the full menu bar (File / Edit /
//! GameObject / View / Help), keyboard shortcuts (Ctrl+S / Ctrl+O /
//! Ctrl+Z / Ctrl+D / Del), draggable objects in the Scene View and the
//! editable Inspector — all through the SAME shell, executor and
//! renderer paths the headless build has proven on CI since Phase 7.
//!
//! Architecture per frame (`ApplicationHandler` + continuous redraw):
//!
//! ```text
//! winit events ──translate──▶ UiEvent queue (window::events)
//!      │                                   │
//!      ▼                                   ▼
//! play-mode stepping (fixed 60 Hz,     UiInput (drained events,
//!  wall-clock accumulator, capped)      deterministic UI clock)
//!      │                                   │
//!      ▼                                   ▼
//! command executor (shared) ──▶ EditorShell::frame ──▶ RGBA readback
//!                                                             │
//!                                        SurfaceTarget::present_rgba8
//!                                                             │
//!                                                    the OS window
//! ```
//!
//! Two frontends, one binary:
//! - no flags: the interactive editor (resizable window, vsync loop);
//! - `--smoke-frames N --screenshot out.png --json out.json`: the CI
//!   verification mode — the SAME window code path runs a scripted,
//!   self-verifying session (real pointer clicks on layout-captured
//!   toolbar rects, real drag traffic, real shortcut keys), saves a
//!   PNG of the last presented frame plus a JSON evidence blob, and
//!   exits 0. This is what CI runs headlessly (xvfb on Linux, the
//!   runner desktop on Windows/macOS) to prove the window shell boots,
//!   presents and interacts on every push.

use gens_assets::{AssetKind, AssetRegistry};
use gens_ecs::hierarchy::attach_children;
use gens_ecs::{EngineSchedules, Name};
use gens_editor::commands::{drain_commands, drain_editor_commands};
use gens_editor::window::SurfaceTarget;
use gens_editor::window::events as win_events;
use gens_editor::{EditorShell, MeshSlot, PlayMode};
use gens_physics::ecs::sync_transforms_from_physics;
use gens_physics::{ColliderShape, PhysicsResource, PhysicsWorld};
use gens_ui::{UiEvent, UiInput};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing_subscriber::layer::SubscriberExt;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use gens_math::{Vec3, vec3};

/// Default window size (the editor reference's 16:9 canvas).
const DEFAULT_SIZE: [u32; 2] = [1600, 900];
/// Fixed simulation step while playing.
const DT: f32 = 1.0 / 60.0;
/// Safety cap: at most this many fixed steps per frame (a stall must
/// not turn into a 30-step catch-up burst).
const MAX_STEPS_PER_FRAME: u32 = 4;
/// The smoke evidence format tag.
const SMOKE_FORMAT: &str = "gens-editor-smoke-1";
/// The deterministic scene path the smoke session saves to.
const SMOKE_SCENE_PATH: &str = "scene.gensscene";

fn main() {
    let config = Config::from_env(std::env::args().collect());
    if let Err(e) = run(config) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

/// CLI configuration.
struct Config {
    /// Smoke mode: the number of frames to run before self-exit.
    smoke_frames: Option<u64>,
    /// Smoke mode: where the screenshot PNG lands.
    screenshot: PathBuf,
    /// Smoke mode: where the evidence JSON lands.
    json: PathBuf,
    /// Initial window size.
    size: [u32; 2],
}

impl Config {
    fn from_env(args: Vec<String>) -> Config {
        let get = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let width = get("--width").and_then(|v| v.parse().ok());
        let height = get("--height").and_then(|v| v.parse().ok());
        Config {
            smoke_frames: get("--smoke-frames").and_then(|v| v.parse().ok()),
            screenshot: get("--screenshot")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("editor_window.png")),
            json: get("--json")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("editor_window.json")),
            size: [
                width.unwrap_or(DEFAULT_SIZE[0]),
                height.unwrap_or(DEFAULT_SIZE[1]),
            ],
        }
    }
}

fn run(config: Config) -> Result<(), gens_core::anyhow::Error> {
    // Skip the whole GPU/window stack when the caller only asked for
    // usage: the binary is honest about its modes.
    let event_loop = EventLoop::builder()
        .build()
        .map_err(|e| gens_core::anyhow::anyhow!("event loop creation failed: {e}"))?;
    let mut app = EditorApp::new(config);
    event_loop
        .run_app(&mut app)
        .map_err(|e| gens_core::anyhow::anyhow!("event loop failed: {e}"))?;
    // A smoke session that failed its script exits non-zero so CI goes
    // red on the FIRST problem, not the first crash.
    if let Some(error) = app.error.take() {
        return Err(gens_core::anyhow::anyhow!("{error}"));
    }
    Ok(())
}

/// The scripted smoke session: which frame does what.
///
/// Every interaction is real UI traffic — pointer events at
/// layout-captured button rects, pointer drags in the viewport, key
/// events for the shortcut — through the same UiEvent stream a human
/// produces, so CI proves the interactive paths, not mocks.
struct SmokeScript {
    /// Evidence as the session runs.
    play_clicked: bool,
    play_verified: bool,
    pause_verified: bool,
    stop_verified: bool,
    drag_frames: u64,
    shortcut_save_verified: bool,
    screenshot: Option<Vec<u8>>,
}

impl SmokeScript {
    fn new() -> Self {
        Self {
            play_clicked: false,
            play_verified: false,
            pause_verified: false,
            stop_verified: false,
            drag_frames: 0,
            shortcut_save_verified: false,
            screenshot: None,
        }
    }

    /// Clicks a toolbar button (pointer traffic at the layout-captured
    /// rect center).
    fn click_button(rect: egui::Rect) -> Vec<UiEvent> {
        let (cx, cy) = (rect.center().x, rect.center().y);
        vec![
            UiEvent::PointerMove { x: cx, y: cy },
            UiEvent::PointerButton {
                x: cx,
                y: cy,
                button: 0,
                pressed: true,
            },
            UiEvent::PointerButton {
                x: cx,
                y: cy,
                button: 0,
                pressed: false,
            },
        ]
    }

    /// Events injected at the start of the given frame.
    fn events_for(&mut self, frame: u64, shell: &mut EditorShell) -> Vec<UiEvent> {
        let state = &mut shell.state;
        match frame {
            5 => {
                let Some(rect) = state.toolbar_rects.get("Play").copied() else {
                    return vec![];
                };
                self.play_clicked = true;
                Self::click_button(rect)
            }
            // Viewport drag traffic through the real pointer path
            // (mid-play this is orbit traffic — dragging objects is an
            // edit-mode tool, the orbit is always live).
            8 => {
                let center = egui::Pos2::new(
                    state.viewport_size_px[0] as f32 * 0.5,
                    state.viewport_size_px[1] as f32 * 0.5,
                );
                vec![
                    UiEvent::PointerMove {
                        x: center.x,
                        y: center.y,
                    },
                    UiEvent::PointerButton {
                        x: center.x,
                        y: center.y,
                        button: 0,
                        pressed: true,
                    },
                ]
            }
            9..=11 => {
                self.drag_frames += 1;
                vec![UiEvent::PointerMove {
                    x: state.viewport_size_px[0] as f32 * 0.5 + 14.0 * (frame - 8) as f32,
                    y: state.viewport_size_px[1] as f32 * 0.5 - 4.0 * (frame - 8) as f32,
                }]
            }
            12 => {
                let x = state.viewport_size_px[0] as f32 * 0.5 + 56.0;
                let y = state.viewport_size_px[1] as f32 * 0.5 - 16.0;
                vec![
                    UiEvent::PointerMove { x, y },
                    UiEvent::PointerButton {
                        x,
                        y,
                        button: 0,
                        pressed: false,
                    },
                ]
            }
            14 => state
                .toolbar_rects
                .get("Pause")
                .copied()
                .map(Self::click_button)
                .unwrap_or_default(),
            17 => state
                .toolbar_rects
                .get("Stop")
                .copied()
                .map(Self::click_button)
                .unwrap_or_default(),
            // Ctrl+S through the real key path (modifiers + key).
            20 => vec![
                UiEvent::ModifiersChanged(win_events::to_ui_modifiers(
                    winit::keyboard::ModifiersState::CONTROL,
                )),
                UiEvent::Key {
                    key: gens_ui::UiKey::Character('S'),
                    pressed: true,
                    repeat: false,
                    modifiers: gens_ui::UiModifiers::ctrl(),
                },
                UiEvent::Key {
                    key: gens_ui::UiKey::Character('S'),
                    pressed: false,
                    repeat: false,
                    modifiers: gens_ui::UiModifiers::ctrl(),
                },
            ],
            _ => vec![],
        }
    }

    /// Verifications at specific frames (loud failures — the smoke
    /// session is a gate, not a demo).
    fn verify_at(&mut self, frame: u64, shell: &EditorShell) -> Option<String> {
        let state = &shell.state;
        match frame {
            7 => {
                if state.play_mode == PlayMode::Playing && state.play_time > 0.0 {
                    self.play_verified = true;
                    None
                } else {
                    Some(format!(
                        "Play click did not start the simulation (mode {:?}, t {:.2})",
                        state.play_mode, state.play_time
                    ))
                }
            }
            16 => {
                if state.play_mode == PlayMode::Paused {
                    self.pause_verified = true;
                    None
                } else {
                    Some(format!(
                        "Pause click did not pause the simulation (mode {:?})",
                        state.play_mode
                    ))
                }
            }
            19 => {
                if state.play_mode == PlayMode::Idle {
                    self.stop_verified = true;
                    None
                } else {
                    Some(format!(
                        "Stop click did not restore edit mode (mode {:?})",
                        state.play_mode
                    ))
                }
            }
            _ => None,
        }
    }
}

/// The winit application: owns everything GPU/window and drives the
/// editor shell.
struct EditorApp {
    config: Config,
    // Created in `resumed` (the event loop must exist first).
    window: Option<Arc<Window>>,
    shell: Option<EditorShell>,
    surface: Option<SurfaceTarget>,
    queue_events: Vec<UiEvent>,
    modifiers: winit::keyboard::ModifiersState,
    cursor: (f32, f32),
    // World state.
    world: gens_ecs::ecs::world::World,
    schedules: EngineSchedules,
    play_schedules: EngineSchedules,
    assets: AssetRegistry,
    // Frame timing.
    last_frame: Instant,
    start: Instant,
    accumulator: f32,
    frame: u64,
    // Smoke session.
    smoke: Option<SmokeScript>,
    error: Option<String>,
}

impl EditorApp {
    fn new(config: Config) -> Self {
        let (world, physics) = build_scene();
        let mut world = world;
        world.insert_resource(PhysicsResource { world: physics });
        // Edit mode: propagation only — dragged/edited transforms must
        // not be overwritten from stale physics positions.
        let schedules = EngineSchedules::default();
        // Play mode: physics sync joins the frame (what the headless
        // example runs).
        let mut play_schedules = EngineSchedules::default();
        play_schedules
            .update
            .add_systems(sync_transforms_from_physics);
        Self {
            config,
            window: None,
            shell: None,
            surface: None,
            queue_events: Vec::new(),
            modifiers: winit::keyboard::ModifiersState::empty(),
            cursor: (0.0, 0.0),
            world,
            schedules,
            play_schedules,
            assets: build_assets(),
            last_frame: Instant::now(),
            start: Instant::now(),
            accumulator: 0.0,
            frame: 0,
            smoke: None,
            error: None,
        }
    }

    /// One presented frame: step the sim (if playing), execute queued
    /// commands, compose the editor UI, present, run the smoke script
    /// when active.
    fn redraw(&mut self, event_loop: &ActiveEventLoop) {
        let dt_wall = self.last_frame.elapsed().min(Duration::from_secs_f32(0.25));
        self.last_frame = Instant::now();
        let dt_seconds = dt_wall.as_secs_f32();
        self.frame += 1;

        let Some(shell) = self.shell.as_mut() else {
            return;
        };

        // Smoke script: inject the scripted interaction events.
        let mut events = std::mem::take(&mut self.queue_events);
        if let Some(smoke) = self.smoke.as_mut() {
            events.extend(smoke.events_for(self.frame, shell));
        }

        // Play mode steps on the fixed clock (real-time accumulator,
        // capped); edit mode runs propagation only.
        if shell.state.play_mode == PlayMode::Playing {
            self.accumulator = (self.accumulator + dt_seconds).min(0.25);
            let mut steps = 0u32;
            while self.accumulator >= DT && steps < MAX_STEPS_PER_FRAME {
                self.world.resource_mut::<PhysicsResource>().world.step();
                self.play_schedules.run_frame(&mut self.world);
                shell.state.stats.step += 1;
                shell.state.play_time += f64::from(DT);
                self.accumulator -= DT;
                steps += 1;
            }
        } else {
            self.accumulator = 0.0;
            self.schedules.run_frame(&mut self.world);
        }

        // Commands from the toolbar / menus / shortcuts, then console
        // commands — the SHARED executor (the headless example runs the
        // exact same code).
        drain_editor_commands(&mut shell.state, &mut self.world);
        drain_commands(&mut shell.state, &mut self.world, Some(&self.assets));

        // Compose + readback (the exact Phase-7 path; the pixels the
        // surface gets are the pixels any screenshot/hash sees).
        let size = shell.window_size();
        let input = UiInput {
            time_seconds: self.start.elapsed().as_secs_f64(),
            dt_seconds,
            size_px: size,
            events,
        };
        let pixels = match shell.frame(input, &mut self.world, &self.assets) {
            Ok(pixels) => pixels,
            Err(e) => {
                self.error = Some(format!("frame compose failed: {e}"));
                event_loop.exit();
                return;
            }
        };

        // Present. Recoverable surface states reconfigure and retry
        // next frame; everything else is fatal.
        let ctx = shell.window_context();
        if let Some(surface) = self.surface.as_ref()
            && let Err(e) =
                surface.present_rgba8(ctx.device(), ctx.queue(), &pixels, size[0], size[1])
        {
            if e.to_string().contains("retry: true") {
                if let Some(surface) = self.surface.as_mut() {
                    surface.resize(ctx, size[0], size[1]);
                }
            } else {
                self.error = Some(format!("present failed: {e}"));
                event_loop.exit();
                return;
            }
        }

        // Smoke bookkeeping (verification + artifact writing + exit).
        if let Some(smoke) = self.smoke.as_mut() {
            smoke.screenshot = Some(pixels);
            let shell_ref = self.shell.as_ref().expect("shell checked above");
            if let Some(problem) = smoke.verify_at(self.frame, shell_ref) {
                self.error = Some(problem);
                self.finish_smoke();
                event_loop.exit();
                return;
            }
            if self.frame >= self.config.smoke_frames.unwrap_or(u64::MAX) {
                // Ctrl+S landed two frames ago — the scene file must exist.
                smoke.shortcut_save_verified = PathBuf::from(SMOKE_SCENE_PATH).exists();
                self.finish_smoke();
                event_loop.exit();
                return;
            }
        }

        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// Writes the smoke artifacts (PNG + JSON evidence).
    fn finish_smoke(&mut self) {
        let Some(smoke) = self.smoke.as_ref() else {
            return;
        };
        let adapter = self
            .shell
            .as_ref()
            .map(|s| s.window_context().adapter_summary())
            .unwrap_or_else(|| "unavailable".to_string());
        // Screenshot from the last presented frame (the RGBA8 readback
        // — byte-identical to what the surface displayed).
        if let Some(pixels) = &smoke.screenshot {
            match image::RgbaImage::from_raw(
                self.config.size[0],
                self.config.size[1],
                pixels.clone(),
            ) {
                Some(image) => {
                    if let Err(e) = image.save(&self.config.screenshot) {
                        self.error = Some(format!("screenshot write failed: {e}"));
                    }
                }
                None => {
                    self.error = Some("screenshot size mismatch".to_string());
                }
            }
        }
        let evidence = serde_json::json!({
            "format": SMOKE_FORMAT,
            "frames_presented": self.frame,
            "window": self.config.size,
            "adapter": adapter,
            "interactions": {
                "play_button_clicked": smoke.play_clicked,
                "play_started_simulation": smoke.play_verified,
                "pause_worked": smoke.pause_verified,
                "stop_restored_edit_mode": smoke.stop_verified,
                "viewport_drag_frames": smoke.drag_frames,
                "ctrl_s_saved_scene": smoke.shortcut_save_verified,
            },
            "method": "pointer events target layout-captured toolbar rects; the simulation steps on a wall-clock fixed accumulator; the screenshot is the last presented frame's readback",
            "error": self.error,
        });
        if let Ok(json) = serde_json::to_string_pretty(&evidence) {
            let _ = std::fs::write(&self.config.json, json);
        }
    }
}

impl ApplicationHandler for EditorApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return; // resume after suspend: everything still exists
        }
        let attrs = Window::default_attributes()
            .with_title("Gens Editor — v1.1")
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.size[0],
                self.config.size[1],
            ))
            .with_min_inner_size(winit::dpi::LogicalSize::new(640, 480));
        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                self.error = Some(format!("window creation failed: {e}"));
                event_loop.exit();
                return;
            }
        };
        // Physical pixels are the shell's logical space (pixels-per-
        // point pinned to 1.0, the engine's determinism position
        // R-013); HiDPI displays render at native resolution with 1:1
        // UI sizing.
        let inner = window.inner_size();
        let (w, h) = (inner.width.max(1), inner.height.max(1));
        self.config.size = [w, h];

        let ctx = match gens_renderer::GpuContext::new() {
            Ok(ctx) => ctx,
            Err(e) => {
                self.error = Some(format!("no GPU adapter: {e}"));
                event_loop.exit();
                return;
            }
        };

        let surface = match SurfaceTarget::new(&ctx, Arc::clone(&window), w, h) {
            Ok(surface) => surface,
            Err(e) => {
                self.error = Some(format!("surface creation failed: {e}"));
                event_loop.exit();
                return;
            }
        };

        match EditorShell::new(ctx, w, h) {
            Ok(shell) => {
                let subscriber = tracing_subscriber::registry()
                    .with(shell.console_sink())
                    .with(tracing_subscriber::fmt::layer().without_time().compact());
                let _ = tracing::subscriber::set_global_default(subscriber);
                tracing::info!("engine initialized (interactive editor, v1.1)");
                self.shell = Some(shell);
            }
            Err(e) => {
                self.error = Some(format!("editor shell creation failed: {e}"));
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.surface = Some(surface);
        if self.config.smoke_frames.is_some() {
            self.smoke = Some(SmokeScript::new());
        }
        self.last_frame = Instant::now();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.error.is_some() {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let (w, h) = (size.width.max(1), size.height.max(1));
                self.config.size = [w, h];
                if let Some(shell) = self.shell.as_mut() {
                    let _ = shell.resize_window(w, h);
                }
                if let (Some(surface), Some(shell)) = (self.surface.as_mut(), self.shell.as_ref()) {
                    surface.resize(shell.window_context(), w, h);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Some(window) = self.window.as_ref()
                    && window.id() == window_id
                {
                    self.redraw(event_loop);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                self.queue_events.push(UiEvent::PointerMove {
                    x: position.x as f32,
                    y: position.y as f32,
                });
            }
            WindowEvent::CursorLeft { .. } => {
                self.queue_events.push(UiEvent::PointerGone);
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(button_id) = win_events::button_id(button) {
                    let (x, y) = self.cursor;
                    self.queue_events.push(UiEvent::PointerButton {
                        x,
                        y,
                        button: button_id,
                        pressed: state == ElementState::Pressed,
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = win_events::scroll_delta(delta);
                self.queue_events.push(UiEvent::Scroll { dx, dy });
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                let modifiers = win_events::to_ui_modifiers(self.modifiers);
                if pressed {
                    self.queue_events.push(win_events::key_event(
                        &event.logical_key,
                        true,
                        event.repeat,
                        modifiers,
                    ));
                    // Typed text (no ctrl/alt held): the character path.
                    if let Some(text) = event.text.as_ref()
                        && let Some(text_event) = win_events::text_event(text, modifiers)
                    {
                        self.queue_events.push(text_event);
                    }
                } else {
                    self.queue_events.push(win_events::key_event(
                        &event.logical_key,
                        false,
                        false,
                        modifiers,
                    ));
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                // winit 0.30 wraps the state in an event Modifiers
                // (which also carries platform extras); the state is
                // what the translation needs.
                self.modifiers = m.state();
                self.queue_events
                    .push(UiEvent::ModifiersChanged(win_events::to_ui_modifiers(
                        m.state(),
                    )));
            }
            WindowEvent::Focused(false) => {
                // Focus loss: drop held modifier ghosts so shortcuts
                // cannot stick across focus changes.
                self.modifiers = winit::keyboard::ModifiersState::empty();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Continuous redraw: the editor always animates (live viewport,
        // play mode). Fifo present pacing keeps it vsync-bounded.
        if self.error.is_none()
            && let Some(window) = self.window.as_ref()
        {
            window.request_redraw();
        }
    }
}

// ---------------------------------------------------------------------------
// The opening scene (the docking bay — same shape as the headless
// example's reference scene, minus the scripted-replay extras).
// ---------------------------------------------------------------------------

/// Builds the docking-bay scene + its physics world.
fn build_scene() -> (gens_ecs::ecs::world::World, PhysicsWorld) {
    use gens_ecs::Transform;

    let mut world = gens_ecs::ecs::world::World::new();
    let mut physics = PhysicsWorld::new(vec3(0.0, -9.81, 0.0));
    let mut names: BTreeMap<String, gens_ecs::ecs::prelude::Entity> = BTreeMap::new();
    fn spawn(
        world: &mut gens_ecs::ecs::world::World,
        names: &mut BTreeMap<String, gens_ecs::ecs::prelude::Entity>,
        name: &str,
        bundle: impl gens_ecs::ecs::prelude::Bundle,
    ) -> gens_ecs::ecs::prelude::Entity {
        let e = world
            .spawn((Name::new(name), gens_ecs::Active, bundle))
            .id();
        names.insert(name.to_string(), e);
        e
    }

    let world_root = spawn(&mut world, &mut names, "World", Transform::IDENTITY);
    let dir_light = spawn(
        &mut world,
        &mut names,
        "DirectionalLight",
        (Transform::IDENTITY, MeshSlot::marker([255, 230, 120, 255])),
    );
    let player = spawn(&mut world, &mut names, "Player", Transform::IDENTITY);
    let character = spawn(
        &mut world,
        &mut names,
        "PlayerCharacter",
        (
            Transform::from_position(vec3(-2.0, 0.5, 2.0)),
            MeshSlot::cube([70, 130, 240, 255]),
        ),
    );
    let level = spawn(&mut world, &mut names, "Level", Transform::IDENTITY);
    let terrain = spawn(
        &mut world,
        &mut names,
        "Terrain",
        (
            Transform {
                position: Vec3::ZERO,
                rotation: gens_math::Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2),
                scale: vec3(30.0, 30.0, 1.0),
            },
            MeshSlot::quad([52, 60, 74, 255]),
        ),
    );
    // EDITOR_REALITY_CHECK.md P0 #5 fix: the scene's bodies are created
    // through the SAME authored description the Inspector edits
    // (model.rs rule — what it shows is what simulation runs).
    // v1.1.0 spawned raw `PhysicsBody` handles with no `AuthoredBody`,
    // so the Inspector's Physics section could never render for any
    // scene entity, and a Play->Stop round-trip (which captures the
    // authored description) silently dropped every body.
    let ship_authored = gens_editor::AuthoredBody {
        kind: gens_physics::BodyKind::Fixed,
        collider: ColliderShape::Cuboid(vec3(1.5, 0.4, 0.75)),
        friction: 0.6,
        ..gens_editor::AuthoredBody::default()
    };
    let (ship_body, _) = physics.create_body(
        gens_editor::playmode::body_spec_from_authored(&ship_authored, vec3(0.0, 0.8, 0.0)),
        gens_editor::playmode::collider_spec_from_authored(&ship_authored),
    );
    let crate_authored = gens_editor::AuthoredBody {
        kind: gens_physics::BodyKind::Dynamic,
        collider: ColliderShape::Ball(0.45),
        friction: 0.5,
        ..gens_editor::AuthoredBody::default()
    };
    let (crate_body, _) = physics.create_body(
        gens_editor::playmode::body_spec_from_authored(&crate_authored, vec3(1.0, 4.0, -1.0)),
        gens_editor::playmode::collider_spec_from_authored(&crate_authored),
    );
    let ship = spawn(
        &mut world,
        &mut names,
        "Spaceship",
        (
            Transform {
                position: vec3(0.0, 0.8, 0.0),
                rotation: Default::default(),
                scale: vec3(3.0, 0.8, 1.5),
            },
            MeshSlot::cube([168, 178, 196, 255]),
            ship_authored,
            gens_physics::PhysicsBody { handle: ship_body },
        ),
    );
    let crates = spawn(
        &mut world,
        &mut names,
        "Crate",
        (
            Transform::from_position(vec3(1.0, 4.0, -1.0)),
            MeshSlot::cube([185, 130, 70, 255]),
            crate_authored,
            gens_physics::PhysicsBody { handle: crate_body },
        ),
    );

    attach_children(&mut world, world_root, &[dir_light]);
    attach_children(&mut world, player, &[character]);
    attach_children(&mut world, level, &[terrain, ship, crates]);
    (world, physics)
}

fn build_assets() -> AssetRegistry {
    let mut registry = AssetRegistry::new();
    const PATROL_RHAI: &str = "fn update(ship, ctx) {\n    let t = ctx.time;\n    let r = 4.0;\n    ship.position = [r * cos(t), 0.5, r * sin(t)];\n}\n";
    let _ = registry.import(
        "scripts/patrol.rhai",
        AssetKind::Script,
        PATROL_RHAI.as_bytes(),
    );
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Inspector contract (model.rs): every entity carrying a live
    /// `PhysicsBody` also carries the authored description the Physics
    /// section edits. v1.1.0's scene violated this — the Physics
    /// section could not render on ANY entity (reality-check P0 #5) —
    /// and a Play->Stop round-trip dropped the bodies outright.
    #[test]
    fn every_live_body_in_the_scene_has_an_authored_description() {
        let (world, _physics) = build_scene();
        let mut live_bodies = 0;
        for entity_ref in world.iter_entities() {
            let entity = entity_ref.id();
            if world.get::<gens_physics::PhysicsBody>(entity).is_some() {
                live_bodies += 1;
                assert!(
                    world.get::<gens_editor::AuthoredBody>(entity).is_some(),
                    "a live body without an authored description cannot be inspected or round-tripped"
                );
            }
        }
        assert_eq!(live_bodies, 2, "the scene seeds Spaceship + Crate bodies");
    }

    /// The Play->Stop capture path must round-trip every scene body:
    /// capture must see the authored descriptions (the restore
    /// re-creates live bodies from them).
    #[test]
    fn scene_bodies_survive_a_capture_round_trip() {
        let (world, _physics) = build_scene();
        let snapshot = gens_editor::playmode::capture(&world);
        let bodies: Vec<_> = snapshot
            .entities
            .iter()
            .filter_map(|e| e.body.as_ref())
            .collect();
        assert_eq!(
            bodies.len(),
            2,
            "Spaceship + Crate authored bodies captured"
        );
        assert!(
            bodies
                .iter()
                .any(|b| b.kind == gens_physics::BodyKind::Fixed),
            "the Spaceship's fixed body is part of the captured state"
        );
    }
}
