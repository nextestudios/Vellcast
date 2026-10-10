// Protocolo binário v1 — espelho de crates/protocol (docs/STREAMING_PROTOCOL.md).

export const VERSION = 1;
export const HEADER_LEN = 28;

export const MessageType = {
  VideoInit: 0x01,
  VideoFrame: 0x02,
  AudioInit: 0x03,
  AudioFrame: 0x04,
  Ping: 0x10,
  Pong: 0x11,
  KeyframeRequest: 0x12,
  StreamEnd: 0x20,
  StreamPaused: 0x21,
  Auth: 0x30,
  AuthOk: 0x31,
  Error: 0x32,
  /** Participantes ativos da sala (payload JSON `Presence`). */
  Presence: 0x40,
  /** Servidor → espectador: quem compartilha agora (JSON `ShareInfo`); o fluxo anterior acabou. */
  ShareChanged: 0x22,
  /** Servidor → transmissor: outro participante assumiu o compartilhamento. */
  PublisherReplaced: 0x23,
} as const;

/** Um participante ativo (`Participant` em crates/protocol/src/messages.rs). */
export interface Participant {
  /** Identificador opaco da sessão; não é token nem convite. */
  id: string;
  /** `null` quando o participante não informou nome. Sempre exibir como texto. */
  name: string | null;
  role: 'publisher' | 'viewer';
}

/** Payload de `Presence`: lista completa, não incremental. */
export interface Presence {
  count: number;
  participants: Participant[];
}

/** Payload de `ShareChanged` (crates/protocol/src/messages.rs). */
export interface ShareInfo {
  /** Nome exibido de quem compartilha (opcional). */
  name?: string;
  /** `true` quando outro participante assumiu a vaga de quem compartilhava. */
  handoff?: boolean;
}

/** Códigos de erro do payload JSON de `Error` (crates/protocol/src/messages.rs). */
export type ErrorCode =
  | 'unauthorized'
  | 'forbidden'
  | 'room_not_found'
  | 'room_full'
  | 'room_closed'
  | 'publisher_exists'
  | 'auth_timeout'
  | 'idle_timeout'
  | 'message_too_large'
  | 'rate_limited'
  | 'bitrate_exceeded'
  | 'protocol_error'
  | 'internal';
export type MessageType = (typeof MessageType)[keyof typeof MessageType];

export const Flags = { Keyframe: 1 << 0, Discontinuity: 1 << 1 } as const;

export interface Header {
  type: number;
  flags: number;
  streamId: number;
  sequence: number;
  /** Timestamp de mídia (µs). */
  ptsUs: number;
  /** Relógio de parede do servidor (Unix µs). */
  sentAtUs: number;
}

export interface Packet {
  header: Header;
  payload: Uint8Array;
}

// u64 em µs cabe com folga em Number (2^53 µs ≈ 285 anos).
function readU64(view: DataView, offset: number): number {
  return view.getUint32(offset, true) + view.getUint32(offset + 4, true) * 2 ** 32;
}

function writeU64(view: DataView, offset: number, value: number): void {
  view.setUint32(offset, value % 2 ** 32, true);
  view.setUint32(offset + 4, Math.floor(value / 2 ** 32), true);
}

export function decodePacket(buffer: ArrayBuffer): Packet | null {
  if (buffer.byteLength < HEADER_LEN) return null;
  const view = new DataView(buffer);
  if (view.getUint8(0) !== VERSION) return null;
  return {
    header: {
      type: view.getUint8(1),
      flags: view.getUint16(2, true),
      streamId: view.getUint32(4, true),
      sequence: view.getUint32(8, true),
      ptsUs: readU64(view, 12),
      sentAtUs: readU64(view, 20),
    },
    payload: new Uint8Array(buffer, HEADER_LEN),
  };
}

export function encodePacket(type: MessageType, streamId: number, sequence: number, payload?: Uint8Array): ArrayBuffer {
  const buffer = new ArrayBuffer(HEADER_LEN + (payload?.byteLength ?? 0));
  const view = new DataView(buffer);
  view.setUint8(0, VERSION);
  view.setUint8(1, type);
  view.setUint32(4, streamId, true);
  view.setUint32(8, sequence, true);
  writeU64(view, 20, 0);
  if (payload) new Uint8Array(buffer, HEADER_LEN).set(payload);
  return buffer;
}
