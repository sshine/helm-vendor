//! `remove`: drop charts from charts.toml and delete what they vendored.
//!
//! Names are validated up front so a typo in the last one cannot leave the
//! earlier ones half-removed.

use std::path::Path;

use crate::chart;
use crate::edit;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Chart(#[from] chart::Error),
    #[error(transparent)]
    Edit(#[from] edit::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn run(config_path: &Path, names: &[String], keep_files: bool) -> Result<(), Error> {
    let registry = chart::load(config_path)?;
    for name in names {
        registry.get(name)?;
    }

    let charts_dir = chart::charts_dir(config_path);
    let mut doc = edit::read(config_path)?;
    for name in names {
        edit::remove(&mut doc, name)?;
    }
    edit::write(config_path, &doc)?;

    for name in names {
        let dir = charts_dir.join(name);
        if !keep_files && dir.is_dir() {
            std::fs::remove_dir_all(&dir)?;
        }
        println!("  removed {name}");
    }
    Ok(())
}
