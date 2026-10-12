//! Scripting errors.

/// Errors raised by the scripting layer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ScriptError {
    /// The source failed to parse or validate.
    #[error("script {name:?} failed to compile: {source}")]
    Compile {
        /// Script name (for diagnostics).
        name: String,
        /// Rhai's parse error.
        source: rhai::ParseError,
    },
    /// A script function call failed at runtime (limits, panics,
    /// missing function, type mismatch — Rhai's one error type).
    #[error("script {script:?} call {fn_name:?} failed: {source}")]
    Runtime {
        /// Script name (for diagnostics).
        script: String,
        /// The function that was being called.
        fn_name: String,
        /// Rhai's evaluation error.
        source: Box<rhai::EvalAltResult>,
    },
    /// The call succeeded but the return value could not be converted
    /// to the requested engine type.
    #[error("script {script:?} call {fn_name:?} returned an unexpected type (expected {expected})")]
    WrongReturn {
        /// Script name (for diagnostics).
        script: String,
        /// The function that was called.
        fn_name: String,
        /// The engine-side type name that was expected.
        expected: &'static str,
    },
    /// The variable bag rejected an update (type conflict on set).
    #[error("variable {name:?} holds a {existing}; cannot store {attempted}")]
    VarTypeConflict {
        /// Variable name.
        name: String,
        /// The stored value's kind.
        existing: crate::vars::ScriptValueKind,
        /// The attempted value's kind.
        attempted: crate::vars::ScriptValueKind,
    },
}

/// Convenience alias for scripting results.
pub type ScriptResult<T> = Result<T, ScriptError>;
