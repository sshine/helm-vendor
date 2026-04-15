use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML error: {0}")]
    Toml(#[from] toml_edit::TomlError),
    #[error("chart not found in charts.toml: {0}")]
    NotFound(String),
}

pub fn run(config_path: &Path, name: &str, version: &str) -> Result<(), Error> {
    let content = std::fs::read_to_string(config_path)?;
    let mut doc = content.parse::<toml_edit::DocumentMut>()?;

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

    std::fs::write(config_path, doc.to_string())?;
    println!("  updated {name} to {version}");

    Ok(())
}
