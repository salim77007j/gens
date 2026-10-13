//! Runtime UI and text rendering (Phase 4).
//!
//! `gens-ui` wraps **egui** (immediate mode, the delegation's UI
//! shortlist) behind two layers:
//!
//! - [`context::UiContext`] — the pure, headless core: neutral
//!   [`frame::UiEvent`] streams go in, fully tessellated
//!   [`frame::UiFrame`]s (pixel-space meshes + texture uploads) come
//!   out. No wgpu, no window system — the same (state, input) sequence
//!   produces byte-identical frames, pinned by a replay test.
//! - [`backend::UiRenderer`] — the wgpu backend that draws frames
//!   through the gens-renderer facade with premultiplied-alpha blending
//!   and scissor clipping (vertices converted to NDC on the CPU, so the
//!   UI pipeline needs no uniforms at all).
//!
//! Why egui: immediate-mode fits the engine's deterministic, replay-
//! tested simulation style (UI layout is a function of state + input),
//! the default fonts are embedded (epaint_default_fonts — no system
//! font dependency on CI), and egui 0.36's headless `run_ui` contract
//! matches the crate's per-pass frame model exactly. Why not
//! `egui-wgpu`: it pins its own wgpu minor while gens pins wgpu
//! 30.0.1 — rendering egui's tessellated output through gens' own
//! pipeline keeps exactly one wgpu in the dependency tree (same
//! unification discipline as the rapier glam tree in Phase 3).
//!
//! Determinism: `pixels_per_point` is pinned to 1.0, the clock is the
//! engine's simulation time, and texture deltas are emitted sorted —
//! see `docs/RISKS.md` R-013 for the platform bit-identity position.

pub mod backend;
pub mod context;
pub mod frame;

pub use backend::{UiRender, UiRenderer};
pub use context::UiContext;
pub use frame::{
    TextureFree, TextureRef, TextureUpload, UiEvent, UiFrame, UiInput, UiKey, UiMesh, UiModifiers,
    UiVertex,
};
