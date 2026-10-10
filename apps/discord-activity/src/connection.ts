// Conexão WebSocket com o servidor: reconexão com backoff, heartbeat e
// sincronização de relógio estilo NTP (Ping/Pong).

import {
  MessageType,
  decodePacket,
  encodePacket,
  type ErrorCode,
  type Packet,
  type Participant,
  type Presence,
  type ShareInfo,
} from './protocol';
import { nowMs } from './metrics';

const PING_INTERVAL_MS = 1000;
const BACKOFF_MS = [500, 1000, 2000, 4000, 5000];
/** Amostras de relógio mantidas; usa-se a de menor RTT (mais precisa). */
const CLOCK_SAMPLES = 10;

export type ConnectionStatus =
  | 'conectando'
  | 'conectado'
  | 'reconectando'
  /** Conectado à sala, mas sem transmissor ativo (ou transmissor reconectando). */
  | 'aguardando'
  | 'encerrado';

export interface ConnectionEvents {
  onPacket(packet: Packet): void;
  /** Disparado a cada nova conexão, antes dos primeiros pacotes. */
  onConnected(): void;
  onStatus(status: ConnectionStatus, detail?: string): void;
  /** Erro enviado pelo relay (`Error`). `fatal` = reconectar com o mesmo token não adianta. */
  onError?(code: ErrorCode, message: string, fatal: boolean): void;
  /** Lista de participantes ativos (`Presence`), enviada na entrada e a cada mudança. */
  onPresence?(presence: Presence): void;
  /** Quem compartilha mudou (`ShareChanged`); com `handoff`, o fluxo anterior acabou. */
  onShare?(info: ShareInfo): void;
}

export interface ConnectionTarget {
  url: string;
  /** Token de espectador do relay; enviado como primeira mensagem (`Auth`), nunca na URL. */
  viewerToken?: string;
}

/** Espelho de `ErrorCode::is_fatal` (crates/protocol/src/messages.rs). */
const FATAL_ERRORS: ReadonlySet<ErrorCode> = new Set<ErrorCode>([
  'unauthorized',
  'forbidden',
  'room_not_found',
  'room_closed',
  'publisher_exists',
  'protocol_error',
  'message_too_large',
]);

export function isFatalError(code: ErrorCode): boolean {
  return FATAL_ERRORS.has(code);
}

/**
 * URL do WebSocket na mesma origem da página.
 * Dentro do Discord a origem é https://<client_id>.discordsays.com e o
 * URL Mapping `/` encaminha para o servidor (docs/DISCORD_INTEGRATION.md).
 */
export function wsUrl(stream?: string): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  const query = stream === undefined ? '' : `?stream=${encodeURIComponent(stream)}`;
  return `${proto}//${location.host}/ws${query}`;
}

export class Connection {
  private ws: WebSocket | null = null;
  private pingTimer: number | undefined;
  private retryTimer: number | undefined;
  private attempt = 0;
  private pingSeq = 0;
  private closed = false;
  private clock: { rtt: number; offset: number }[] = [];
  reconnects = 0;

  constructor(
    private readonly target: ConnectionTarget,
    private readonly events: ConnectionEvents,
  ) {
    this.connect();
  }

  private connect(): void {
    this.events.onStatus(this.attempt === 0 ? 'conectando' : 'reconectando');
    const ws = new WebSocket(this.target.url);
    ws.binaryType = 'arraybuffer';
    this.ws = ws;

    ws.onopen = () => {
      if (this.target.viewerToken) {
        // O relay exige `Auth` como primeira mensagem; o resto espera o `AuthOk`.
        const auth = JSON.stringify({ token: this.target.viewerToken, role: 'viewer' });
        this.send(encodePacket(MessageType.Auth, 0, 0, new TextEncoder().encode(auth)));
      } else {
        this.onReady(true);
      }
    };
    ws.onmessage = (ev) => {
      if (!(ev.data instanceof ArrayBuffer)) return;
      const packet = decodePacket(ev.data);
      if (!packet) return;
      switch (packet.header.type) {
        case MessageType.Pong:
          this.onPong(packet);
          break;
        case MessageType.AuthOk: {
          const ok = parseJson<{ live?: boolean }>(packet.payload);
          this.onReady(ok?.live !== false);
          break;
        }
        case MessageType.Error: {
          const body = parseJson<{ code?: ErrorCode; message?: string }>(packet.payload);
          const code = body?.code ?? 'internal';
          const fatal = isFatalError(code);
          // Um erro fatal encerra as tentativas; os demais seguem o backoff normal.
          if (fatal) this.closed = true;
          this.events.onError?.(code, body?.message ?? '', fatal);
          if (fatal) this.events.onStatus('encerrado', body?.message);
          break;
        }
        case MessageType.Presence: {
          const presence = parsePresence(packet.payload);
          if (presence) this.events.onPresence?.(presence);
          break;
        }
        case MessageType.ShareChanged:
          this.events.onShare?.(parseJson<ShareInfo>(packet.payload) ?? {});
          break;
        case MessageType.StreamPaused:
          this.events.onStatus('aguardando', 'o transmissor está reconectando');
          break;
        case MessageType.StreamEnd:
          if (this.target.viewerToken) {
            this.events.onStatus('aguardando', 'a transmissão terminou; aguardando o transmissor');
          } else {
            this.events.onStatus('encerrado', 'transmissão encerrada pelo servidor');
          }
          break;
        case MessageType.VideoInit:
          // Nova configuração de vídeo = transmissor ativo (após pausa ou novo início).
          if (this.target.viewerToken) this.events.onStatus('conectado');
          this.events.onPacket(packet);
          break;
        default:
          this.events.onPacket(packet);
      }
    };
    ws.onclose = (ev) => {
      window.clearInterval(this.pingTimer);
      this.ws = null;
      if (this.closed) return;
      // Jitter de ±20% evita que todos os espectadores reconectem no mesmo instante.
      const base = BACKOFF_MS[Math.min(this.attempt, BACKOFF_MS.length - 1)];
      const delay = Math.round(base * (0.8 + Math.random() * 0.4));
      this.attempt++;
      this.events.onStatus('reconectando', `código ${ev.code}; nova tentativa em ${delay} ms`);
      this.retryTimer = window.setTimeout(() => this.connect(), delay);
    };
  }

  /** Conexão pronta para mídia (direto no modo stage0; após `AuthOk` no relay). */
  private onReady(live: boolean): void {
    if (this.attempt > 0) this.reconnects++;
    this.attempt = 0;
    this.clock = [];
    this.events.onConnected();
    this.events.onStatus(live ? 'conectado' : 'aguardando', live ? undefined : 'a sala ainda não tem transmissão');
    this.sendPing();
    window.clearInterval(this.pingTimer);
    this.pingTimer = window.setInterval(() => this.sendPing(), PING_INTERVAL_MS);
  }

  private sendPing(): void {
    const payload = new Uint8Array(8);
    new DataView(payload.buffer).setFloat64(0, nowMs(), true);
    this.send(encodePacket(MessageType.Ping, 0, this.pingSeq++, payload));
  }

  private onPong({ header, payload }: Packet): void {
    if (payload.byteLength < 8) return;
    const t0 = new DataView(payload.buffer, payload.byteOffset, 8).getFloat64(0, true);
    const t1 = nowMs();
    const rtt = t1 - t0;
    const offset = header.sentAtUs / 1000 - (t0 + t1) / 2;
    this.clock.push({ rtt, offset });
    if (this.clock.length > CLOCK_SAMPLES) this.clock.shift();
  }

  private bestClock(): { rtt: number; offset: number } | null {
    return this.clock.reduce<{ rtt: number; offset: number } | null>(
      (best, s) => (best === null || s.rtt < best.rtt ? s : best),
      null,
    );
  }

  rttMs(): number | null {
    return this.clock.at(-1)?.rtt ?? null;
  }

  /** Diferença relógio do servidor − relógio local (ms). */
  clockOffsetMs(): number | null {
    return this.bestClock()?.offset ?? null;
  }

  serverToLocalMs(serverUs: number): number | null {
    const offset = this.clockOffsetMs();
    return offset === null ? null : serverUs / 1000 - offset;
  }

  requestKeyframe(): void {
    this.send(encodePacket(MessageType.KeyframeRequest, 0, 0));
  }

  private send(data: ArrayBuffer): void {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(data);
  }

  isClosed(): boolean {
    return this.closed;
  }

  close(): void {
    this.closed = true;
    window.clearInterval(this.pingTimer);
    window.clearTimeout(this.retryTimer);
    this.ws?.close(1000);
  }
}

/** Valida o payload de `Presence`; entradas fora do formato são descartadas. */
export function parsePresence(payload: Uint8Array): Presence | null {
  const body = parseJson<{ count?: unknown; participants?: unknown }>(payload);
  if (!body || !Array.isArray(body.participants)) return null;
  const participants = body.participants.filter(
    (p: Partial<Participant> | null): p is Participant =>
      typeof p?.id === 'string' &&
      (p.name === null || typeof p.name === 'string') &&
      (p.role === 'publisher' || p.role === 'viewer'),
  );
  const count = typeof body.count === 'number' ? body.count : participants.length;
  return { count, participants };
}

function parseJson<T>(payload: Uint8Array): T | null {
  try {
    return JSON.parse(new TextDecoder().decode(payload)) as T;
  } catch {
    return null;
  }
}
