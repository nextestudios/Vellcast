// Fluxo de sala com compartilhamento por participantes (issue #4), ponta a
// ponta contra um vellcast-relay real: criar sala → convite → dois
// participantes entram → Ana compartilha → Bia assume (troca explícita) →
// o espectador recebe o fluxo novo, sem quadros velhos → Bia para e a vaga
// fica livre.
//
// Uso: node tests/e2e/share-handoff.mjs
// Requer Node 22+ (WebSocket global) e o relay em VELLCAST_E2E_BASE
// (padrão http://127.0.0.1:8788). Nenhum token ou convite é impresso.

const base = process.env.VELLCAST_E2E_BASE ?? 'http://127.0.0.1:8788';
const wsUrl = base.replace(/^http/, 'ws') + '/ws';
const HEADER_LEN = 28;
const T = {
  VideoInit: 0x01, VideoFrame: 0x02, Pong: 0x11, StreamEnd: 0x20, StreamPaused: 0x21,
  ShareChanged: 0x22, PublisherReplaced: 0x23, Auth: 0x30, AuthOk: 0x31, Error: 0x32, Presence: 0x40,
};
const KEYFRAME = 1;

function packet(type, payload = new Uint8Array(), flags = 0, seq = 0) {
  const buf = new Uint8Array(HEADER_LEN + payload.byteLength);
  const v = new DataView(buf.buffer);
  v.setUint8(0, 1);
  v.setUint8(1, type);
  v.setUint16(2, flags, true);
  v.setUint32(8, seq, true);
  v.setUint32(12, seq * 16666, true);
  buf.set(payload, HEADER_LEN);
  return buf;
}
const json = (type, body) => packet(type, new TextEncoder().encode(JSON.stringify(body)));
const text = (b) => new TextDecoder().decode(b.subarray(HEADER_LEN));

async function api(method, path, token, body) {
  const res = await fetch(base + path, {
    method,
    headers: {
      'content-type': 'application/json',
      ...(token ? { authorization: `Bearer ${token}` } : {}),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = res.status === 204 ? null : await res.json().catch(() => null);
  return { status: res.status, data };
}

function check(cond, msg) {
  if (!cond) throw new Error(`FALHOU: ${msg}`);
  console.log(`ok - ${msg}`);
}

/** Cliente WebSocket com fila de mensagens (ignora Pong; guarda a última `Presence`). */
async function client(token, role) {
  const ws = new WebSocket(wsUrl);
  ws.binaryType = 'arraybuffer';
  const self = { presence: null };
  const queue = [];
  const waiters = [];
  let closed = false;
  ws.onmessage = (ev) => {
    const b = new Uint8Array(ev.data);
    if (b[1] === T.Pong) return;
    if (b[1] === T.Presence) {
      self.presence = JSON.parse(text(b));
      return;
    }
    const w = waiters.shift();
    if (w) w(b);
    else queue.push(b);
  };
  ws.onclose = () => {
    closed = true;
    while (waiters.length) waiters.shift()(null);
  };
  await new Promise((ok, err) => {
    ws.onopen = ok;
    ws.onerror = err;
  });
  const next = (ms = 5000) =>
    queue.length
      ? Promise.resolve(queue.shift())
      : closed
        ? Promise.resolve(null)
        : new Promise((ok, err) => {
            const t = setTimeout(() => err(new Error('timeout esperando mensagem')), ms);
            waiters.push((m) => {
              clearTimeout(t);
              ok(m);
            });
          });
  ws.send(json(T.Auth, { token, role }));
  const first = await next();
  return Object.assign(self, { ws, next, first, send: (b) => ws.send(b) });
}

const created = await api('POST', '/api/rooms', null, { name: 'E2E troca' });
check(created.status === 201, 'sala criada');
const room = created.data.room.id;
const owner = created.data.ownerToken;
const inv = await api('POST', `/api/rooms/${room}/invites`, owner, { maxUses: 3 });
check(inv.status === 201, 'convite criado');
const join = async () => (await api('POST', `/api/rooms/${room}/join`, null, { invite: inv.data.code })).data.viewerToken;
const [ana, bia, watcher] = [await join(), await join(), await join()];
const exhausted = await api('POST', `/api/rooms/${room}/join`, null, { invite: inv.data.code });
check(exhausted.status === 403 && !exhausted.data?.viewerToken, 'convite esgotado não dá acesso');

const viewer = await client(watcher, 'viewer');
check(viewer.first[1] === T.AuthOk, 'espectador conectado');

// Ana compartilha.
const anaShare = await api('POST', `/api/rooms/${room}/share`, ana, { name: 'Ana' });
check(anaShare.status === 200, 'participante Ana obtém permissão de compartilhar');
const anaPub = await client(anaShare.data.publisherToken, 'publisher');
check(anaPub.first[1] === T.AuthOk, 'Ana conectada como transmissora');
let m = await viewer.next();
check(m[1] === T.ShareChanged && JSON.parse(text(m)).name === 'Ana', 'espectador vê "Ana está compartilhando"');
const init = (w) => json(T.VideoInit, { codec: 'avc1.64002A', codedWidth: w, codedHeight: 720, framerate: 60 });
anaPub.send(init(1280));
anaPub.send(packet(T.VideoFrame, new TextEncoder().encode('ana-key'), KEYFRAME, 1));
check((await viewer.next())[1] === T.VideoInit, 'espectador recebe a configuração de Ana');
check(text(await viewer.next()) === 'ana-key', 'espectador recebe o keyframe de Ana');

// Bia assume.
const biaShare = await api('POST', `/api/rooms/${room}/share`, bia, { name: 'Bia' });
const biaPub = await client(biaShare.data.publisherToken, 'publisher');
check(biaPub.first[1] === T.AuthOk, 'Bia conectada: troca explícita');
m = await anaPub.next();
check(m[1] === T.PublisherReplaced && JSON.parse(text(m)).name === 'Bia', 'Ana é avisada de que Bia assumiu');
m = await anaPub.next();
check(m[1] === T.Error && JSON.parse(text(m)).code === 'publisher_exists', 'Ana recebe publisher_exists e para');
m = await viewer.next();
const info = JSON.parse(text(m));
check(m[1] === T.ShareChanged && info.name === 'Bia' && info.handoff === true, 'espectador é avisado da troca');
biaPub.send(init(1920));
biaPub.send(packet(T.VideoFrame, new TextEncoder().encode('bia-key'), KEYFRAME, 1));
m = await viewer.next();
check(m[1] === T.VideoInit && text(m).includes('1920'), 'espectador recebe a configuração nova');
check(text(await viewer.next()) === 'bia-key', 'espectador recebe o fluxo de Bia a partir do keyframe');
const view = await api('GET', `/api/rooms/${room}`, owner);
check(view.data.state === 'live' && view.data.sharerName === 'Bia', 'sala indica Bia como quem compartilha');
await new Promise((r) => setTimeout(r, 200));
const publishers = (viewer.presence?.participants ?? []).filter((p) => p.role === 'publisher').map((p) => p.name);
check(publishers.length === 1 && publishers[0] === 'Bia', 'lista de participantes mostra só Bia transmitindo');

// Não existem dois transmissores: o token comum é recusado.
const legacy = await client(created.data.publisherToken, 'publisher');
check(legacy.first[1] === T.Error && JSON.parse(text(legacy.first)).code === 'publisher_exists', 'segundo transmissor sem troca é recusado');

// Bia para: a vaga fica livre.
biaPub.send(packet(T.StreamEnd));
check((await viewer.next())[1] === T.StreamEnd, 'espectador recebe StreamEnd');
const idle = await api('GET', `/api/rooms/${room}`, owner);
check(idle.data.state === 'idle' && idle.data.sharerName === undefined, 'vaga liberada ao parar');

// Sem autorização válida não há permissão de compartilhar.
const forged = await api('POST', `/api/rooms/${room}/share`, 'v1.forjado.assinatura', {});
check(forged.status === 401, 'token forjado não compartilha');

for (const c of [viewer, anaPub, biaPub, legacy]) c.ws.close();
await api('DELETE', `/api/rooms/${room}`, owner);
console.log('E2E de troca de compartilhamento: tudo certo');
