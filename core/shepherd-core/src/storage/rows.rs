use chrono::{DateTime, SecondsFormat, Utc};
use sqlx::{Executor, Row, Sqlite};
use uuid::Uuid;

use super::StorageError;

pub fn format_ts(value: &DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Millis, true)
}

pub fn parse_ts(column: &str, value: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|parsed| parsed.with_timezone(&Utc))
        .map_err(|err| StorageError::Corrupt(format!("{column}: {value:?}: {err}")))
}

pub fn parse_uuid(column: &str, value: &str) -> Result<Uuid, StorageError> {
    Uuid::parse_str(value)
        .map_err(|err| StorageError::Corrupt(format!("{column}: {value:?}: {err}")))
}

pub fn parse_flag(column: &str, value: i64) -> Result<bool, StorageError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(StorageError::Corrupt(format!("{column}: {other}"))),
    }
}

pub struct SchemaMetaRow {
    pub version: i64,
    pub export_version: String,
}

pub async fn read_schema_meta<'e, E>(executor: E) -> Result<SchemaMetaRow, StorageError>
where
    E: Executor<'e, Database = Sqlite>,
{
    let row = sqlx::query("SELECT version, export_version FROM schema_meta")
        .fetch_one(executor)
        .await?;
    Ok(SchemaMetaRow {
        version: row.try_get("version")?,
        export_version: row.try_get("export_version")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_format_fixed_width_and_round_trip() {
        let now: DateTime<Utc> = "2026-09-14T00:00:01.020Z".parse().unwrap();
        let text = format_ts(&now);
        assert_eq!(text, "2026-09-14T00:00:01.020Z");
        assert_eq!(text.len(), 24);
        assert_eq!(parse_ts("created_at", &text).unwrap(), now);
    }

    #[test]
    fn fixed_width_timestamps_sort_lexicographically() {
        let earlier = format_ts(&"2026-09-14T00:00:09.999Z".parse().unwrap());
        let later = format_ts(&"2026-09-14T00:00:10.000Z".parse().unwrap());
        assert!(earlier < later);
    }

    #[test]
    fn malformed_stored_values_are_corrupt_errors() {
        assert!(matches!(
            parse_ts("created_at", "2026-02-30T00:00:00Z"),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            parse_ts("created_at", "not-a-time"),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            parse_uuid("id", "not-a-uuid"),
            Err(StorageError::Corrupt(_))
        ));
        assert!(matches!(
            parse_flag("revoked", 2),
            Err(StorageError::Corrupt(_))
        ));
    }

    #[test]
    fn flags_parse_stored_integers() {
        assert!(!parse_flag("revoked", 0).unwrap());
        assert!(parse_flag("revoked", 1).unwrap());
    }

    #[test]
    fn uuids_round_trip_through_text() {
        let id = Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext));
        assert_eq!(parse_uuid("id", &id.to_string()).unwrap(), id);
    }
}
