//! Game loop, time system, type-safe events, logging and error handling.
//!
//! Implemented across Phase 1 sub-items (see docs/WORKLOG.md):
//! - [`error`]: [`EngineError`] + [`Result`] alias + anyhow re-export
//! - [`logging`]: tracing setup with `GENS_LOG` filter
//! - `time`: clocks with scaling (P1-5)
//! - `events`: type-safe double-buffered event bus (P1-6)
//! - `loop_`: fixed-timestep game loop (P1-7)

pub mod error;
pub mod events;
pub mod game_loop;
pub mod logging;
pub mod time;

/// The anyhow crate, re-exported for game code (see `error` module docs).
pub use anyhow;
pub use error::{EngineError, Result};
