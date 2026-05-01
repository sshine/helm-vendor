//! # helm-vendor
//!
//! CLI for vendoring Helm charts from a charts.toml registry
mod args;
mod chart;
mod check;
mod fetch;
mod outdated;
mod resolver;
mod update;

use std::process::ExitCode;

use clap::Parser;

use crate::args::{Args, Command};
use crate::resolver::Latest;

fn main() -> ExitCode {
    let args = Args::parse();

    let config_path = match args.config_path() {
        Some(p) => p,
        None => {
            eprintln!("error: could not find charts.toml in current directory or any parent");
            return ExitCode::FAILURE;
        }
    };

    match args.command {
        Command::Fetch { name, all } => match fetch::run(&config_path, name.as_deref(), all) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Check { name, all } => match check::run(&config_path, name.as_deref(), all) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Update {
            name,
            version,
            latest,
            all,
        } => match update::run(
            &config_path,
            name.as_deref(),
            version.as_deref(),
            latest,
            all,
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::List { check } => match chart::load(&config_path) {
            Ok(registry) => {
                list_charts(&registry, check);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Outdated { name } => match outdated::run(&config_path, name.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

fn list_charts(registry: &chart::ChartRegistry, check: bool) {
    let name_width = registry.charts.keys().map(|n| n.len()).max().unwrap_or(4);
    let version_width = registry
        .charts
        .values()
        .map(|c| c.version.len())
        .max()
        .unwrap_or(7);

    let latest_cells: Vec<String> = if check {
        registry
            .charts
            .values()
            .map(|c| match resolver::latest_version(c) {
                Ok(Latest::Version(v)) => {
                    let formatted = if c.version.starts_with('v') {
                        format!("v{}", v.trim_start_matches('v'))
                    } else {
                        v.trim_start_matches('v').to_string()
                    };
                    if resolver::is_up_to_date(&c.version, &v) {
                        formatted
                    } else {
                        format!("{formatted} ↑")
                    }
                }
                Ok(Latest::Unknown(_)) => "?".to_string(),
                Err(_) => "?".to_string(),
            })
            .collect()
    } else {
        Vec::new()
    };

    if check {
        let latest_width = latest_cells
            .iter()
            .map(|c| c.len())
            .max()
            .unwrap_or(6)
            .max(6);
        println!(
            "{:name_width$}  {:version_width$}  {:latest_width$}  source",
            "NAME", "VERSION", "LATEST"
        );
        for ((name, chart), latest_cell) in registry.charts.iter().zip(latest_cells.iter()) {
            let source = match &chart.source {
                chart::ChartSource::Repo { repo } => repo.as_str(),
                chart::ChartSource::Oci { oci } => oci.as_str(),
            };
            println!(
                "{name:name_width$}  {:version_width$}  {latest_cell:latest_width$}  {source}",
                chart.version
            );
        }
    } else {
        println!(
            "{:name_width$}  {:version_width$}  source",
            "NAME", "VERSION"
        );
        for (name, chart) in &registry.charts {
            let source = match &chart.source {
                chart::ChartSource::Repo { repo } => repo.as_str(),
                chart::ChartSource::Oci { oci } => oci.as_str(),
            };
            println!(
                "{name:name_width$}  {:version_width$}  {source}",
                chart.version
            );
        }
    }
}
