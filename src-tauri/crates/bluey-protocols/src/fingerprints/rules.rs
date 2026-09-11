//! Rules: what a provider's request must look like, as data.
//!
//! Every row of the verified tables in `docs/PROVIDER_ACCOUNTS.md` becomes a
//! [`HeaderRule`] or [`FieldRule`] here, and every rule names the doc row it keeps
//! true (`doc`), so a drift report can say which column to fix.

use super::Provider;
use crate::request_shaper::FingerprintInfo;

/// How a header, query parameter or body field is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Values (or whole subtrees) must be identical after placeholder normalisation.
    /// The default for requests. Objects and arrays recurse, and every child looks up
    /// its own rule.
    Exact,
    /// Presence differences are informational; when both sides are present the
    /// children are compared with their own rules (tool lists, optional wrappers).
    Children,
    /// The value must match the regex; a different value that still matches is a
    /// `Version` finding ("format unchanged — update the doc column").
    Pattern(&'static str),
    /// A comma-separated header or an array of strings compared as a set: `required`
    /// members must be present, members outside `required ∪ optional` are drift,
    /// `optional` differences are informational.
    Set {
        required: &'static [&'static str],
        optional: &'static [&'static str],
    },
    /// Must be present; the value is not compared.
    Present,
    /// Must not be present.
    Absent,
    /// Must be present; value differences are informational.
    Info,
    /// Presence and value differences are informational. The default for responses.
    Optional,
    /// Compare JSON kinds only: objects report differing key sets, arrays differing
    /// lengths, both as information.
    Shape,
    /// Ignored entirely (`content-length`, retry counters, per-request timing).
    Volatile,
}

#[derive(Debug, Clone, Copy)]
pub struct HeaderRule {
    /// Lower-case header (or query parameter) name; a trailing `*` matches a prefix.
    pub name: &'static str,
    pub rule: Rule,
    /// The `docs/PROVIDER_ACCOUNTS.md` row this rule keeps true.
    pub doc: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct FieldRule {
    /// Body path: `system[1].text`, `input[*].content`, `request.contents`, `tools[*].*`.
    pub path: &'static str,
    pub rule: Rule,
    pub doc: &'static str,
}

/// One upstream endpoint of a provider.
#[derive(Debug, Clone, Copy)]
pub struct Endpoint {
    /// File-name-safe name (`messages`, `models`, `responses`, `stream`).
    pub name: &'static str,
    pub method: &'static str,
    /// Exact path (no query).
    pub path: &'static str,
    /// Header rules that override the provider's for this endpoint.
    pub headers: &'static [HeaderRule],
    /// Body rules that override the provider's for this endpoint.
    pub body: &'static [FieldRule],
}

/// What the scrubber writes at a body path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    /// A string becomes `<TEXT n>` (n = characters).
    Text,
    /// Every string in the subtree becomes `<TEXT n>`.
    TextDeep,
    /// A base64 string becomes `<BASE64 n>` (n ≈ decoded bytes).
    Base64,
    /// A string becomes `<NAME n>`.
    Opaque(&'static str),
    /// A string becomes exactly this placeholder.
    Fixed(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct ScrubRule {
    pub path: &'static str,
    pub placeholder: Placeholder,
    /// Only elements at or after this index of the first `[*]` are scrubbed (Claude keeps
    /// its two fingerprint system blocks, everything after them is caller content).
    pub from_index: usize,
}

/// Everything the harness knows about one provider.
pub struct ProviderRules {
    pub provider: Provider,
    pub info: FingerprintInfo,
    /// The official client version the tables were verified against.
    pub client_version: &'static str,
    /// Default forward target of the capture proxy.
    pub upstream: &'static str,
    /// Hosts that belong to this provider (HAR import filter, host → provider).
    pub hosts: &'static [&'static str],
    pub endpoints: &'static [Endpoint],
    pub request_headers: &'static [HeaderRule],
    pub query: &'static [HeaderRule],
    pub body: &'static [FieldRule],
    pub response_headers: &'static [HeaderRule],
    pub response_body: &'static [FieldRule],
    pub scrub: &'static [ScrubRule],
}

/// One segment of a concrete body path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// `system[1].text` for `[Key(system), Index(1), Key(text)]`; empty path → `$`.
pub fn render_path(path: &[Seg]) -> String {
    if path.is_empty() {
        return "$".to_string();
    }
    let mut out = String::new();
    for seg in path {
        match seg {
            Seg::Key(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Index(i) => out.push_str(&format!("[{i}]")),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PatSeg {
    Key(String),
    AnyKey,
    Index(usize),
    AnyIndex,
}

fn parse_pattern(pattern: &str) -> Vec<PatSeg> {
    let mut segs = Vec::new();
    for token in pattern.split('.') {
        if token.is_empty() {
            continue;
        }
        let (key, rest) = match token.find('[') {
            Some(i) => (&token[..i], &token[i..]),
            None => (token, ""),
        };
        if !key.is_empty() {
            segs.push(if key == "*" {
                PatSeg::AnyKey
            } else {
                PatSeg::Key(key.to_string())
            });
        }
        for part in rest.split('[').filter(|p| !p.is_empty()) {
            let inner = part.trim_end_matches(']');
            segs.push(if inner == "*" {
                PatSeg::AnyIndex
            } else {
                PatSeg::Index(inner.parse().unwrap_or(usize::MAX))
            });
        }
    }
    segs
}

/// How well a pattern matched: longer and more concrete patterns win.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity {
    pub len: usize,
    pub concrete: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathMatch {
    pub specificity: Specificity,
    /// The concrete index the first `[*]` matched, if any.
    pub first_any_index: Option<usize>,
}

/// Whether `pattern` matches exactly `path` (same depth; `*` / `[*]` match one key /
/// one index). Rules apply to the node they name; what happens below it is the
/// rule's business (`Exact` recurses, `Shape` / `Volatile` / `Optional` stop).
pub fn match_pattern(pattern: &str, path: &[Seg]) -> Option<PathMatch> {
    let pat = parse_pattern(pattern);
    if pat.len() != path.len() {
        return None;
    }
    let mut concrete = 0;
    let mut first_any_index = None;
    for (p, s) in pat.iter().zip(path.iter()) {
        match (p, s) {
            (PatSeg::Key(k), Seg::Key(actual)) if k == actual => concrete += 1,
            (PatSeg::AnyKey, Seg::Key(_)) => {}
            (PatSeg::Index(i), Seg::Index(actual)) if i == actual => concrete += 1,
            (PatSeg::AnyIndex, Seg::Index(actual)) => {
                if first_any_index.is_none() {
                    first_any_index = Some(*actual);
                }
            }
            _ => return None,
        }
    }
    Some(PathMatch {
        specificity: Specificity {
            len: pat.len(),
            concrete,
        },
        first_any_index,
    })
}

fn best_header_rule<'a>(rules: &'a [HeaderRule], name: &str) -> Option<&'a HeaderRule> {
    if let Some(exact) = rules.iter().find(|r| r.name == name) {
        return Some(exact);
    }
    rules
        .iter()
        .filter(|r| r.name.ends_with('*') && name.starts_with(&r.name[..r.name.len() - 1]))
        .max_by_key(|r| r.name.len())
}

fn best_field_rule<'a>(rules: &'a [FieldRule], path: &[Seg]) -> Option<&'a FieldRule> {
    rules
        .iter()
        .filter_map(|r| match_pattern(r.path, path).map(|m| (m.specificity, r)))
        .max_by_key(|(s, _)| *s)
        .map(|(_, r)| r)
}

impl ProviderRules {
    /// The endpoint a request path belongs to.
    pub fn endpoint_for(&self, method: &str, path: &str) -> Option<&'static Endpoint> {
        self.endpoints
            .iter()
            .find(|e| e.method.eq_ignore_ascii_case(method) && e.path == path)
    }

    pub fn endpoint_named(&self, name: &str) -> Option<&'static Endpoint> {
        self.endpoints.iter().find(|e| e.name == name)
    }

    /// Request header rule: the endpoint's override first, then the provider's.
    pub fn header_rule(&self, endpoint: Option<&Endpoint>, name: &str) -> Option<&HeaderRule> {
        endpoint
            .and_then(|e| best_header_rule(e.headers, name))
            .or_else(|| best_header_rule(self.request_headers, name))
    }

    pub fn query_rule(&self, name: &str) -> Option<&HeaderRule> {
        best_header_rule(self.query, name)
    }

    pub fn response_header_rule(&self, name: &str) -> Option<&HeaderRule> {
        best_header_rule(self.response_headers, name)
    }

    /// Body rule for a path: the most specific of the endpoint's and the provider's.
    pub fn body_rule(&self, endpoint: Option<&Endpoint>, path: &[Seg]) -> Option<&FieldRule> {
        let from_endpoint = endpoint.and_then(|e| best_field_rule(e.body, path));
        let from_provider = best_field_rule(self.body, path);
        match (from_endpoint, from_provider) {
            (Some(e), Some(p)) => {
                let se = match_pattern(e.path, path).map(|m| m.specificity);
                let sp = match_pattern(p.path, path).map(|m| m.specificity);
                if sp > se {
                    Some(p)
                } else {
                    Some(e)
                }
            }
            (e, p) => e.or(p),
        }
    }

    pub fn response_body_rule(&self, path: &[Seg]) -> Option<&FieldRule> {
        best_field_rule(self.response_body, path)
    }

    /// Scrub rule for a request body path, honouring `from_index`.
    pub fn scrub_rule(&self, path: &[Seg]) -> Option<&ScrubRule> {
        self.scrub
            .iter()
            .filter_map(|r| {
                let m = match_pattern(r.path, path)?;
                let applies =
                    r.from_index == 0 || m.first_any_index.is_none_or(|i| i >= r.from_index);
                applies.then_some((m.specificity, r))
            })
            .max_by_key(|(s, _)| *s)
            .map(|(_, r)| r)
    }

    pub fn is_provider_host(&self, host: &str) -> bool {
        let host = host.split(':').next().unwrap_or(host);
        self.hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
    }
}
