use std::path::Path;

use crate::chart::{self, ChartSource};
use crate::color;
use crate::progress;
use crate::resolver::{self, Latest};
use crate::{ResolveEvent, resolve_in_parallel};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("chart error: {0}")]
    Chart(#[from] chart::Error),
    #[error("{count} chart(s) outdated")]
    HasOutdated { count: usize },
}

const MIN_PINNED_WIDTH: usize = 8;
const LATEST_VERSION_WIDTH: usize = 7;
const LATEST_WIDTH: usize = LATEST_VERSION_WIDTH + 2;

pub fn run(config_path: &Path, name: Option<&str>) -> Result<(), Error> {
    let registry = chart::load(config_path)?;

    let targets: Vec<(&String, &chart::Chart)> = if let Some(n) = name {
        let chart_entry = registry.get(n)?;
        let key = registry
            .charts
            .keys()
            .find(|k| k.as_str() == n)
            .expect("get() succeeded, key must exist");
        vec![(key, chart_entry)]
    } else {
        registry.charts.iter().collect()
    };

    let name_width = targets
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    let pw = targets
        .iter()
        .map(|(_, c)| c.version.chars().count())
        .max()
        .unwrap_or(MIN_PINNED_WIDTH)
        .max(MIN_PINNED_WIDTH);
    let lw = LATEST_WIDTH;
    let header = format!(
        "{:name_width$}  {:pw$}  {:lw$}  source",
        "NAME", "PINNED", "LATEST"
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
            let Some(row) = build_row(name, chart, &result) else {
                return;
            };

            if !progress.interactive() {
                progress.clear();
                if !header_printed {
                    println!("{header}");
                    header_printed = true;
                }
                println!("{}", format_row(&row, name_width, pw, color_on));
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
        for r in &rows {
            println!("{}", format_row(r, name_width, pw, color_on));
        }
    }

    let outdated = count_outdated(&rows);
    if outdated > 0 {
        Err(Error::HasOutdated { count: outdated })
    } else {
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
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

/// Classify one resolved chart into a printable row, or `None` when it's up to
/// date (those are omitted from the table). This is the pure heart of
/// `outdated` rendering — every branch is unit-tested.
fn build_row(
    name: &str,
    chart: &chart::Chart,
    result: &Result<Latest, resolver::Error>,
) -> Option<Row> {
    let pinned = &chart.version;
    let source = source_str(&chart.source).to_string();
    let make = |latest: String, state: State, style: Style, note: Option<String>| {
        Some(Row {
            name: name.to_string(),
            pinned: pinned.clone(),
            latest,
            source: source.clone(),
            state,
            style,
            note,
        })
    };

    match result {
        Ok(Latest::Version(latest)) => {
            if resolver::is_up_to_date(pinned, latest) {
                return None;
            }
            match resolver::compare(pinned, latest) {
                // pinned == latest by SemVer but differing strings: up to date.
                Some(std::cmp::Ordering::Equal) => None,
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
/// `Failed` rows are informational and never make `outdated` exit non-zero.
fn count_outdated(rows: &[Row]) -> usize {
    rows.iter().filter(|r| r.state == State::Outdated).count()
}

fn format_row(r: &Row, name_w: usize, pw: usize, color_on: bool) -> String {
    // The painted cell is placed with `{}` (no `:lw$`): the plain text is
    // already padded, and ANSI escapes would inflate a width count and over-pad.
    let mut line = format!(
        "{:name_w$}  {:pw$}  {}  {}",
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
    }
}

fn source_str(source: &ChartSource) -> &str {
    match source {
        ChartSource::Repo { repo } => repo.as_str(),
        ChartSource::Oci { oci } => oci.as_str(),
    }
}

/// `<latest-version padded to 7> <arrow>`, with the `v`-prefix matched to the
/// pin so e.g. a `v1.20.2` pin yields `v1.20.3 ↑`, not `1.20.3 ↑`.
fn arrow_cell(pinned: &str, latest: &str, arrow: char) -> String {
    let vw = LATEST_VERSION_WIDTH;
    let stripped = latest.trim_start_matches('v');
    let prefixed = if pinned.starts_with('v') {
        format!("v{stripped}")
    } else {
        stripped.to_string()
    };
    format!("{prefixed:vw$} {arrow}")
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
        let row = build_row("etcd", &chart, &http_err())
            .expect("a chart that fails to fetch must still produce a row");
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
        )
        .expect("unknown upstream must still produce a row");
        assert_eq!(row.state, State::Failed);
        assert!(row.latest.contains('?'));
        assert!(matches!(row.style, Style::Muted));
        assert_eq!(row.note.as_deref(), Some("no parseable SemVer versions"));
    }

    #[test]
    fn failure_reason_trails_after_source_keeping_columns_aligned() {
        let chart = repo_chart("1.0.0");
        let row = build_row("broken", &chart, &http_err()).unwrap();
        let line = format_row(&row, 6, 8, false);
        // source comes before the reason; the reason is the line's tail.
        let src = source_str(&chart.source);
        let src_at = line.find(src).expect("source present");
        let note = row.note.as_deref().unwrap();
        let note_at = line.find(note).expect("note present");
        assert!(note_at > src_at, "reason must trail source: {line:?}");
        assert!(line.trim_end().ends_with(note));
    }

    #[test]
    fn outdated_row_has_no_trailing_note() {
        let chart = repo_chart("1.0.0");
        let row = build_row("x", &chart, &Ok(Latest::Version("2.0.0".into()))).unwrap();
        assert!(row.note.is_none());
        let line = format_row(&row, 4, 8, false);
        assert!(line.trim_end().ends_with(source_str(&chart.source)));
    }

    #[test]
    fn upstream_lower_than_pinned_is_ahead_not_outdated() {
        // Source URL changed and now resolves a version *older* than our pin.
        let chart = repo_chart("12.0.18");
        let row = build_row("etcd", &chart, &Ok(Latest::Version("11.0.0".into())))
            .expect("a downgrade-available row must render");
        assert_eq!(row.state, State::Ahead);
        assert!(row.latest.contains('↓'), "latest cell: {:?}", row.latest);
        assert!(!row.latest.contains('↑'));
        assert!(matches!(row.style, Style::Muted));
    }

    #[test]
    fn genuinely_outdated_renders_a_severity_arrow() {
        let chart = repo_chart("1.2.3");
        let row = build_row("x", &chart, &Ok(Latest::Version("1.3.0".into())))
            .expect("an outdated row must render");
        assert_eq!(row.state, State::Outdated);
        assert!(row.latest.contains('↑'));
        assert!(matches!(row.style, Style::Bump(_)));
    }

    #[test]
    fn up_to_date_renders_no_row() {
        let chart = repo_chart("1.2.3");
        assert!(build_row("x", &chart, &Ok(Latest::Version("1.2.3".into()))).is_none());
    }

    #[test]
    fn only_outdated_rows_count_toward_failure() {
        let rows = [
            build_row(
                "a",
                &repo_chart("1.0.0"),
                &Ok(Latest::Version("2.0.0".into())),
            )
            .unwrap(),
            build_row(
                "b",
                &repo_chart("12.0.0"),
                &Ok(Latest::Version("11.0.0".into())),
            )
            .unwrap(),
            build_row("c", &repo_chart("1.0.0"), &http_err()).unwrap(),
        ];
        assert_eq!(count_outdated(&rows), 1);
    }

    #[test]
    fn muted_cell_is_gray_when_color_is_on() {
        let chart = repo_chart("12.0.0");
        let row = build_row("etcd", &chart, &Ok(Latest::Version("11.0.0".into()))).unwrap();
        let painted = paint_latest(&row, true);
        assert!(
            painted.starts_with("\x1b[90m") && painted.ends_with("\x1b[0m"),
            "expected gray ANSI wrap, got {painted:?}"
        );
    }
}
