// Teste do bitrate adaptativo (S-001) no Windows: o transmissor envia ao relay
// por um proxy TCP que limita a banda de subida; o alvo de bitrate deve cair.
//
// Uso: node tests/e2e/abr-check.mjs <publisher.exe> <fonte> [kbps-limite=300] [segundos=45]
// Requer o relay em VELLCAST_E2E_BASE (padrão http://127.0.0.1:8788).

import net from 'node:net';
import { spawn } from 'node:child_process';
import { appendFileSync } from 'node:fs';

const [publisherExe, source, limitArg = '300', secsArg = '45'] = process.argv.slice(2);
const base = process.env.VELLCAST_E2E_BASE ?? 'http://127.0.0.1:8788';
const relayUrl = new URL(base);
const limitBps = (Number(limitArg) * 1000) / 8;
const secs = Number(secsArg);
const startKbps = 3000;

async function api(method, path, token, body) {
  const res = await fetch(base + path, {
    method,
    headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${await res.text()}`);
  return res.json();
}

// Proxy com limite de banda só na subida (transmissor → relay).
const proxy = net.createServer((client) => {
  const upstream = net.connect(Number(relayUrl.port || 80), relayUrl.hostname);
  let nextFree = Date.now();
  let backlog = 0;
  client.on('data', (chunk) => {
    const now = Date.now();
    nextFree = Math.max(nextFree, now) + (chunk.length / limitBps) * 1000;
    backlog += chunk.length;
    if (backlog > 64 * 1024) client.pause();
    setTimeout(() => {
      upstream.write(chunk);
      backlog -= chunk.length;
      if (backlog < 16 * 1024) client.resume();
    }, nextFree - now);
  });
  upstream.on('data', (chunk) => client.write(chunk));
  const close = () => {
    client.destroy();
    upstream.destroy();
  };
  client.on('error', close);
  upstream.on('error', close);
  client.on('close', close);
  upstream.on('close', close);
});
await new Promise((r) => proxy.listen(0, '127.0.0.1', r));
const proxyPort = proxy.address().port;

const created = await api('POST', '/api/rooms', null, { name: 'abr-check' });
const publisher = spawn(
  publisherExe,
  [`ws://127.0.0.1:${proxyPort}/ws`, source, String(secs), '30', String(startKbps), 'off'],
  { env: { ...process.env, VELLCAST_PUBLISHER_TOKEN: created.publisherToken, RUST_LOG: 'warn' }, stdio: ['ignore', 'pipe', 'pipe'] },
);
const stats = [];
for (const stream of [publisher.stdout, publisher.stderr]) {
  stream.setEncoding('utf8');
  stream.on('data', (chunk) => {
    process.stdout.write(chunk);
    for (const line of chunk.split('\n')) {
      const m = line.match(/^STATS (\{.*\})/);
      if (m) stats.push(JSON.parse(m[1]));
    }
  });
}
await new Promise((r) => publisher.on('exit', r));
proxy.close();

const targets = stats.map((s) => s.target_kbps).filter((v) => typeof v === 'number');
const summary = {
  limitKbps: Number(limitArg),
  startKbps,
  targetKbps: targets,
  minTargetKbps: targets.length ? Math.min(...targets) : null,
  encodedBitrateKbps: stats.map((s) => Math.round(s.bitrate_kbps)),
  networkDropped: stats.at(-1)?.net_dropped ?? null,
};
console.log('\nABR ' + JSON.stringify(summary, null, 2));
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### Bitrate adaptativo com subida limitada a ${limitArg} kbps\n\n\`\`\`json\n${JSON.stringify(summary, null, 2)}\n\`\`\`\n`,
  );
}
if (summary.minTargetKbps === null || summary.minTargetKbps >= startKbps * 0.8) {
  console.error('FALHA: o alvo de bitrate não reduziu com a banda limitada');
  process.exit(1);
}
console.log('ABR OK');
