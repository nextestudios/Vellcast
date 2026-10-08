// Conexão WebSocket com o servidor: reconexão com backoff, heartbeat e
// sincronização de relógio estilo NTP (Ping/Pong).

import { MessageType, decodePacket, encodePacket, type Packet } from './protocol';
import { nowMs } from './metrics';

const PING_INTERVAL_MS = 1000;
const BACKOFF_MS = [500, 1000, 2000, 4000, 5000];
/** Amostras de relógio mantidas; usa-se a de menor RTT (mais precisa). */
const CLOCK_SAMPLES = 10;

export type ConnectionStatus = 'conectando' | 'conectado' | 'reconectando' | 'encerrado';

export interface ConnectionEvents {
  onPacket(packet: Packet): void;
  /** Disparado a cada nova conexão, antes dos primeiros pacotes. */
  onConnected(): void;
  onStatus(status: ConnectionStatus, detail?: string): void;
}

/**
 * URL do WebSocket na mesma origem da página.
 * Dentro do Discord a origem é https://<client_id>.discordsays.com e o
 * URL Mapping `/` encaminha para o servidor (docs/DISCORD_INTEGRATION.md).
 */
export function wsUrl(stream: string): string {
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${proto}//${location.host}/ws?stream=${encodeURIComponent(stream)}`;
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
    private readonly stream: string,
    private readonly events: ConnectionEvents,
  ) {
    this.connect();
  }

  private connect(): void {
    this.events.onStatus(this.attempt === 0 ? 'conectando' : 'reconectando');
    const ws = new WebSocket(wsUrl(this.stream));
    ws.binaryType = 'arraybuffer';
    this.ws = ws;

    ws.onopen = () => {
      if (this.attempt > 0) this.reconnects++;
      this.attempt = 0;
      this.clock = [];
      this.events.onConnected();
      this.events.onStatus('conectado');
      this.sendPing();
      this.pingTimer = window.setInterval(() => this.sendPing(), PING_INTERVAL_MS);
    };
    ws.onmessage = (ev) => {
      if (!(ev.data instanceof ArrayBuffer)) return;
      const packet = decodePacket(ev.data);
      if (!packet) return;
      if (packet.header.type === MessageType.Pong) {
        this.onPong(packet);
      } else if (packet.header.type === MessageType.StreamEnd) {
        this.events.onStatus('encerrado', 'transmissão encerrada pelo servidor');
      } else {
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

  close(): void {
    this.closed = true;
    window.clearInterval(this.pingTimer);
    window.clearTimeout(this.retryTimer);
    this.ws?.close(1000);
  }
}
