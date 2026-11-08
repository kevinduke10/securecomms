use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use croaker_local::{Workspace, router};
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
    response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .into()
}
async fn replacement(app: &Router, cookie: &str, label: &str) -> (String, String) {
    let id = MessageId::new();
    assert_eq!(
        call(
            app,
            "/local-auth/devices/request",
            Some(cookie),
            Some(json!({"id":id,"label":label}))
        )
        .await
        .status(),
        StatusCode::ACCEPTED
    );
    let account = value(call(app, "/local-auth/me", Some(cookie), None).await).await;
    let request = account["account"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|request| request["id"] == id.to_string())
        .unwrap();
    let fingerprint = request["fingerprint"].as_str().unwrap().to_string();
    assert_eq!(call(app,"/local-auth/devices/decision",Some(cookie),Some(json!({"account_id":account["account"]["id"],"id":id,"fingerprint":fingerprint,"approve":true}))).await.status(),StatusCode::NO_CONTENT);
    let activated = value(
        call(
            app,
            "/local-auth/devices/activate",
            Some(cookie),
            Some(json!({"id":id})),
        )
        .await,
    )
    .await;
    (activated["profile"].as_str().unwrap().into(), fingerprint)
}
async fn state(app: &Router, cookie: &str, alias: &str) -> Value {
    let response = call(
        app,
        &format!("/local-api/state?profile={alias}"),
        Some(cookie),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    value(response).await
}
fn messages(state: &Value, conversation: &str) -> Vec<String> {
    state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|chat| chat["id"] == conversation)
        .unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["text"].as_str().unwrap().to_string())
        .collect()
}
async fn send(app: &Router, cookie: &str, alias: &str, conversation: &str, text: &str) {
    let response=call(app,&format!("/local-api/{alias}/send"),Some(cookie),Some(json!({"conversation_id":conversation,"request_id":MessageId::new(),"content":{"version":1,"text":text,"attachment":null}}))).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn owner_invites_verified_replacement_without_copying_old_history() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path().into()).await.unwrap();
    let app = router(workspace, PathBuf::from("missing-assets"), 3000);
    let alice = signin(&app, "alice").await;
    let bob = signin(&app, "bob").await;
    let initial = state(&app, &alice, "alice").await;
    let group = initial["conversations"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    send(
        &app,
        &alice,
        "alice",
        &group,
        "Before replacement admission",
    )
    .await;
    let (new_alias, fingerprint) = replacement(&app, &bob, "Bob replacement group fixture").await;
    assert!(
        state(&app, &bob, &new_alias).await["conversations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let path = format!("/local-api/alice/conversations/{group}/members");
    let candidates = value(call(&app, &path, Some(&alice), None).await).await;
    assert_eq!(candidates["can_invite"], true);
    assert!(
        candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["profile"] == new_alias)
    );
    let request_id = MessageId::new();
    let input =
        json!({"request_id":request_id,"target_profile":new_alias,"fingerprint":fingerprint});
    let mut wrong = input.clone();
    wrong["fingerprint"] = json!("0".repeat(64));
    assert_eq!(
        call(&app, &path, Some(&alice), Some(wrong)).await.status(),
        StatusCode::BAD_REQUEST
    );
    let unauthorized = format!("/local-api/bob/conversations/{group}/members");
    assert_eq!(
        call(&app, &unauthorized, Some(&bob), Some(input.clone()))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, &path, None, Some(input.clone())).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&app, &path, Some(&alice), Some(input.clone()))
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(&app, &path, Some(&alice), Some(input)).await.status(),
        StatusCode::NO_CONTENT
    );
    let new_state = state(&app, &bob, &new_alias).await;
    assert!(messages(&new_state, &group).is_empty());
    send(&app, &alice, "alice", &group, "After replacement admission").await;
    assert_eq!(
        messages(&state(&app, &bob, &new_alias).await, &group),
        vec!["After replacement admission"]
    );
    let old_messages = messages(&state(&app, &bob, "bob").await, &group);
    assert!(old_messages.contains(&"Before replacement admission".into()));
    assert!(old_messages.contains(&"After replacement admission".into()));
    assert_eq!(
        state(&app, &alice, "alice").await["conversations"][0]["members"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|member| **member == new_alias)
            .count(),
        1
    );
    drop(app);
    let workspace = Workspace::open(directory.path().into()).await.unwrap();
    let app = router(workspace, PathBuf::from("missing-assets"), 3000);
    let bob = signin(&app, "bob").await;
    assert_eq!(
        messages(&state(&app, &bob, &new_alias).await, &group),
        vec!["After replacement admission"]
    );
}

#[tokio::test]
async fn existing_offline_member_keeps_saved_envelope_and_newcomer_skips_old_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = Workspace::open(directory.path().into()).await.unwrap();
    let app = router(workspace, PathBuf::from("missing-assets"), 3000);
    let alice = signin(&app, "alice").await;
    let bob = signin(&app, "bob").await;
    let initial = state(&app, &alice, "alice").await;
    let group = initial["conversations"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Catch Bob up before disconnecting so the saved message is from the old epoch.
    state(&app, &bob, "bob").await;
    assert_eq!(
        call(
            &app,
            "/local-api/bob/online",
            Some(&bob),
            Some(json!({"online":false}))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    send(
        &app,
        &bob,
        "bob",
        &group,
        "Offline message saved before admission",
    )
    .await;
    let (new_alias, fingerprint) = replacement(&app, &bob, "Second offline fixture device").await;
    let path = format!("/local-api/alice/conversations/{group}/members");
    assert_eq!(call(&app,&path,Some(&alice),Some(json!({"request_id":MessageId::new(),"target_profile":new_alias,"fingerprint":fingerprint}))).await.status(),StatusCode::NO_CONTENT);
    let offline = state(&app, &bob, "bob").await;
    assert_eq!(offline["online"], false);
    assert!(
        offline["conversations"][0]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |message| message["text"] == "Offline message saved before admission"
                    && message["pending"] == true
            )
    );
    state(&app, &bob, &new_alias).await;
    assert_eq!(
        call(
            &app,
            "/local-api/bob/online",
            Some(&bob),
            Some(json!({"online":true}))
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert!(
        messages(&state(&app, &alice, "alice").await, &group)
            .contains(&"Offline message saved before admission".into())
    );
    assert!(
        !messages(&state(&app, &bob, &new_alias).await, &group)
            .contains(&"Offline message saved before admission".into())
    );
    send(&app, &bob, "bob", &group, "Message after group key update").await;
    assert!(
        messages(&state(&app, &bob, &new_alias).await, &group)
            .contains(&"Message after group key update".into())
    );
}
