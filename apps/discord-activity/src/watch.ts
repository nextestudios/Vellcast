// Lógica do modo "assistir pelo relay" (S-011), sem React, para ser testável.
//
// O link de convite é `<relay>/watch#room=<id>&invite=<código>`. O fragmento
// nunca chega ao servidor; o código é trocado uma única vez por um token de
// espectador (POST /api/rooms/{id}/join), guardado só nesta aba. A troca só
// acontece quando o convidado envia o nome (lista de participantes): abrir o
// link e sair sem enviar não gasta o convite.

import type { ErrorCode } from './protocol';

export interface InviteParams {
  room: string;
  invite: string | null;
  /** Token de espectador já obtido (o app desktop abre o player sem gastar outro convite). */
  token: string | null;
}

export interface RoomView {
  id: string;
  name: string;
  state: string;
  viewers: number;
  maxViewers: number;
  /** Nome de quem compartilha agora, quando informado. */
  sharerName?: string;
}

export interface Joined {
  viewerToken: string;
  room: RoomView;
}

const ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
const TOKEN_PATTERN = /^v1\.[A-Za-z0-9_-]{1,1000}\.[A-Za-z0-9_-]{1,100}$/;
const SESSION_KEY = 'vellcast.viewer.';

export function isWatchPath(pathname: string, hash: string): boolean {
  return pathname.startsWith('/watch') || /(^|[#&])room=/.test(hash);
}

/** Lê `room`, `invite` e `token` do fragmento; valores fora do formato são ignorados. */
export function parseInvite(hash: string): InviteParams | null {
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const room = params.get('room');
  if (!room || !ID_PATTERN.test(room)) return null;
  const invite = params.get('invite');
  const token = params.get('token');
  return {
    room,
    invite: invite && ID_PATTERN.test(invite) ? invite : null,
    token: token && TOKEN_PATTERN.test(token) ? token : null,
  };
}

/** Remove o código de convite da barra de endereços e do histórico. */
export function scrubInviteFromUrl(room: string): void {
  try {
    history.replaceState(null, '', `/watch#room=${encodeURIComponent(room)}`);
  } catch {
    // Ambientes que bloqueiam a History API: o convite já foi consumido de qualquer forma.
  }
}

export function loadViewerToken(room: string): string | null {
  try {
    return sessionStorage.getItem(SESSION_KEY + room);
  } catch {
    return null;
  }
}

export function saveViewerToken(room: string, token: string | null): void {
  try {
    if (token) sessionStorage.setItem(SESSION_KEY + room, token);
    else sessionStorage.removeItem(SESSION_KEY + room);
  } catch {
    // Sem armazenamento: recarregar a página exigirá um novo convite.
  }
}

export class JoinError extends Error {
  constructor(
    readonly status: number,
    readonly code: ErrorCode | null,
  ) {
    super(`join falhou: ${status}`);
  }
}

/** Limite do nome do convidado (`MAX_PARTICIPANT_NAME_CHARS` no relay). */
export const MAX_GUEST_NAME = 32;
const PENDING_INVITE_KEY = 'vellcast.invite.';
const GUEST_NAME_KEY = 'vellcast.guestName';

/**
 * Normaliza o nome do convidado como o relay faz: remove caracteres de
 * controle, apara espaços e limita o tamanho. `null` = nome vazio.
 */
export function sanitizeGuestName(raw: string): string | null {
  const name = Array.from(raw.replace(/[\u0000-\u001f\u007f-\u009f]/g, '').trim())
    .slice(0, MAX_GUEST_NAME)
    .join('')
    .trimEnd();
  return name ? name : null;
}

/**
 * Convite ainda não usado desta aba. Fica fora da URL (que é limpa ao abrir)
 * até o convidado enviar o nome; só então é trocado por um token.
 */
export function loadPendingInvite(room: string): string | null {
  try {
    return sessionStorage.getItem(PENDING_INVITE_KEY + room);
  } catch {
    return null;
  }
}

export function savePendingInvite(room: string, invite: string | null): void {
  try {
    if (invite) sessionStorage.setItem(PENDING_INVITE_KEY + room, invite);
    else sessionStorage.removeItem(PENDING_INVITE_KEY + room);
  } catch {
    // Sem armazenamento: o convite fica só na memória desta página.
  }
}

/** Último nome usado neste navegador (conveniência para preencher o campo). */
export function loadGuestName(): string {
  try {
    return localStorage.getItem(GUEST_NAME_KEY) ?? '';
  } catch {
    return '';
  }
}

export function saveGuestName(name: string): void {
  try {
    localStorage.setItem(GUEST_NAME_KEY, name);
  } catch {
    // Opcional.
  }
}

export async function joinRoom(room: string, invite: string, name: string | null = null): Promise<Joined> {
  const res = await fetch(`/api/rooms/${encodeURIComponent(room)}/join`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(name === null ? { invite } : { invite, name }),
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
  });
  if (!res.ok) {
    let code: ErrorCode | null = null;
    try {
      code = ((await res.json()) as { code?: ErrorCode }).code ?? null;
    } catch {
      // Corpo sem JSON: fica só o status HTTP.
    }
    throw new JoinError(res.status, code);
  }
  return (await res.json()) as Joined;
}

/** Mensagem para o espectador a partir de um erro do relay. */
export function friendlyError(code: ErrorCode | null, status?: number): string {
  switch (code) {
    case 'unauthorized':
      return 'Seu acesso expirou ou é inválido. Peça um novo link de convite.';
    case 'forbidden':
      return status === 403
        ? 'Convite inválido, expirado ou já usado. Peça um novo link a quem está transmitindo.'
        : 'Este acesso não permite assistir a esta sala.';
    case 'room_not_found':
      return 'Esta sala não existe mais.';
    case 'room_closed':
      return 'A sala foi encerrada por quem estava transmitindo.';
    case 'room_full':
      return 'A sala está cheia. Tente novamente em instantes.';
    case 'rate_limited':
      return 'Muitas tentativas seguidas. Aguarde alguns segundos e recarregue.';
    case null:
      return status ? `Não foi possível entrar na sala (HTTP ${status}).` : 'Não foi possível conectar ao servidor.';
    default:
      return `Erro de conexão (${code}).`;
  }
}
