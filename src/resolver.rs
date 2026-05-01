//! Upstream version resolution.
//!
//! Looks up the newest available version of a chart at its source. Two
//! source-specific paths exist:
//!
//! - `ChartSource::Repo` — fetch `{repo}/index.yaml` and pick the highest
//!   non-prerelease SemVer in `entries[chart]`.
//! - `ChartSource::Oci` — call the OCI distribution `tags/list` endpoint
//!   (with the anonymous Bearer-token dance for ghcr.io et al.) and pick the
//!   highest non-prerelease SemVer tag.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use crate::chart::{Chart, ChartSource};

const USER_AGENT: &str = concat!("helm-vendor/", env!("CARGO_PKG_VERSION"));
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("HTTP {status} from {url}")]
    HttpStatus { status: u16, url: String },
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid OCI reference: {0}")]
    BadOci(String),
    #[error("invalid WWW-Authenticate header: {0}")]
    BadAuthChallenge(String),
}

#[derive(Debug, Clone)]
pub enum Latest {
    /// Resolved version as a bare SemVer string (no leading `v`); the caller
    /// re-adds a prefix to match the existing pin.
    Version(String),
    /// Upstream returned data we can't interpret; reason attached.
    Unknown(String),
}

pub fn latest_version(chart: &Chart) -> Result<Latest, Error> {
    match &chart.source {
        ChartSource::Repo { repo } => latest_from_repo(repo, &chart.chart),
        ChartSource::Oci { oci } => latest_from_oci(oci),
    }
}

/// True if `pinned` matches `latest` ignoring an optional leading `v`.
pub fn is_up_to_date(pinned: &str, latest: &str) -> bool {
    pinned.trim_start_matches('v') == latest.trim_start_matches('v')
}

fn http_client() -> Result<reqwest::blocking::Client, Error> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(HTTP_TIMEOUT)
        .build()?)
}

// -------------------------------------------------------------------------
// HTTP repo path
// -------------------------------------------------------------------------

fn latest_from_repo(repo: &str, chart: &str) -> Result<Latest, Error> {
    let url = format!("{}/index.yaml", repo.trim_end_matches('/'));
    let client = http_client()?;
    let response = client.get(&url).send()?;
    if !response.status().is_success() {
        return Err(Error::HttpStatus {
            status: response.status().as_u16(),
            url,
        });
    }
    let body = response.text()?;
    pick_latest_from_index(&body, chart)
}

#[derive(Deserialize)]
struct HelmIndex {
    entries: HashMap<String, Vec<HelmIndexEntry>>,
}

#[derive(Deserialize)]
struct HelmIndexEntry {
    version: String,
}

fn pick_latest_from_index(yaml: &str, chart: &str) -> Result<Latest, Error> {
    let index: HelmIndex = serde_yaml::from_str(yaml)?;
    let entries = match index.entries.get(chart) {
        Some(es) => es,
        None => {
            return Ok(Latest::Unknown(format!(
                "chart '{chart}' not in index.yaml entries"
            )));
        }
    };
    Ok(pick_latest(entries.iter().map(|e| e.version.as_str())))
}

// -------------------------------------------------------------------------
// OCI path
// -------------------------------------------------------------------------

fn latest_from_oci(oci: &str) -> Result<Latest, Error> {
    let (registry, repository) = parse_oci_ref(oci)?;
    let base = format!("https://{registry}");
    let mut next_url = format!("{base}/v2/{repository}/tags/list");
    let client = http_client()?;
    let mut token: Option<String> = None;
    let mut all_tags: Vec<String> = Vec::new();
    let mut pages = 0usize;

    loop {
        pages += 1;
        if pages > MAX_OCI_PAGES {
            break;
        }

        let response = {
            let mut req = client.get(&next_url);
            if let Some(t) = &token {
                req = req.bearer_auth(t);
            }
            req.send()?
        };

        let response = if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let challenge = response
                .headers()
                .get(reqwest::header::WWW_AUTHENTICATE)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| Error::BadAuthChallenge("no WWW-Authenticate header on 401".into()))?
                .to_string();
            token = Some(fetch_bearer_token(&client, &challenge)?);
            client
                .get(&next_url)
                .bearer_auth(token.as_ref().unwrap())
                .send()?
        } else {
            response
        };

        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                status: response.status().as_u16(),
                url: next_url,
            });
        }

        let link = response
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);

        let body = response.text()?;
        let parsed: OciTags = serde_json::from_str(&body)?;
        all_tags.extend(parsed.tags);

        match link.as_deref().and_then(parse_next_link) {
            Some(next) => {
                next_url = if next.starts_with("http://") || next.starts_with("https://") {
                    next
                } else {
                    format!(
                        "{base}{}",
                        if next.starts_with('/') {
                            next
                        } else {
                            format!("/{next}")
                        }
                    )
                };
            }
            None => break,
        }
    }

    Ok(pick_latest(all_tags.iter().map(String::as_str)))
}

const MAX_OCI_PAGES: usize = 200;

/// Extract the `next` URL from an RFC 5988 Link header. Tolerant of multiple
/// links and surrounding whitespace.
fn parse_next_link(header: &str) -> Option<String> {
    for part in header.split(',') {
        let part = part.trim();
        let Some(close) = part.find('>') else {
            continue;
        };
        let Some(open) = part.find('<') else { continue };
        if open >= close {
            continue;
        }
        let url = &part[open + 1..close];
        let params = &part[close + 1..];
        if params
            .split(';')
            .map(str::trim)
            .any(|p| matches!(p, "rel=\"next\"" | "rel=next"))
        {
            return Some(url.to_string());
        }
    }
    None
}

fn parse_oci_ref(oci: &str) -> Result<(&str, &str), Error> {
    let rest = oci
        .strip_prefix("oci://")
        .ok_or_else(|| Error::BadOci(format!("missing oci:// prefix: {oci}")))?;
    let (registry, repository) = rest
        .split_once('/')
        .ok_or_else(|| Error::BadOci(format!("no path component: {oci}")))?;
    if registry.is_empty() || repository.is_empty() {
        return Err(Error::BadOci(format!(
            "empty registry or repository: {oci}"
        )));
    }
    Ok((registry, repository))
}

fn fetch_bearer_token(
    client: &reqwest::blocking::Client,
    challenge: &str,
) -> Result<String, Error> {
    let challenge = challenge.trim();
    let challenge = challenge
        .strip_prefix("Bearer ")
        .or_else(|| challenge.strip_prefix("bearer "))
        .ok_or_else(|| Error::BadAuthChallenge(format!("not a Bearer challenge: {challenge}")))?;

    let params = parse_challenge_params(challenge);
    let realm = params
        .get("realm")
        .ok_or_else(|| Error::BadAuthChallenge("no realm in challenge".into()))?;

    let mut req = client.get(*realm);
    if let Some(service) = params.get("service") {
        req = req.query(&[("service", service)]);
    }
    if let Some(scope) = params.get("scope") {
        req = req.query(&[("scope", scope)]);
    }
    let response = req.send()?;
    if !response.status().is_success() {
        return Err(Error::HttpStatus {
            status: response.status().as_u16(),
            url: realm.to_string(),
        });
    }

    #[derive(Deserialize)]
    struct TokenResponse {
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        access_token: Option<String>,
    }
    let body: TokenResponse = serde_json::from_str(&response.text()?)?;
    body.token
        .or(body.access_token)
        .ok_or_else(|| Error::BadAuthChallenge("token endpoint returned no token".into()))
}

fn parse_challenge_params(s: &str) -> HashMap<&str, &str> {
    let mut out = HashMap::new();
    let mut rest = s;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
        let Some(eq) = rest.find('=') else { break };
        let key = rest[..eq].trim();
        let after = &rest[eq + 1..];
        let (value, next) = if let Some(after) = after.strip_prefix('"') {
            match after.find('"') {
                Some(end) => (&after[..end], &after[end + 1..]),
                None => break,
            }
        } else {
            let end = after.find(',').unwrap_or(after.len());
            (after[..end].trim(), &after[end..])
        };
        out.insert(key, value);
        rest = next;
    }
    out
}

#[derive(Deserialize)]
struct OciTags {
    #[serde(default)]
    tags: Vec<String>,
}

#[cfg(test)]
fn pick_latest_from_tags(json: &str) -> Result<Latest, Error> {
    let parsed: OciTags = serde_json::from_str(json)?;
    Ok(pick_latest(parsed.tags.iter().map(String::as_str)))
}

// -------------------------------------------------------------------------
// Shared SemVer ranking
// -------------------------------------------------------------------------

fn pick_latest<'a, I: Iterator<Item = &'a str>>(versions: I) -> Latest {
    let mut best: Option<(semver::Version, String)> = None;
    let mut saw_any = false;
    for raw in versions {
        saw_any = true;
        let stripped = raw.trim_start_matches('v');
        let Ok(parsed) = semver::Version::parse(stripped) else {
            continue;
        };
        if !parsed.pre.is_empty() {
            continue;
        }
        match &best {
            Some((current, _)) if &parsed <= current => {}
            _ => best = Some((parsed, stripped.to_string())),
        }
    }
    match best {
        Some((_, s)) => Latest::Version(s),
        None if saw_any => Latest::Unknown("no parseable SemVer versions".into()),
        None => Latest::Unknown("no versions returned".into()),
    }
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_highest_stable_from_index_yaml() {
        let yaml = r#"
apiVersion: v1
entries:
  alloy:
    - name: alloy
      version: 1.7.0
    - name: alloy
      version: 1.8.0
    - name: alloy
      version: 1.8.0-rc.1
    - name: alloy
      version: 1.6.5
  other-chart:
    - name: other-chart
      version: 9.9.9
"#;
        let latest = pick_latest_from_index(yaml, "alloy").unwrap();
        assert!(matches!(latest, Latest::Version(ref v) if v == "1.8.0"));
    }

    #[test]
    fn unknown_chart_in_index_yields_unknown() {
        let yaml = r#"
apiVersion: v1
entries:
  alloy:
    - name: alloy
      version: 1.7.0
"#;
        let latest = pick_latest_from_index(yaml, "missing").unwrap();
        assert!(matches!(latest, Latest::Unknown(_)));
    }

    #[test]
    fn v_prefix_versions_parse_and_strip() {
        let yaml = r#"
apiVersion: v1
entries:
  cert-manager:
    - name: cert-manager
      version: v1.20.0
    - name: cert-manager
      version: v1.20.1
"#;
        let latest = pick_latest_from_index(yaml, "cert-manager").unwrap();
        assert!(matches!(latest, Latest::Version(ref v) if v == "1.20.1"));
    }

    #[test]
    fn oci_tags_picks_newest_skipping_garbage() {
        let json = r#"{"name":"x/y","tags":["7.8.3","7.8.4","sha-abc","main","8.0.0-rc.1"]}"#;
        let latest = pick_latest_from_tags(json).unwrap();
        assert!(matches!(latest, Latest::Version(ref v) if v == "7.8.4"));
    }

    #[test]
    fn oci_tags_empty_yields_unknown() {
        let latest = pick_latest_from_tags(r#"{"name":"x","tags":[]}"#).unwrap();
        assert!(matches!(latest, Latest::Unknown(_)));
    }

    #[test]
    fn oci_tags_only_garbage_yields_unknown() {
        let latest = pick_latest_from_tags(r#"{"tags":["main","sha-abc"]}"#).unwrap();
        assert!(matches!(latest, Latest::Unknown(_)));
    }

    #[test]
    fn parses_ghcr_oci_reference() {
        let (reg, repo) = parse_oci_ref("oci://ghcr.io/argoproj/argo-helm/argo-cd").unwrap();
        assert_eq!(reg, "ghcr.io");
        assert_eq!(repo, "argoproj/argo-helm/argo-cd");
    }

    #[test]
    fn parses_forgejo_oci_reference() {
        let (reg, repo) = parse_oci_ref("oci://code.forgejo.org/forgejo-helm/forgejo").unwrap();
        assert_eq!(reg, "code.forgejo.org");
        assert_eq!(repo, "forgejo-helm/forgejo");
    }

    #[test]
    fn rejects_oci_reference_without_path() {
        assert!(parse_oci_ref("oci://ghcr.io").is_err());
    }

    #[test]
    fn parses_bearer_challenge_with_quoted_params() {
        let params = parse_challenge_params(
            r#"realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:foo:pull""#,
        );
        assert_eq!(
            params.get("realm").copied(),
            Some("https://auth.docker.io/token")
        );
        assert_eq!(params.get("service").copied(), Some("registry.docker.io"));
        assert_eq!(params.get("scope").copied(), Some("repository:foo:pull"));
    }

    #[test]
    fn parses_bearer_challenge_with_unquoted_params() {
        let params = parse_challenge_params("realm=https://example.com/auth,service=demo");
        assert_eq!(
            params.get("realm").copied(),
            Some("https://example.com/auth")
        );
        assert_eq!(params.get("service").copied(), Some("demo"));
    }

    #[test]
    fn parses_next_link_with_quoted_rel() {
        let header = r#"</v2/foo/tags/list?last=6.7.12&n=0>; rel="next""#;
        assert_eq!(
            parse_next_link(header).as_deref(),
            Some("/v2/foo/tags/list?last=6.7.12&n=0")
        );
    }

    #[test]
    fn ignores_link_without_next_rel() {
        let header = r#"</v2/foo>; rel="prev""#;
        assert!(parse_next_link(header).is_none());
    }

    #[test]
    fn is_up_to_date_handles_v_prefix_mismatch() {
        assert!(is_up_to_date("v1.20.1", "1.20.1"));
        assert!(is_up_to_date("1.20.1", "v1.20.1"));
        assert!(!is_up_to_date("1.20.0", "1.20.1"));
    }
}
