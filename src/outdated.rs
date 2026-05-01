use std::path::Path;

use crate::chart::{self, ChartSource};
use crate::color;
use crate::progress;
use crate::resolver::{self, Latest};
use crate::{ResolveEvent, resolve_in_parallel};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("chart error: {0}")]
    Chart(#[from] chart::Error),
    #[error("{count} chart(s) outdated")]
    HasOutdated { count: usize },
}

const PINNED_WIDTH: usize = 8;
const LATEST_VERSION_WIDTH: usize = 7;
const LATEST_WIDTH: usize = LATEST_VERSION_WIDTH + 2;

pub fn run(config_path: &Path, name: Option<&str>) -> Result<(), Error> {
    let registry = chart::load(config_path)?;

    let targets: Vec<(&String, &chart::Chart)> = if let Some(n) = name {
        let chart_entry = registry.get(n)?;
        let key = registry
            .charts
            .keys()
            .find(|k| k.as_str() == n)
            .expect("get() succeeded, key must exist");
        vec![(key, chart_entry)]
    } else {
        registry.charts.iter().collect()
    };

    let name_width = targets
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let pw = PINNED_WIDTH;
    let lw = LATEST_WIDTH;
    let header = format!(
        "{:name_width$}  {:pw$}  {:lw$}  source",
        "NAME", "PINNED", "LATEST"
    );

    let mut progress = progress::Progress::new(targets.len());
    let color_on = progress.interactive() && color::enabled();
    let mut rows: Vec<Row> = Vec::new();
    let mut header_printed = false;

    resolve_in_parallel(&targets, |event| match event {
        ResolveEvent::Completed { name } => progress.tick(name),
        ResolveEvent::Row {
            name,
            chart,
            result,
        } => {
            let row = match result {
                Ok(Latest::Version(latest)) => {
                    if resolver::is_up_to_date(&chart.version, &latest) {
                        return;
                    }
                    Row {
                        name: name.to_string(),
                        pinned: chart.version.clone(),
                        latest: latest_with_arrow(&chart.version, &latest, color_on),
                        source: source_str(&chart.source).to_string(),
                        state: State::Outdated,
                    }
                }
                Ok(Latest::Unknown(reason)) => Row {
                    name: name.to_string(),
                    pinned: chart.version.clone(),
                    latest: format!("{:LATEST_WIDTH$}", format!("? ({reason})")),
                    source: source_str(&chart.source).to_string(),
                    state: State::Unknown,
                },
                Err(e) => Row {
                    name: name.to_string(),
                    pinned: chart.version.clone(),
                    latest: format!("{:LATEST_WIDTH$}", format!("? ({e})")),
                    source: source_str(&chart.source).to_string(),
                    state: State::Unknown,
                },
            };

            if !progress.interactive() {
                progress.clear();
                if !header_printed {
                    println!("{header}");
                    header_printed = true;
                }
                println!("{}", format_row(&row, name_width, pw, lw));
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            rows.push(row);
        }
    });

    progress.clear();

    if rows.is_empty() {
        println!("all charts up to date");
        return Ok(());
    }

    if progress.interactive() {
        println!("{header}");
        for r in &rows {
            println!("{}", format_row(r, name_width, pw, lw));
        }
    }

    let outdated = rows.iter().filter(|r| r.state == State::Outdated).count();
    if outdated > 0 {
        Err(Error::HasOutdated { count: outdated })
    } else {
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Outdated,
    Unknown,
}

struct Row {
    name: String,
    pinned: String,
    latest: String,
    source: String,
    state: State,
}

fn format_row(r: &Row, name_w: usize, pw: usize, _lw: usize) -> String {
    // r.latest is pre-padded to LATEST_WIDTH visible chars (or wider when
    // an ugly Unknown reason exceeds the budget); we don't `:lw$` it again
    // because color escapes would inflate the width count and over-pad.
    format!(
        "{:name_w$}  {:pw$}  {}  {}",
        r.name, r.pinned, r.latest, r.source
    )
}

fn source_str(source: &ChartSource) -> &str {
    match source {
        ChartSource::Repo { repo } => repo.as_str(),
        ChartSource::Oci { oci } => oci.as_str(),
    }
}

/// Render a LATEST cell for `outdated`. Rows always represent an outdated
/// chart here (up-to-date rows are filtered upstream), so the cell always
/// carries the ` ↑` marker; `color_on` wraps it with severity-colored ANSI.
fn latest_with_arrow(existing: &str, latest: &str, color_on: bool) -> String {
    let vw = LATEST_VERSION_WIDTH;
    let stripped = latest.trim_start_matches('v');
    let prefixed = if existing.starts_with('v') {
        format!("v{stripped}")
    } else {
        stripped.to_string()
    };
    let inner = format!("{prefixed:vw$} ↑");
    let sev = color::severity(existing, latest);
    color::paint(&inner, sev, color_on)
}
