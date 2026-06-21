use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "helm-vendor",
    version,
    about = "Vendor Helm charts from a declarative charts.toml registry"
)]
pub struct Args {
    /// Path to charts.toml. Auto-detected by walking up from CWD if omitted.
    #[arg(long = "config", global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add a new chart to charts.toml and vendor it. Creates charts.toml if
    /// it does not exist.
    Add {
        /// Chart name. Used as the charts.toml key and the chart name to pull.
        name: String,

        /// Source URL: an `https://` Helm repo or an `oci://` reference.
        url: String,

        /// Version to pin. Omit to resolve and pin the latest upstream version.
        version: Option<String>,
    },

    /// Pull chart(s) from upstream into the charts/ directory.
    #[command(alias = "vendor")]
    Fetch {
        /// Chart name to fetch. Omit when using --all.
        name: Option<String>,

        /// Fetch all charts declared in charts.toml.
        #[arg(long)]
        all: bool,
    },

    /// Verify vendored charts match their upstream source.
    Check {
        /// Chart name to check. Omit when using --all.
        name: Option<String>,

        /// Check all charts declared in charts.toml.
        #[arg(long)]
        all: bool,
    },

    /// Update the version of a chart in charts.toml.
    Update {
        /// Chart name. Omit when using --all --latest.
        name: Option<String>,

        /// New version string. Mutually exclusive with --latest.
        #[arg(conflicts_with = "latest")]
        version: Option<String>,

        /// Resolve and write the latest upstream version.
        #[arg(long)]
        latest: bool,

        /// Apply --latest to every chart in charts.toml.
        #[arg(long, requires = "latest")]
        all: bool,
    },

    /// List all charts and their versions.
    List {
        /// Query upstream and add a LATEST column.
        #[arg(long)]
        check: bool,
    },

    /// Show charts whose pinned version is older than what's upstream.
    Outdated {
        /// Limit to a single chart.
        name: Option<String>,
    },

    /// Print a shell completion script. Source it with e.g.
    /// `eval "$(helm-vendor completion zsh)"`.
    Completion {
        /// Shell to generate the completion script for.
        #[arg(value_enum, required_unless_present = "chart_names")]
        shell: Option<Shell>,

        /// Print chart names from charts.toml. Used internally by the
        /// generated completion scripts to populate dynamic chart-name
        /// completion; prints nothing (and exits 0) when no charts.toml
        /// is reachable.
        #[arg(long, hide = true, conflicts_with = "shell")]
        chart_names: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Args {
    pub fn config_path(&self) -> Option<PathBuf> {
        if let Some(p) = &self.config {
            return Some(p.clone());
        }
        find_charts_toml()
    }
}

fn find_charts_toml() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let candidate = dir.join("charts.toml");
        if candidate.is_file() {
            return Some(candidate);
        }
        if !dir.pop() {
            return None;
        }
    }
}
