//! Offline request-schema validators and normalized public rejections.
#![allow(dead_code)]
use super::errors::{InvalidParameter, ProblemDetails, RequestValidationRejection};
const VALIDATION_SCHEMA: &str = "{\"$defs\":{\"components\":{\"component_ApprovalRequest\":{\"additionalProperties\":false,\"description\":\"Request body for approving a proposed task.\",\"properties\":{\"comment\":{\"description\":\"Optional comment explaining the approval.\",\"maxLength\":2000,\"minLength\":1,\"pattern\":\"^[\\\\s\\\\S]+$\",\"type\":\"string\"}},\"type\":\"object\"},\"component_BlockRequest\":{\"additionalProperties\":false,\"description\":\"Request body for blocking a task.\",\"properties\":{\"reason\":{\"description\":\"Explanation for why the task is blocked.\",\"maxLength\":2000,\"minLength\":1,\"pattern\":\"^[\\\\s\\\\S]+$\",\"type\":\"string\"}},\"required\":[\"reason\"],\"type\":\"object\"},\"component_Identity\":{\"additionalProperties\":false,\"description\":\"Self-declared caller descriptor. Not authentication — callers identify themselves on claim and report calls. The harness (e.g. Claude Code, Cursor), the agent/model (e.g. Opus 5), a session identifier, and an optional label. Stored on sessions and claims; humans get an identity too.\",\"properties\":{\"agent_model\":{\"description\":\"The agent or model performing the work (e.g. opus-5, human).\",\"maxLength\":100,\"minLength\":1,\"pattern\":\"^[\\\\x20-\\\\x7E]+$\",\"type\":\"string\"},\"harness\":{\"description\":\"The agent harness or tool making the call (e.g. claude-code, cursor, custom-script).\",\"maxLength\":100,\"minLength\":1,\"pattern\":\"^[\\\\x20-\\\\x7E]+$\",\"type\":\"string\"},\"label\":{\"description\":\"Optional human-readable label for this session (e.g. \\\"S3 domain core\\\", \\\"bug fix #42\\\").\",\"maxLength\":200,\"minLength\":1,\"pattern\":\"^[\\\\x20-\\\\x7E]+$\",\"type\":\"string\"},\"session_id\":{\"description\":\"Caller-generated session identifier for correlating work across multiple API calls within a single work session.\",\"maxLength\":200,\"minLength\":1,\"pattern\":\"^[\\\\x20-\\\\x7E]+$\",\"type\":\"string\"}},\"required\":[\"harness\",\"agent_model\",\"session_id\"],\"type\":\"object\"},\"component_NullableIdentity\":{\"description\":\"Currently assigned identity, or null if unassigned.\",\"oneOf\":[{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_Identity\"},{\"type\":\"null\"}]},\"component_ProjectCreate\":{\"additionalProperties\":false,\"description\":\"Request body for creating a project.\",\"properties\":{\"description\":{\"description\":\"Project description.\",\"maxLength\":5000,\"minLength\":0,\"pattern\":\"^[\\\\s\\\\S]*$\",\"type\":\"string\"},\"name\":{\"description\":\"Human-readable project name.\",\"maxLength\":200,\"minLength\":1,\"pattern\":\"^\\\\S[\\\\s\\\\S]*$\",\"type\":\"string\"},\"settings\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_ProjectSettings\"}},\"required\":[\"name\"],\"type\":\"object\"},\"component_ProjectSettings\":{\"additionalProperties\":false,\"description\":\"Project-level settings. The review gate controls whether agent- completed tasks require human approval.\",\"properties\":{\"review_gate\":{\"description\":\"When true (default), tasks transition from `in_progress` to `in_review` on successful session. When false, they go directly to `done`.\",\"type\":\"boolean\"}},\"required\":[\"review_gate\"],\"type\":\"object\"},\"component_ProjectUpdate\":{\"additionalProperties\":false,\"description\":\"Request body for partially updating a project. Only provided fields are changed.\",\"properties\":{\"description\":{\"description\":\"Project description.\",\"maxLength\":5000,\"minLength\":0,\"pattern\":\"^[\\\\s\\\\S]*$\",\"type\":\"string\"},\"name\":{\"description\":\"Human-readable project name.\",\"maxLength\":200,\"minLength\":1,\"pattern\":\"^\\\\S[\\\\s\\\\S]*$\",\"type\":\"string\"},\"settings\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_ProjectSettings\"}},\"type\":\"object\"},\"component_RejectionRequest\":{\"additionalProperties\":false,\"description\":\"Request body for rejecting a task in review.\",\"properties\":{\"reason\":{\"description\":\"Explanation for the rejection.\",\"maxLength\":2000,\"minLength\":1,\"pattern\":\"^[\\\\s\\\\S]+$\",\"type\":\"string\"}},\"required\":[\"reason\"],\"type\":\"object\"},\"component_RelationCreate\":{\"additionalProperties\":false,\"description\":\"Request body for creating a relation. The source task is the task in the URL path; only the target and relation type are specified.\",\"properties\":{\"target_task_id\":{\"description\":\"The target task for this relation.\",\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"type\":{\"description\":\"Relation type.\",\"enum\":[\"decomposition\",\"depends_on\"],\"maxLength\":13,\"minLength\":10,\"type\":\"string\"}},\"required\":[\"type\",\"target_task_id\"],\"type\":\"object\"},\"component_TaskCreate\":{\"additionalProperties\":false,\"description\":\"Request body for creating a task.\",\"properties\":{\"assignee\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_Identity\"},\"description\":{\"description\":\"Detailed task description.\",\"maxLength\":10000,\"minLength\":0,\"pattern\":\"^[\\\\s\\\\S]*$\",\"type\":\"string\"},\"graph_role\":{\"description\":\"Optional graph positioning hints.\",\"items\":{\"enum\":[\"start\",\"end\",\"milestone\"],\"maxLength\":9,\"minLength\":3,\"type\":\"string\"},\"maxItems\":3,\"minItems\":0,\"type\":\"array\"},\"metadata\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_TaskMetadata\"},\"status\":{\"description\":\"Initial status. `proposed` requires human approval; `approved` proceeds to `ready` when dependencies are met. Defaults to `proposed`.\",\"enum\":[\"proposed\",\"approved\"],\"maxLength\":8,\"minLength\":8,\"type\":\"string\"},\"title\":{\"description\":\"Short task title.\",\"maxLength\":500,\"minLength\":1,\"pattern\":\"^\\\\S[\\\\s\\\\S]*$\",\"type\":\"string\"},\"type\":{\"description\":\"Task type.\",\"enum\":[\"code\",\"question\",\"refactor\",\"review\",\"research\"],\"maxLength\":8,\"minLength\":4,\"type\":\"string\"}},\"required\":[\"title\",\"type\"],\"type\":\"object\"},\"component_TaskMetadata\":{\"additionalProperties\":true,\"description\":\"Freeform structured metadata. No schema enforcement in v1 — per-type schemas are a post-v1 follow-up (F7).\",\"maxProperties\":200,\"type\":\"object\"},\"component_TaskUpdate\":{\"additionalProperties\":false,\"description\":\"Request body for partially updating a task. Only provided fields are changed. Status transitions use dedicated action endpoints.\",\"properties\":{\"assignee\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_NullableIdentity\"},\"description\":{\"description\":\"Detailed task description.\",\"maxLength\":10000,\"minLength\":0,\"pattern\":\"^[\\\\s\\\\S]*$\",\"type\":\"string\"},\"graph_role\":{\"description\":\"Optional graph positioning hints.\",\"items\":{\"enum\":[\"start\",\"end\",\"milestone\"],\"maxLength\":9,\"minLength\":3,\"type\":\"string\"},\"maxItems\":3,\"minItems\":0,\"type\":\"array\"},\"metadata\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_TaskMetadata\"},\"title\":{\"description\":\"Short task title.\",\"maxLength\":500,\"minLength\":1,\"pattern\":\"^\\\\S[\\\\s\\\\S]*$\",\"type\":\"string\"},\"type\":{\"description\":\"Task type.\",\"enum\":[\"code\",\"question\",\"refactor\",\"review\",\"research\"],\"maxLength\":8,\"minLength\":4,\"type\":\"string\"}},\"type\":\"object\"}},\"targets\":{\"v0\":{\"maxLength\":256,\"minLength\":1,\"pattern\":\"^[a-zA-Z0-9_=-]+$\",\"type\":\"string\"},\"v1\":{\"default\":25,\"format\":\"int32\",\"maximum\":100,\"minimum\":1,\"type\":\"integer\"},\"v10\":{\"enum\":[\"code\",\"question\",\"refactor\",\"review\",\"research\"],\"maxLength\":8,\"minLength\":4,\"type\":\"string\"},\"v11\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v12\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_TaskCreate\"},\"v13\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v14\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v15\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v16\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_TaskUpdate\"},\"v17\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v18\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v19\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v2\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_ProjectCreate\"},\"v20\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v21\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_ApprovalRequest\"},\"v22\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v23\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v24\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_RejectionRequest\"},\"v25\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v26\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v27\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_BlockRequest\"},\"v28\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v29\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v3\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v30\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v31\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v32\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v33\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v34\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v35\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v36\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v37\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_RelationCreate\"},\"v38\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v39\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v4\":{\"$ref\":\"urn:openapi-to-rust:request-validation#/$defs/components/component_ProjectUpdate\"},\"v40\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v41\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v42\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v5\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v6\":{\"format\":\"uuid\",\"maxLength\":36,\"minLength\":36,\"pattern\":\"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$\",\"type\":\"string\"},\"v7\":{\"maxLength\":256,\"minLength\":1,\"pattern\":\"^[a-zA-Z0-9_=-]+$\",\"type\":\"string\"},\"v8\":{\"default\":25,\"format\":\"int32\",\"maximum\":100,\"minimum\":1,\"type\":\"integer\"},\"v9\":{\"enum\":[\"proposed\",\"approved\",\"ready\",\"in_progress\",\"in_review\",\"done\",\"blocked\",\"cancelled\"],\"maxLength\":11,\"minLength\":4,\"type\":\"string\"}}},\"$id\":\"urn:openapi-to-rust:request-validation\",\"$schema\":\"https://json-schema.org/draft/2020-12/schema\"}";
const MAX_VALIDATION_ERRORS: usize = 16usize;
pub(crate) const VALIDATION_TARGET_0_QUERY_0: &str = "#/$defs/targets/v0";
pub(crate) const VALIDATION_TARGET_1_QUERY_1: &str = "#/$defs/targets/v1";
pub(crate) const VALIDATION_TARGET_2_BODY: &str = "#/$defs/targets/v2";
pub(crate) const VALIDATION_TARGET_3_PATH_0: &str = "#/$defs/targets/v3";
pub(crate) const VALIDATION_TARGET_4_BODY: &str = "#/$defs/targets/v4";
pub(crate) const VALIDATION_TARGET_5_PATH_0: &str = "#/$defs/targets/v5";
pub(crate) const VALIDATION_TARGET_6_PATH_0: &str = "#/$defs/targets/v6";
pub(crate) const VALIDATION_TARGET_7_QUERY_0: &str = "#/$defs/targets/v7";
pub(crate) const VALIDATION_TARGET_8_QUERY_1: &str = "#/$defs/targets/v8";
pub(crate) const VALIDATION_TARGET_9_QUERY_2: &str = "#/$defs/targets/v9";
pub(crate) const VALIDATION_TARGET_10_QUERY_3: &str = "#/$defs/targets/v10";
pub(crate) const VALIDATION_TARGET_11_PATH_4: &str = "#/$defs/targets/v11";
pub(crate) const VALIDATION_TARGET_12_BODY: &str = "#/$defs/targets/v12";
pub(crate) const VALIDATION_TARGET_13_PATH_0: &str = "#/$defs/targets/v13";
pub(crate) const VALIDATION_TARGET_14_PATH_0: &str = "#/$defs/targets/v14";
pub(crate) const VALIDATION_TARGET_15_PATH_1: &str = "#/$defs/targets/v15";
pub(crate) const VALIDATION_TARGET_16_BODY: &str = "#/$defs/targets/v16";
pub(crate) const VALIDATION_TARGET_17_PATH_0: &str = "#/$defs/targets/v17";
pub(crate) const VALIDATION_TARGET_18_PATH_1: &str = "#/$defs/targets/v18";
pub(crate) const VALIDATION_TARGET_19_PATH_0: &str = "#/$defs/targets/v19";
pub(crate) const VALIDATION_TARGET_20_PATH_1: &str = "#/$defs/targets/v20";
pub(crate) const VALIDATION_TARGET_21_BODY: &str = "#/$defs/targets/v21";
pub(crate) const VALIDATION_TARGET_22_PATH_0: &str = "#/$defs/targets/v22";
pub(crate) const VALIDATION_TARGET_23_PATH_1: &str = "#/$defs/targets/v23";
pub(crate) const VALIDATION_TARGET_24_BODY: &str = "#/$defs/targets/v24";
pub(crate) const VALIDATION_TARGET_25_PATH_0: &str = "#/$defs/targets/v25";
pub(crate) const VALIDATION_TARGET_26_PATH_1: &str = "#/$defs/targets/v26";
pub(crate) const VALIDATION_TARGET_27_BODY: &str = "#/$defs/targets/v27";
pub(crate) const VALIDATION_TARGET_28_PATH_0: &str = "#/$defs/targets/v28";
pub(crate) const VALIDATION_TARGET_29_PATH_1: &str = "#/$defs/targets/v29";
pub(crate) const VALIDATION_TARGET_30_PATH_0: &str = "#/$defs/targets/v30";
pub(crate) const VALIDATION_TARGET_31_PATH_1: &str = "#/$defs/targets/v31";
pub(crate) const VALIDATION_TARGET_32_PATH_0: &str = "#/$defs/targets/v32";
pub(crate) const VALIDATION_TARGET_33_PATH_1: &str = "#/$defs/targets/v33";
pub(crate) const VALIDATION_TARGET_34_PATH_0: &str = "#/$defs/targets/v34";
pub(crate) const VALIDATION_TARGET_35_PATH_0: &str = "#/$defs/targets/v35";
pub(crate) const VALIDATION_TARGET_36_PATH_1: &str = "#/$defs/targets/v36";
pub(crate) const VALIDATION_TARGET_37_BODY: &str = "#/$defs/targets/v37";
pub(crate) const VALIDATION_TARGET_38_PATH_0: &str = "#/$defs/targets/v38";
pub(crate) const VALIDATION_TARGET_39_PATH_1: &str = "#/$defs/targets/v39";
pub(crate) const VALIDATION_TARGET_40_PATH_0: &str = "#/$defs/targets/v40";
pub(crate) const VALIDATION_TARGET_41_PATH_1: &str = "#/$defs/targets/v41";
pub(crate) const VALIDATION_TARGET_42_PATH_2: &str = "#/$defs/targets/v42";
static VALIDATORS: ::std::sync::LazyLock<::std::result::Result<::jsonschema::ValidatorMap, ()>> =
    ::std::sync::LazyLock::new(|| {
        let schema: ::serde_json::Value =
            ::serde_json::from_str(VALIDATION_SCHEMA).map_err(|_| ())?;
        ::jsonschema::options()
            .with_draft(::jsonschema::Draft::Draft202012)
            .should_validate_formats(true)
            .with_pattern_options(::jsonschema::PatternOptions::regex())
            .build_map(&schema)
            .map_err(|_| ())
    });
pub(crate) async fn decode_json_body<T>(
    request: ::axum::extract::Request,
    target: &str,
    expected_media_type: &str,
    required: bool,
    max_body_bytes: usize,
) -> ::std::result::Result<::std::option::Option<T>, RequestValidationRejection>
where
    T: ::serde::de::DeserializeOwned,
{
    let (parts, body) = request.into_parts();
    let content_type = parts
        .headers
        .get(::axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());
    let is_json = content_type.is_some_and(|value| media_type_is(value, expected_media_type));
    if !is_json {
        if content_type.is_none() && !required {
            let bytes = read_body(body, max_body_bytes).await?;
            if bytes.is_empty() {
                return Ok(None);
            }
        }
        return Err(unsupported_media_type());
    }
    let bytes = read_body(body, max_body_bytes).await?;
    if bytes.is_empty() {
        return if required {
            Err(malformed_request())
        } else {
            Ok(None)
        };
    }
    let instance: ::serde_json::Value =
        ::serde_json::from_slice(&bytes).map_err(|_| malformed_request())?;
    validate(target, "/body", &instance)?;
    let typed = ::serde_json::from_value(instance).map_err(|_| generated_contract_error())?;
    Ok(Some(typed))
}
pub(crate) fn decode_parameter<T>(
    raw: &str,
    target: &str,
    location: &str,
    string_wire: bool,
) -> ::std::result::Result<T, RequestValidationRejection>
where
    T: ::serde::de::DeserializeOwned + ::serde::Serialize,
{
    let typed = if string_wire {
        let instance = ::serde_json::Value::String(raw.to_string());
        validate(target, location, &instance)?;
        ::serde_json::from_value(instance).map_err(|_| generated_contract_error())?
    } else {
        ::serde_json::from_value(::serde_json::Value::String(raw.to_string()))
            .or_else(|_| ::serde_json::from_str(raw))
            .map_err(|_| malformed_parameter(location))?
    };
    validate_parameter(target, location, &typed)?;
    Ok(typed)
}
pub(crate) fn validate_string_parameter(
    target: &str,
    location: &str,
    raw: &str,
) -> ::std::result::Result<(), RequestValidationRejection> {
    validate(
        target,
        location,
        &::serde_json::Value::String(raw.to_string()),
    )
}
pub(crate) fn validate_parameter<T>(
    target: &str,
    location: &str,
    value: &T,
) -> ::std::result::Result<(), RequestValidationRejection>
where
    T: ::serde::Serialize + ?Sized,
{
    let instance = ::serde_json::to_value(value).map_err(|_| generated_contract_error())?;
    validate(target, location, &instance)
}
pub(crate) fn parse_cookies(
    headers: &::axum::http::HeaderMap,
) -> ::std::result::Result<::std::collections::BTreeMap<String, String>, RequestValidationRejection>
{
    let mut cookies = ::std::collections::BTreeMap::new();
    for header in headers.get_all(::axum::http::header::COOKIE).iter() {
        let line = header
            .to_str()
            .map_err(|_| malformed_parameter("/cookie"))?;
        for field in line.split(';') {
            let field = field.trim();
            if field.is_empty() {
                continue;
            }
            let (name, value) = field
                .split_once('=')
                .ok_or_else(|| malformed_parameter("/cookie"))?;
            let name = name.trim();
            if name.is_empty()
                || cookies
                    .insert(name.to_string(), value.to_string())
                    .is_some()
            {
                return Err(malformed_parameter("/cookie"));
            }
        }
    }
    Ok(cookies)
}
async fn read_body(
    body: ::axum::body::Body,
    max_body_bytes: usize,
) -> ::std::result::Result<::axum::body::Bytes, RequestValidationRejection> {
    ::axum::body::to_bytes(body, max_body_bytes)
        .await
        .map_err(|error| {
            let source = ::std::error::Error::source(&error);
            if source.is_some_and(|source| source.is::<::http_body_util::LengthLimitError>()) {
                request_body_too_large()
            } else {
                malformed_request()
            }
        })
}
fn media_type_is(content_type: &str, expected: &str) -> bool {
    let Ok(content_type) = content_type.parse::<::mime::Mime>() else {
        return false;
    };
    let Ok(expected) = expected.parse::<::mime::Mime>() else {
        return false;
    };
    content_type.type_() == expected.type_()
        && content_type.subtype() == expected.subtype()
        && content_type.suffix() == expected.suffix()
        && expected.params().all(|(name, value)| {
            content_type
                .get_param(name)
                .is_some_and(|actual| actual == value)
        })
}
pub(crate) fn validate(
    target: &str,
    location: &str,
    instance: &::serde_json::Value,
) -> ::std::result::Result<(), RequestValidationRejection> {
    let validators = VALIDATORS
        .as_ref()
        .map_err(|_| generated_contract_error())?;
    let Some(validator) = validators.get(target) else {
        return Err(generated_contract_error());
    };
    let mut errors: ::std::vec::Vec<InvalidParameter> = ::std::vec::Vec::new();
    for error in validator.iter_errors(instance) {
        let keyword = error.kind().keyword();
        let (code, message) = public_violation(keyword);
        let mut pointer = format!("{location}{}", error.instance_path());
        if let ::jsonschema::error::ValidationErrorKind::Required { property } = error.kind() {
            if let Some(property) = property.as_str() {
                pointer.push('/');
                pointer.push_str(&escape_pointer_token(property));
            }
        }
        let violation = InvalidParameter {
            code: code.to_string(),
            location: pointer,
            message: message.to_string(),
        };
        if !errors.contains(&violation) {
            errors.push(violation);
            if errors.len() == MAX_VALIDATION_ERRORS {
                break;
            }
        }
    }
    errors.sort_by(|left, right| {
        (&left.location, &left.code, &left.message).cmp(&(
            &right.location,
            &right.code,
            &right.message,
        ))
    });
    if errors.is_empty() {
        return Ok(());
    }
    Err(RequestValidationRejection(ProblemDetails {
        r#type: "https://openapi-to-rust.dev/problems/validation".to_string(),
        title: "Request validation failed".to_string(),
        status: 422,
        code: "request_validation_failed".to_string(),
        errors,
    }))
}
pub(crate) fn malformed_request() -> RequestValidationRejection {
    public_problem(
        400,
        "https://openapi-to-rust.dev/problems/malformed-request",
        "Malformed request",
        "malformed_request",
    )
}
pub(crate) fn malformed_parameter(location: &str) -> RequestValidationRejection {
    parameter_problem(
        400,
        "https://openapi-to-rust.dev/problems/malformed-parameter",
        "Malformed request parameter",
        "malformed_parameter",
        "malformed",
        location,
        "is malformed",
    )
}
pub(crate) fn missing_parameter(location: &str) -> RequestValidationRejection {
    parameter_problem(
        422,
        "https://openapi-to-rust.dev/problems/validation",
        "Request validation failed",
        "request_validation_failed",
        "required",
        location,
        "is required",
    )
}
fn schema_parameter_problem(
    location: &str,
    code: &str,
    message: &str,
) -> RequestValidationRejection {
    parameter_problem(
        422,
        "https://openapi-to-rust.dev/problems/validation",
        "Request validation failed",
        "request_validation_failed",
        code,
        location,
        message,
    )
}
fn parameter_problem(
    status: u16,
    problem_type: &str,
    title: &str,
    code: &str,
    error_code: &str,
    location: &str,
    message: &str,
) -> RequestValidationRejection {
    RequestValidationRejection(ProblemDetails {
        r#type: problem_type.to_string(),
        title: title.to_string(),
        status,
        code: code.to_string(),
        errors: vec![InvalidParameter {
            code: error_code.to_string(),
            location: location.to_string(),
            message: message.to_string(),
        }],
    })
}
pub(crate) fn request_body_too_large() -> RequestValidationRejection {
    public_problem(
        413,
        "https://openapi-to-rust.dev/problems/request-body-too-large",
        "Request body too large",
        "request_body_too_large",
    )
}
pub(crate) fn unsupported_media_type() -> RequestValidationRejection {
    public_problem(
        415,
        "https://openapi-to-rust.dev/problems/unsupported-media-type",
        "Unsupported media type",
        "unsupported_media_type",
    )
}
pub(crate) fn generated_contract_error() -> RequestValidationRejection {
    public_problem(
        500,
        "https://openapi-to-rust.dev/problems/generated-contract-error",
        "Internal server error",
        "generated_contract_error",
    )
}
fn public_problem(
    status: u16,
    problem_type: &str,
    title: &str,
    code: &str,
) -> RequestValidationRejection {
    RequestValidationRejection(ProblemDetails {
        r#type: problem_type.to_string(),
        title: title.to_string(),
        status,
        code: code.to_string(),
        errors: Vec::new(),
    })
}
fn public_violation(keyword: &str) -> (&'static str, &'static str) {
    match keyword {
        "required" => ("required", "is required"),
        "type" => ("type", "has an invalid type"),
        "enum" => ("enum", "has an unsupported value"),
        "const" => ("const", "has an unsupported value"),
        "format" => ("format", "has an invalid format"),
        "pattern" => ("pattern", "does not match the required format"),
        "minLength" => ("min_length", "does not meet the length constraint"),
        "maxLength" => ("max_length", "does not meet the length constraint"),
        "minimum" => ("minimum", "is outside the allowed range"),
        "maximum" => ("maximum", "is outside the allowed range"),
        "exclusiveMinimum" => ("exclusive_minimum", "is outside the allowed range"),
        "exclusiveMaximum" => ("exclusive_maximum", "is outside the allowed range"),
        "multipleOf" => ("multiple_of", "is outside the allowed range"),
        "minItems" => ("min_items", "does not meet the item-count constraint"),
        "maxItems" => ("max_items", "does not meet the item-count constraint"),
        "uniqueItems" => ("unique_items", "contains duplicate items"),
        "minProperties" => (
            "min_properties",
            "does not meet the property-count constraint",
        ),
        "maxProperties" => (
            "max_properties",
            "does not meet the property-count constraint",
        ),
        "additionalProperties" => ("additional_properties", "contains unsupported properties"),
        "unevaluatedProperties" => ("unevaluated_properties", "contains unsupported properties"),
        "anyOf" => ("any_of", "does not match the required shape"),
        "oneOf" => ("one_of", "does not match the required shape"),
        "not" => ("not", "does not match the required shape"),
        _ => ("invalid_value", "is invalid"),
    }
}
fn escape_pointer_token(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
