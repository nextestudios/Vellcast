//! Modo hospedado (E7-T04): limites de plano via token de entitlement da Vellcast Cloud.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use vellcast_entitlement::Claims;
use vellcast_relay::Relay;
use vellcast_relay::config::Config;

const SECRET: &[u8] = b"entitlement-test-secret-0123456789";
const CLOUD_KEY: &[u8] = b"cloud-entitlement-key-0123456789abcdef";

async fn start(cloud: bool) -> SocketAddr {
    let mut config = Config::with_secret(SECRET.to_vec());
    config.cloud_entitlement_key = cloud.then(|| CLOUD_KEY.to_vec());
    let relay = Relay::new(config);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(vellcast_relay::serve(Arc::clone(&relay), listener));
    addr
}

/// `POST /api/rooms` cru; devolve status e corpo.
async fn create_room(
    addr: SocketAddr,
    entitlement: Option<&str>,
    max_viewers: u32,
) -> (u16, String) {
    let body = format!("{{\"name\":\"sala\",\"maxViewers\":{max_viewers}}}");
    let extra = entitlement
        .map(|t| format!("X-Vellcast-Entitlement: {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "POST /api/rooms HTTP/1.1\r\nHost: t\r\nConnection: close\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text[9..12].parse().unwrap();
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    (status, body)
}

fn token(sub: &str, max_rooms: u32, max_viewers: u32, exp_offset: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    vellcast_entitlement::sign(
        CLOUD_KEY,
        &Claims {
            sub: sub.into(),
            plan: "free".into(),
            max_viewers,
            max_rooms,
            max_height: 1080,
            max_fps: 60,
            exp: (now + exp_offset) as u64,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn self_hosted_relay_needs_no_entitlement() {
    let addr = start(false).await;
    let (status, _) = create_room(addr, None, 5).await;
    assert_eq!(
        status, 201,
        "self-hosting não depende da Vellcast Cloud (W-013)"
    );
}

#[tokio::test]
async fn hosted_relay_requires_valid_entitlement() {
    let addr = start(true).await;
    assert_eq!(create_room(addr, None, 5).await.0, 401, "sem token");
    assert_eq!(
        create_room(addr, Some("ve1.lixo.lixo"), 5).await.0,
        401,
        "token inválido"
    );
    assert_eq!(
        create_room(addr, Some(&token("acc_a", 2, 10, -10)), 5)
            .await
            .0,
        401,
        "expirado"
    );
    let forged = {
        let other = b"outra-chave-outra-chave-outra-chave-xx";
        vellcast_entitlement::sign(
            other,
            &Claims {
                sub: "acc_a".into(),
                plan: "community".into(),
                max_viewers: 999,
                max_rooms: 99,
                max_height: 2160,
                max_fps: 60,
                exp: u64::MAX / 2,
            },
        )
        .unwrap()
    };
    assert_eq!(
        create_room(addr, Some(&forged), 5).await.0,
        401,
        "chave de outra origem"
    );
}

#[tokio::test]
async fn hosted_relay_enforces_plan_limits() {
    let addr = start(true).await;
    let t = token("acc_b", 2, 3, 600);
    let (status, body) = create_room(addr, Some(&t), 50).await;
    assert_eq!(status, 201, "{body}");
    assert!(
        body.contains("\"maxViewers\":3"),
        "espectadores limitados pelo plano: {body}"
    );
    assert_eq!(create_room(addr, Some(&t), 1).await.0, 201);
    assert_eq!(
        create_room(addr, Some(&t), 1).await.0,
        403,
        "limite de 2 salas do plano"
    );
    // Outra conta tem a própria cota.
    assert_eq!(
        create_room(addr, Some(&token("acc_c", 1, 3, 600)), 1)
            .await
            .0,
        201
    );
}
