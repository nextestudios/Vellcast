// Teste do modo competitivo (S-002) no Windows: com a CPU sobrecarregada, o
// encoder não acompanha 60 FPS e o modo competitivo deve reduzir o FPS-alvo.
//
// Uso: node tests/e2e/competitive-check.mjs <publisher.exe> <fonte> [segundos=45]
// Requer o relay em VELLCAST_E2E_BASE (padrão http://127.0.0.1:8788).

import { spawn } from 'node:child_process';
import { appendFileSync } from 'node:fs';

const [publisherExe, source, secsArg = '45'] = process.argv.slice(2);
const base = process.env.VELLCAST_E2E_BASE ?? 'http://127.0.0.1:8788';
const secs = Number(secsArg);

const res = await fetch(base + '/api/rooms', {
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ name: 'competitive-check' }),
});
if (!res.ok) throw new Error(`criar sala: ${res.status}`);
const created = await res.json();

const wsUrl = base.replace(/^http/, 'ws') + '/ws';
const publisher = spawn(publisherExe, [wsUrl, source, String(secs), '60', '8000', 'off', 'competitive'], {
  env: { ...process.env, VELLCAST_PUBLISHER_TOKEN: created.publisherToken, RUST_LOG: 'warn' },
  stdio: ['ignore', 'pipe', 'pipe'],
});
const stats = [];
const warnings = [];
for (const stream of [publisher.stdout, publisher.stderr]) {
  stream.setEncoding('utf8');
  stream.on('data', (chunk) => {
    process.stdout.write(chunk);
    for (const line of chunk.split('\n')) {
      const m = line.match(/^STATS (\{.*\})/);
      if (m) stats.push(JSON.parse(m[1]));
      const w = line.match(/Warning \{ message: "([^"]+)"/);
      if (w) warnings.push(w[1]);
    }
  });
}
await new Promise((r) => publisher.on('exit', r));

const fps = stats.map((s) => s.target_fps).filter((v) => typeof v === 'number');
const summary = {
  requestedFps: 60,
  targetFps: fps,
  minTargetFps: fps.length ? Math.min(...fps) : null,
  targetKbps: stats.map((s) => s.target_kbps),
  encodeMs: stats.map((s) => Number((s.encode_ms ?? 0).toFixed(1))),
  encodedFps: stats.map((s) => Number((s.encoded_fps ?? 0).toFixed(1))),
  warnings,
};
console.log('\nCOMPETITIVO ' + JSON.stringify(summary, null, 2));
if (process.env.GITHUB_STEP_SUMMARY) {
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### Modo competitivo com CPU sobrecarregada\n\n\`\`\`json\n${JSON.stringify(summary, null, 2)}\n\`\`\`\n`,
  );
}
if (summary.minTargetFps === null || summary.minTargetFps >= 60) {
  console.error('FALHA: o modo competitivo não reduziu o FPS com o encoder sobrecarregado');
  process.exit(1);
}
console.log('COMPETITIVO OK');
