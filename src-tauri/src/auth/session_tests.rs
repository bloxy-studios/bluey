//! The sign-in session's Keychain use (ADR 0011) and callback handling:
//! sign-out never decrypts and never fails half-way, a callback must answer
//! the flow in progress, and only a rejection signs a restored user out.

use std::sync::Arc;
use std::time::Instant;

use bluey_core::{BlueyError, BlueyErrorKind};
use bluey_protocols::oauth::CallbackOutcome;
use tokio_util::sync::CancellationToken;

use super::{restore_signs_out, AuthManager, PendingSignIn};
use crate::events::EventBus;
use crate::secrets::backend::fake::CountingFake;
use crate::secrets::{SecretsStore, CLERK_OAUTH_TOKENS_KEY};
use crate::state::StateHub;
use crate::storage::Storage;

fn manager(fake: &Arc<CountingFake>) -> AuthManager {
    let secrets = Arc::new(SecretsStore::with_backend(fake.clone()));
    secrets.preload_presence().unwrap();
    let bus = Arc::new(EventBus::new());
    let hub = Arc::new(StateHub::new("default", bus.clone()));
    let storage = Arc::new(Storage::in_memory());
    AuthManager::load(secrets, storage, hub, bus, reqwest::Client::new()).unwrap()
}

fn pending(state: &str) -> PendingSignIn {
    PendingSignIn {
        state: state.into(),
        code_verifier: "verifier".into(),
        nonce: "nonce".into(),
        redirect_uri: "bluey://auth/callback".into(),
        started: Instant::now(),
        origin_window: None,
        cancel: CancellationToken::new(),
    }
}

fn code(state: &str) -> CallbackOutcome {
    CallbackOutcome::Code {
        code: "code".into(),
        state: state.into(),
    }
}

#[test]
fn a_callback_for_another_flow_leaves_the_sign_in_running() {
    let auth = manager(&Arc::new(CountingFake::default()));
    *auth.pending.lock() = Some(pending("expected"));

    let forged = auth.take_pending_for(&code("forged")).err().unwrap();
    assert_eq!(forged.code, "auth.state_mismatch");
    let stateless = CallbackOutcome::Denied {
        error: "access_denied".into(),
        description: Some("Click here: https://evil.example".into()),
        state: None,
    };
    assert!(auth.take_pending_for(&stateless).is_err());
    assert!(auth.pending.lock().is_some(), "the real flow is untouched");

    let taken = auth.take_pending_for(&code("expected")).unwrap();
    assert_eq!(taken.state, "expected");
    assert!(auth.pending.lock().is_none());
}

#[tokio::test]
async fn sign_out_never_decrypts_and_completes_when_the_delete_fails() {
    let fake = Arc::new(CountingFake::with_items(&[(CLERK_OAUTH_TOKENS_KEY, "{}")]));
    fake.fail_remove(CLERK_OAUTH_TOKENS_KEY, -25293);
    let auth = manager(&fake);

    let error = auth.clear_session().await.unwrap_err();
    assert_eq!(error.code, "storage.keychain_access_denied");
    assert!(auth.user.lock().is_none());
    assert!(auth.pending.lock().is_none());
    assert_eq!(fake.reads(), 0, "revocation uses in-memory tokens only");
}

#[test]
fn only_a_rejection_signs_a_restored_user_out() {
    let offline = BlueyError::network("request", "the sign-in request failed");
    assert!(!restore_signs_out(&offline));
    let rejected = BlueyError::authentication("invalid_grant", "rejected");
    assert_eq!(rejected.kind, BlueyErrorKind::Authentication);
    assert!(restore_signs_out(&rejected));
}

#[tokio::test]
async fn a_keychain_failure_does_not_fail_the_status() {
    let fake = Arc::new(CountingFake::default());
    fake.fail_lookups(-25291);
    let secrets = Arc::new(SecretsStore::with_backend(fake.clone()));
    let bus = Arc::new(EventBus::new());
    let hub = Arc::new(StateHub::new("default", bus.clone()));
    let storage = Arc::new(Storage::in_memory());
    let auth = AuthManager::load(secrets, storage, hub, bus, reqwest::Client::new()).unwrap();
    let status = auth.status().await.unwrap();
    assert!(!status.has_stored_session);
}

/// A manager configured against `issuer`, holding an unexpired stored sign-in.
fn restoring(fake: &Arc<CountingFake>, issuer: String) -> AuthManager {
    let mut auth = manager(fake);
    auth.config = Some(super::OAuthConfig {
        issuer,
        client_id: "client".into(),
        account_portal: None,
        redirect: bluey_core::types::SignInRedirect::DeepLink,
    });
    auth
}

fn stored_tokens() -> String {
    let expires_at = bluey_oauth::unix_now() + 3600;
    format!(r#"{{"access_token":"a","refresh_token":"r","expires_at":{expires_at}}}"#)
}

/// Answers one HTTP request with `body` (a stand-in for Clerk's `userinfo`).
async fn one_shot_server(body: &'static str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = [0u8; 4096];
        let _ = socket.read(&mut head).await;
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(reply.as_bytes()).await;
    });
    issuer
}

#[tokio::test]
async fn restoring_a_valid_sign_in_does_not_rewrite_the_keychain_item() {
    let fake = Arc::new(CountingFake::with_items(&[(
        CLERK_OAUTH_TOKENS_KEY,
        &stored_tokens(),
    )]));
    let issuer = one_shot_server(r#"{"sub":"user_1","email":"a@b.c"}"#).await;
    let auth = restoring(&fake, issuer);

    auth.restore().await;

    assert!(auth.user.lock().is_some(), "the sign-in was restored");
    assert_eq!(fake.writes(), 0, "unchanged tokens are never re-saved");
    assert_eq!(fake.count(crate::secrets::backend::fake::Op::Remove), 0);
}

#[tokio::test]
async fn restoring_while_offline_keeps_the_stored_sign_in() {
    let fake = Arc::new(CountingFake::with_items(&[(
        CLERK_OAUTH_TOKENS_KEY,
        &stored_tokens(),
    )]));
    // A port nobody listens on: the userinfo request fails at the transport.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let issuer = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let auth = restoring(&fake, issuer);

    auth.restore().await;

    assert!(
        fake.value(CLERK_OAUTH_TOKENS_KEY).is_some(),
        "not signed out"
    );
    assert_eq!(fake.count(crate::secrets::backend::fake::Op::Remove), 0);
}
