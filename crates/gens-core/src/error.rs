//! Engine-wide error types (Phase 1).
//!
//! Gens uses two complementary error strategies, matching common Rust
//! practice: library code returns [`EngineError`] via the [`Result`] alias
//! (typed, cheap, thiserror-generated), while application-level code (game
//! scripts, the runtime shell, the editor) may collapse failures into
//! [`anyhow::Error`] — re-exported here so games depend only on gens crates.

use thiserror::Error;

/// Convenience alias used across gens crates.
pub type Result<T, E = EngineError> = std::result::Result<T, E>;

/// The engine's canonical error type.
#[derive(Debug, Error)]
pub enum EngineError {
    /// An asset path could not be resolved or loaded (Phase 5 pipeline).
    #[error("asset not found: {0}")]
    AssetNotFound(String),

    /// An operation was invoked in a state that forbids it, with context
    /// naming the subsystem and a human-readable problem description.
    #[error("invalid state in {context}: {problem}")]
    InvalidState {
        /// Subsystem that rejected the operation (e.g. "game_loop").
        context: &'static str,
        /// What exactly was wrong.
        problem: String,
    },

    /// A feature gated behind a later phase was requested. Makes the
    /// roadmap explicit at runtime instead of panicking.
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),

    /// Transparent passthrough of I/O failures (file, network, stdio).
    #[error("io error")]
    Io(#[from] std::io::Error),
}

impl EngineError {
    /// Builds an [`EngineError::InvalidState`] with less ceremony.
    pub fn invalid(context: &'static str, problem: impl Into<String>) -> EngineError {
        EngineError::InvalidState {
            context,
            problem: problem.into(),
        }
    }
}

/// Re-exported so game code can depend on `gens-core` alone.
pub use anyhow;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn display_messages_are_actionable() {
        let e = EngineError::AssetNotFound("res://models/ship.glb".into());
        assert_eq!(e.to_string(), "asset not found: res://models/ship.glb");

        let e = EngineError::invalid("game_loop", "time_scale must be >= 0");
        assert_eq!(
            e.to_string(),
            "invalid state in game_loop: time_scale must be >= 0"
        );

        let e = EngineError::NotImplemented("renderer");
        assert_eq!(e.to_string(), "not implemented yet: renderer");
    }

    #[test]
    fn io_errors_convert_transparently() {
        fn produces_io() -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::NotFound, "missing file"))
        }
        let result: Result<()> = produces_io().map_err(EngineError::from);
        let err = result.unwrap_err();
        assert!(matches!(err, EngineError::Io(_)));
        // anyhow interop works through the standard From chain.
        let any: anyhow::Error = err.into();
        assert!(any.to_string().contains("io error"));
    }
}
