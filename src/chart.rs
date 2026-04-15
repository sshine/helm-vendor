use std::collections::BTreeMap;
use std::path::Path;

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
}

#[derive(Deserialize)]
struct RawChart {
    repo: Option<String>,
    oci: Option<String>,
    chart: String,
    version: String,
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
