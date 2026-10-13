//! The windowing shell's platform layer (v1.1, ADR-0036).
//!
//! Two responsibilities, both deliberately thin and pure where
//! possible:
//!
//! - [`surface`] — swapchain ownership and the upload-present loop
//!   layered over the offscreen composition path;
//! - [`events`] — winit device traffic translated into the neutral
//!   [`gens_ui::UiEvent`] stream (ADR-0015's promise fulfilled).
//!
//! The winit event loop itself lives in the `gens-editor` binary
//! (`src/bin/gens-editor.rs`): an [`winit::application::ApplicationHandler`]
//! that owns the [`crate::EditorShell`], steps the simulation when play
//! mode is active, drains editor commands, and presents every frame.

pub mod events;
pub mod surface;

pub use surface::{SurfaceTarget, choose_surface_format, rgba8_to_surface_bytes};
