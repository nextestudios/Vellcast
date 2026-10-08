// Teste de carga da ETAPA 0: N espectadores simultâneos em uma transmissão.
//
// Uso: node tests/benchmarks/stage0-viewers.mjs [url-base] [stream] [viewers] [segundos]
// Ex.:  node tests/benchmarks/stage0-viewers.mjs ws://127.0.0.1:8787 1080p60 20 30
//
// Mede, por espectador: quadros de vídeo/s, Mbps, maior intervalo entre quadros
// e latência servidor → recebimento (mesma máquina, relógio compartilhado).
// Requer Node 22+ (WebSocket global).

const [base = 'ws://127.0.0.1:8787', stream = '1080p60', n = '10', secs = '20'] = process.argv.slice(2);
const viewers = Number(n);
const durationMs = Number(secs) * 1000;

const VIDEO_FRAME = 0x02;
const results = [];

function viewer(id) {
  return new Promise((resolve) => {
    const ws = new WebSocket(`${base}/ws?stream=${encodeURIComponent(stream)}`);
    ws.binaryType = 'arraybuffer';
    const r = { id, frames: 0, bytes: 0, maxGapMs: 0, latencies: [], closed: null };
    let last = 0;
    let ping;
    ws.onopen = () => {
      // Mantém a conexão viva (o servidor encerra clientes inativos).
      ping = setInterval(() => {
        const buf = new ArrayBuffer(36);
        const v = new DataView(buf);
        v.setUint8(0, 1);
        v.setUint8(1, 0x10);
        ws.send(buf);
      }, 1000);
    };
    ws.onmessage = (ev) => {
      const v = new DataView(ev.data);
      if (v.getUint8(1) !== VIDEO_FRAME) return;
      const now = performance.timeOrigin + performance.now();
      if (last) r.maxGapMs = Math.max(r.maxGapMs, now - last);
      last = now;
      r.frames++;
      r.bytes += ev.data.byteLength;
      const sentUs = v.getUint32(20, true) + v.getUint32(24, true) * 2 ** 32;
      r.latencies.push(now - sentUs / 1000);
    };
    ws.onclose = (ev) => (r.closed = ev.code);
    setTimeout(() => {
      clearInterval(ping);
      ws.close();
      resolve(r);
    }, durationMs);
  });
}

const pct = (a, p) => {
  const s = [...a].sort((x, y) => x - y);
  return s.length ? s[Math.min(s.length - 1, Math.floor((p / 100) * s.length))] : NaN;
};

console.log(`${viewers} espectadores em ${stream} por ${secs}s…`);
results.push(...(await Promise.all(Array.from({ length: viewers }, (_, i) => viewer(i)))));

const s = Number(secs);
const fps = results.map((r) => r.frames / s);
const all = results.flatMap((r) => r.latencies);
console.log(
  JSON.stringify(
    {
      stream,
      viewers,
      seconds: s,
      fpsMin: Math.min(...fps).toFixed(1),
      fpsMax: Math.max(...fps).toFixed(1),
      mbpsPerViewer: (results.reduce((a, r) => a + r.bytes, 0) * 8 / 1e6 / s / viewers).toFixed(2),
      maxGapMs: Math.max(...results.map((r) => r.maxGapMs)).toFixed(1),
      latencyMs: { p50: pct(all, 50).toFixed(2), p95: pct(all, 95).toFixed(2), p99: pct(all, 99).toFixed(2) },
      abnormalCloses: results.filter((r) => r.closed !== null && r.closed !== 1000 && r.closed !== 1005).length,
    },
    null,
    2,
  ),
);
