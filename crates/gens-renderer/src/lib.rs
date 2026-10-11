//! wgpu-based renderer targeting Vulkan / DirectX 12 / Metal / WebGPU (Phase 2).
//!
//! Gens renders **offscreen-first** (ADR-0010): the primary path draws into
//! engine-owned color+depth textures and reads pixels back to CPU memory, so
//! the full renderer runs on machines with no GPU driver beyond a software
//! rasterizer (CI uses Mesa's lavapipe — ADR-0011) and no window system.
//! Window presentation is layered on top in a later phase.
//!
//! Modules:
//! - [`context`]: GPU instance/adapter/device management with the ADR-0003
//!   selection policy (discrete > integrated > software).
//!
//! All shaders are WGSL (ADR-0003). Engine code depends on this crate, never
//! on wgpu directly (ADR-0001), except through explicitly re-exported types.

pub mod context;
pub mod mesh;
pub mod target;

pub use context::{AdapterClass, GpuContext};
