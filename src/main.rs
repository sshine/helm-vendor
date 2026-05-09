//! # helm-vendor
//!
//! CLI for vendoring Helm charts from a charts.toml registry
mod args;
mod chart;
mod check;
mod color;
mod completion;
mod fetch;
mod outdated;
mod progress;
mod resolver;
mod update;

use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use clap::Parser;

use crate::args::{Args, Command};
use crate::resolver::Latest;

/// Upstream resolution is HTTP-bound; pull a small pool of charts in parallel
/// so the wall clock for `list --check` / `outdated` is roughly max-latency
/// rather than sum-of-latencies. 8 keeps us polite to upstream registries
/// without materially increasing rate-limit risk for typical chart counts.
pub(crate) const RESOLVE_CONCURRENCY: usize = 8;

fn main() -> ExitCode {
    let args = Args::parse();

    // `completion` must work without a charts.toml — users may invoke it from
    // anywhere to install shell hooks, and the hidden `--chart-names` callback
    // is expected to silently emit nothing when no registry is reachable.
    if let Command::Completion { shell, chart_names } = &args.command {
        return completion::run(args.config_path(), *shell, *chart_names);
    }

    let config_path = match args.config_path() {
        Some(p) => p,
        None => {
            eprintln!("error: could not find charts.toml in current directory or any parent");
            return ExitCode::FAILURE;
        }
    };

    match args.command {
        Command::Completion { .. } => unreachable!(),
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

/// Column widths for `list` / `list --check` tables. Name width is derived
/// from the chart names (known up front); the version columns use a fixed
/// margin so rows can be streamed in non-interactive mode without buffering
/// for alignment. A larger-than-budget version makes that one row ugly but
/// doesn't reflow earlier rows.
///
/// The LATEST cell is built as `{latest:LATEST_VERSION_WIDTH} ↑` (or two
/// spaces in lieu of ` ↑` when up-to-date), so the arrow slot is always
/// reserved and rows have predictable width regardless of which charts are
/// outdated.
const VERSION_WIDTH: usize = 8;
const LATEST_VERSION_WIDTH: usize = 7;
const LATEST_WIDTH: usize = LATEST_VERSION_WIDTH + 2;

fn list_charts(registry: &chart::ChartRegistry, check: bool) {
    let name_width = registry
        .charts
        .keys()
        .map(|n| n.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let vw = VERSION_WIDTH;
    let lw = LATEST_WIDTH;

    if !check {
        println!("{:name_width$}  {:vw$}  source", "NAME", "VERSION");
        for (name, chart) in &registry.charts {
            println!(
                "{name:name_width$}  {:vw$}  {}",
                chart.version,
                source_str(&chart.source)
            );
        }
        return;
    }

    let header = format!(
        "{:name_width$}  {:vw$}  {:lw$}  source",
        "NAME", "VERSION", "LATEST"
    );

    let mut progress = progress::Progress::new(registry.charts.len());
    let color_on = progress.interactive() && color::enabled();
    let mut buffered: Vec<String> = Vec::new();

    if !progress.interactive() {
        println!("{header}");
    }

    let charts: Vec<(&String, &chart::Chart)> = registry.charts.iter().collect();

    resolve_in_parallel(&charts, |event| match event {
        ResolveEvent::Completed { name } => progress.tick(name),
        ResolveEvent::Row {
            name,
            chart,
            result,
        } => {
            let cell = latest_cell(&chart.version, &result, color_on);
            let row = format!(
                "{name:name_width$}  {:vw$}  {cell}  {}",
                chart.version,
                source_str(&chart.source)
            );
            if progress.interactive() {
                buffered.push(row);
            } else {
                println!("{row}");
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
        }
    });

    if progress.interactive() {
        progress.clear();
        println!("{header}");
        for r in &buffered {
            println!("{r}");
        }
    }
}

pub(crate) enum ResolveEvent<'a> {
    /// Fires whenever any worker finishes, in arbitrary order. Used to tick
    /// the progress counter as soon as work completes, so slow charts don't
    /// stall it.
    Completed { name: &'a str },
    /// Fires once per chart in registry order, buffered behind any earlier
    /// slot still in flight.
    Row {
        name: &'a str,
        chart: &'a chart::Chart,
        result: Result<Latest, resolver::Error>,
    },
}

/// Resolve every chart concurrently and report progress through a single
/// event stream. Combining `Completed` and `Row` into one closure keeps the
/// caller's borrow surface to a single `&mut` over its progress/buffer state.
pub(crate) fn resolve_in_parallel<F>(charts: &[(&String, &chart::Chart)], mut on_event: F)
where
    F: FnMut(ResolveEvent<'_>),
{
    let total = charts.len();
    if total == 0 {
        return;
    }
    let next_job = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Result<Latest, resolver::Error>)>();
    let workers = RESOLVE_CONCURRENCY.min(total);

    std::thread::scope(|s| {
        let next_job = &next_job;
        for _ in 0..workers {
            let tx = tx.clone();
            s.spawn(move || {
                loop {
                    let idx = next_job.fetch_add(1, Ordering::Relaxed);
                    if idx >= total {
                        break;
                    }
                    let result = resolver::latest_version(charts[idx].1);
                    if tx.send((idx, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut slots: Vec<Option<Result<Latest, resolver::Error>>> =
            (0..total).map(|_| None).collect();
        let mut next_emit = 0usize;
        while let Ok((idx, result)) = rx.recv() {
            on_event(ResolveEvent::Completed {
                name: charts[idx].0,
            });
            slots[idx] = Some(result);
            while next_emit < total && slots[next_emit].is_some() {
                let result = slots[next_emit].take().unwrap();
                let (name, chart) = (charts[next_emit].0.as_str(), charts[next_emit].1);
                on_event(ResolveEvent::Row {
                    name,
                    chart,
                    result,
                });
                next_emit += 1;
            }
        }
    });
}

/// Build a LATEST cell from a pre-resolved result. The cell has predictable
/// width (`LATEST_VERSION_WIDTH + 2`): `<latest-padded-to-7> ↑` for outdated
/// rows, `<latest-padded-to-7>  ` (two spaces) for up-to-date or unknown.
/// When `color_on`, outdated cells are wrapped in green/yellow/orange ANSI
/// codes by SemVer-diff severity.
fn latest_cell(pinned: &str, result: &Result<Latest, resolver::Error>, color_on: bool) -> String {
    let vw = LATEST_VERSION_WIDTH;
    match result {
        Ok(Latest::Version(v)) => {
            let formatted = if pinned.starts_with('v') {
                format!("v{}", v.trim_start_matches('v'))
            } else {
                v.trim_start_matches('v').to_string()
            };
            if resolver::is_up_to_date(pinned, v) {
                format!("{formatted:vw$}  ")
            } else {
                let sev = color::severity(pinned, v);
                color::paint(&format!("{formatted:vw$} ↑"), sev, color_on)
            }
        }
        Ok(Latest::Unknown(_)) | Err(_) => format!("{:vw$}  ", "?"),
    }
}

fn source_str(source: &chart::ChartSource) -> &str {
    match source {
        chart::ChartSource::Repo { repo } => repo.as_str(),
        chart::ChartSource::Oci { oci } => oci.as_str(),
    }
}
