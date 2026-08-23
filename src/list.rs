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
const MIN_LATEST_WIDTH: usize = 7;

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
                    println!("{}", header(name_w, vw, MIN_LATEST_WIDTH));
                    header_printed = true;
                }
                println!(
                    "{}",
                    format_row(&row, name_w, vw, MIN_LATEST_WIDTH, color_on)
                );
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
        let lw = latest_width(&rows);
        println!("{}", header(name_w, vw, lw));
        for row in &rows {
            println!("{}", format_row(row, name_w, vw, lw, color_on));
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
    /// The resolved version as text, unpadded, or `?` when upstream could not be
    /// read. Padding happens at print time, once the column width is known.
    latest: String,
    /// `↑` for an available upgrade, `↓` for an upstream regression, `None`
    /// otherwise. Kept apart from `latest` so the two occupy separate columns.
    arrow: Option<char>,
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
    let make =
        |latest: String, arrow: Option<char>, state: State, style: Style, note: Option<String>| {
            Row {
                name: name.to_string(),
                pinned: pinned.clone(),
                latest,
                arrow,
                source: source.clone(),
                state,
                style,
                note,
            }
        };
    let current = |latest: &str| {
        make(
            preserve_prefix(pinned, latest),
            None,
            State::Current,
            Style::Plain,
            None,
        )
    };
    let unresolved = |reason: String| {
        make(
            "?".to_string(),
            None,
            State::Failed,
            Style::Muted,
            Some(reason),
        )
    };

    match result {
        Ok(Latest::Version(latest)) => {
            if resolver::is_up_to_date(pinned, latest) {
                return current(latest);
            }
            match resolver::compare(pinned, latest) {
                // pinned == latest by SemVer but differing strings: up to date.
                Some(std::cmp::Ordering::Equal) => current(latest),
                // pinned > latest: upstream regressed below our pin.
                Some(std::cmp::Ordering::Greater) => make(
                    preserve_prefix(pinned, latest),
                    Some('↓'),
                    State::Ahead,
                    Style::Muted,
                    None,
                ),
                // pinned < latest, or incomparable: treat as a real bump so we
                // never silently hide an available upgrade.
                Some(std::cmp::Ordering::Less) | None => {
                    let sev = color::severity(pinned, latest);
                    make(
                        preserve_prefix(pinned, latest),
                        Some('↑'),
                        State::Outdated,
                        Style::Bump(sev),
                        None,
                    )
                }
            }
        }
        Ok(Latest::Unknown(reason)) => unresolved(reason.clone()),
        Err(e) => unresolved(e.to_string()),
    }
}

/// Count only rows that represent a genuine upstream upgrade — `Ahead` and
/// `Failed` rows are informational and never make `--outdated` exit non-zero.
fn count_outdated(rows: &[Row]) -> usize {
    rows.iter().filter(|r| r.state == State::Outdated).count()
}

fn header(name_w: usize, vw: usize, lw: usize) -> String {
    let latest_w = lw + 2;
    format!(
        "{:name_w$}  {:vw$}  {:latest_w$}  source",
        "NAME", "VERSION", "LATEST"
    )
}

/// The width the LATEST version field needs for a whole table: the longest
/// resolved version, floored so a narrow table still clears the header.
fn latest_width(rows: &[Row]) -> usize {
    rows.iter()
        .map(|r| r.latest.chars().count())
        .max()
        .unwrap_or(MIN_LATEST_WIDTH)
        .max(MIN_LATEST_WIDTH)
}

fn format_row(r: &Row, name_w: usize, vw: usize, lw: usize, color_on: bool) -> String {
    // The painted cell is placed with `{}` (no width): it is already padded, and
    // ANSI escapes would inflate a width count and over-pad.
    let mut line = format!(
        "{:name_w$}  {:vw$}  {}  {}",
        r.name,
        r.pinned,
        paint_latest(r, lw, color_on),
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

/// `<latest padded to lw> <arrow-or-space>`. Every row reserves the arrow slot,
/// so which charts are behind never shifts the source column.
fn latest_cell(r: &Row, lw: usize) -> String {
    match r.arrow {
        Some(arrow) => format!("{:lw$} {arrow}", r.latest),
        None => format!("{:lw$}  ", r.latest),
    }
}

fn paint_latest(r: &Row, lw: usize, color_on: bool) -> String {
    let cell = latest_cell(r, lw);
    match r.style {
        Style::Bump(sev) => color::paint(&cell, sev, color_on),
        Style::Muted => color::gray(&cell, color_on),
        Style::Plain => cell,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Chart, ChartSource};

    const SOURCE: &str = "https://example.com";

    fn repo_chart(version: &str) -> Chart {
        Chart {
            chart: "x".into(),
            version: version.into(),
            source: ChartSource::Repo {
                repo: SOURCE.into(),
            },
        }
    }

    fn http_err() -> Result<Latest, resolver::Error> {
        Err(resolver::Error::HttpStatus {
            status: 503,
            url: "https://charts.bitnami.com/bitnami/index.yaml".into(),
        })
    }

    fn row(pinned: &str, latest: &str) -> Row {
        build_row(
            "x",
            &repo_chart(pinned),
            &Ok(Latest::Version(latest.into())),
        )
    }

    /// One row of every state a table can contain, so alignment assertions
    /// cover the arrow, no-arrow and unresolved cases together.
    fn one_of_each() -> Vec<Row> {
        vec![
            row("1.2.3", "1.2.3"),
            row("1.2.3", "1.3.0"),
            row("9.0.0", "1.0.0"),
            build_row("x", &repo_chart("1.2.3"), &http_err()),
        ]
    }

    /// Character offset, not byte offset: the arrows are three bytes each, so
    /// `str::find` would report arrow rows as further right than they render.
    fn char_offset(line: &str, needle: &str) -> usize {
        let byte_at = line.find(needle).expect("needle present in line");
        line[..byte_at].chars().count()
    }

    #[test]
    fn failed_fetch_still_renders_a_muted_question_row() {
        let row = build_row("etcd", &repo_chart("12.0.18"), &http_err());
        assert_eq!(row.state, State::Failed);
        assert_eq!(row.latest, "?");
        assert_eq!(row.arrow, None);
        assert!(matches!(row.style, Style::Muted));
        // The reason lives in `note` (printed after source), not in the cell.
        assert!(
            row.note.as_deref().is_some_and(|n| n.contains("503")),
            "note: {:?}",
            row.note
        );
    }

    #[test]
    fn unknown_upstream_renders_muted_question_row() {
        let row = build_row(
            "x",
            &repo_chart("1.2.3"),
            &Ok(Latest::Unknown("no parseable SemVer versions".into())),
        );
        assert_eq!(row.state, State::Failed);
        assert_eq!(row.latest, "?");
        assert!(matches!(row.style, Style::Muted));
        assert_eq!(row.note.as_deref(), Some("no parseable SemVer versions"));
    }

    #[test]
    fn failure_reason_trails_after_source_keeping_columns_aligned() {
        let row = build_row("broken", &repo_chart("1.0.0"), &http_err());
        let line = format_row(&row, 6, 8, MIN_LATEST_WIDTH, false);
        let note = row.note.as_deref().expect("failed rows carry a note");
        assert!(
            char_offset(&line, note) > char_offset(&line, SOURCE),
            "reason must trail source: {line:?}"
        );
        assert!(line.trim_end().ends_with(note));
    }

    #[test]
    fn outdated_row_has_no_trailing_note() {
        let row = row("1.0.0", "2.0.0");
        assert!(row.note.is_none());
        let line = format_row(&row, 4, 8, MIN_LATEST_WIDTH, false);
        assert!(line.trim_end().ends_with(SOURCE));
    }

    #[test]
    fn upstream_lower_than_pinned_is_ahead_not_outdated() {
        // Source URL changed and now resolves a version *older* than our pin.
        let row = row("12.0.18", "11.0.0");
        assert_eq!(row.state, State::Ahead);
        assert_eq!(row.arrow, Some('↓'));
        assert!(matches!(row.style, Style::Muted));
    }

    #[test]
    fn genuinely_outdated_renders_a_severity_arrow() {
        let row = row("1.2.3", "1.3.0");
        assert_eq!(row.state, State::Outdated);
        assert_eq!(row.arrow, Some('↑'));
        assert!(matches!(row.style, Style::Bump(_)));
    }

    #[test]
    fn up_to_date_renders_a_plain_arrowless_row() {
        let row = row("1.2.3", "1.2.3");
        assert_eq!(row.state, State::Current);
        assert_eq!(row.latest, "1.2.3");
        assert_eq!(row.arrow, None);
        assert!(matches!(row.style, Style::Plain));
    }

    #[test]
    fn every_row_state_yields_the_same_latest_cell_width() {
        let widths: Vec<usize> = one_of_each()
            .iter()
            .map(|r| latest_cell(r, MIN_LATEST_WIDTH).chars().count())
            .collect();
        assert!(
            widths.iter().all(|w| *w == MIN_LATEST_WIDTH + 2),
            "cell widths: {widths:?}"
        );
    }

    #[test]
    fn every_row_state_starts_source_at_the_same_column() {
        let offsets: Vec<usize> = one_of_each()
            .iter()
            .map(|r| char_offset(&format_row(r, 4, 8, MIN_LATEST_WIDTH, false), SOURCE))
            .collect();
        assert!(
            offsets.windows(2).all(|w| w[0] == w[1]),
            "source offsets: {offsets:?}"
        );
    }

    #[test]
    fn header_puts_source_where_the_rows_do() {
        let rows = one_of_each();
        let lw = latest_width(&rows);
        let header_at = char_offset(&header(4, 8, lw), "source");
        for r in &rows {
            let line = format_row(r, 4, 8, lw, false);
            assert_eq!(
                char_offset(&line, SOURCE),
                header_at,
                "row misaligned with header: {line:?}"
            );
        }
    }

    /// A version longer than the floor widens the column for the whole table
    /// rather than overhanging on its own row. `v1.21.10` is eight characters,
    /// one past the floor, which is what used to push `source` right.
    #[test]
    fn a_version_past_the_floor_widens_every_row_alike() {
        let mut rows = one_of_each();
        rows.push(row("v1.2.3", "v1.21.10"));
        let lw = latest_width(&rows);
        assert_eq!(lw, 8);

        let offsets: Vec<usize> = rows
            .iter()
            .map(|r| char_offset(&format_row(r, 4, 8, lw, false), SOURCE))
            .collect();
        assert!(
            offsets.windows(2).all(|w| w[0] == w[1]),
            "source offsets: {offsets:?}"
        );
    }

    #[test]
    fn latest_width_never_drops_below_the_floor() {
        assert_eq!(latest_width(&[]), MIN_LATEST_WIDTH);
        assert_eq!(latest_width(&[row("1.0", "1.0")]), MIN_LATEST_WIDTH);
    }

    #[test]
    fn only_outdated_rows_count_toward_failure() {
        assert_eq!(count_outdated(&one_of_each()), 1);
    }

    #[test]
    fn muted_cell_is_gray_when_color_is_on() {
        let row = row("12.0.0", "11.0.0");
        let painted = paint_latest(&row, MIN_LATEST_WIDTH, true);
        assert!(
            painted.starts_with("\x1b[90m") && painted.ends_with("\x1b[0m"),
            "expected gray ANSI wrap, got {painted:?}"
        );
    }
}
