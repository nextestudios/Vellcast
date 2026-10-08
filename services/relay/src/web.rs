//! Player web (S-011) servido pelo próprio relay.
//!
//! O player é uma SPA estática: `/` e `/watch` devolvem o `index.html` e o
//! convite viaja no fragmento da URL (`#room=..&invite=..`), que o navegador
//! nunca envia ao servidor. Assim, nenhum token aparece em logs de acesso.

use std::path::Path;

use axum::Router;
use axum::extract::Request;
use axum::http::{HeaderName, HeaderValue, header};
use axum::middleware::{self, Next};
use axum::response::Response;
use tower_http::services::{ServeDir, ServeFile};

/// CSP do player: só recursos da própria origem; WebSocket na mesma origem.
/// `frame-ancestors` permite a futura Discord Activity (ADR-013).
pub const PLAYER_CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
img-src 'self' data:; connect-src 'self'; media-src 'self' blob:; object-src 'none'; \
base-uri 'none'; form-action 'none'; frame-ancestors 'self' https://discord.com https://*.discordsays.com";

/// Rotas estáticas como fallback: as rotas de API e `/ws` têm prioridade.
pub fn with_player(router: Router, dir: &Path) -> Router {
    let index = dir.join("index.html");
    let spa = ServeDir::new(dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index));
    router
        .fallback_service(spa)
        .layer(middleware::from_fn(player_headers))
}

/// Cabeçalhos de segurança em todas as respostas; CSP só em HTML.
pub async fn player_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let is_html = h
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if is_html {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(PLAYER_CSP),
        );
        h.insert(
            HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        );
        // O HTML muda a cada versão; os assets têm hash no nome e podem ficar em cache.
        h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }
    response
}
