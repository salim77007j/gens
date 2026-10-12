//! Scripting layer for game logic (Phase 5).
//!
//! `gens-scripting` embeds the Rhai interpreter behind an engine-owned,
//! **sandboxed, deterministic** facade:
//!
//! - [`engine::ScriptEngine`] — `Engine::new_raw()` (no packages at
//!   all: no filesystem, no network, no `print`, no module resolver,
//!   so `import` statements cannot resolve) plus hard limits on
//!   operations, call depth, expression depth and collection sizes;
//! - [`Script`](engine::Script) — a compiled, named AST;
//! - [`vars::ScriptVars`] — the script-visible state bag
//!   (`BTreeMap<String, ScriptValue>`): serializable, ordered, and the
//!   ONLY state a script can touch besides its arguments.
//!
//! Determinism contract (ADR-0019): Rhai is pinned without
//! `ahash/runtime-rng` (fixed compile-time-rng hashing), its script
//! maps are `BTreeMap`s, there is no clock and no RNG in the sandbox —
//! the same script, scope and argument values always produce the same
//! results within a process (the R-013 scope). Cross-process
//! determinism is not claimed.

pub mod engine;
pub mod error;
pub mod host;
pub mod vars;

pub use engine::{Script, ScriptEngine};
pub use error::ScriptError;
pub use host::{ScriptComponent, ScriptFrame, ScriptHost, ScriptRequests, run_scripts};
pub use vars::{ScriptValue, ScriptVars};
