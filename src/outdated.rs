use std::path::Path;

use crate::chart::{self, ChartSource};
use crate::resolver::{self, Latest};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("chart error: {0}")]
    Chart(#[from] chart::Error),
    #[error("{count} chart(s) outdated")]
    HasOutdated { count: usize },
}

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

    let mut rows: Vec<Row> = Vec::new();
    for (key, chart_entry) in targets {
        let row = match resolver::latest_version(chart_entry) {
            Ok(Latest::Version(latest)) => {
                if resolver::is_up_to_date(&chart_entry.version, &latest) {
                    continue;
                }
                Row {
                    name: key.clone(),
                    pinned: chart_entry.version.clone(),
                    latest: latest_with_prefix(&chart_entry.version, &latest),
                    source: source_str(&chart_entry.source).to_string(),
                    state: State::Outdated,
                }
            }
            Ok(Latest::Unknown(reason)) => Row {
                name: key.clone(),
                pinned: chart_entry.version.clone(),
                latest: format!("? ({reason})"),
                source: source_str(&chart_entry.source).to_string(),
                state: State::Unknown,
            },
            Err(e) => Row {
                name: key.clone(),
                pinned: chart_entry.version.clone(),
                latest: format!("? ({e})"),
                source: source_str(&chart_entry.source).to_string(),
                state: State::Unknown,
            },
        };
        rows.push(row);
    }

    if rows.is_empty() {
        println!("all charts up to date");
        return Ok(());
    }

    print_rows(&rows);

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

fn print_rows(rows: &[Row]) {
    let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
    let pinned_w = rows
        .iter()
        .map(|r| r.pinned.len())
        .max()
        .unwrap_or(7)
        .max(7);
    let latest_w = rows
        .iter()
        .map(|r| r.latest.len())
        .max()
        .unwrap_or(6)
        .max(6);

    println!(
        "{:name_w$}  {:pinned_w$}  {:latest_w$}  source",
        "NAME", "PINNED", "LATEST"
    );
    for r in rows {
        println!(
            "{:name_w$}  {:pinned_w$}  {:latest_w$}  {}",
            r.name, r.pinned, r.latest, r.source
        );
    }
}

fn source_str(source: &ChartSource) -> &str {
    match source {
        ChartSource::Repo { repo } => repo.as_str(),
        ChartSource::Oci { oci } => oci.as_str(),
    }
}

fn latest_with_prefix(existing: &str, latest: &str) -> String {
    let stripped = latest.trim_start_matches('v');
    if existing.starts_with('v') {
        format!("v{stripped}")
    } else {
        stripped.to_string()
    }
}
