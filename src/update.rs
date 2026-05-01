use std::path::Path;

use crate::chart;
use crate::progress;
use crate::resolver::{self, Latest};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML error: {0}")]
    Toml(#[from] toml_edit::TomlError),
    #[error("chart error: {0}")]
    Chart(#[from] chart::Error),
    #[error("resolver error: {0}")]
    Resolver(#[from] resolver::Error),
    #[error("chart not found in charts.toml: {0}")]
    NotFound(String),
    #[error("provide either <NAME> <VERSION>, <NAME> --latest, or --all --latest")]
    BadInvocation,
    #[error("could not resolve latest version for {name}: {reason}")]
    Unresolved { name: String, reason: String },
    #[error("failed to update one or more charts: {names}")]
    Multi { names: String },
}

pub fn run(
    config_path: &Path,
    name: Option<&str>,
    version: Option<&str>,
    latest: bool,
    all: bool,
) -> Result<(), Error> {
    match (name, version, latest, all) {
        (Some(n), Some(v), false, false) => set_version(config_path, n, v),
        (Some(n), None, true, false) => bump_to_latest(config_path, n),
        (None, None, true, true) => bump_all_to_latest(config_path),
        _ => Err(Error::BadInvocation),
    }
}

fn set_version(config_path: &Path, name: &str, version: &str) -> Result<(), Error> {
    let content = std::fs::read_to_string(config_path)?;
    let mut doc = content.parse::<toml_edit::DocumentMut>()?;
    write_version(&mut doc, name, version)?;
    std::fs::write(config_path, doc.to_string())?;
    println!("  updated {name} to {version}");
    Ok(())
}

fn bump_to_latest(config_path: &Path, name: &str) -> Result<(), Error> {
    let registry = chart::load(config_path)?;
    let chart_entry = registry.get(name)?;

    let new_version = match resolver::latest_version(chart_entry)? {
        Latest::Version(v) => preserve_v_prefix(&chart_entry.version, &v),
        Latest::Unknown(reason) => {
            return Err(Error::Unresolved {
                name: name.to_string(),
                reason,
            });
        }
    };

    if new_version == chart_entry.version {
        println!("  {name} already at {new_version}");
        return Ok(());
    }

    set_version(config_path, name, &new_version)
}

fn bump_all_to_latest(config_path: &Path) -> Result<(), Error> {
    let registry = chart::load(config_path)?;
    let content = std::fs::read_to_string(config_path)?;
    let mut doc = content.parse::<toml_edit::DocumentMut>()?;

    let mut progress = progress::Progress::new(registry.charts.len());
    let mut failed = Vec::new();
    for (name, chart_entry) in &registry.charts {
        progress.tick(name);
        match resolver::latest_version(chart_entry) {
            Ok(Latest::Version(v)) => {
                let new_version = preserve_v_prefix(&chart_entry.version, &v);
                if new_version == chart_entry.version {
                    if !progress.interactive() {
                        println!("  {name} already at {new_version}");
                    }
                    continue;
                }
                if let Err(e) = write_version(&mut doc, name, &new_version) {
                    progress.clear();
                    eprintln!("  failed {name}: {e}");
                    failed.push(name.clone());
                } else {
                    progress.clear();
                    println!("  updated {name} to {new_version}");
                }
            }
            Ok(Latest::Unknown(reason)) => {
                progress.clear();
                eprintln!("  unresolved {name}: {reason}");
                failed.push(name.clone());
            }
            Err(e) => {
                progress.clear();
                eprintln!("  error {name}: {e}");
                failed.push(name.clone());
            }
        }
    }
    progress.clear();

    std::fs::write(config_path, doc.to_string())?;

    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::Multi {
            names: failed.join(", "),
        })
    }
}

fn write_version(doc: &mut toml_edit::DocumentMut, name: &str, version: &str) -> Result<(), Error> {
    let table = doc
        .get_mut(name)
        .ok_or_else(|| Error::NotFound(name.to_string()))?
        .as_table_mut()
        .ok_or_else(|| Error::NotFound(name.to_string()))?;

    let version_key = "version";
    if !table.contains_key(version_key) {
        table.insert(version_key, toml_edit::value(version));
    } else if let Some(item) = table.get_mut(version_key) {
        *item = toml_edit::value(version);
    }
    Ok(())
}

/// If the existing pin uses a leading `v` (e.g. cert-manager's `v1.20.1`),
/// keep that style on the new version.
fn preserve_v_prefix(existing: &str, new: &str) -> String {
    let stripped = new.trim_start_matches('v').to_string();
    if existing.starts_with('v') {
        format!("v{stripped}")
    } else {
        stripped
    }
}
