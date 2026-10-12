//! The sandboxed Rhai engine wrapper (P5-3, ADR-0019).
//!
//! [`ScriptEngine`] builds on `Engine::new_raw()` — **no packages**:
//! no filesystem, no network, no `print`/`debug` functions, no module
//! resolver (so `import` statements cannot resolve anything), and
//! therefore no IO surface at all. Hard limits (below) turn runaway or
//! hostile scripts into clean errors instead of hangs — CI must never
//! trust content.
//!
//! Everything the script may touch beyond pure computation arrives as
//! explicit host-registered functions (P5-4) or its [`ScriptVars`]
//! bag — and the host registers NO clock and NO RNG, keeping script
//! evaluation a pure function of (source, scope, args).

use crate::error::{ScriptError, ScriptResult};

/// Hard limit: script operations per call (`set_max_operations`).
pub const MAX_OPERATIONS: u64 = 100_000;
/// Hard limit: nested function-call levels.
pub const MAX_CALL_LEVELS: usize = 32;
/// Hard limit: expression nesting depth (top level and functions).
pub const MAX_EXPR_DEPTH: usize = 32;
/// Hard limit: object-map size.
pub const MAX_MAP_SIZE: usize = 1_024;
/// Hard limit: array size.
pub const MAX_ARRAY_SIZE: usize = 1_024;
/// Hard limit: string size in bytes.
pub const MAX_STRING_SIZE: usize = 8_192;

/// A compiled, named script (the Rhai AST plus its identity).
///
/// `Clone` — compiling once and sharing is the intended use; ASTs are
/// immutable after compilation.
#[derive(Debug, Clone)]
pub struct Script {
    name: String,
    ast: rhai::AST,
}

impl Script {
    /// The script's identity (its asset key or logical name).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The compiled AST (for advanced host use).
    pub fn ast(&self) -> &rhai::AST {
        &self.ast
    }
}

/// The sandboxed script engine.
#[derive(Debug)]
pub struct ScriptEngine {
    engine: rhai::Engine,
}

impl Default for ScriptEngine {
    fn default() -> Self {
        ScriptEngine::new()
    }
}

impl ScriptEngine {
    /// Builds the sandbox: raw engine (no packages) + hard limits.
    pub fn new() -> ScriptEngine {
        let mut engine = rhai::Engine::new_raw();
        engine
            .set_max_operations(MAX_OPERATIONS)
            .set_max_call_levels(MAX_CALL_LEVELS)
            .set_max_expr_depths(MAX_EXPR_DEPTH, MAX_EXPR_DEPTH)
            .set_max_map_size(MAX_MAP_SIZE)
            .set_max_array_size(MAX_ARRAY_SIZE)
            .set_max_string_size(MAX_STRING_SIZE);
        ScriptEngine { engine }
    }

    /// Compiles `source` into a [`Script`].
    ///
    /// # Errors
    /// [`ScriptError::Compile`] with Rhai's diagnostics.
    pub fn compile(&self, name: &str, source: &str) -> ScriptResult<Script> {
        let ast = self
            .engine
            .compile(source)
            .map_err(|source| ScriptError::Compile {
                name: name.to_string(),
                source,
            })?;
        Ok(Script {
            name: name.to_string(),
            ast,
        })
    }

    /// Calls a script-defined function with no script-scope variables.
    ///
    /// The result crosses the boundary as a [`rhai::Dynamic`] and is
    /// converted at the wrapper edge — the return type is checked
    /// HERE, not inside Rhai, so the sandbox surface stays public-API
    /// only.
    ///
    /// # Errors
    /// [`ScriptError::Runtime`] (limits, missing function, panics) or
    /// [`ScriptError::WrongReturn`] on type mismatch.
    pub fn call_fn<T, A>(&self, script: &Script, fn_name: &str, args: A) -> ScriptResult<T>
    where
        T: std::any::Any + Clone,
        A: rhai::FuncArgs,
    {
        let mut scope = rhai::Scope::new();
        self.call_fn_with_scope(&mut scope, script, fn_name, args)
    }

    /// Calls a script function with an explicit scope (the host may
    /// expose [`ScriptVars`] values here via P5-4's API layer).
    ///
    /// # Errors
    /// Same as [`Self::call_fn`].
    pub fn call_fn_with_scope<T, A>(
        &self,
        scope: &mut rhai::Scope,
        script: &Script,
        fn_name: &str,
        args: A,
    ) -> ScriptResult<T>
    where
        T: std::any::Any + Clone,
        A: rhai::FuncArgs,
    {
        let result: rhai::Dynamic = self
            .engine
            .call_fn(scope, script.ast(), fn_name, args)
            .map_err(|source| ScriptError::Runtime {
                script: script.name().to_string(),
                fn_name: fn_name.to_string(),
                source,
            })?;
        result
            .try_cast::<T>()
            .ok_or_else(|| ScriptError::WrongReturn {
                script: script.name().to_string(),
                fn_name: fn_name.to_string(),
                expected: std::any::type_name::<T>(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vars::ScriptVars;

    fn engine() -> ScriptEngine {
        ScriptEngine::new()
    }

    #[test]
    fn compiles_and_calls() {
        let e = engine();
        let s = e
            .compile("t.rhai", "fn add(a, b) { a + b } fn two() { 2.0 * 21.0 }")
            .expect("compile");
        let sum: f64 = e.call_fn(&s, "add", (3.0, 4.0)).expect("call");
        assert_eq!(sum, 7.0);
        let two: f64 = e.call_fn(&s, "two", ()).expect("call");
        assert_eq!(two, 42.0);
        assert_eq!(s.name(), "t.rhai");
    }

    #[test]
    fn compile_error_reports_diagnostics() {
        let e = engine();
        let err = e.compile("bad.rhai", "fn ( {").expect_err("syntax error");
        assert!(matches!(err, ScriptError::Compile { .. }));
        let msg = err.to_string();
        assert!(msg.contains("bad.rhai"));
    }

    #[test]
    fn runtime_missing_function_errors() {
        let e = engine();
        let s = e.compile("t.rhai", "fn f() { 1 }").expect("compile");
        let err = e
            .call_fn::<f64, _>(&s, "no_such_fn", ())
            .expect_err("missing fn");
        assert!(matches!(err, ScriptError::Runtime { .. }));
    }

    #[test]
    fn runaway_loop_hits_operation_limit() {
        let e = engine();
        // A pure-Rhai infinite loop; the operation cap converts the
        // hang into an error (CI must never trust content).
        let s = e
            .compile("spin.rhai", "fn spin() { let x = 0; loop { x += 1; } }")
            .expect("compile");
        let err = e
            .call_fn::<i64, _>(&s, "spin", ())
            .expect_err("must hit cap");
        match err {
            ScriptError::Runtime { source, .. } => {
                // Rhai's Display: "Too many operations".
                assert!(source.to_string().contains("operations"), "{source}");
            }
            other => panic!("unexpected: {other}"),
        }
    }

    #[test]
    fn runaway_recursion_hits_call_level_limit() {
        let e = engine();
        let s = e
            .compile("rec.rhai", "fn r(n) { r(n + 1) }")
            .expect("compile");
        let err = e
            .call_fn::<i64, _>(&s, "r", (0_i64,))
            .expect_err("must hit cap");
        assert!(matches!(err, ScriptError::Runtime { .. }));
    }

    #[test]
    fn no_print_function_in_sandbox() {
        let e = engine();
        // `print` lives in a package; new_raw has none.
        let s = e
            .compile("p.rhai", "fn p() { print(\"hi\") }")
            .expect("compile");
        let err = e
            .call_fn::<(), _>(&s, "p", ())
            .expect_err("print must be absent");
        assert!(matches!(err, ScriptError::Runtime { .. }));
    }

    #[test]
    fn no_module_import_in_sandbox() {
        let e = engine();
        // No module resolver: import PARSES but fails to resolve at
        // evaluation time (the resolver is where the filesystem would
        // be consulted; new_raw has none).
        let s = e
            .compile("imp.rhai", "fn drive() { import \"evil\" as x; x.f() }")
            .expect("import parses");
        let err = e
            .call_fn::<i64, _>(&s, "drive", ())
            .expect_err("import must not resolve");
        assert!(matches!(err, ScriptError::Runtime { .. }));
    }

    #[test]
    fn deterministic_replay_of_traced_calls() {
        // The core P5 promise: identical calls -> identical results,
        // asserted across many runs in this process (R-013 scope).
        let e = engine();
        let s = e
            .compile(
                "det.rhai",
                "fn step(x, v) { let a = x * 2.0 + v; if a > 10.0 { a - 10.0 } else { a } }",
            )
            .expect("compile");
        let mut trace = String::new();
        for i in 0..50 {
            let v: f64 = e.call_fn(&s, "step", (i as f64, 0.25)).expect("call");
            trace.push_str(&format!("{v};"));
        }
        let mut trace2 = String::new();
        for i in 0..50 {
            let v: f64 = e.call_fn(&s, "step", (i as f64, 0.25)).expect("call");
            trace2.push_str(&format!("{v};"));
        }
        assert_eq!(trace, trace2);
    }

    #[test]
    fn script_maps_are_ordered_and_cloneable() {
        let e = engine();
        // Rhai's object maps are BTreeMap-backed (see ADR-0019): the
        // HOST-side iteration order is key-sorted regardless of the
        // script's insertion order.
        let s = e
            .compile("map.rhai", "fn m() { #{z: 1, a: 2, m: 3} }")
            .expect("compile");
        let map: rhai::Map = e.call_fn(&s, "m", ()).expect("call");
        let names: Vec<String> = map.keys().map(|k| k.to_string()).collect();
        assert_eq!(names, vec!["a", "m", "z"]);
    }

    #[test]
    fn scope_roundtrip_with_vars_bag() {
        // The P5-4 pattern preview: ScriptVars feed a Scope, script
        // writes come back into the bag (kind-checked).
        let e = engine();
        let mut vars = ScriptVars::new();
        vars.set_number("speed", 3.0).expect("set");
        let s = e
            .compile("bag.rhai", "fn step() { speed = speed + 1.0; speed }")
            .expect("compile");
        let mut scope = rhai::Scope::new();
        scope.push("speed", vars.get_number("speed").expect("get"));
        let result: f64 = e
            .call_fn_with_scope(&mut scope, &s, "step", ())
            .expect("call");
        assert_eq!(result, 4.0);
        let new_speed = scope.get_value::<f64>("speed").expect("written back");
        vars.set_number("speed", new_speed).expect("set");
        assert_eq!(vars.get_number("speed").expect("get"), 4.0);
    }
}
