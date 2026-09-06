//! Integration tests: full request → router → response, without binding a
//! socket. Contract conformance against the spec is layered on in S4.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn router() -> axum::Router {
    // Point the UI fallback at a directory that never exists so these tests
    // exercise the API surface, not the filesystem.
    shepherd_server::router("does-not-exist")
}

#[tokio::test]
async fn health_reports_pass_and_core_version() {
    let response = router()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/health+json"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "pass");
    assert_eq!(json["version"], shepherd_core::version());
    assert_eq!(json["description"], "shepherd local daemon");
}

#[tokio::test]
async fn openapi_spec_is_served_as_yaml() {
    let response = router()
        .oneshot(
            Request::get("/api/v1/openapi.yaml")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/yaml");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.starts_with(b"openapi:"));
}

#[tokio::test]
async fn unknown_path_falls_through_to_404() {
    let response = router()
        .oneshot(Request::get("/nope").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
