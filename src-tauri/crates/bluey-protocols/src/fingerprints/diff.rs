//! The diff: one capture against another (a fresh capture against the documented
//! one, or against the blessed golden), header by header and field by field, every
//! finding graded and pointed at the doc row it concerns.
//!
//! * **drift** — something the documented fingerprint does not explain: a header or
//!   field added, removed or changed outside its documented format. `fingerprints:diff`
//!   exits 1.
//! * **version** — a value changed but still matches the documented format (a client
//!   version bump): update the column in `docs/PROVIDER_ACCOUNTS.md`.
//! * **info** — a difference the rules call informational (model, optional wrappers).

use std::collections::{BTreeMap, BTreeSet};

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use super::capture::{Body, Capture, Header};
use super::rules::{render_path, Endpoint, ProviderRules, Rule, Seg};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Version,
    Drift,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub severity: Severity,
    /// `header user-agent`, `body system[1].text`, `query beta`, `response header request-id`.
    pub location: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    pub note: String,
    /// The `docs/PROVIDER_ACCOUNTS.md` row to update.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffReport {
    pub provider: String,
    pub endpoint: Option<String>,
    pub method: String,
    pub path: String,
    pub expected_label: String,
    pub actual_label: String,
    pub findings: Vec<Finding>,
}

impl DiffReport {
    pub fn has_drift(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Drift)
    }

    /// `(drift, version, info)`.
    pub fn counts(&self) -> (usize, usize, usize) {
        let count = |s| self.findings.iter().filter(|f| f.severity == s).count();
        (
            count(Severity::Drift),
            count(Severity::Version),
            count(Severity::Info),
        )
    }

    /// Human-readable report; drift first.
    pub fn render(&self) -> String {
        let mut out = format!(
            "{} — {} {}: {} vs {}\n",
            self.provider,
            self.method,
            self.endpoint
                .as_deref()
                .map(|e| format!("{} ({e})", self.path))
                .unwrap_or_else(|| self.path.clone()),
            self.actual_label,
            self.expected_label
        );
        let mut findings = self.findings.clone();
        findings.sort_by(|a, b| {
            b.severity
                .cmp(&a.severity)
                .then(a.location.cmp(&b.location))
        });
        for f in &findings {
            let tag = match f.severity {
                Severity::Drift => "DRIFT",
                Severity::Version => "VERSION",
                Severity::Info => "INFO",
            };
            out.push_str(&format!("  {tag:<8}{}: {}\n", f.location, f.note));
            if let Some(e) = &f.expected {
                out.push_str(&format!("          expected: {e}\n"));
            }
            if let Some(a) = &f.actual {
                out.push_str(&format!("          actual:   {a}\n"));
            }
            if let Some(doc) = &f.doc {
                out.push_str(&format!("          → docs/PROVIDER_ACCOUNTS.md › {doc}\n"));
            }
        }
        let (d, v, i) = self.counts();
        if d + v + i == 0 {
            out.push_str("  no differences\n");
        } else {
            out.push_str(&format!("  {d} drift · {v} version · {i} info\n"));
        }
        out
    }
}

struct Cx<'a> {
    rules: &'a ProviderRules,
    endpoint: Option<&'static Endpoint>,
    findings: Vec<Finding>,
}

impl Cx<'_> {
    fn push(
        &mut self,
        severity: Severity,
        location: String,
        expected: Option<String>,
        actual: Option<String>,
        note: impl Into<String>,
        doc: Option<&str>,
    ) {
        self.findings.push(Finding {
            severity,
            location,
            expected,
            actual,
            note: note.into(),
            doc: doc
                .filter(|d| !d.is_empty())
                .map(|d| format!("{} › {d}", self.rules.provider.display_name())),
        });
    }
}

/// `<TEXT 12>` and `<TEXT 40>` are the same placeholder.
fn normalize(s: &str) -> String {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"<([A-Z0-9_]+) \d+>").expect("placeholder regex"));
    re.replace_all(s.trim(), "<$1>").into_owned()
}

fn normalize_value(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(normalize(s)),
        Value::Array(items) => Value::Array(items.iter().map(normalize_value).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (k.clone(), normalize_value(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn short(v: &Value) -> String {
    match v {
        Value::String(s) => {
            let mut t: String = s.chars().take(100).collect();
            if s.chars().count() > 100 {
                t.push('…');
            }
            format!("{t:?}")
        }
        Value::Array(items) => format!("[… {} items]", items.len()),
        Value::Object(map) => {
            if map.len() <= 6 {
                format!("{{{}}}", map.keys().cloned().collect::<Vec<_>>().join(", "))
            } else {
                format!("{{… {} keys}}", map.len())
            }
        }
        other => other.to_string(),
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn split_set(s: &str) -> BTreeSet<String> {
    s.split(',')
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .collect()
}

fn set_of(v: &Value) -> Option<BTreeSet<String>> {
    match v {
        Value::String(s) => Some(split_set(s)),
        Value::Array(items) => items
            .iter()
            .map(|i| i.as_str().map(|s| s.to_string()))
            .collect(),
        _ => None,
    }
}

fn compare_set(
    cx: &mut Cx,
    location: &str,
    required: &[&str],
    optional: &[&str],
    expected: Option<&BTreeSet<String>>,
    actual: Option<&BTreeSet<String>>,
    doc: &str,
) {
    let Some(actual) = actual else {
        cx.push(
            Severity::Drift,
            location.to_string(),
            None,
            None,
            "missing",
            Some(doc),
        );
        return;
    };
    for r in required {
        if !actual.contains(*r) {
            cx.push(
                Severity::Drift,
                location.to_string(),
                None,
                None,
                format!("missing required member `{r}`"),
                Some(doc),
            );
        }
    }
    for m in actual {
        if !required.contains(&m.as_str()) && !optional.contains(&m.as_str()) {
            cx.push(
                Severity::Drift,
                location.to_string(),
                None,
                Some(m.clone()),
                "member not in the documented set",
                Some(doc),
            );
        }
    }
    if let Some(expected) = expected {
        for o in optional {
            let (e, a) = (expected.contains(*o), actual.contains(*o));
            if e != a {
                cx.push(
                    Severity::Info,
                    location.to_string(),
                    None,
                    None,
                    format!(
                        "optional member `{o}` {}",
                        if a { "present" } else { "absent" }
                    ),
                    Some(doc),
                );
            }
        }
    }
}

fn compare_scalar(
    cx: &mut Cx,
    rule: Rule,
    doc: &str,
    location: String,
    expected: Option<&str>,
    actual: Option<&str>,
) {
    let norm = |s: Option<&str>| s.map(normalize);
    match rule {
        Rule::Volatile => {}
        Rule::Absent => {
            if let Some(a) = actual {
                cx.push(
                    Severity::Drift,
                    location,
                    None,
                    Some(a.to_string()),
                    "must be absent",
                    Some(doc),
                );
            }
        }
        Rule::Present | Rule::Shape => {
            if actual.is_none() {
                cx.push(Severity::Drift, location, None, None, "missing", Some(doc));
            }
        }
        Rule::Optional => {
            if norm(expected) != norm(actual) {
                cx.push(
                    Severity::Info,
                    location,
                    expected.map(String::from),
                    actual.map(String::from),
                    "differs",
                    Some(doc),
                );
            }
        }
        Rule::Info => match actual {
            None => cx.push(
                Severity::Drift,
                location,
                expected.map(String::from),
                None,
                "missing",
                Some(doc),
            ),
            Some(a) => {
                if expected.is_some() && norm(expected) != norm(Some(a)) {
                    cx.push(
                        Severity::Info,
                        location,
                        expected.map(String::from),
                        Some(a.to_string()),
                        "differs",
                        Some(doc),
                    );
                }
            }
        },
        Rule::Exact | Rule::Children => match (expected, actual) {
            (None, None) => {}
            (Some(e), None) => cx.push(
                Severity::Drift,
                location,
                Some(e.to_string()),
                None,
                "missing",
                Some(doc),
            ),
            (None, Some(a)) => cx.push(
                Severity::Drift,
                location,
                None,
                Some(a.to_string()),
                "not in the documented fingerprint",
                Some(doc),
            ),
            (Some(e), Some(a)) => {
                if normalize(e) != normalize(a) {
                    cx.push(
                        Severity::Drift,
                        location,
                        Some(e.to_string()),
                        Some(a.to_string()),
                        "differs",
                        Some(doc),
                    );
                }
            }
        },
        Rule::Pattern(re) => {
            let Some(a) = actual else {
                cx.push(
                    Severity::Drift,
                    location,
                    expected.map(String::from),
                    None,
                    "missing",
                    Some(doc),
                );
                return;
            };
            let regex = Regex::new(re).expect("rule pattern compiles");
            if !regex.is_match(a) {
                cx.push(
                    Severity::Drift,
                    location,
                    expected.map(String::from),
                    Some(a.to_string()),
                    format!("does not match the documented format `{re}`"),
                    Some(doc),
                );
            } else if let Some(e) = expected {
                if normalize(e) != normalize(a) {
                    cx.push(
                        Severity::Version,
                        location,
                        Some(e.to_string()),
                        Some(a.to_string()),
                        "format unchanged — update the documented value",
                        Some(doc),
                    );
                }
            }
        }
        Rule::Set { required, optional } => {
            let e = expected.map(split_set);
            let a = actual.map(split_set);
            compare_set(
                cx,
                &location,
                required,
                optional,
                e.as_ref(),
                a.as_ref(),
                doc,
            );
        }
    }
}

fn group(headers: &[Header]) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for h in headers {
        out.entry(h.name.to_ascii_lowercase())
            .and_modify(|v| {
                v.push_str(", ");
                v.push_str(&h.value);
            })
            .or_insert_with(|| h.value.clone());
    }
    out
}

fn compare_named<'r>(
    cx: &mut Cx<'r>,
    kind: &str,
    expected: &[Header],
    actual: &[Header],
    lookup: impl Fn(&ProviderRules, Option<&Endpoint>, &str) -> Option<(Rule, &'r str)>,
    default: Rule,
) {
    let e = group(expected);
    let a = group(actual);
    let names: BTreeSet<&String> = e.keys().chain(a.keys()).collect();
    for name in names {
        let (rule, doc) = lookup(cx.rules, cx.endpoint, name).unwrap_or((default, ""));
        compare_scalar(
            cx,
            rule,
            doc,
            format!("{kind} {name}"),
            e.get(name).map(String::as_str),
            a.get(name).map(String::as_str),
        );
    }
}

fn recurse(
    cx: &mut Cx,
    path: &mut Vec<Seg>,
    expected: Option<&Value>,
    actual: Option<&Value>,
    response: bool,
    inherited: Rule,
) {
    let present = expected.or(actual);
    match present {
        Some(Value::Object(_)) => {
            let keys: BTreeSet<String> = [expected, actual]
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .flat_map(|m| m.keys().cloned())
                .collect();
            for key in keys {
                path.push(Seg::Key(key.clone()));
                compare_json(
                    cx,
                    path,
                    expected.and_then(|v| v.get(&key)),
                    actual.and_then(|v| v.get(&key)),
                    response,
                    inherited,
                );
                path.pop();
            }
        }
        Some(Value::Array(_)) => {
            let len = [expected, actual]
                .into_iter()
                .flatten()
                .filter_map(Value::as_array)
                .map(Vec::len)
                .max()
                .unwrap_or(0);
            for i in 0..len {
                path.push(Seg::Index(i));
                compare_json(
                    cx,
                    path,
                    expected.and_then(|v| v.get(i)),
                    actual.and_then(|v| v.get(i)),
                    response,
                    inherited,
                );
                path.pop();
            }
        }
        _ => {}
    }
}

fn compare_json(
    cx: &mut Cx,
    path: &mut Vec<Seg>,
    expected: Option<&Value>,
    actual: Option<&Value>,
    response: bool,
    inherited: Rule,
) {
    let explicit = if response {
        cx.rules.response_body_rule(path).map(|r| (r.rule, r.doc))
    } else {
        cx.rules
            .body_rule(cx.endpoint, path)
            .map(|r| (r.rule, r.doc))
    };
    let (rule, doc, is_explicit) = match explicit {
        Some((rule, doc)) => (rule, doc, true),
        None => (inherited, "", false),
    };
    let location = format!(
        "{}{}",
        if response { "response body " } else { "body " },
        render_path(path)
    );
    let both_containers = matches!(
        (expected, actual),
        (Some(Value::Object(_)), Some(Value::Object(_)))
            | (Some(Value::Array(_)), Some(Value::Array(_)))
    );
    match rule {
        Rule::Volatile => {}
        Rule::Absent => {
            if let Some(a) = actual {
                cx.push(
                    Severity::Drift,
                    location,
                    None,
                    Some(short(a)),
                    "must be absent",
                    Some(doc),
                );
            }
        }
        Rule::Present => {
            if actual.is_none() {
                cx.push(Severity::Drift, location, None, None, "missing", Some(doc));
            }
        }
        Rule::Optional => {
            let differs = expected.map(normalize_value) != actual.map(normalize_value);
            if differs {
                cx.push(
                    Severity::Info,
                    location,
                    expected.map(short),
                    actual.map(short),
                    "differs",
                    Some(doc),
                );
            }
        }
        Rule::Info => match actual {
            None => cx.push(
                Severity::Drift,
                location,
                expected.map(short),
                None,
                "missing",
                Some(doc),
            ),
            Some(a) => {
                if expected.is_some() && expected.map(normalize_value) != Some(normalize_value(a)) {
                    cx.push(
                        Severity::Info,
                        location,
                        expected.map(short),
                        Some(short(a)),
                        "differs",
                        Some(doc),
                    );
                }
            }
        },
        Rule::Shape => match (expected, actual) {
            (Some(e), Some(a)) => {
                if kind(e) != kind(a) {
                    cx.push(
                        Severity::Drift,
                        location,
                        Some(kind(e).into()),
                        Some(kind(a).into()),
                        "kind differs",
                        Some(doc),
                    );
                } else if let (Value::Object(eo), Value::Object(ao)) = (e, a) {
                    let ek: BTreeSet<&String> = eo.keys().collect();
                    let ak: BTreeSet<&String> = ao.keys().collect();
                    if ek != ak {
                        cx.push(
                            Severity::Info,
                            location,
                            Some(short(e)),
                            Some(short(a)),
                            "keys differ",
                            Some(doc),
                        );
                    }
                } else if let (Value::Array(ea), Value::Array(aa)) = (e, a) {
                    if ea.len() != aa.len() {
                        cx.push(
                            Severity::Info,
                            location,
                            Some(short(e)),
                            Some(short(a)),
                            "length differs",
                            Some(doc),
                        );
                    }
                }
            }
            (Some(e), None) => cx.push(
                Severity::Drift,
                location,
                Some(short(e)),
                None,
                "missing",
                Some(doc),
            ),
            (None, Some(a)) => cx.push(
                Severity::Drift,
                location,
                None,
                Some(short(a)),
                "not in the documented fingerprint",
                Some(doc),
            ),
            (None, None) => {}
        },
        Rule::Pattern(_) | Rule::Set { .. } => {
            if let Rule::Set { required, optional } = rule {
                let e = expected.and_then(set_of);
                let a = actual.and_then(set_of);
                compare_set(
                    cx,
                    &location,
                    required,
                    optional,
                    e.as_ref(),
                    a.as_ref(),
                    doc,
                );
            } else {
                fn as_str(v: Option<&Value>) -> Option<&str> {
                    v.and_then(Value::as_str)
                }
                if matches!(actual, Some(v) if v.as_str().is_none()) {
                    cx.push(
                        Severity::Drift,
                        location,
                        expected.map(short),
                        actual.map(short),
                        "must be a string",
                        Some(doc),
                    );
                } else {
                    compare_scalar(cx, rule, doc, location, as_str(expected), as_str(actual));
                }
            }
        }
        Rule::Children => {
            if is_explicit && expected.is_some() != actual.is_some() {
                cx.push(
                    Severity::Info,
                    location.clone(),
                    expected.map(short),
                    actual.map(short),
                    if actual.is_some() {
                        "present, not in the documented fingerprint"
                    } else {
                        "absent"
                    },
                    Some(doc),
                );
            }
            if both_containers || expected.is_none() || actual.is_none() {
                recurse(cx, path, expected, actual, response, Rule::Children);
            } else if let (Some(e), Some(a)) = (expected, actual) {
                if normalize_value(e) != normalize_value(a) {
                    cx.push(
                        Severity::Info,
                        location,
                        Some(short(e)),
                        Some(short(a)),
                        "differs",
                        Some(doc),
                    );
                }
            }
        }
        Rule::Exact => match (expected, actual) {
            (None, None) => {}
            (Some(e), None) => cx.push(
                Severity::Drift,
                location,
                Some(short(e)),
                None,
                "missing",
                Some(doc),
            ),
            (None, Some(a)) => cx.push(
                Severity::Drift,
                location,
                None,
                Some(short(a)),
                "not in the documented fingerprint",
                Some(doc),
            ),
            (Some(e), Some(a)) => {
                if both_containers {
                    recurse(cx, path, expected, actual, response, Rule::Exact);
                } else if kind(e) != kind(a) {
                    cx.push(
                        Severity::Drift,
                        location,
                        Some(short(e)),
                        Some(short(a)),
                        "kind differs",
                        Some(doc),
                    );
                } else if normalize_value(e) != normalize_value(a) {
                    cx.push(
                        Severity::Drift,
                        location,
                        Some(short(e)),
                        Some(short(a)),
                        "differs",
                        Some(doc),
                    );
                }
            }
        },
    }
}

fn compare_body(cx: &mut Cx, expected: &Body, actual: &Body, response: bool) {
    let location = if response { "response body" } else { "body" };
    match (expected.as_json(), actual.as_json()) {
        (Some(e), Some(a)) => {
            let inherited = if response {
                Rule::Children
            } else {
                Rule::Exact
            };
            compare_json(cx, &mut Vec::new(), Some(&e), Some(&a), response, inherited);
        }
        _ => {
            let severity = if response {
                Severity::Info
            } else {
                Severity::Drift
            };
            if expected.kind_name() != actual.kind_name() {
                cx.push(
                    severity,
                    location.to_string(),
                    Some(expected.kind_name().into()),
                    Some(actual.kind_name().into()),
                    "body kind differs",
                    None,
                );
            } else if let (Body::Text { text: e }, Body::Text { text: a }) = (expected, actual) {
                if normalize(e) != normalize(a) {
                    cx.push(
                        severity,
                        location.to_string(),
                        Some(short(&Value::String(e.clone()))),
                        Some(short(&Value::String(a.clone()))),
                        "differs",
                        None,
                    );
                }
            }
        }
    }
}

fn pairs_as_headers(pairs: Vec<(String, String)>) -> Vec<Header> {
    pairs
        .into_iter()
        .map(|(k, v)| Header { name: k, value: v })
        .collect()
}

/// Compare `actual` against `expected` under the provider's rules.
pub fn diff(
    rules: &ProviderRules,
    expected: &Capture,
    actual: &Capture,
    expected_label: &str,
    actual_label: &str,
) -> DiffReport {
    let path = actual.request.path();
    let endpoint = rules.endpoint_for(&actual.request.method, &path);
    let mut cx = Cx {
        rules,
        endpoint,
        findings: Vec::new(),
    };
    let report = |cx: Cx| DiffReport {
        provider: rules.provider.id().to_string(),
        endpoint: endpoint.map(|e| e.name.to_string()),
        method: actual.request.method.clone(),
        path: path.clone(),
        expected_label: expected_label.to_string(),
        actual_label: actual_label.to_string(),
        findings: cx.findings,
    };

    if !expected
        .request
        .method
        .eq_ignore_ascii_case(&actual.request.method)
    {
        cx.push(
            Severity::Drift,
            "method".into(),
            Some(expected.request.method.clone()),
            Some(actual.request.method.clone()),
            "differs",
            None,
        );
    }
    let expected_path = expected.request.path();
    if expected_path != path {
        cx.push(
            Severity::Drift,
            "path".into(),
            Some(expected_path),
            Some(path.clone()),
            "different endpoint — nothing else compared",
            None,
        );
        return report(cx);
    }
    if expected.request.host() != actual.request.host() {
        cx.push(
            Severity::Info,
            "host".into(),
            expected.request.host(),
            actual.request.host(),
            "differs",
            None,
        );
    }
    compare_named(
        &mut cx,
        "query",
        &pairs_as_headers(expected.request.query_pairs()),
        &pairs_as_headers(actual.request.query_pairs()),
        |r, _, name| r.query_rule(name).map(|h| (h.rule, h.doc)),
        Rule::Exact,
    );
    compare_named(
        &mut cx,
        "header",
        &expected.request.headers,
        &actual.request.headers,
        |r, endpoint, name| r.header_rule(endpoint, name).map(|h| (h.rule, h.doc)),
        Rule::Exact,
    );
    compare_body(&mut cx, &expected.request.body, &actual.request.body, false);

    match (&expected.response, &actual.response) {
        (Some(e), Some(a)) => {
            if e.status != a.status {
                cx.push(
                    Severity::Info,
                    "response status".into(),
                    Some(e.status.to_string()),
                    Some(a.status.to_string()),
                    "differs",
                    None,
                );
            }
            compare_named(
                &mut cx,
                "response header",
                &e.headers,
                &a.headers,
                |r, _, name| r.response_header_rule(name).map(|h| (h.rule, h.doc)),
                Rule::Volatile,
            );
            compare_body(&mut cx, &e.body, &a.body, true);
        }
        (Some(_), None) => cx.push(
            Severity::Info,
            "response".into(),
            None,
            None,
            "no response recorded",
            None,
        ),
        _ => {}
    }
    report(cx)
}
