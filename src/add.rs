//! `add`: declare a chart in charts.toml and vendor it.
//!
//! Flow: resolve the version (explicit pin, or latest from upstream when
//! omitted), `helm pull` it into the `charts/` directory, and only on a
//! successful pull write the entry to charts.toml — creating the file if it
//! does not yet exist. Pulling first means a bad pin (a version that doesn't
//! exist upstream) fails the command and leaves charts.toml untouched. With
//! `--no-fetch` there is nothing to pull, so an explicit pin goes in unverified.
//!
//! If the chart name already exists, `add` degrades to `update` semantics when
//! the requested source matches the recorded one (rewriting the pinned version
//! in place); a different source is a conflict and fails.

use std::path::Path;

use crate::chart::{self, Chart, ChartSource};
use crate::edit;
use crate::resolver::{self, Latest};
use crate::sync;
use crate::update::preserve_v_prefix;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Chart(#[from] chart::Error),
    #[error(transparent)]
    Edit(#[from] edit::Error),
    #[error(transparent)]
    Sync(#[from] sync::Error),
    #[error(transparent)]
    Resolver(#[from] resolver::Error),
    #[error(
        "chart '{name}' already exists with a different source ({existing}); \
         refusing to overwrite"
    )]
    SourceConflict { name: String, existing: String },
    #[error("could not resolve latest version for {name}: {reason}")]
    Unresolved { name: String, reason: String },
}

pub fn run(
    config_path: &Path,
    name: &str,
    url: &str,
    version: Option<&str>,
    no_fetch: bool,
) -> Result<(), Error> {
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
                    existing: prev.source.url().to_string(),
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
            let candidate = Chart {
                chart: chart_name.clone(),
                version: String::new(),
                source: source.clone(),
            };
            match resolver::latest_version(&candidate)? {
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

    if !no_fetch {
        sync::vendor(name, &chart, &chart::charts_dir(config_path), false)?;
    }

    let mut doc = edit::read(config_path)?;
    if existing.is_some() {
        edit::set_version(&mut doc, name, &version)?;
    } else {
        edit::insert(&mut doc, name, &chart);
    }
    edit::write(config_path, &doc)?;

    let verb = if existing.is_some() {
        "updated"
    } else {
        "added"
    };
    let trailer = if no_fetch { " (not vendored)" } else { "" };
    println!("  {verb} {name} {version}{trailer}");
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
