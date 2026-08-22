//! # helm-vendor
//!
//! CLI for vendoring Helm charts from a declarative `charts.toml` registry.
//!
//! Charts are declared in a `charts.toml` file and vendored into a sibling `charts/` directory.
//!
//! `charts.toml` is auto-detected by walking up from the current directory; override with `--config <path>`.
//!
//! ## Usage
//!
//! ```text
//! $ helm-vendor --help
//! Vendor Helm charts from a declarative charts.toml registry
//!
//! Usage: helm-vendor [OPTIONS] <COMMAND>
//!
//! Commands:
//!   add         Declare a chart in charts.toml and vendor it. Creates charts.toml if it does not exist
//!   remove      Undeclare charts and delete their vendored directories
//!   sync        Vendor charts so charts/ matches charts.toml. Defaults to every chart
//!   update      Re-pin charts to the latest upstream version and vendor them
//!   list        List charts and their pinned versions
//!   completion  Print a shell completion script. Source it with e.g. `eval "$(helm-vendor completion zsh)"`
//!   help        Print this message or the help of the given subcommand(s)
//!
//! Options:
//!       --config <CONFIG>  Path to charts.toml. Auto-detected by walking up from CWD if omitted
//!   -h, --help             Print help
//!   -V, --version          Print version
//! ```
//!
//! Every chart-selecting command takes zero or more chart names, where naming none means all of
//! them. So `sync` vendors the whole registry and `sync cilium cert-manager` vendors two charts.
//!
//! ### Adding a new chart
//!
//! Omit the version to pin whatever upstream currently offers.
//!
//! ```console
//! $ helm-vendor add cert-manager https://charts.jetstack.io/
//!   added cert-manager 1.21.1
//!
//! $ git add charts.toml charts/cert-manager/
//! $ git commit -m "feat(charts): Vendored cert-manager 1.21.1"
//! ```
//!
//! ### Updating charts to their latest upstream versions
//!
//! `update` re-pins and vendors in one step, so charts.toml and `charts/` never disagree.
//!
//! ```console
//! $ helm-vendor update
//!   cert-manager v1.20.1 -> v1.21.1
//!   updated cert-manager
//! synced 1 of 1 chart(s)
//!
//! $ git add charts.toml charts/cert-manager/
//! $ git commit -m "chore(charts): Updated cert-manager to v1.21.1"
//! ```
//!
//! To see what is available before committing to it, ask `list`:
//!
//! ```console
//! $ helm-vendor list --outdated
//! NAME          VERSION   LATEST     source
//! cert-manager  v1.20.1   v1.21.1 ↑  https://charts.jetstack.io/
//! ```
//!
//! ### Removing a chart
//!
//! ```console
//! $ helm-vendor remove cert-manager
//!   removed cert-manager
//! ```
//!
//! Pass `--keep-files` to drop the charts.toml entry but leave the vendored directory behind.
//! `add` and `update` take the converse, `--no-fetch`, to write charts.toml without downloading.
//!
//! ### Verifying vendored charts in CI
//!
//! `sync --check` pulls every chart and compares it against what is committed, exiting non-zero
//! when they differ. `list --outdated` exits non-zero when an upgrade is available.
//!
//! ```console
//! $ helm-vendor sync --check
//! 18 chart(s) in sync
//! ```
//!
//! ## Installation
//!
//! For Nix/NixOS: Add `helm-vendor` to your flake inputs and include its overlay:
//!
//! ```nix
//! {
//!   inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
//!   inputs.helm-vendor = {
//!     url = "git+https://git.shine.town/infra/helm-vendor.git";
//!     inputs.nixpkgs.follows = "nixpkgs";
//!   };
//!
//!   outputs = { self, nixpkgs, helm-vendor, ... }:
//!     let
//!       system = "x86_64-linux";
//!       pkgs = import nixpkgs {
//!         inherit system;
//!         overlays = [ helm-vendor.overlays.default ];
//!       };
//!     in
//!     {
//!       devShells.${system}.default = pkgs.mkShell {
//!         packages = [
//!           pkgs.helm-vendor
//!           pkgs.kubernetes-helm
//!         ];
//!       };
//!     };
//! }
//! ```
//!
//! Or use it directly:
//!
//! ```console
//! $ nix run git+https://git.shine.town/infra/helm-vendor.git#helm-vendor -- --help
//! ```
mod add;
mod args;
mod chart;
mod color;
mod completion;
mod edit;
mod list;
mod progress;
mod remove;
mod resolver;
mod sync;
mod update;

use std::fmt::Display;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use clap::Parser;

use crate::args::{Args, Command};
use crate::resolver::Latest;

/// Upstream resolution is HTTP-bound; pull a small pool of charts in parallel
/// so the wall clock for `list --check` is roughly max-latency rather than
/// sum-of-latencies. 8 keeps us polite to upstream registries without
/// materially increasing rate-limit risk for typical chart counts.
pub(crate) const RESOLVE_CONCURRENCY: usize = 8;

fn main() -> ExitCode {
    let args = Args::parse();

    // `completion` must work without a charts.toml — users may invoke it from
    // anywhere to install shell hooks, and the hidden `--chart-names` callback
    // is expected to silently emit nothing when no registry is reachable.
    if let Command::Completion { shell, chart_names } = &args.command {
        return completion::run(args.config_path(), *shell, *chart_names);
    }

    // `add` also works without a pre-existing charts.toml: it falls back to
    // creating one in the current directory when none is found up the tree.
    if let Command::Add {
        name,
        url,
        version,
        no_fetch,
    } = &args.command
    {
        let config_path = args.config_path().unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join("charts.toml")
        });
        return report(add::run(
            &config_path,
            name,
            url,
            version.as_deref(),
            *no_fetch,
        ));
    }

    let config_path = match args.config_path() {
        Some(p) => p,
        None => {
            eprintln!("error: could not find charts.toml in current directory or any parent");
            return ExitCode::FAILURE;
        }
    };

    match args.command {
        Command::Completion { .. } | Command::Add { .. } => unreachable!(),
        Command::Remove { names, keep_files } => {
            report(remove::run(&config_path, &names, keep_files))
        }
        Command::Sync { names, check } => report(sync::run(&config_path, &names, check)),
        Command::Update {
            names,
            to,
            no_fetch,
        } => report(update::run(&config_path, &names, to.as_deref(), no_fetch)),
        Command::List {
            names,
            check,
            outdated,
        } => report(list::run(&config_path, &names, check, outdated)),
    }
}

fn report<E: Display>(result: Result<(), E>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) enum ResolveEvent<'a> {
    /// Fires whenever any worker finishes, in arbitrary order. Used to tick
    /// the progress counter as soon as work completes, so slow charts don't
    /// stall it.
    Completed { name: &'a str },
    /// Fires once per chart in registry order, buffered behind any earlier
    /// slot still in flight.
    Row {
        name: &'a str,
        chart: &'a chart::Chart,
        result: Result<Latest, resolver::Error>,
    },
}

/// Resolve every chart concurrently and report progress through a single
/// event stream. Combining `Completed` and `Row` into one closure keeps the
/// caller's borrow surface to a single `&mut` over its progress/buffer state.
pub(crate) fn resolve_in_parallel<F>(charts: &[(&String, &chart::Chart)], mut on_event: F)
where
    F: FnMut(ResolveEvent<'_>),
{
    let total = charts.len();
    if total == 0 {
        return;
    }
    let next_job = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Result<Latest, resolver::Error>)>();
    let workers = RESOLVE_CONCURRENCY.min(total);

    std::thread::scope(|s| {
        let next_job = &next_job;
        for _ in 0..workers {
            let tx = tx.clone();
            s.spawn(move || {
                loop {
                    let idx = next_job.fetch_add(1, Ordering::Relaxed);
                    if idx >= total {
                        break;
                    }
                    let result = resolver::latest_version(charts[idx].1);
                    if tx.send((idx, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        let mut slots: Vec<Option<Result<Latest, resolver::Error>>> =
            (0..total).map(|_| None).collect();
        let mut next_emit = 0usize;
        while let Ok((idx, result)) = rx.recv() {
            on_event(ResolveEvent::Completed {
                name: charts[idx].0,
            });
            slots[idx] = Some(result);
            while next_emit < total {
                let Some(result) = slots[next_emit].take() else {
                    break;
                };
                let (name, chart) = (charts[next_emit].0.as_str(), charts[next_emit].1);
                on_event(ResolveEvent::Row {
                    name,
                    chart,
                    result,
                });
                next_emit += 1;
            }
        }
    });
}
