//! Contadores agregados. Nunca incluem conteúdo, tokens ou IPs.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

#[derive(Default)]
pub struct Metrics {
    pub bytes_in: AtomicU64,
    pub bytes_out: AtomicU64,
    pub messages_in: AtomicU64,
    pub messages_out: AtomicU64,
    pub viewer_sessions_total: AtomicU64,
    pub publisher_sessions_total: AtomicU64,
    pub lagged_messages: AtomicU64,
    pub stale_frames_dropped: AtomicU64,
    pub keyframe_requests_forwarded: AtomicU64,
    pub auth_failures: AtomicU64,
    pub rate_limited: AtomicU64,
    pub rooms_created: AtomicU64,
    pub rooms_closed: AtomicU64,
    pub slow_viewers_disconnected: AtomicU64,
    /// Trocas de quem compartilha (um participante assumiu a vaga de outro).
    pub publisher_handoffs: AtomicU64,
}

pub fn inc(c: &AtomicU64) {
    c.fetch_add(1, Ordering::Relaxed);
}

pub fn add(c: &AtomicU64, n: u64) {
    c.fetch_add(n, Ordering::Relaxed);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub uptime_secs: u64,
    pub rooms: usize,
    pub rooms_live: usize,
    pub viewers: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub messages_in: u64,
    pub messages_out: u64,
    pub viewer_sessions_total: u64,
    pub publisher_sessions_total: u64,
    pub lagged_messages: u64,
    pub stale_frames_dropped: u64,
    pub keyframe_requests_forwarded: u64,
    pub auth_failures: u64,
    pub rate_limited: u64,
    pub rooms_created: u64,
    pub rooms_closed: u64,
    pub slow_viewers_disconnected: u64,
    pub publisher_handoffs: u64,
}

impl Metrics {
    pub fn snapshot(
        &self,
        uptime_secs: u64,
        rooms: usize,
        rooms_live: usize,
        viewers: usize,
    ) -> Snapshot {
        let l = |c: &AtomicU64| c.load(Ordering::Relaxed);
        Snapshot {
            uptime_secs,
            rooms,
            rooms_live,
            viewers,
            bytes_in: l(&self.bytes_in),
            bytes_out: l(&self.bytes_out),
            messages_in: l(&self.messages_in),
            messages_out: l(&self.messages_out),
            viewer_sessions_total: l(&self.viewer_sessions_total),
            publisher_sessions_total: l(&self.publisher_sessions_total),
            lagged_messages: l(&self.lagged_messages),
            stale_frames_dropped: l(&self.stale_frames_dropped),
            keyframe_requests_forwarded: l(&self.keyframe_requests_forwarded),
            auth_failures: l(&self.auth_failures),
            rate_limited: l(&self.rate_limited),
            rooms_created: l(&self.rooms_created),
            rooms_closed: l(&self.rooms_closed),
            slow_viewers_disconnected: l(&self.slow_viewers_disconnected),
            publisher_handoffs: l(&self.publisher_handoffs),
        }
    }
}
