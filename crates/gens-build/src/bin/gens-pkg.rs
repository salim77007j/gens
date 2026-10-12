//! `gens-pkg` — build and verify GENSPKG1 game packages (Phase 8).
//!
//! ```text
//! gens-pkg build --name demo --scene scenes/main.json --asset scripts/patrol.rhai --out demo.genspkg
//! gens-pkg verify demo.genspkg
//! ```
//!
//! Keys are the argument paths as given (forward-slashed). Scenes
//! reference scripts by those keys, so run the CLI from the project root
//! with project-relative paths — a scene pointing at a key that is not in
//! the package fails at LOAD time (`ScriptNotRegistered`), not silently.

use gens_assets::kind_from_path;
use gens_build::{build_package, verify_package};
use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    let mut args = std::env::args().skip(1);
    let cmd = args.next().context("usage: gens-pkg build|verify ...")?;

    match cmd.as_str() {
        "build" => {
            let mut name = String::new();
            let mut scene: Option<PathBuf> = None;
            let mut assets: Vec<PathBuf> = Vec::new();
            let mut out = PathBuf::from("game.genspkg");
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--name" => name = args.next().context("--name needs a value")?,
                    "--scene" => {
                        scene = Some(PathBuf::from(args.next().context("--scene needs a value")?))
                    }
                    "--asset" => {
                        assets.push(PathBuf::from(args.next().context("--asset needs a value")?))
                    }
                    "--out" => out = PathBuf::from(args.next().context("--out needs a value")?),
                    other => anyhow::bail!("unknown argument: {other}"),
                }
            }
            let scene = scene.context("--scene is required")?;
            anyhow::ensure!(!name.is_empty(), "--name is required");

            // Deterministic order: scene first, then assets sorted by path.
            let scene_bytes = std::fs::read(&scene)
                .with_context(|| format!("reading scene {}", scene.display()))?;
            let scene_key = scene.to_string_lossy().replace('\\', "/");
            assets.sort();

            let mut files = Vec::new();
            for path in assets {
                let bytes = std::fs::read(&path)
                    .with_context(|| format!("reading asset {}", path.display()))?;
                let key = path.to_string_lossy().replace('\\', "/");
                let kind = kind_from_path(&key);
                files.push((key, kind, bytes));
            }

            let pkg = build_package(&name, &scene_key, &scene_bytes, files)?;
            std::fs::write(&out, &pkg).with_context(|| format!("writing {}", out.display()))?;
            let verified = verify_package(&pkg).context("built package failed verification")?;
            info!(
                files = verified.manifest.files.len(),
                bytes = pkg.len(),
                scene = %verified.manifest.scene_key,
                path = %out.display(),
                %filter,
                "package built and re-verified"
            );
        }
        "verify" => {
            let path = PathBuf::from(args.next().context("verify needs a package path")?);
            let bytes =
                std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let verified = verify_package(&bytes)
                .with_context(|| format!("{} failed verification", path.display()))?;
            info!(
                files = verified.manifest.files.len(),
                entities = verified.scene.entities.len(),
                name = %verified.manifest.name,
                engine = %verified.manifest.engine_version,
                "package verified: all file hashes match"
            );
            for file in &verified.manifest.files {
                info!(key = %file.key, kind = %file.kind, len = file.len, hash = format!("{:016x}", file.hash), "entry");
            }
        }
        other => anyhow::bail!("unknown command: {other} (use build|verify)"),
    }
    Ok(())
}
