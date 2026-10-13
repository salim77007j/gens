//! The `gens-release-1` release manifest (Phase 10, row 10
//! "validation").
//!
//! A machine-readable inventory of what the engine IS at the final
//! phase — derived from the repository, never asserted by hand:
//!
//! - workspace members (parsed from the root `Cargo.toml` members
//!   list — the real crate set, including Phase 9/10 additions),
//! - the ADR count and the risk-register ids (from `docs/`), so the
//!   decision ledger is inventoried, not remembered,
//! - the README roadmap rows with their honest status strings,
//! - the engine's wire-format tags (`gens-*-1`) discovered by scanning
//!   the workspace sources — the formats downstream consumers can rely
//!   on,
//! - toolchain + environment disclosure, and the git commit the
//!   manifest describes.
//!
//! The manifest VALIDATES ITS OWN SHAPE before writing (format tag
//! present, members non-empty, ledger counts positive, exactly 10
//! roadmap rows) and exits non-zero on any discrepancy — running it is
//! itself a gate, not a report generator.
//!
//! ```text
//! cargo run -p gens-build --example release_manifest -- --out gens-release-manifest.json
//! ```

use gens_core::anyhow;
use gens_core::anyhow::Context;
use gens_core::logging::{info, init};
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The manifest format tag.
const FORMAT_TAG: &str = "gens-release-1";

#[derive(Debug, Serialize)]
struct ReleaseManifest {
    format: &'static str,
    engine: EngineInfo,
    toolchain: ToolchainInfo,
    environment: EnvironmentInfo,
    git_commit: String,
    members: Vec<MemberInfo>,
    adrs: usize,
    risk_ids: Vec<String>,
    roadmap: Vec<RoadmapRow>,
    format_tags: Vec<String>,
    advisory: &'static str,
}

#[derive(Debug, Serialize)]
struct EngineInfo {
    name: &'static str,
    version: &'static str,
    license: &'static str,
}

#[derive(Debug, Serialize)]
struct ToolchainInfo {
    channel: String,
    rustc: String,
}

#[derive(Debug, Serialize)]
struct EnvironmentInfo {
    os: &'static str,
    arch: &'static str,
    parallelism: usize,
}

#[derive(Debug, Serialize)]
struct MemberInfo {
    path: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct RoadmapRow {
    phase: u32,
    scope: String,
    status: String,
}

fn repo_root() -> anyhow::Result<PathBuf> {
    // CARGO_MANIFEST_DIR is crates/gens-build at compile time; the
    // repository root is two levels up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .context("repository root")
}

/// Parses the root `Cargo.toml` members list (we own the file's shape;
/// a `toml` dependency would be heavier than the ten lines it saves).
fn parse_members(root: &Path) -> anyhow::Result<Vec<MemberInfo>> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).context("root Cargo.toml")?;
    let mut members = Vec::new();
    let mut in_members = false;
    for line in text.lines() {
        if line.trim_start().starts_with("members") && line.contains('[') {
            in_members = true;
            continue;
        }
        if in_members {
            if line.trim_start().starts_with(']') {
                break;
            }
            let path = line
                .trim()
                .trim_end_matches(',')
                .trim()
                .trim_matches('"')
                .to_string();
            if path.is_empty() || path.starts_with('#') {
                continue;
            }
            let cargo_toml = root.join(&path).join("Cargo.toml");
            let crate_text = std::fs::read_to_string(&cargo_toml)
                .with_context(|| cargo_toml.display().to_string())?;
            let name = crate_text
                .lines()
                .find_map(|l| {
                    l.strip_prefix("name = \"")
                        .map(|rest| rest.trim_end_matches('"').to_string())
                })
                .with_context(|| format!("crate name in {path}"))?;
            members.push(MemberInfo { path, name });
        }
    }
    Ok(members)
}

/// Counts `## ADR-` entries in docs/DECISIONS.md.
fn count_adrs(root: &Path) -> anyhow::Result<usize> {
    let text = std::fs::read_to_string(root.join("docs/DECISIONS.md")).context("DECISIONS.md")?;
    Ok(text.lines().filter(|l| l.starts_with("## ADR-")).count())
}

/// Collects the unique `R-###` risk ids from docs/RISKS.md (sorted —
/// BTreeSet, so the manifest is byte-stable run to run).
fn risk_ids(root: &Path) -> anyhow::Result<Vec<String>> {
    let text = std::fs::read_to_string(root.join("docs/RISKS.md")).context("RISKS.md")?;
    let mut ids = BTreeSet::new();
    for line in text.lines() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i + 5 <= bytes.len() {
            // Byte-wise scan with a char-boundary guard: docs contain
            // multi-byte punctuation, and slicing into a char panics.
            if line.is_char_boundary(i)
                && bytes[i] == b'R'
                && bytes[i + 1] == b'-'
                && bytes[i + 2].is_ascii_digit()
                && bytes[i + 3].is_ascii_digit()
                && bytes[i + 4].is_ascii_digit()
            {
                ids.insert(line[i..i + 5].to_string());
                i += 5;
            } else {
                i += 1;
            }
        }
    }
    Ok(ids.into_iter().collect())
}

/// Parses the README roadmap table rows (`| N | scope | status | ...`).
fn parse_roadmap(root: &Path) -> anyhow::Result<Vec<RoadmapRow>> {
    let text = std::fs::read_to_string(root.join("README.md")).context("README.md")?;
    let mut rows = Vec::new();
    for line in text.lines() {
        if !line.starts_with("| ") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // split('|') yields an empty first cell for the leading pipe;
        // a data row is ["", phase, scope, status, report, ""].
        if cells.len() < 5 {
            continue;
        }
        if let Ok(phase) = cells[1].parse::<u32>() {
            rows.push(RoadmapRow {
                phase,
                scope: cells[2].to_string(),
                status: cells[3].to_string(),
            });
        }
    }
    Ok(rows)
}

/// Scans the workspace's `.rs` files for `gens-*-1` wire-format tags.
fn discover_format_tags(root: &Path) -> anyhow::Result<Vec<String>> {
    let mut tags = BTreeSet::new();
    let mut stack = vec![root.join("crates")];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).with_context(|| dir.display().to_string())?;
        for entry in entries {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                for token in text.split("gens-").skip(1) {
                    let tag: String = token
                        .chars()
                        .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
                        .collect();
                    // Wire formats are named `gens-<thing>-N` with a
                    // single-digit version; keep those only.
                    if tag.len() > 2 && tag.ends_with("-1") && !tag.contains("--") {
                        tags.insert(format!("gens-{tag}"));
                    }
                }
            }
        }
    }
    Ok(tags.into_iter().collect())
}

fn command_output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd)
        .args(args)
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn main() -> anyhow::Result<()> {
    let mut out = PathBuf::from("gens-release-manifest.json");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => out = PathBuf::from(args.next().context("--out needs a value")?),
            other => anyhow::bail!("unknown argument: {other} (use --out PATH)"),
        }
    }
    let filter = init().unwrap_or_else(|_| "(logging already initialized)".to_string());
    info!("Gens engine - release manifest (gens-release-1)");
    info!(?out, %filter, "run configuration");

    let root = repo_root()?;
    let members = parse_members(&root)?;
    let adrs = count_adrs(&root)?;
    let risks = risk_ids(&root)?;
    let roadmap = parse_roadmap(&root)?;
    let format_tags = discover_format_tags(&root)?;

    let manifest = ReleaseManifest {
        format: FORMAT_TAG,
        engine: EngineInfo {
            name: "gens",
            version: env!("CARGO_PKG_VERSION"),
            license: "MIT OR Apache-2.0",
        },
        toolchain: ToolchainInfo {
            channel: command_output("rustc", &["--version"]),
            rustc: command_output("rustc", &["--version", "--verbose"]),
        },
        environment: EnvironmentInfo {
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            parallelism: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
        },
        git_commit: command_output("git", &["rev-parse", "HEAD"]),
        members,
        adrs,
        risk_ids: risks,
        roadmap,
        format_tags,
        advisory: "release inventory: describes THIS commit and environment; the roadmap status strings are the phases' own honest reports",
    };

    // Shape validation IS the gate: a manifest that cannot vouch for
    // itself fails the run instead of shipping quietly.
    if manifest.format != FORMAT_TAG {
        anyhow::bail!("manifest format tag mismatch");
    }
    if manifest.members.len() < 20 {
        anyhow::bail!(
            "expected >= 20 workspace members, got {} — the crate set regressed",
            manifest.members.len()
        );
    }
    if manifest.adrs < 30 {
        anyhow::bail!(
            "expected >= 30 ADRs in the decision ledger, got {}",
            manifest.adrs
        );
    }
    if manifest.risk_ids.len() < 25 {
        anyhow::bail!("expected >= 25 risk ids, got {}", manifest.risk_ids.len());
    }
    if manifest.roadmap.len() != 11 {
        anyhow::bail!(
            "expected exactly 11 roadmap rows (Phase 0 + phases 1-10), got {}",
            manifest.roadmap.len()
        );
    }
    for (expected, row) in manifest.roadmap.iter().enumerate() {
        if row.phase != expected as u32 {
            anyhow::bail!(
                "roadmap row out of order: expected phase {}, got {}",
                expected,
                row.phase
            );
        }
    }
    if manifest.format_tags.len() < 5 {
        anyhow::bail!(
            "expected >= 5 wire-format tags, got {:?}",
            manifest.format_tags
        );
    }

    let json = serde_json::to_string_pretty(&manifest)?;
    std::fs::write(&out, &json).with_context(|| format!("writing {}", out.display()))?;
    info!(
        members = manifest.members.len(),
        adrs = manifest.adrs,
        risks = manifest.risk_ids.len(),
        tags = manifest.format_tags.len(),
        bytes = json.len(),
        path = %out.display(),
        "release manifest written and validated"
    );
    Ok(())
}
