//! The Gens visual editor (Phase 7).
//!
//! A docked-panel editor in the shape of the reference design
//! (`docs/design/REFERENCE.png`): menu bar, scene hierarchy, 3D
//! viewport, inspector, asset browser, developer console and status
//! bar — built on egui 0.36 and the engine's own crates, with **no
//! windowing dependency**: the editor runs offscreen-first and renders
//! to a final RGBA readback (the same doctrine as every phase since
//! ADR-0003), so CI on all three OSes can exercise and snapshot the
//! real UI. A native window shell is future work once a winit
//! integration lands (see ADR-0025).
//!
//! Composition pipeline per frame:
//!
//! ```text
//! ECS world ──snapshot──▶ EditorState (selection, tree, assets, console)
//!      │                                    │
//!      ▼                                    ▼
//! viewport Renderer (3D scene, own offscreen target)
//!      └─ readback RGBA ──▶ egui managed texture ──▶ ui.image(...)
//!                                                     │
//! window Renderer (full editor UI, UiRenderer::render)
//!      └─ final RGBA ──▶ PNG / FNV hash (evidence)
//! ```
//!
//! Phase 9 adds the [`cinema`] panel data: the Game View / Animator /
//! Network tabs consume real snapshots (cinematic tracks + clock, camera
//! pose, session counters) and render honest empty states when absent.
//!
//! Panels reflect **real engine vocabulary only** (see ADR-0025): the
//! hierarchy mirrors the actual ECS tree, the inspector edits actual
//! components, the asset browser mirrors the actual [`gens_assets`]
//! registry and the console captures actual `tracing` events.

pub mod assets;
pub mod cinema;
pub mod commands;
pub mod console;
pub mod hierarchy;
pub mod inspector;
pub mod model;
pub mod playmode;
pub mod shell;
pub mod telemetry;
pub mod theme;
pub mod viewport;
pub mod window;

pub use model::{AuthoredBody, EditorCommand, MeshSlot, ShapeKind};
pub use playmode::{EntitySnapshot, WorldSnapshot};
pub use shell::{BottomTab, EditorShell, EditorState, FrameStats, ViewTab};
