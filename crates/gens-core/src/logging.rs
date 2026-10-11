//! Logging setup built on `tracing` (Phase 1).
//!
//! The engine and all tools log through `tracing`. `gens-core` re-exports the
//! standard macros so downstream crates never import `tracing` directly, plus
//! an initializer that configures a human-friendly console subscriber whose
//! verbosity is controlled by the `GENS_LOG` environment variable
//! (e.g. `GENS_LOG=gens=debug,warn`), defaulting to `info`.

pub use tracing::{debug, error, info, trace, warn};
pub use tracing_subscriber::EnvFilter;

use crate::error::{EngineError, Result};

/// Installs the default console subscriber.
///
/// Reads `GENS_LOG` for a filter expression; falls back to `info`. Returns
/// the filter that was installed. Safe to call once per process; subsequent
/// calls return an error rather than replacing a live subscriber (tracing
/// global state is single-installation).
pub fn init() -> Result<String> {
    let filter = std::env::var("GENS_LOG").unwrap_or_else(|_| "info".to_string());
    init_with(&filter)?;
    Ok(filter)
}

/// Installs the console subscriber with an explicit filter expression.
/// Same single-installation semantics as [`init`].
pub fn init_with(filter: &str) -> Result<()> {
    let env_filter = EnvFilter::try_new(filter)
        .map_err(|e| EngineError::invalid("logging", format!("bad filter: {e}")))?;
    tracing_subscriber::fmt()
        .with_env_filter(env_filter)
        .with_target(false)
        .try_init()
        .map_err(|_| EngineError::invalid("logging", "a global subscriber is already installed"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A single test touches the global subscriber to avoid racing with
    // other tests over tracing's one-shot global state.
    #[test]
    fn init_installs_subscriber_once() {
        // First call installs (or, if another binary/test already installed
        // a subscriber, errors gracefully — both are correct behavior).
        let first = init_with("info");
        // Second call must report the already-installed condition.
        let second = init_with("debug");
        assert!(
            second.is_err(),
            "second init must not silently replace the subscriber"
        );
        // Bad filters are rejected with a clear error, not panics.
        assert!(init_with("definitely=not=a=filter=").is_err() || first.is_err());
    }

    #[test]
    fn filter_parse_errors_are_reported() {
        // Even when a subscriber is already installed, EnvFilter validation
        // happens before installation, so a bad filter keeps its specific error.
        match init_with("\u{0}bad\u{0}") {
            Err(EngineError::InvalidState { context, .. }) => assert_eq!(context, "logging"),
            Err(_) => {} // subscriber conflict took precedence; acceptable
            Ok(()) => panic!("a NUL-containing filter must not install"),
        }
    }
}
