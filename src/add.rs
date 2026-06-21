//! `add` subcommand: register a new chart in charts.toml and vendor it.
//!
//! Flow: resolve the version (explicit pin, or latest from upstream when
//! omitted), `helm pull` it into the `charts/` directory, and only on a
//! successful pull write the entry to charts.toml — creating the file if it
//! does not yet exist. Pulling first means a bad pin (a version that doesn't
//! exist upstream) fails the command and leaves charts.toml untouched.
//!
//! If the chart name already exists, `add` degrades to `update` semantics when
//! the requested source matches the recorded one (rewriting the pinned version
//! in place); a different source is a conflict and fails.

use std::path::Path;

use crate::chart::{self, Chart, ChartSource};
use crate::fetch;
use crate::resolver::{self, Latest};
use crate::update::preserve_v_prefix;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML error: {0}")]
    Toml(#[from] toml_edit::TomlError),
    #[error("chart error: {0}")]
    Chart(#[from] chart::Error),
    #[error("fetch error: {0}")]
    Fetch(#[from] fetch::Error),
    #[error("resolver error: {0}")]
    Resolver(#[from] resolver::Error),
    #[error(
        "chart '{name}' already exists with a different source ({existing}); \
         refusing to overwrite"
    )]
    SourceConflict { name: String, existing: String },
    #[error("could not resolve latest version for {name}: {reason}")]
    Unresolved { name: String, reason: String },
}

pub fn run(config_path: &Path, name: &str, url: &str, version: Option<&str>) -> Result<(), Error> {
    let requested = source_from_url(url);

    // An existing entry turns this into an `update`: same source rewrites the
    // pinned version in place; a different source is a hard conflict.
    let existing = if config_path.is_file() {
        chart::load(config_path)?.charts.remove(name)
    } else {
        None
    };

    let (source, chart_name) = match &existing {
        Some(prev) => {
            if !same_source(&prev.source, &requested) {
                return Err(Error::SourceConflict {
                    name: name.to_string(),
                    existing: source_url(&prev.source).to_string(),
                });
            }
            // Keep the recorded source and chart name — the `chart` field can
            // differ from the charts.toml key, so don't clobber it.
            (prev.source.clone(), prev.chart.clone())
        }
        None => (requested, name.to_string()),
    };

    // Resolve up front so the helm pull and the charts.toml entry agree on a
    // concrete version. With no pin given, ask upstream for the newest stable
    // release. (`latest_version` only reads the source + chart name.)
    let version = match version {
        Some(v) => v.to_string(),
        None => {
            let probe = Chart {
                chart: chart_name.clone(),
                version: String::new(),
                source: source.clone(),
            };
            match resolver::latest_version(&probe)? {
                // Match the existing pin's `v`-prefix style when updating.
                Latest::Version(v) => match &existing {
                    Some(prev) => preserve_v_prefix(&prev.version, &v),
                    None => v,
                },
                Latest::Unknown(reason) => {
                    return Err(Error::Unresolved {
                        name: name.to_string(),
                        reason,
                    });
                }
            }
        }
    };

    let chart = Chart {
        chart: chart_name,
        version: version.clone(),
        source,
    };

    let charts_dir = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("charts");

    // Pull before writing: a failed pull (e.g. a version that doesn't exist
    // upstream) must leave charts.toml untouched.
    fetch::pull_chart(name, &chart, &charts_dir)?;

    if existing.is_some() {
        write_version(config_path, name, &version)?;
        println!("  updated {name} to {version}");
    } else {
        write_new_entry(config_path, name, &chart)?;
        println!("  added {name} {version}");
    }
    Ok(())
}

fn source_from_url(url: &str) -> ChartSource {
    if url.starts_with("oci://") {
        ChartSource::Oci {
            oci: url.to_string(),
        }
    } else {
        ChartSource::Repo {
            repo: url.to_string(),
        }
    }
}

fn source_url(source: &ChartSource) -> &str {
    match source {
        ChartSource::Repo { repo } => repo,
        ChartSource::Oci { oci } => oci,
    }
}

/// Two sources match iff they are the same kind and the same URL, ignoring a
/// trailing slash (which the resolver also normalizes away).
fn same_source(a: &ChartSource, b: &ChartSource) -> bool {
    match (a, b) {
        (ChartSource::Repo { repo: x }, ChartSource::Repo { repo: y })
        | (ChartSource::Oci { oci: x }, ChartSource::Oci { oci: y }) => {
            x.trim_end_matches('/') == y.trim_end_matches('/')
        }
        _ => false,
    }
}

/// Append a `[name]` table to charts.toml (creating the file if absent) while
/// preserving any existing formatting via toml_edit.
fn write_new_entry(config_path: &Path, name: &str, chart: &Chart) -> Result<(), Error> {
    let mut doc = if config_path.is_file() {
        std::fs::read_to_string(config_path)?.parse::<toml_edit::DocumentMut>()?
    } else {
        toml_edit::DocumentMut::new()
    };

    let mut table = toml_edit::Table::new();
    match &chart.source {
        ChartSource::Repo { repo } => {
            table.insert("repo", toml_edit::value(repo.as_str()));
        }
        ChartSource::Oci { oci } => {
            table.insert("oci", toml_edit::value(oci.as_str()));
        }
    }
    table.insert("chart", toml_edit::value(chart.chart.as_str()));
    table.insert("version", toml_edit::value(chart.version.as_str()));

    doc.insert(name, toml_edit::Item::Table(table));

    std::fs::write(config_path, doc.to_string())?;
    Ok(())
}

/// Rewrite just the `version` field of an existing entry, leaving the rest of
/// the table (and the file's formatting) intact.
fn write_version(config_path: &Path, name: &str, version: &str) -> Result<(), Error> {
    let mut doc = std::fs::read_to_string(config_path)?.parse::<toml_edit::DocumentMut>()?;
    if let Some(table) = doc.get_mut(name).and_then(|i| i.as_table_mut()) {
        table.insert("version", toml_edit::value(version));
    }
    std::fs::write(config_path, doc.to_string())?;
    Ok(())
}
