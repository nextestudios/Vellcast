// Teste de ponta a ponta no Windows: captura real → encoder → relay → espectadores.
//
// Uso (na CI Windows, com o relay já rodando):
//   node tests/e2e/windows-e2e.mjs <publisher.exe> [segundos=30] [espectadores=3] [fps=30] [kbps=3000] [audio=system] [fonte=monitor]
//
// Variáveis: VELLCAST_E2E_BASE (padrão http://127.0.0.1:8788), VELLCAST_E2E_RELAY_PID (amostra de memória do relay).
// Sai com código 1 se os espectadores não receberem vídeo em ritmo razoável,
// se houver erros de protocolo ou se a memória do transmissor crescer sem parar.

import { spawn, execFileSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';

const [publisherExe, secsArg = '30', viewersArg = '3', fpsArg = '30', kbpsArg = '3000', audio = 'system', source = 'monitor'] =
  process.argv.slice(2);
if (!publisherExe) {
  console.error('uso: windows-e2e.mjs <publisher.exe> [segundos] [espectadores] [fps] [kbps] [audio] [fonte]');
  process.exit(2);
}
const base = process.env.VELLCAST_E2E_BASE ?? 'http://127.0.0.1:8788';
const wsUrl = base.replace(/^http/, 'ws') + '/ws';
const secs = Number(secsArg);
const viewers = Number(viewersArg);
const fps = Number(fpsArg);
const relayPid = process.env.VELLCAST_E2E_RELAY_PID ? Number(process.env.VELLCAST_E2E_RELAY_PID) : null;

async function api(method, url, token, body) {
  const res = await fetch(base + url, {
    method,
    headers: { 'content-type': 'application/json', ...(token ? { authorization: `Bearer ${token}` } : {}) },
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

/** Memória (working set, MB) de um processo Windows via tasklist. */
function workingSetMb(pid) {
  try {
    const out = execFileSync('tasklist', ['/FI', `PID eq ${pid}`, '/FO', 'CSV', '/NH'], { encoding: 'utf8' });
    const field = out.trim().split('","').pop()?.replace(/[^0-9]/g, '');
    return field ? Number(field) / 1024 : null;
  } catch {
    return null;
  }
}

const created = await api('POST', '/api/rooms', null, { name: 'e2e-windows', maxViewers: viewers });
const roomId = created.room.id;
const invite = await api('POST', `/api/rooms/${roomId}/invites`, created.ownerToken, { maxUses: viewers });
const tokens = [];
for (let i = 0; i < viewers; i++) {
  tokens.push((await api('POST', `/api/rooms/${roomId}/join`, null, { invite: invite.code })).viewerToken);
}

const publisherLog = [];
const publisher = spawn(publisherExe, [wsUrl, source, String(secs + 8), String(fps), kbpsArg, audio], {
  env: { ...process.env, VELLCAST_PUBLISHER_TOKEN: created.publisherToken, RUST_LOG: 'info' },
  stdio: ['ignore', 'pipe', 'pipe'],
});
for (const stream of [publisher.stdout, publisher.stderr]) {
  stream.setEncoding('utf8');
  stream.on('data', (chunk) => {
    process.stdout.write(chunk);
    publisherLog.push(chunk);
  });
}
const publisherExit = new Promise((resolve) => publisher.on('exit', (code) => resolve(code)));

const memory = { publisher: [], relay: [] };
const sampler = setInterval(() => {
  const p = workingSetMb(publisher.pid);
  if (p !== null) memory.publisher.push(p);
  if (relayPid) {
    const r = workingSetMb(relayPid);
    if (r !== null) memory.relay.push(r);
  }
}, 10_000);

// Dá tempo para captura, encoder e conexão iniciarem.
await new Promise((r) => setTimeout(r, 4000));

function viewer(token) {
  return new Promise((resolve) => {
    const ws = new WebSocket(wsUrl);
    ws.binaryType = 'arraybuffer';
    const r = { video: 0, keyframes: 0, audio: 0, bytes: 0, firstFrameMs: null, maxGapMs: 0, errors: [], codec: null };
    const opened = performance.now();
    let last = 0;
    let ping;
    ws.onopen = () => {
      ws.send(packet(0x30, new TextEncoder().encode(JSON.stringify({ token, role: 'viewer' }))));
      ping = setInterval(() => ws.send(packet(0x10, new Uint8Array(8))), 1000);
    };
    ws.onmessage = (ev) => {
      const v = new DataView(ev.data);
      const type = v.getUint8(1);
      if (type === 0x32) r.errors.push(new TextDecoder().decode(new Uint8Array(ev.data, 28)));
      if (type === 0x01) r.codec = JSON.parse(new TextDecoder().decode(new Uint8Array(ev.data, 28))).codec;
      if (type === 0x04) r.audio++;
      if (type !== 0x02) return;
      const now = performance.now();
      if (r.firstFrameMs === null) r.firstFrameMs = now - opened;
      if (last) r.maxGapMs = Math.max(r.maxGapMs, now - last);
      last = now;
      r.video++;
      if (v.getUint16(2, true) & 1) r.keyframes++;
      r.bytes += ev.data.byteLength;
    };
    ws.onerror = () => r.errors.push('erro de WebSocket');
    setTimeout(() => {
      clearInterval(ping);
      ws.close();
      resolve(r);
    }, secs * 1000);
  });
}

console.log(`\n${viewers} espectadores por ${secs}s em ${wsUrl}…`);
const results = await Promise.all(tokens.map(viewer));
clearInterval(sampler);
const metrics = await (await fetch(base + '/metrics')).json();
publisher.kill();
await Promise.race([publisherExit, new Promise((r) => setTimeout(r, 5000))]);

const log = publisherLog.join('');
const videoFps = results.map((r) => r.video / secs);
const growth = (series) => (series.length >= 3 ? series[series.length - 1] - series[1] : null);
const summary = {
  seconds: secs,
  viewers,
  targetFps: fps,
  audioMode: audio,
  source,
  codec: results[0]?.codec ?? null,
  encoder: (log.match(/encoder: "([^"]+)"/) ?? log.match(/ENCODER ([^\n]+)/))?.[1] ?? null,
  videoFpsPerViewer: { min: Math.min(...videoFps).toFixed(1), max: Math.max(...videoFps).toFixed(1) },
  keyframesPerViewer: Math.min(...results.map((r) => r.keyframes)),
  audioPacketsPerViewer: Math.min(...results.map((r) => r.audio)),
  mbpsPerViewer: ((results.reduce((a, r) => a + r.bytes, 0) * 8) / 1e6 / secs / viewers).toFixed(2),
  firstFrameMs: Math.max(...results.map((r) => r.firstFrameMs ?? Infinity)).toFixed(0),
  maxGapMs: Math.max(...results.map((r) => r.maxGapMs)).toFixed(0),
  errors: results.flatMap((r) => r.errors),
  audioWarnings: [...log.matchAll(/Warning \{ message: "([^"]+)"/g)].map((m) => m[1]),
  memoryMb: {
    publisher: memory.publisher.map((m) => Math.round(m)),
    relay: memory.relay.map((m) => Math.round(m)),
    publisherGrowthAfterWarmup: growth(memory.publisher),
  },
  relay: {
    laggedMessages: metrics.laggedMessages,
    staleFramesDropped: metrics.staleFramesDropped,
    slowViewersDisconnected: metrics.slowViewersDisconnected,
  },
};
console.log('\nRESULTADO ' + JSON.stringify(summary, null, 2));

const failures = [];
if (Math.min(...videoFps) < fps * 0.5) failures.push(`FPS por espectador abaixo de 50% do alvo (${summary.videoFpsPerViewer.min} < ${fps * 0.5})`);
if (summary.keyframesPerViewer < 1) failures.push('nenhum keyframe recebido');
if (summary.errors.length) failures.push(`erros de protocolo: ${summary.errors.join('; ')}`);
// Crescimento contínuo de memória (> 100 MB após o aquecimento) indica vazamento.
if (summary.memoryMb.publisherGrowthAfterWarmup !== null && summary.memoryMb.publisherGrowthAfterWarmup > 100)
  failures.push(`memória do transmissor cresceu ${summary.memoryMb.publisherGrowthAfterWarmup.toFixed(0)} MB`);
if (!/connected|Connected/.test(log)) failures.push('o transmissor não reportou conexão ao relay');

if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### E2E Windows (${secs}s, ${viewers} espectadores, ${source}, áudio ${audio})\n\n` +
      '```json\n' + JSON.stringify(summary, null, 2) + '\n```\n\n' +
      (failures.length ? `**Falhas:**\n${failures.map((f) => `- ${f}`).join('\n')}\n` : '**OK**\n'),
  );
}
if (failures.length) {
  console.error('\nFALHAS:\n- ' + failures.join('\n- '));
  process.exit(1);
}
console.log('\nE2E OK');
