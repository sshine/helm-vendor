//! `update`: re-pin charts and vendor the result.
//!
//! Pinning and vendoring are one command because doing only the first leaves
//! charts.toml describing a `charts/` directory that does not exist yet, and
//! nothing in the repository records that the two disagree.

use std::path::Path;

use crate::chart;
use crate::edit;
use crate::progress;
use crate::resolver::Latest;
use crate::sync;
use crate::{ResolveEvent, resolve_in_parallel};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Chart(#[from] chart::Error),
    #[error(transparent)]
    Edit(#[from] edit::Error),
    #[error(transparent)]
    Sync(#[from] sync::Error),
    #[error("--to pins one version, so it needs exactly one chart name")]
    ToNeedsOneChart,
    #[error("failed to update: {names}")]
    Failed { names: String },
}

pub fn run(
    config_path: &Path,
    names: &[String],
    to: Option<&str>,
    no_fetch: bool,
) -> Result<(), Error> {
    let registry = chart::load(config_path)?;

    let (changed, failed) = match to {
        Some(version) => {
            let [name] = names else {
                return Err(Error::ToNeedsOneChart);
            };
            registry.get(name)?;
            let mut doc = edit::read(config_path)?;
            edit::set_version(&mut doc, name, version)?;
            edit::write(config_path, &doc)?;
            println!("  {name} pinned to {version}");
            (vec![name.clone()], Vec::new())
        }
        None => bump_to_latest(config_path, &registry, names)?,
    };

    if !changed.is_empty() && !no_fetch {
        sync::run(config_path, &changed, false)?;
    }

    if failed.is_empty() {
        Ok(())
    } else {
        Err(Error::Failed {
            names: failed.join(", "),
        })
    }
}

/// Resolve every target concurrently, writing each new pin into the same
/// document so charts.toml is rewritten once at the end.
fn bump_to_latest(
    config_path: &Path,
    registry: &chart::ChartRegistry,
    names: &[String],
) -> Result<(Vec<String>, Vec<String>), Error> {
    let targets = registry.select(names)?;
    let mut doc = edit::read(config_path)?;

    let mut progress = progress::Progress::new(targets.len());
    let mut changed: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();

    resolve_in_parallel(&targets, |event| match event {
        ResolveEvent::Completed { name } => progress.tick(name),
        ResolveEvent::Row {
            name,
            chart,
            result,
        } => match result {
            Ok(Latest::Version(latest)) => {
                let version = preserve_v_prefix(&chart.version, &latest);
                if version == chart.version {
                    return;
                }
                progress.clear();
                match edit::set_version(&mut doc, name, &version) {
                    Ok(()) => {
                        println!("  {name} {} -> {version}", chart.version);
                        changed.push(name.to_string());
                    }
                    Err(e) => {
                        eprintln!("  failed {name}: {e}");
                        failed.push(name.to_string());
                    }
                }
            }
            Ok(Latest::Unknown(reason)) => {
                progress.clear();
                eprintln!("  unresolved {name}: {reason}");
                failed.push(name.to_string());
            }
            Err(e) => {
                progress.clear();
                eprintln!("  error {name}: {e}");
                failed.push(name.to_string());
            }
        },
    });
    progress.clear();

    if changed.is_empty() {
        println!("{} chart(s) already at their latest version", targets.len());
    } else {
        edit::write(config_path, &doc)?;
    }

    Ok((changed, failed))
}

/// If the existing pin uses a leading `v` (e.g. cert-manager's `v1.20.1`),
/// keep that style on the new version.
pub(crate) fn preserve_v_prefix(existing: &str, new: &str) -> String {
    let stripped = new.trim_start_matches('v').to_string();
    if existing.starts_with('v') {
        format!("v{stripped}")
    } else {
        stripped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v_prefix_follows_the_existing_pin() {
        assert_eq!(preserve_v_prefix("v1.2.3", "1.3.0"), "v1.3.0");
        assert_eq!(preserve_v_prefix("v1.2.3", "v1.3.0"), "v1.3.0");
        assert_eq!(preserve_v_prefix("1.2.3", "v1.3.0"), "1.3.0");
        assert_eq!(preserve_v_prefix("1.2.3", "1.3.0"), "1.3.0");
    }
}
