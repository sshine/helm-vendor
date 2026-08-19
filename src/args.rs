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

/// Chart selection is a variadic positional everywhere, where naming none means
/// the whole registry. Verifying is a `--check` mode on the command that owns
/// the data rather than a command of its own.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Declare a chart in charts.toml and vendor it. Creates charts.toml if it
    /// does not exist.
    Add {
        /// Chart name. Used as the charts.toml key and the directory name.
        name: String,

        /// Source URL: an `https://` Helm repo or an `oci://` reference.
        url: String,

        /// Version to pin. Omit to resolve and pin the latest upstream version.
        version: Option<String>,

        /// Record the chart without downloading it.
        #[arg(long)]
        no_fetch: bool,
    },

    /// Undeclare charts and delete their vendored directories.
    #[command(alias = "rm")]
    Remove {
        /// Chart names. At least one; `remove` never defaults to everything.
        #[arg(required = true)]
        names: Vec<String>,

        /// Drop the charts.toml entry but leave the vendored directory on disk.
        #[arg(long)]
        keep_files: bool,
    },

    /// Vendor charts so charts/ matches charts.toml. Defaults to every chart.
    #[command(aliases = ["fetch", "vendor"])]
    Sync {
        /// Chart names. Omit for every chart in charts.toml.
        names: Vec<String>,

        /// Report what is out of sync and exit non-zero instead of writing.
        #[arg(long)]
        check: bool,
    },

    /// Re-pin charts to the latest upstream version and vendor them.
    Update {
        /// Chart names. Omit for every chart in charts.toml.
        names: Vec<String>,

        /// Pin this exact version instead of resolving the latest. Takes
        /// exactly one chart name.
        #[arg(long, value_name = "VERSION")]
        to: Option<String>,

        /// Rewrite charts.toml without vendoring the new versions.
        #[arg(long)]
        no_fetch: bool,
    },

    /// List charts and their pinned versions.
    #[command(alias = "ls")]
    List {
        /// Chart names. Omit for every chart in charts.toml.
        names: Vec<String>,

        /// Query upstream and add a LATEST column.
        #[arg(long)]
        check: bool,

        /// Show only charts that are behind upstream, and exit non-zero when
        /// any are. Implies --check.
        #[arg(long)]
        outdated: bool,
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
