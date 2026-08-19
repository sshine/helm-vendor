use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error reading charts.toml: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    Parse(String),
    #[error("chart not found: {0}")]
    NotFound(String),
}

#[derive(Debug, Clone)]
pub enum ChartSource {
    Repo { repo: String },
    Oci { oci: String },
}

impl ChartSource {
    pub fn url(&self) -> &str {
        match self {
            ChartSource::Repo { repo } => repo.as_str(),
            ChartSource::Oci { oci } => oci.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chart {
    pub chart: String,
    pub version: String,
    pub source: ChartSource,
}

#[derive(Debug)]
pub struct ChartRegistry {
    pub charts: BTreeMap<String, Chart>,
}

impl ChartRegistry {
    pub fn get(&self, name: &str) -> Result<&Chart, Error> {
        self.charts
            .get(name)
            .ok_or_else(|| Error::NotFound(name.to_string()))
    }

    /// Resolve the names a subcommand was given, where none means every chart.
    /// Unknown names fail here, before any network or filesystem work starts.
    pub fn select(&self, names: &[String]) -> Result<Vec<(&String, &Chart)>, Error> {
        if names.is_empty() {
            return Ok(self.charts.iter().collect());
        }
        names
            .iter()
            .map(|name| {
                self.charts
                    .get_key_value(name.as_str())
                    .ok_or_else(|| Error::NotFound(name.clone()))
            })
            .collect()
    }
}

/// Charts are vendored into a `charts/` directory beside charts.toml.
pub fn charts_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("charts")
}

#[derive(Deserialize)]
struct RawChart {
    repo: Option<String>,
    oci: Option<String>,
    chart: String,
    version: String,
}

/// Find the single chart directory inside a helm-pull output dir. `helm pull
/// --untar` writes both `<chart>.tgz` and `<chart>/` named after Chart.yaml's
/// `name` field — which may differ from our TOML key — so callers locate it
/// by structure instead of by name.
pub fn locate_extracted(dir: &Path) -> std::io::Result<PathBuf> {
    let mut found: Option<PathBuf> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if found.is_some() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "helm pull produced multiple directories",
                ));
            }
            found = Some(entry.path());
        }
    }
    found.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "helm pull produced no extracted directory",
        )
    })
}

pub fn load(path: &Path) -> Result<ChartRegistry, Error> {
    let content = std::fs::read_to_string(path)?;
    let raw: BTreeMap<String, RawChart> =
        toml_edit::de::from_str(&content).map_err(|e| Error::Parse(e.to_string()))?;

    let charts = raw
        .into_iter()
        .map(|(name, raw)| {
            let source = match (raw.repo, raw.oci) {
                (Some(repo), None) => ChartSource::Repo { repo },
                (None, Some(oci)) => ChartSource::Oci { oci },
                (Some(_), Some(_)) => {
                    return Err(Error::Parse(format!(
                        "chart '{name}' has both 'repo' and 'oci'; specify only one"
                    )));
                }
                (None, None) => {
                    return Err(Error::Parse(format!(
                        "chart '{name}' has neither 'repo' nor 'oci'"
                    )));
                }
            };
            Ok((
                name,
                Chart {
                    chart: raw.chart,
                    version: raw.version,
                    source,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, Error>>()?;

    Ok(ChartRegistry { charts })
}
