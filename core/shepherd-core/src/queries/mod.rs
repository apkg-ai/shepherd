pub(crate) mod hierarchy;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Sqlite};
use uuid::Uuid;

use crate::error::DomainError;
use crate::model::ProjectId;
use crate::storage::rows::{format_ts, parse_ts};

#[derive(Debug, Clone, Default)]
pub struct ListParams {
    pub limit: Option<u32>,
    pub cursor: Option<String>,
    pub include_archived: bool,
}

#[derive(Debug)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

pub(crate) fn effective_limit(params: &ListParams) -> Result<i64, DomainError> {
    match params.limit {
        None => Ok(50),
        Some(value) if (1..=200).contains(&value) => Ok(i64::from(value)),
        Some(_) => Err(DomainError::Validation {
            field: "limit",
            message: "must be between 1 and 200".into(),
        }),
    }
}

// Normalized filter fingerprints: a cursor is only valid with the filters that produced it.
pub(crate) fn projects_filter(include_archived: bool) -> String {
    format!("include_archived={include_archived}")
}

pub(crate) fn scoped_filter(project: &ProjectId, include_archived: bool) -> String {
    format!("project={project}&include_archived={include_archived}")
}

fn decode_after(
    params: &ListParams,
    endpoint: &str,
    filter: &str,
) -> Result<Option<(String, String)>, DomainError> {
    params
        .cursor
        .as_deref()
        .map(|cursor| decode_cursor(cursor, endpoint, filter))
        .transpose()
}

// Shared keyset-pagination clauses: created_at ASC, id ASC across every list endpoint.
fn push_page_clauses(
    builder: &mut QueryBuilder<Sqlite>,
    scope: Option<&ProjectId>,
    include_archived: bool,
    after: &Option<(String, String)>,
    limit: i64,
) {
    let mut prefix = " WHERE ";
    if let Some(project) = scope {
        builder
            .push(prefix)
            .push("project_id = ")
            .push_bind(project.to_string());
        prefix = " AND ";
    }
    if !include_archived {
        builder.push(prefix).push("archived = 0");
        prefix = " AND ";
    }
    if let Some((created_at, id)) = after {
        builder
            .push(prefix)
            .push("(created_at > ")
            .push_bind(created_at.clone())
            .push(" OR (created_at = ")
            .push_bind(created_at.clone())
            .push(" AND id > ")
            .push_bind(id.clone())
            .push("))");
    }
    // Fetch one extra row to learn whether a next page exists.
    builder
        .push(" ORDER BY created_at ASC, id ASC LIMIT ")
        .push_bind(limit + 1);
}

fn split_page<T>(mut items: Vec<T>, limit: i64, encode: impl Fn(&T) -> String) -> Page<T> {
    let limit = limit as usize;
    if items.len() > limit {
        items.truncate(limit);
        let cursor = encode(items.last().expect("page limit is at least one"));
        Page {
            items,
            next_cursor: Some(cursor),
        }
    } else {
        Page {
            items,
            next_cursor: None,
        }
    }
}

// base64url JSON of sort tuple, endpoint and filter fingerprint (plan/07).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CursorPayload {
    v: u8,
    e: String,
    f: String,
    c: String,
    i: String,
}

pub(crate) fn encode_cursor(endpoint: &str, filter: &str, created_at: &str, id: &str) -> String {
    let payload = CursorPayload {
        v: 1,
        e: endpoint.to_string(),
        f: filter.to_string(),
        c: created_at.to_string(),
        i: id.to_string(),
    };
    // A struct of plain strings cannot fail to serialize.
    let json = serde_json::to_vec(&payload).expect("cursor payload serializes");
    URL_SAFE_NO_PAD.encode(json)
}

pub(crate) fn decode_cursor(
    cursor: &str,
    endpoint: &str,
    filter: &str,
) -> Result<(String, String), DomainError> {
    let invalid = |reason: &str| DomainError::InvalidCursor(reason.to_string());
    let bytes = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| invalid("malformed base64url"))?;
    let payload: CursorPayload =
        serde_json::from_slice(&bytes).map_err(|_| invalid("malformed payload"))?;
    if payload.v != 1 {
        return Err(invalid("unsupported version"));
    }
    if payload.e != endpoint {
        return Err(invalid("endpoint mismatch"));
    }
    if payload.f != filter {
        return Err(invalid("filter mismatch"));
    }
    let created_at =
        parse_ts("cursor.c", &payload.c).map_err(|_| invalid("malformed sort timestamp"))?;
    let id = Uuid::parse_str(&payload.i).map_err(|_| invalid("malformed sort id"))?;
    Ok((format_ts(&created_at), id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip_their_sort_tuple() {
        let id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let filter = projects_filter(false);
        let cursor = encode_cursor("listProjects", &filter, "2026-09-14T00:00:00.000Z", &id);
        let (created_at, decoded_id) = decode_cursor(&cursor, "listProjects", &filter).unwrap();
        assert_eq!(created_at, "2026-09-14T00:00:00.000Z");
        assert_eq!(decoded_id, id);
    }

    #[test]
    fn cursors_reject_endpoint_and_filter_mismatches() {
        let id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let cursor = encode_cursor(
            "listProjects",
            &projects_filter(false),
            "2026-09-14T00:00:00.000Z",
            &id,
        );
        assert!(matches!(
            decode_cursor(&cursor, "listGoals", &projects_filter(false)),
            Err(DomainError::InvalidCursor(_))
        ));
        assert!(matches!(
            decode_cursor(&cursor, "listProjects", &projects_filter(true)),
            Err(DomainError::InvalidCursor(_))
        ));
    }

    #[test]
    fn tampered_cursors_are_invalid() {
        let id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
        let filter = projects_filter(false);
        let cursor = encode_cursor("listProjects", &filter, "2026-09-14T00:00:00.000Z", &id);
        for tampered in [
            "not base64url!".to_string(),
            URL_SAFE_NO_PAD.encode(b"{\"v\":1}"),
            URL_SAFE_NO_PAD.encode(b"plain text"),
            format!("{cursor}=="),
            cursor.chars().rev().collect::<String>(),
        ] {
            assert!(matches!(
                decode_cursor(&tampered, "listProjects", &filter),
                Err(DomainError::InvalidCursor(_))
            ));
        }
    }

    #[test]
    fn split_page_truncates_and_encodes_the_last_returned_item() {
        let page = split_page(vec![1, 2, 3, 4], 3, |n| format!("cursor-{n}"));
        assert_eq!(page.items, vec![1, 2, 3]);
        assert_eq!(page.next_cursor.as_deref(), Some("cursor-3"));
    }

    #[test]
    fn split_page_omits_next_cursor_when_exhausted() {
        let page = split_page(vec![1, 2, 3], 3, |n| format!("cursor-{n}"));
        assert_eq!(page.items, vec![1, 2, 3]);
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn limits_default_and_reject_out_of_range_values() {
        let params = |limit| ListParams {
            limit,
            cursor: None,
            include_archived: false,
        };
        assert_eq!(effective_limit(&params(None)).unwrap(), 50);
        assert_eq!(effective_limit(&params(Some(1))).unwrap(), 1);
        assert_eq!(effective_limit(&params(Some(200))).unwrap(), 200);
        for out_of_range in [0, 201] {
            assert!(matches!(
                effective_limit(&params(Some(out_of_range))),
                Err(DomainError::Validation { field: "limit", .. })
            ));
        }
    }
}
