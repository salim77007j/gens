//! `gens-player` — run a GENSPKG1 package headlessly and emit a
//! `gens-run-report-1` JSON (Phase 8).
//!
//! ```text
//! gens-player --package demo.genspkg --steps 600 --out report.json
//! ```

use gens_build::run_package;
use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use gens_profiler::Profiler;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut steps: u64 = 600;
    let mut package = PathBuf::from("game.genspkg");
    let mut out = PathBuf::from("run_report.json");
    let mut profile = true;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--package" => package = PathBuf::from(args.next().context("--package needs a value")?),
            "--steps" => {
                steps = args
                    .next()
                    .context("--steps needs a value")?
                    .parse()
                    .context("--steps must be a number")?;
            }
            "--out" => out = PathBuf::from(args.next().context("--out needs a value")?),
            "--no-profile" => profile = false,
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens player - headless package runner");
    info!(?package, steps, ?out, %filter, "run configuration");

    let bytes =
        std::fs::read(&package).with_context(|| format!("reading {}", package.display()))?;
    let mut profiler = if profile {
        Profiler::enabled()
    } else {
        Profiler::disabled()
    };
    let report = run_package(&bytes, steps, &mut profiler).context("package run failed")?;

    anyhow::ensure!(
        report.run.violations.is_empty(),
        "world validation failed after the run: {:?}",
        report.run.violations
    );

    let json = serde_json::to_string_pretty(&report)?;
    std::fs::write(&out, &json).with_context(|| format!("writing {}", out.display()))?;
    info!(
        entities = report.run.entities,
        ticks = report.run.script_ticks,
        final_hash = format!("{:016x}", report.run.final_hash),
        bytes = json.len(),
        path = %out.display(),
        "run report written"
    );
    Ok(())
}
