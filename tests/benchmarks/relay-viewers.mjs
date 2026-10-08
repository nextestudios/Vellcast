// Teste de carga do vellcast-relay: 1 transmissor (mídia sintética) → N espectadores autenticados.
//
// Pré-requisitos:
//   cargo build --release -p vellcast-relay -p vellcast-transport --example publish_fixture
//   VELLCAST_RELAY_SECRET=<32+ bytes> VELLCAST_BIND=127.0.0.1:8788 target/release/vellcast-relay
// Uso:
//   node tests/benchmarks/relay-viewers.mjs [http://127.0.0.1:8788] [1080p60] [viewers] [segundos]
//
// Mede, por espectador: quadros de vídeo/s, Mbps e latência transmissor → espectador
// (sent_at_us do transmissor até o recebimento; mesma máquina, mesmo relógio).
// Requer Node 22+ (fetch e WebSocket globais).

import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const [base = 'http://127.0.0.1:8788', profile = '1080p60', n = '20', secs = '30'] = process.argv.slice(2);
const viewers = Number(n);
const durationMs = Number(secs) * 1000;
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const wsUrl = base.replace(/^http/, 'ws') + '/ws';

async function api(method, url, token, body) {
  const res = await fetch(base + url, {
    method,
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) throw new Error(`${method} ${url}: ${res.status} ${await res.text()}`);
  return res.json();
}

function packet(type, payload = new Uint8Array()) {
  const buf = new Uint8Array(28 + payload.length);
  buf[0] = 1;
  buf[1] = type;
  buf.set(payload, 28);
  return buf;
}

const created = await api('POST', '/api/rooms', null, { name: 'benchmark', maxViewers: viewers });
const roomId = created.room.id;
const invite = await api('POST', `/api/rooms/${roomId}/invites`, created.ownerToken, { maxUses: viewers });
const tokens = [];
for (let i = 0; i < viewers; i++) {
  tokens.push((await api('POST', `/api/rooms/${roomId}/join`, null, { invite: invite.code })).viewerToken);
}

const publisher = spawn(path.join(root, 'target/release/examples/publish_fixture'), [wsUrl, profile, String(Number(secs) + 5)], {
  cwd: root,
  env: { ...process.env, VELLCAST_PUBLISHER_TOKEN: created.publisherToken, RUST_LOG: 'warn' },
  stdio: ['ignore', 'inherit', 'inherit'],
});
await new Promise((r) => setTimeout(r, 1500));

function viewer(token) {
  return new Promise((resolve) => {
    const ws = new WebSocket(wsUrl);
    ws.binaryType = 'arraybuffer';
    const r = { frames: 0, bytes: 0, latencies: [], maxGapMs: 0, error: null };
    let last = 0;
    let ping;
    ws.onopen = () => {
      ws.send(packet(0x30, new TextEncoder().encode(JSON.stringify({ token, role: 'viewer' }))));
      ping = setInterval(() => ws.send(packet(0x10, new Uint8Array(8))), 1000);
    };
    ws.onmessage = (ev) => {
      const v = new DataView(ev.data);
      const type = v.getUint8(1);
      if (type === 0x32) r.error = new TextDecoder().decode(new Uint8Array(ev.data, 28));
      if (type !== 0x02) return;
      const now = performance.timeOrigin + performance.now();
      if (last) r.maxGapMs = Math.max(r.maxGapMs, now - last);
      last = now;
      r.frames++;
      r.bytes += ev.data.byteLength;
      const sentUs = v.getUint32(20, true) + v.getUint32(24, true) * 2 ** 32;
      r.latencies.push(now - sentUs / 1000);
    };
    setTimeout(() => {
      clearInterval(ping);
      ws.close();
      resolve(r);
    }, durationMs);
  });
}

console.log(`${viewers} espectadores em ${profile} por ${secs}s via relay…`);
const results = await Promise.all(tokens.map(viewer));
const metrics = await (await fetch(base + '/metrics')).json();
publisher.kill('SIGINT');

const pct = (a, p) => {
  const s = [...a].sort((x, y) => x - y);
  return s.length ? s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))] : NaN;
};
const s = Number(secs);
const fps = results.map((r) => r.frames / s);
const all = results.flatMap((r) => r.latencies);
console.log(
  JSON.stringify(
    {
      profile,
      viewers,
      seconds: s,
      fpsMin: Math.min(...fps).toFixed(1),
      fpsMax: Math.max(...fps).toFixed(1),
      mbpsPerViewer: ((results.reduce((a, r) => a + r.bytes, 0) * 8) / 1e6 / s / viewers).toFixed(2),
      maxGapMs: Math.max(...results.map((r) => r.maxGapMs)).toFixed(1),
      latencyMs: { p50: pct(all, 50).toFixed(2), p95: pct(all, 95).toFixed(2), p99: pct(all, 99).toFixed(2) },
      errors: results.filter((r) => r.error).map((r) => r.error),
      relay: {
        laggedMessages: metrics.laggedMessages,
        staleFramesDropped: metrics.staleFramesDropped,
        slowViewersDisconnected: metrics.slowViewersDisconnected,
        bytesIn: metrics.bytesIn,
        bytesOut: metrics.bytesOut,
      },
    },
    null,
    2,
  ),
);
