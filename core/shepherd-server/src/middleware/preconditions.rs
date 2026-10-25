use std::sync::LazyLock;

use axum::http::Method;
use serde_json::Value;

/// Required command headers for one operation, derived from the served spec so
/// the table cannot drift from the contract (plan/07: missing header -> 428).
pub(crate) struct Precondition {
    method: Method,
    segments: Vec<Segment>,
    pub(crate) idempotency_key: bool,
    pub(crate) if_match: bool,
    pub(crate) lease: bool,
}

enum Segment {
    Literal(String),
    Param,
}

pub(crate) struct PreconditionTable {
    entries: Vec<Precondition>,
}

impl PreconditionTable {
    fn from_spec(spec: &Value) -> Self {
        let mut entries = Vec::new();
        let paths = spec["paths"].as_object().expect("spec has paths");
        for (path, item) in paths {
            for (method, operation) in item.as_object().expect("path item is an object") {
                let method = method.to_uppercase().parse().expect("spec method is valid");
                let mut entry = Precondition {
                    method,
                    segments: segments(path),
                    idempotency_key: false,
                    if_match: false,
                    lease: false,
                };
                for parameter in operation["parameters"].as_array().into_iter().flatten() {
                    if parameter["in"] != "header" || parameter["required"] != true {
                        continue;
                    }
                    match parameter["name"].as_str().unwrap_or("") {
                        "Idempotency-Key" => entry.idempotency_key = true,
                        "If-Match" => entry.if_match = true,
                        "X-Lease-Token" => entry.lease = true,
                        _ => {}
                    }
                }
                if entry.idempotency_key || entry.if_match || entry.lease {
                    entries.push(entry);
                }
            }
        }
        Self { entries }
    }

    pub(crate) fn lookup(&self, method: &Method, path: &str) -> Option<&Precondition> {
        let request: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        self.entries.iter().find(|entry| {
            entry.method == *method
                && entry.segments.len() == request.len()
                && entry
                    .segments
                    .iter()
                    .zip(&request)
                    .all(|(seg, got)| match seg {
                        Segment::Literal(expected) => expected == got,
                        Segment::Param => !got.is_empty(),
                    })
        })
    }
}

fn segments(path: &str) -> Vec<Segment> {
    path.split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            if segment.starts_with('{') {
                Segment::Param
            } else {
                Segment::Literal(segment.to_string())
            }
        })
        .collect()
}

pub(crate) static PRECONDITIONS: LazyLock<PreconditionTable> = LazyLock::new(|| {
    let spec: Value =
        serde_json::from_str(crate::OPENAPI_SPEC).expect("embedded spec is JSON-syntax YAML");
    PreconditionTable::from_spec(&spec)
});

#[cfg(test)]
mod tests {
    use super::*;

    // The catalog's booleans mean "operation uses this header"; the gate only
    // pre-checks headers the spec marks required (reviewSubmission's lease is
    // optional: human reviewers hold no lease).
    #[test]
    fn precondition_table_matches_the_operations_catalog() {
        let catalog: Value =
            serde_json::from_str(include_str!("../../../../plan/contracts/operations.json"))
                .expect("operations catalog parses");
        let deferred = ["exportProject", "importProject"];
        let mut checked = 0;
        for op in catalog.as_array().unwrap() {
            let id = op["operation_id"].as_str().unwrap();
            if deferred.contains(&id) {
                continue;
            }
            let method: Method = op["method"]
                .as_str()
                .unwrap()
                .to_uppercase()
                .parse()
                .unwrap();
            let path = op["path"].as_str().unwrap();
            let entry = PRECONDITIONS.lookup(&method, &sample_path(path));
            let idempotency = entry.is_some_and(|e| e.idempotency_key);
            let if_match = entry.is_some_and(|e| e.if_match);
            let lease = entry.is_some_and(|e| e.lease);
            assert_eq!(idempotency, op["idempotency"] == true, "{id} idempotency");
            assert_eq!(if_match, op["if_match"] == true, "{id} if_match");
            if lease {
                assert_eq!(op["lease"], true, "{id} lease required but not cataloged");
            }
            if op["lease"] == true {
                assert_eq!(lease, op["role"] == "claimant", "{id} lease requiredness");
            }
            checked += 1;
        }
        assert_eq!(checked, 68, "all served operations cross-checked");
    }

    #[test]
    fn lookup_matches_templated_paths_and_misses_unknown_ones() {
        let path = "/api/v1/projects/8c0e7f6a-0000-7000-8000-000000000000";
        let entry = PRECONDITIONS.lookup(&Method::PATCH, path).unwrap();
        assert!(entry.idempotency_key && entry.if_match && !entry.lease);
        assert!(PRECONDITIONS.lookup(&Method::GET, path).is_none());
        assert!(
            PRECONDITIONS
                .lookup(&Method::POST, "/api/v1/unknown")
                .is_none()
        );
    }

    fn sample_path(template: &str) -> String {
        template
            .split('/')
            .map(|seg| {
                if seg.starts_with('{') {
                    "00000000-0000-7000-8000-000000000000"
                } else {
                    seg
                }
            })
            .collect::<Vec<_>>()
            .join("/")
    }
}
