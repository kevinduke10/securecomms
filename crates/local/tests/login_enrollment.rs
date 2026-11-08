use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use croaker_local::{Workspace, auth::AuthOptions, router, router_with_auth};
use croaker_protocol::MessageId;
use serde_json::{Value, json};
use std::path::PathBuf;
use tower::ServiceExt;
async fn call(
    app: &Router,
    path: &str,
    cookie: Option<&str>,
    body: Option<Value>,
) -> axum::response::Response {
    let mut request = if body.is_some() {
        Request::post(path)
    } else {
        Request::get(path)
    }
    .header("host", "127.0.0.1:3000")
    .header("x-croaker-local", "1");
    if let Some(cookie) = cookie {
        request = request.header("cookie", cookie);
    }
    if body.is_some() {
        request = request.header("content-type", "application/json");
    }
    app.clone()
        .oneshot(
            request
                .body(
                    body.map(|v| Body::from(v.to_string()))
                        .unwrap_or(Body::empty()),
                )
                .unwrap(),
        )
        .await
        .unwrap()
}
async fn value(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap()).unwrap()
}
async fn signin(app: &Router, profile: &str) -> String {
    let response = call(
        app,
        "/local-auth/demo",
        None,
        Some(json!({"profile":profile})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    cookie.split(';').next().unwrap().into()
}
#[tokio::test]
async fn sessions_scope_accounts_and_replacement_keys_without_restoring_membership() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(dir.path().into()).await.unwrap();
    let app = router(workspace, PathBuf::from("missing"), 3000);
    assert_eq!(
        call(&app, "/local-api/state?profile=alice", None, None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let bob = signin(&app, "bob").await;
    assert_eq!(
        call(&app, "/local-api/state?profile=alice", Some(&bob), None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(&app, "/local-api/alice/backups/archives", Some(&bob), None)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let initial = value(call(&app, "/local-auth/me", Some(&bob), None).await).await;
    let account = initial["account"]["id"].as_str().unwrap();
    let old = initial["account"]["devices"][0]["binding"]["device_id"].clone();
    let id = MessageId::new();
    let request = json!({"id":id,"label":"Bob replacement"});
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/request",
            Some(&bob),
            Some(request.clone())
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/request",
            Some(&bob),
            Some(request)
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/activate",
            Some(&bob),
            Some(json!({"id":id}))
        )
        .await
        .status(),
        StatusCode::BAD_REQUEST
    );
    let pending = value(call(&app, "/local-auth/me", Some(&bob), None).await).await;
    let fingerprint = pending["account"]["requests"][0]["fingerprint"]
        .as_str()
        .unwrap();
    let maya = signin(&app, "maya").await;
    let decision = json!({"account_id":account,"id":id,"fingerprint":fingerprint,"approve":true});
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/decision",
            Some(&maya),
            Some(decision.clone())
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let mut wrong = decision.clone();
    wrong["fingerprint"] = json!("0".repeat(64));
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/decision",
            Some(&bob),
            Some(wrong)
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/decision",
            Some(&bob),
            Some(decision)
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let activated = value(
        call(
            &app,
            "/local-auth/devices/activate",
            Some(&bob),
            Some(json!({"id":id})),
        )
        .await,
    )
    .await;
    let alias = activated["profile"].as_str().unwrap();
    let state = value(
        call(
            &app,
            &format!("/local-api/state?profile={alias}"),
            Some(&bob),
            None,
        )
        .await,
    )
    .await;
    assert_eq!(state["conversations"], json!([]));
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/revoke",
            Some(&bob),
            Some(json!({"device_id":old}))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, "/local-api/state?profile=bob", Some(&bob), None)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, "/local-auth/logout", Some(&bob), Some(json!({})))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            &format!("/local-api/state?profile={alias}"),
            Some(&bob),
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    drop(app);
    let reopened = Workspace::open(dir.path().into()).await.unwrap();
    let app = router(reopened, PathBuf::from("missing"), 3000);
    let bob = signin(&app, "bob").await;
    assert_eq!(
        call(
            &app,
            "/local-auth/devices/select",
            Some(&bob),
            Some(json!({"alias":alias}))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            &app,
            &format!("/local-api/state?profile={alias}"),
            Some(&bob),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
}
#[tokio::test]
async fn cognito_pkce_start_is_bound_to_browser_and_demo_login_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(dir.path().into()).await.unwrap();
    let options = AuthOptions {
        auto_login: false,
        config: Some(croaker_relay::login::LoginConfig {
            issuer: "https://cognito-idp.us-east-1.amazonaws.com/pool".into(),
            client_id: "client".into(),
            domain: "https://fixture.auth.us-east-1.amazoncognito.com".into(),
            scope: "openid croaker/access".into(),
        }),
        base_url: Some("http://127.0.0.1:3000".into()),
        subjects: Default::default(),
    };
    let app = router_with_auth(workspace, PathBuf::from("missing"), 3000, options);
    assert_eq!(
        call(
            &app,
            "/local-auth/demo",
            None,
            Some(json!({"profile":"alice"}))
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let response = call(&app, "/local-auth/start", None, None).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let url = reqwest::Url::parse(response.headers()["location"].to_str().unwrap()).unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["response_type"], "code");
    assert!(!query.contains_key("code_verifier"));
    assert!(
        response.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .contains("HttpOnly; SameSite=Lax")
    );
    let response = call(
        &app,
        &format!(
            "/local-auth/callback?code=not-a-real-code&state={}",
            query["state"]
        ),
        None,
        None,
    )
    .await;
    assert_eq!(response.headers()["location"], "/?login_error=1");
    let response = app
        .oneshot(
            Request::post("/local-auth/logout")
                .header("host", "evil.example")
                .header("x-croaker-local", "1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
