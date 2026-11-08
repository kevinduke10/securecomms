use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use croaker_protocol::*;
use croaker_relay::{Store, api, identity::*, login::*, sqlite::SqliteStore};
use jsonwebtoken::{Algorithm, EncodingKey, Header, jwk::JwkSet};
use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn login_rejects_wrong_signature_issuer_client_use_scope_and_expiry() {
    let private = RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
    let public = private.to_public_key();
    let set:JwkSet=serde_json::from_value(json!({"keys":[{"kty":"RSA","kid":"fixture","alg":"RS256","use":"sig","n":BASE64_URL_SAFE_NO_PAD.encode(public.n().to_bytes_be()),"e":BASE64_URL_SAFE_NO_PAD.encode(public.e().to_bytes_be())}]})).unwrap();
    let config = LoginConfig {
        issuer: "https://cognito-idp.us-east-1.amazonaws.com/fixture".into(),
        client_id: "fixture-client".into(),
        domain: "https://fixture.auth.us-east-1.amazoncognito.com".into(),
        scope: "openid croaker/access".into(),
    };
    let verifier = Arc::new(LoginVerifier::with_keys(config.clone(), set).await.unwrap());
    let key = EncodingKey::from_rsa_der(private.to_pkcs1_der().unwrap().as_bytes());
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("fixture".into());
    let claims = json!({"iss":config.issuer,"sub":"invited-subject","client_id":config.client_id,"token_use":"access","scope":"openid croaker/access","iat":croaker_relay::now(),"exp":croaker_relay::now()+300});
    let sign = |claims: &serde_json::Value| jsonwebtoken::encode(&header, claims, &key).unwrap();
    let good = sign(&claims);
    assert_eq!(verifier.verify(&good).await.unwrap().sub, "invited-subject");
    for (field, value) in [
        ("iss", json!("https://attacker.example")),
        ("client_id", json!("other-client")),
        ("token_use", json!("id")),
        ("scope", json!("openid")),
        ("exp", json!(croaker_relay::now() - 60)),
        ("iat", json!(croaker_relay::now() + 120)),
    ] {
        let mut bad = claims.clone();
        bad[field] = value;
        assert!(verifier.verify(&sign(&bad)).await.is_err(), "{field}");
    }
    let mut tampered = good.clone().into_bytes();
    let last = tampered.len() - 5;
    tampered[last] = if tampered[last] == b'A' { b'B' } else { b'A' };
    assert!(
        verifier
            .verify(std::str::from_utf8(&tampered).unwrap())
            .await
            .is_err()
    );
    let hmac = jsonwebtoken::encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(b"public-key-is-not-an-hmac-secret"),
    )
    .unwrap();
    assert!(verifier.verify(&hmac).await.is_err());
    let store = Arc::new(SqliteStore::open(":memory:").unwrap());
    let app = api::router_with_login(store.clone(), verifier.clone());
    let account_request = || {
        Request::get("/v1/account")
            .header("authorization", format!("Bearer {good}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(account_request())
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    store
        .grant_account(AccountGrant {
            issuer: config.issuer,
            subject: "invited-subject".into(),
            tenant_id: TenantId::new(),
            role: Role::Member,
            name: "Invited User".into(),
            bootstrap_device: None,
        })
        .await
        .unwrap();
    let response = app.clone().oneshot(account_request()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    // Account authentication is not a device credential for messaging.
    assert_eq!(
        app.oneshot(
            Request::get(format!("/v1/conversations/{}/sync", ConversationId::new()))
                .header("authorization", format!("Bearer {good}"))
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        StatusCode::UNAUTHORIZED
    );
}
