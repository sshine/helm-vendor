//! charts.toml mutation, kept in one place so `add`, `update` and `remove`
//! cannot disagree about how an entry is written back. Everything goes through
//! toml_edit, so hand-written formatting and comments survive a rewrite.

use std::path::Path;

use crate::chart::{Chart, ChartSource};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML error: {0}")]
    Toml(#[from] toml_edit::TomlError),
    #[error("chart not found in charts.toml: {0}")]
    NotFound(String),
}

/// An absent file reads as an empty document, which is what lets `add` create
/// charts.toml on first use.
pub fn read(path: &Path) -> Result<toml_edit::DocumentMut, Error> {
    if path.is_file() {
        Ok(std::fs::read_to_string(path)?.parse()?)
    } else {
        Ok(toml_edit::DocumentMut::new())
    }
}

pub fn write(path: &Path, doc: &toml_edit::DocumentMut) -> Result<(), Error> {
    std::fs::write(path, doc.to_string())?;
    Ok(())
}

pub fn set_version(
    doc: &mut toml_edit::DocumentMut,
    name: &str,
    version: &str,
) -> Result<(), Error> {
    let table = doc
        .get_mut(name)
        .and_then(|item| item.as_table_mut())
        .ok_or_else(|| Error::NotFound(name.to_string()))?;
    table.insert("version", toml_edit::value(version));
    Ok(())
}

pub fn insert(doc: &mut toml_edit::DocumentMut, name: &str, chart: &Chart) {
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
}

pub fn remove(doc: &mut toml_edit::DocumentMut, name: &str) -> Result<(), Error> {
    doc.remove(name)
        .map(|_| ())
        .ok_or_else(|| Error::NotFound(name.to_string()))
}
