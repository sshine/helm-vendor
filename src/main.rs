//! # helm-vendor
//!
//! CLI for vendoring Helm charts from a charts.toml registry
mod args;
mod chart;
mod check;
mod update;
mod vendor;

use std::process::ExitCode;

use clap::Parser;

use crate::args::{Args, Command};

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
        Command::Vendor { name, all } => match vendor::run(&config_path, name.as_deref(), all) {
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
        Command::Update { name, version } => match update::run(&config_path, &name, &version) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::List => match chart::load(&config_path) {
            Ok(registry) => {
                list_charts(&registry);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

fn list_charts(registry: &chart::ChartRegistry) {
    let name_width = registry.charts.keys().map(|n| n.len()).max().unwrap_or(4);
    let version_width = registry
        .charts
        .values()
        .map(|c| c.version.len())
        .max()
        .unwrap_or(7);

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
