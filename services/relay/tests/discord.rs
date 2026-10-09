//! Integração Discord do relay contra uma API do Discord simulada (axum local).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Form, Path};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use vellcast_relay::config::Config;
use vellcast_relay::discord::DiscordConfig;
use vellcast_relay::{Relay, router_with_discord};

const SECRET: &[u8] = b"discord-test-secret-discord-test-";
const ACCESS: &str = "at-123";

/// API do Discord simulada: OAuth2, `users/@me` e instâncias da Activity.
async fn mock_discord() -> SocketAddr {
    let app = Router::new()
        .route(
            "/oauth2/token",
            post(|Form(form): Form<HashMap<String, String>>| async move {
                let ok = form.get("code").map(String::as_str) == Some("good")
                    && form.get("client_secret").map(String::as_str) == Some("shh")
                    && form.get("grant_type").map(String::as_str) == Some("authorization_code");
                if ok {
                    (
                        StatusCode::OK,
                        Json(json!({ "access_token": ACCESS, "token_type": "Bearer" })),
                    )
                } else {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(json!({ "error": "invalid_grant" })),
                    )
                }
            }),
        )
        .route(
            "/users/@me",
            get(|headers: HeaderMap| async move {
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
                if auth == Some("Bearer at-123") {
                    (
                        StatusCode::OK,
                        Json(json!({ "id": "111", "username": "ana" })),
                    )
                } else {
                    (StatusCode::UNAUTHORIZED, Json(json!({})))
                }
            }),
        )
        .route(
            "/applications/{app}/activity-instances/{inst}",
            get(
                |Path((_app, inst)): Path<(String, String)>, headers: HeaderMap| async move {
                    let bot = headers.get("authorization").and_then(|v| v.to_str().ok())
                        == Some("Bot bot");
                    if bot && inst == "i-1" {
                        (
                            StatusCode::OK,
                            Json(json!({ "instance_id": inst, "users": ["111"] })),
                        )
                    } else {
                        (StatusCode::NOT_FOUND, Json(json!({})))
                    }
                },
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

async fn start(discord: Option<DiscordConfig>) -> SocketAddr {
    let relay = Relay::new(Config::with_secret(SECRET.to_vec()));
    let app = router_with_discord(Arc::clone(&relay), discord)
        .into_make_service_with_connect_info::<SocketAddr>();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

fn config(api: SocketAddr, bot: bool) -> DiscordConfig {
    DiscordConfig {
        client_id: "app".into(),
        client_secret: "shh".into(),
        api_base: format!("http://{api}"),
        bot_token: bot.then(|| "bot".into()),
        session_ttl: Duration::from_secs(600),
    }
}

async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<Value>,
) -> (u16, Value) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    if let Some(t) = bearer {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw);
    let status = text[9..12].parse().unwrap();
    let json = text
        .split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    (status, json)
}

async fn session(addr: SocketAddr) -> String {
    let (status, body) = http(
        addr,
        "POST",
        "/api/discord/token",
        None,
        Some(json!({ "code": "good" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["accessToken"], ACCESS);
    assert_eq!(body["user"]["id"], "111");
    body["sessionToken"].as_str().unwrap().to_owned()
}

async fn room(addr: SocketAddr) -> (String, String, String) {
    let (status, body) = http(
        addr,
        "POST",
        "/api/rooms",
        None,
        Some(json!({ "name": "Sexta" })),
    )
    .await;
    assert_eq!(status, 201);
    (
        body["room"]["id"].as_str().unwrap().to_owned(),
        body["ownerToken"].as_str().unwrap().to_owned(),
        body["publisherToken"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn disabled_without_configuration() {
    let addr = start(None).await;
    let (status, body) = http(
        addr,
        "POST",
        "/api/discord/token",
        None,
        Some(json!({ "code": "good" })),
    )
    .await;
    assert_eq!(status, 503, "{body}");
}

#[tokio::test]
async fn exchanges_code_on_the_server_and_rejects_bad_codes() {
    let api = mock_discord().await;
    let addr = start(Some(config(api, false))).await;
    let session = session(addr).await;
    assert!(!session.is_empty());

    let (status, _) = http(
        addr,
        "POST",
        "/api/discord/token",
        None,
        Some(json!({ "code": "bad" })),
    )
    .await;
    assert_eq!(status, 401);
    let (status, _) = http(
        addr,
        "POST",
        "/api/discord/token",
        None,
        Some(json!({ "code": "" })),
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn pairing_binds_instance_to_room_once() {
    let api = mock_discord().await;
    let addr = start(Some(config(api, true))).await;
    let session = session(addr).await;
    let (room_id, owner, publisher) = room(addr).await;

    // Só o dono da sala gera códigos.
    let path = format!("/api/rooms/{room_id}/pairing");
    let (status, _) = http(addr, "POST", &path, Some(&publisher), None).await;
    assert_eq!(status, 403);
    let (status, _) = http(addr, "POST", &path, Some(&session), None).await;
    assert_eq!(status, 403);
    let (status, body) = http(addr, "POST", &path, Some(&owner), None).await;
    assert_eq!(status, 201, "{body}");
    let code = body["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 8);

    // Sem sessão Discord: recusado.
    let pair = json!({ "code": code, "instanceId": "i-1" });
    let (status, _) = http(addr, "POST", "/api/discord/pair", None, Some(pair.clone())).await;
    assert_eq!(status, 401);

    // Usuário fora da instância (verificado com o token de bot).
    let other = json!({ "code": code, "instanceId": "i-2" });
    let (status, _) = http(
        addr,
        "POST",
        "/api/discord/pair",
        Some(&session),
        Some(other),
    )
    .await;
    assert_eq!(status, 403);

    // Código digitado com hífen e minúsculas.
    let typed = format!("{}-{}", code[..4].to_lowercase(), &code[4..]);
    let (status, body) = http(
        addr,
        "POST",
        "/api/discord/pair",
        Some(&session),
        Some(json!({ "code": typed, "instanceId": "i-1" })),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    let streams = body["streams"].as_array().unwrap();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0]["roomId"], room_id.as_str());
    let viewer = streams[0]["viewerToken"].as_str().unwrap().to_owned();

    // O token de espectador emitido vale para a sala.
    let (status, _) = http(
        addr,
        "GET",
        &format!("/api/rooms/{room_id}"),
        Some(&viewer),
        None,
    )
    .await;
    assert_eq!(status, 200);
    // A sessão Discord não serve como token de sala.
    let (status, _) = http(
        addr,
        "GET",
        &format!("/api/rooms/{room_id}"),
        Some(&session),
        None,
    )
    .await;
    assert_eq!(status, 401);

    // Uso único.
    let (status, _) = http(
        addr,
        "POST",
        "/api/discord/pair",
        Some(&session),
        Some(pair),
    )
    .await;
    assert_eq!(status, 403);

    // A instância continua listando a sala.
    let (status, body) = http(
        addr,
        "GET",
        "/api/discord/instances/i-1/streams",
        Some(&session),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["streams"].as_array().unwrap().len(), 1);

    // Sala encerrada some da lista.
    let (status, _) = http(
        addr,
        "DELETE",
        &format!("/api/rooms/{room_id}"),
        Some(&owner),
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (_, body) = http(
        addr,
        "GET",
        "/api/discord/instances/i-1/streams",
        Some(&session),
        None,
    )
    .await;
    assert!(body["streams"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn wrong_codes_are_rate_limited() {
    let api = mock_discord().await;
    let addr = start(Some(config(api, false))).await;
    let session = session(addr).await;
    let mut limited = false;
    for _ in 0..30 {
        let (status, _) = http(
            addr,
            "POST",
            "/api/discord/pair",
            Some(&session),
            Some(json!({ "code": "ZZZZZZZZ", "instanceId": "i-1" })),
        )
        .await;
        if status == 429 {
            limited = true;
            break;
        }
        assert_eq!(status, 403);
    }
    assert!(limited, "tentativas erradas deveriam ser limitadas");
}

#[tokio::test]
async fn rejects_invalid_instance_ids() {
    let api = mock_discord().await;
    let addr = start(Some(config(api, false))).await;
    let session = session(addr).await;
    let (status, _) = http(
        addr,
        "POST",
        "/api/discord/pair",
        Some(&session),
        Some(json!({ "code": "ABCD2345", "instanceId": "../etc" })),
    )
    .await;
    assert_eq!(status, 400);
}
