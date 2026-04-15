use std::path::Path;

use crate::chart::{self, ChartSource};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("chart error: {0}")]
    Chart(#[from] chart::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("helm pull failed for {name}: {stderr}")]
    HelmPull { name: String, stderr: String },
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

    let mut failed = Vec::new();
    for name in &targets {
        let chart_entry = registry.get(name)?;
        if let Err(e) = pull_chart(name, chart_entry, &charts_dir) {
            eprintln!("  failed {name}: {e}");
            failed.push(name.as_str());
        } else {
            println!("  vendored {name}");
        }
    }

    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::HelmPull {
            name: failed.join(", "),
            stderr: "one or more charts failed to pull".into(),
        })
    }
}

fn pull_chart(name: &str, chart: &chart::Chart, charts_dir: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(charts_dir).ok();

    let dest = charts_dir.join(name);
    if dest.exists() {
        std::fs::remove_dir_all(&dest)?;
    }

    let output = match &chart.source {
        ChartSource::Repo { repo } => {
            let mut cmd = std::process::Command::new("helm");
            cmd.args([
                "pull",
                "--untar",
                "-d",
                charts_dir.to_str().unwrap_or("."),
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
                charts_dir.to_str().unwrap_or("."),
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

    Ok(())
}
