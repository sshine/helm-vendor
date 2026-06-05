use std::path::Path;

use crate::chart::{self, ChartSource};
use crate::progress;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("chart error: {0}")]
    Chart(#[from] chart::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("helm pull failed for {name}: {stderr}")]
    HelmPull { name: String, stderr: String },
    #[error("drift detected in: {names}")]
    Drift { names: String },
    #[error("either specify a chart name or use --all")]
    NoTarget,
}

pub fn run(config_path: &Path, name: Option<&str>, all: bool) -> Result<(), Error> {
    let registry = chart::load(config_path)?;
    let charts_dir = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("charts");

    let targets: Vec<String> = if all {
        registry.charts.keys().cloned().collect()
    } else if let Some(name) = name {
        registry.get(name)?;
        vec![name.to_string()]
    } else {
        return Err(Error::NoTarget);
    };

    let mut progress = progress::Progress::new(targets.len());
    let mut drifted = Vec::new();
    let mut errors = Vec::new();

    for name in &targets {
        progress.tick(name);
        let chart_entry = registry.get(name)?;
        match check_chart(name, chart_entry, &charts_dir) {
            Ok(CheckResult::Match) => {
                if !progress.interactive() {
                    println!("  ok {name}");
                }
            }
            Ok(CheckResult::Drift) => {
                progress.clear();
                eprintln!("  drift {name}");
                drifted.push(name.as_str());
            }
            Ok(CheckResult::Missing) => {
                progress.clear();
                eprintln!("  missing {name}");
                drifted.push(name.as_str());
            }
            Err(e) => {
                progress.clear();
                eprintln!("  error {name}: {e}");
                errors.push(name.as_str());
            }
        }
    }
    progress.clear();
    if progress.interactive() && drifted.is_empty() && errors.is_empty() {
        println!("checked {} chart(s)", targets.len());
    }

    if drifted.is_empty() && errors.is_empty() {
        Ok(())
    } else {
        let mut all_names = drifted;
        all_names.extend(errors);
        Err(Error::Drift {
            names: all_names.join(", "),
        })
    }
}

enum CheckResult {
    Match,
    Drift,
    Missing,
}

fn check_chart(name: &str, chart: &chart::Chart, charts_dir: &Path) -> Result<CheckResult, Error> {
    let existing = charts_dir.join(name);
    if !existing.is_dir() {
        return Ok(CheckResult::Missing);
    }

    let tmp = tempfile::tempdir()?;

    let output = match &chart.source {
        ChartSource::Repo { repo } => {
            let mut cmd = std::process::Command::new("helm");
            cmd.args([
                "pull",
                "--untar",
                "-d",
                tmp.path().to_str().unwrap(),
                "--repo",
                repo,
                "--version",
                &chart.version,
                &chart.chart,
            ]);
            cmd.output()?
        }
        ChartSource::Oci { oci } => {
            let mut cmd = std::process::Command::new("helm");
            cmd.args([
                "pull",
                "--untar",
                "-d",
                tmp.path().to_str().unwrap(),
                "--version",
                &chart.version,
                oci,
            ]);
            cmd.output()?
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(Error::HelmPull {
            name: name.to_string(),
            stderr,
        });
    }

    // helm names the extracted dir after Chart.yaml's `name`, which may
    // differ from our TOML key. Locate by structure to stay agnostic.
    let pulled = match chart::locate_extracted(tmp.path()) {
        Ok(p) => p,
        Err(_) => return Ok(CheckResult::Missing),
    };

    let diff = std::process::Command::new("diff")
        .args([
            "-r",
            "--brief",
            pulled.to_str().unwrap(),
            existing.to_str().unwrap(),
        ])
        .output()?;

    if diff.status.success() {
        Ok(CheckResult::Match)
    } else {
        Ok(CheckResult::Drift)
    }
}
