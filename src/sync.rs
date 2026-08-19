//! `sync`: make the `charts/` directory agree with charts.toml.
//!
//! Both modes pull from upstream into a tempdir and compare against what is
//! vendored; only what happens afterwards differs. `--check` reports and exits
//! non-zero, the default replaces the directories that differ.
//!
//! Comparing before replacing is what makes a repeat sync a no-op: charts that
//! already match are left alone, so nothing shows up in `git status` for them.

use std::path::Path;

use crate::chart::{self, Chart, ChartSource};
use crate::progress;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Chart(#[from] chart::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("helm pull failed for {name}: {stderr}")]
    HelmPull { name: String, stderr: String },
    #[error("{count} chart(s) out of sync: {names}")]
    OutOfSync { count: usize, names: String },
    #[error("failed to vendor: {names}")]
    Failed { names: String },
}

/// How a chart's vendored directory compares to its pinned upstream version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Vendored content already equals upstream.
    Current,
    /// Vendored content exists but differs.
    Changed,
    /// Nothing vendored yet.
    Missing,
}

pub fn run(config_path: &Path, names: &[String], check: bool) -> Result<(), Error> {
    let registry = chart::load(config_path)?;
    let targets = registry.select(names)?;
    let charts_dir = chart::charts_dir(config_path);

    let mut progress = progress::Progress::new(targets.len());
    let mut pending: Vec<&str> = Vec::new();
    let mut failed: Vec<&str> = Vec::new();

    for (name, chart) in &targets {
        progress.tick(name);
        match vendor(name, chart, &charts_dir, check) {
            Ok(Outcome::Current) => {
                if !progress.interactive() {
                    println!("  ok {name}");
                }
            }
            Ok(outcome) => {
                progress.clear();
                if check {
                    eprintln!("  {} {name}", check_label(outcome));
                    pending.push(name);
                } else {
                    println!("  {} {name}", sync_label(outcome));
                    pending.push(name);
                }
            }
            Err(e) => {
                progress.clear();
                eprintln!("  error {name}: {e}");
                failed.push(name);
            }
        }
    }
    progress.clear();

    if !failed.is_empty() {
        return Err(Error::Failed {
            names: failed.join(", "),
        });
    }

    if check {
        if pending.is_empty() {
            println!("{} chart(s) in sync", targets.len());
            return Ok(());
        }
        return Err(Error::OutOfSync {
            count: pending.len(),
            names: pending.join(", "),
        });
    }

    if pending.is_empty() {
        println!("{} chart(s) already in sync", targets.len());
    } else if progress.interactive() {
        println!("synced {} of {} chart(s)", pending.len(), targets.len());
    }
    Ok(())
}

fn check_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Missing => "missing",
        _ => "changed",
    }
}

fn sync_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Missing => "vendored",
        _ => "updated",
    }
}

/// Pull one chart and, unless `check_only`, move it into place. Returns how the
/// vendored directory compared *before* any replacement.
pub(crate) fn vendor(
    name: &str,
    chart: &Chart,
    charts_dir: &Path,
    check_only: bool,
) -> Result<Outcome, Error> {
    std::fs::create_dir_all(charts_dir)?;

    // Pull into a tempdir co-located with `charts_dir` so the final rename stays
    // on the same filesystem.
    let tmp = tempfile::tempdir_in(charts_dir)?;
    pull(name, chart, tmp.path())?;

    // helm names the output directory after Chart.yaml's `name`, which may differ
    // from our TOML key (e.g. `[istio-cni]` with `chart = "cni"`); locate it by
    // structure and move it under the TOML key.
    let pulled = chart::locate_extracted(tmp.path())?;
    let dest = charts_dir.join(name);

    let outcome = if !dest.is_dir() {
        Outcome::Missing
    } else if same_tree(&pulled, &dest)? {
        Outcome::Current
    } else {
        Outcome::Changed
    };

    if !check_only && outcome != Outcome::Current {
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        std::fs::rename(&pulled, &dest)?;
    }

    Ok(outcome)
}

fn pull(name: &str, chart: &Chart, dir: &Path) -> Result<(), Error> {
    let dir = path_arg(dir)?;
    let mut cmd = std::process::Command::new("helm");
    match &chart.source {
        ChartSource::Repo { repo } => cmd.args([
            "pull",
            "--untar",
            "-d",
            dir,
            "--repo",
            repo,
            "--version",
            &chart.version,
            &chart.chart,
        ]),
        ChartSource::Oci { oci } => cmd.args([
            "pull",
            "--untar",
            "-d",
            dir,
            "--version",
            &chart.version,
            oci,
        ]),
    };

    let output = cmd.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(Error::HelmPull {
            name: name.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

fn same_tree(a: &Path, b: &Path) -> Result<bool, Error> {
    let output = std::process::Command::new("diff")
        .args(["-r", "--brief", path_arg(a)?, path_arg(b)?])
        .output()?;
    Ok(output.status.success())
}

/// Both `helm` and `diff` take paths as argv strings, so a path we cannot render
/// as UTF-8 has to fail rather than be silently substituted.
fn path_arg(path: &Path) -> Result<&str, Error> {
    path.to_str().ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path is not valid UTF-8: {}", path.display()),
        ))
    })
}
