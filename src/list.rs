//! `list`: the read-only view of charts.toml.
//!
//! Bare `list` prints what is pinned and never touches the network. `--check`
//! adds a LATEST column resolved from upstream, and `--outdated` narrows that
//! same table to the charts that are behind, exiting non-zero when any are.

use std::path::Path;

use crate::chart::{self, Chart};
use crate::color;
use crate::progress;
use crate::resolver::{self, Latest};
use crate::{ResolveEvent, resolve_in_parallel};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Chart(#[from] chart::Error),
    #[error("{count} chart(s) outdated")]
    HasOutdated { count: usize },
}

const MIN_VERSION_WIDTH: usize = 8;
const LATEST_VERSION_WIDTH: usize = 7;
const LATEST_WIDTH: usize = LATEST_VERSION_WIDTH + 2;

pub fn run(
    config_path: &Path,
    names: &[String],
    check: bool,
    outdated_only: bool,
) -> Result<(), Error> {
    let registry = chart::load(config_path)?;
    let targets = registry.select(names)?;

    let name_w = targets
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let vw = targets
        .iter()
        .map(|(_, c)| c.version.chars().count())
        .max()
        .unwrap_or(MIN_VERSION_WIDTH)
        .max(MIN_VERSION_WIDTH);

    if !check && !outdated_only {
        println!("{:name_w$}  {:vw$}  source", "NAME", "VERSION");
        for (name, chart) in &targets {
            println!(
                "{name:name_w$}  {:vw$}  {}",
                chart.version,
                chart.source.url()
            );
        }
        return Ok(());
    }

    let header = format!(
        "{:name_w$}  {:vw$}  {:LATEST_WIDTH$}  source",
        "NAME", "VERSION", "LATEST"
    );

    let mut progress = progress::Progress::new(targets.len());
    let color_on = progress.interactive() && color::enabled();
    let mut rows: Vec<Row> = Vec::new();
    let mut header_printed = false;

    resolve_in_parallel(&targets, |event| match event {
        ResolveEvent::Completed { name } => progress.tick(name),
        ResolveEvent::Row {
            name,
            chart,
            result,
        } => {
            let row = build_row(name, chart, &result);
            if outdated_only && row.state == State::Current {
                return;
            }
            // Non-interactive output streams as results arrive; interactive
            // output buffers so the progress counter can be erased first.
            if !progress.interactive() {
                if !header_printed {
                    println!("{header}");
                    header_printed = true;
                }
                println!("{}", format_row(&row, name_w, vw, color_on));
                let _ = std::io::Write::flush(&mut std::io::stdout());
            }
            rows.push(row);
        }
    });

    progress.clear();

    if rows.is_empty() {
        println!("all charts up to date");
        return Ok(());
    }

    if progress.interactive() {
        println!("{header}");
        for row in &rows {
            println!("{}", format_row(row, name_w, vw, color_on));
        }
    }

    let outdated = count_outdated(&rows);
    if outdated_only && outdated > 0 {
        Err(Error::HasOutdated { count: outdated })
    } else {
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    /// Pinned version is the newest upstream offers.
    Current,
    /// Upstream is newer than the pin — the normal `↑` bump.
    Outdated,
    /// Upstream is *older* than the pin (source URL changed / catalog
    /// reshuffled). Not an upgrade; shown muted with a `↓` and not counted.
    Ahead,
    /// Upstream lookup failed or returned nothing we can interpret. Shown
    /// muted with a `?` so one dud source never hides the rest of the table.
    Failed,
}

/// How the LATEST cell is colored when color is enabled.
enum Style {
    /// SemVer-diff severity (green/yellow/orange) — only for `Outdated`.
    Bump(color::Severity),
    /// Gray — for `Ahead` and `Failed`.
    Muted,
    /// Uncolored — for `Current`.
    Plain,
}

struct Row {
    name: String,
    pinned: String,
    /// Pre-rendered LATEST cell text *without* color escapes (padded so plain
    /// columns line up); `paint_latest` applies `style` at print time.
    latest: String,
    source: String,
    state: State,
    style: Style,
    /// Free-text trailer printed *after* `source` (ragged-right at the very end
    /// of the line, so it never disturbs column alignment). Carries the failure
    /// reason for `Failed` rows; `None` otherwise.
    note: Option<String>,
}

/// Classify one resolved chart into a printable row. This is the pure heart of
/// the table rendering — every branch is unit-tested.
fn build_row(name: &str, chart: &Chart, result: &Result<Latest, resolver::Error>) -> Row {
    let pinned = &chart.version;
    let source = chart.source.url().to_string();
    let make = |latest: String, state: State, style: Style, note: Option<String>| Row {
        name: name.to_string(),
        pinned: pinned.clone(),
        latest,
        source: source.clone(),
        state,
        style,
        note,
    };

    match result {
        Ok(Latest::Version(latest)) => {
            if resolver::is_up_to_date(pinned, latest) {
                return make(
                    plain_cell(pinned, latest),
                    State::Current,
                    Style::Plain,
                    None,
                );
            }
            match resolver::compare(pinned, latest) {
                // pinned == latest by SemVer but differing strings: up to date.
                Some(std::cmp::Ordering::Equal) => make(
                    plain_cell(pinned, latest),
                    State::Current,
                    Style::Plain,
                    None,
                ),
                // pinned > latest: upstream regressed below our pin.
                Some(std::cmp::Ordering::Greater) => make(
                    arrow_cell(pinned, latest, '↓'),
                    State::Ahead,
                    Style::Muted,
                    None,
                ),
                // pinned < latest, or incomparable: treat as a real bump so we
                // never silently hide an available upgrade.
                Some(std::cmp::Ordering::Less) | None => {
                    let sev = color::severity(pinned, latest);
                    make(
                        arrow_cell(pinned, latest, '↑'),
                        State::Outdated,
                        Style::Bump(sev),
                        None,
                    )
                }
            }
        }
        Ok(Latest::Unknown(reason)) => make(
            question_cell(),
            State::Failed,
            Style::Muted,
            Some(reason.clone()),
        ),
        Err(e) => make(
            question_cell(),
            State::Failed,
            Style::Muted,
            Some(e.to_string()),
        ),
    }
}

/// Count only rows that represent a genuine upstream upgrade — `Ahead` and
/// `Failed` rows are informational and never make `--outdated` exit non-zero.
fn count_outdated(rows: &[Row]) -> usize {
    rows.iter().filter(|r| r.state == State::Outdated).count()
}

fn format_row(r: &Row, name_w: usize, vw: usize, color_on: bool) -> String {
    // The painted cell is placed with `{}` (no width): the plain text is already
    // padded, and ANSI escapes would inflate a width count and over-pad.
    let mut line = format!(
        "{:name_w$}  {:vw$}  {}  {}",
        r.name,
        r.pinned,
        paint_latest(r, color_on),
        r.source
    );
    // A `note` (failure reason) trails after `source` so its length never
    // pushes the aligned columns around; it's muted to match the `?` cell.
    if let Some(note) = &r.note {
        line.push_str("  ");
        line.push_str(&color::gray(note, color_on));
    }
    line
}

fn paint_latest(r: &Row, color_on: bool) -> String {
    match r.style {
        Style::Bump(sev) => color::paint(&r.latest, sev, color_on),
        Style::Muted => color::gray(&r.latest, color_on),
        Style::Plain => r.latest.clone(),
    }
}

/// Match the pin's `v`-prefix style, so a `v1.20.2` pin yields `v1.20.3` rather
/// than `1.20.3` in a column that sits right beside it.
fn preserve_prefix(pinned: &str, latest: &str) -> String {
    let stripped = latest.trim_start_matches('v');
    if pinned.starts_with('v') {
        format!("v{stripped}")
    } else {
        stripped.to_string()
    }
}

/// `<latest-version padded to 7> <arrow>`.
fn arrow_cell(pinned: &str, latest: &str, arrow: char) -> String {
    format!(
        "{:LATEST_VERSION_WIDTH$} {arrow}",
        preserve_prefix(pinned, latest)
    )
}

/// An up-to-date cell reserves the arrow slot with two spaces, so rows keep a
/// predictable width regardless of which charts are behind.
fn plain_cell(pinned: &str, latest: &str) -> String {
    format!(
        "{:LATEST_VERSION_WIDTH$}  ",
        preserve_prefix(pinned, latest)
    )
}

/// A bare `?` padded to the LATEST column width. The failure reason rides in
/// the row's `note` (printed after `source`) so this cell stays column-aligned.
fn question_cell() -> String {
    format!("{:LATEST_WIDTH$}", "?")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Chart, ChartSource};

    fn repo_chart(version: &str) -> Chart {
        Chart {
            chart: "x".into(),
            version: version.into(),
            source: ChartSource::Repo {
                repo: "https://example.com".into(),
            },
        }
    }

    fn http_err() -> Result<Latest, resolver::Error> {
        Err(resolver::Error::HttpStatus {
            status: 503,
            url: "https://charts.bitnami.com/bitnami/index.yaml".into(),
        })
    }

    #[test]
    fn failed_fetch_still_renders_a_muted_question_row() {
        let chart = repo_chart("12.0.18");
        let row = build_row("etcd", &chart, &http_err());
        assert_eq!(row.state, State::Failed);
        assert!(row.latest.contains('?'), "latest cell: {:?}", row.latest);
        assert!(!row.latest.contains('↑'));
        assert!(matches!(row.style, Style::Muted));
        // The reason lives in `note` (printed after source), not in the cell.
        assert!(
            row.note.as_deref().is_some_and(|n| n.contains("503")),
            "note: {:?}",
            row.note
        );
        assert!(!row.latest.contains("503"));
    }

    #[test]
    fn unknown_upstream_renders_muted_question_row() {
        let chart = repo_chart("1.2.3");
        let row = build_row(
            "x",
            &chart,
            &Ok(Latest::Unknown("no parseable SemVer versions".into())),
        );
        assert_eq!(row.state, State::Failed);
        assert!(row.latest.contains('?'));
        assert!(matches!(row.style, Style::Muted));
        assert_eq!(row.note.as_deref(), Some("no parseable SemVer versions"));
    }

    #[test]
    fn failure_reason_trails_after_source_keeping_columns_aligned() {
        let chart = repo_chart("1.0.0");
        let row = build_row("broken", &chart, &http_err());
        let line = format_row(&row, 6, 8, false);
        // source comes before the reason; the reason is the line's tail.
        let src = chart.source.url();
        let src_at = line.find(src).expect("source present");
        let note = row.note.as_deref().expect("failed rows carry a note");
        let note_at = line.find(note).expect("note present");
        assert!(note_at > src_at, "reason must trail source: {line:?}");
        assert!(line.trim_end().ends_with(note));
    }

    #[test]
    fn outdated_row_has_no_trailing_note() {
        let chart = repo_chart("1.0.0");
        let row = build_row("x", &chart, &Ok(Latest::Version("2.0.0".into())));
        assert!(row.note.is_none());
        let line = format_row(&row, 4, 8, false);
        assert!(line.trim_end().ends_with(chart.source.url()));
    }

    #[test]
    fn upstream_lower_than_pinned_is_ahead_not_outdated() {
        // Source URL changed and now resolves a version *older* than our pin.
        let chart = repo_chart("12.0.18");
        let row = build_row("etcd", &chart, &Ok(Latest::Version("11.0.0".into())));
        assert_eq!(row.state, State::Ahead);
        assert!(row.latest.contains('↓'), "latest cell: {:?}", row.latest);
        assert!(!row.latest.contains('↑'));
        assert!(matches!(row.style, Style::Muted));
    }

    #[test]
    fn genuinely_outdated_renders_a_severity_arrow() {
        let chart = repo_chart("1.2.3");
        let row = build_row("x", &chart, &Ok(Latest::Version("1.3.0".into())));
        assert_eq!(row.state, State::Outdated);
        assert!(row.latest.contains('↑'));
        assert!(matches!(row.style, Style::Bump(_)));
    }

    #[test]
    fn up_to_date_renders_a_plain_arrowless_row() {
        let chart = repo_chart("1.2.3");
        let row = build_row("x", &chart, &Ok(Latest::Version("1.2.3".into())));
        assert_eq!(row.state, State::Current);
        assert!(row.latest.contains("1.2.3"));
        assert!(!row.latest.contains('↑') && !row.latest.contains('↓'));
        assert!(matches!(row.style, Style::Plain));
    }

    #[test]
    fn up_to_date_and_outdated_cells_are_the_same_width() {
        let current = build_row(
            "x",
            &repo_chart("1.2.3"),
            &Ok(Latest::Version("1.2.3".into())),
        );
        let behind = build_row(
            "x",
            &repo_chart("1.2.3"),
            &Ok(Latest::Version("1.3.0".into())),
        );
        assert_eq!(
            current.latest.chars().count(),
            behind.latest.chars().count()
        );
    }

    #[test]
    fn only_outdated_rows_count_toward_failure() {
        let rows = [
            build_row(
                "a",
                &repo_chart("1.0.0"),
                &Ok(Latest::Version("2.0.0".into())),
            ),
            build_row(
                "b",
                &repo_chart("12.0.0"),
                &Ok(Latest::Version("11.0.0".into())),
            ),
            build_row("c", &repo_chart("1.0.0"), &http_err()),
            build_row(
                "d",
                &repo_chart("1.0.0"),
                &Ok(Latest::Version("1.0.0".into())),
            ),
        ];
        assert_eq!(count_outdated(&rows), 1);
    }

    #[test]
    fn muted_cell_is_gray_when_color_is_on() {
        let chart = repo_chart("12.0.0");
        let row = build_row("etcd", &chart, &Ok(Latest::Version("11.0.0".into())));
        let painted = paint_latest(&row, true);
        assert!(
            painted.starts_with("\x1b[90m") && painted.ends_with("\x1b[0m"),
            "expected gray ANSI wrap, got {painted:?}"
        );
    }
}
