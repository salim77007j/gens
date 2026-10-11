//! Input device abstraction and rebinding action maps (Phase 3, ADR-0015).
//!
//! The Phase 3 core is a **pure, headless-testable input model**:
//!
//! - devices are abstracted as a [`keys::DeviceEvent`] stream fed into
//!   [`state::InputState`] by the platform layer (tests feed events
//!   directly — no window system required);
//! - gameplay never reads raw keys: it queries *actions* ("Jump", "MoveLeft")
//!   bound to keys/buttons through a rebindable [`state::InputMap`];
//! - edge detection ("just pressed/released") is frame-accurate via an
//!   explicit [`state::InputState::update`] rotation at end of frame.
//!
//! Real device backends (winit keyboard/mouse, gilrs gamepads) attach to
//! [`keys::DeviceEvent`] when the windowing phase lands; nothing in the
//! action model changes that day (ADR-0015). The crate is deliberately
//! bevy-free: ECS resource glue is a thin wrapper added alongside the
//! device backends (see ADR-0015 for the bevy `Resource: Component`
//! constraint that shapes that wrapper).

pub mod keys;
pub mod state;

pub use keys::{DeviceEvent, KeyCode, MouseButton};
pub use state::{Action, Binding, InputMap, InputState, axis, axis_pair};
