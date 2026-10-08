// Lógica do modo "assistir pelo relay" (S-011), sem React, para ser testável.
//
// O link de convite é `<relay>/watch#room=<id>&invite=<código>`. O fragmento
// nunca chega ao servidor; o código é trocado uma única vez por um token de
// espectador (POST /api/rooms/{id}/join), guardado só nesta aba.

import type { ErrorCode } from './protocol';

export interface InviteParams {
  room: string;
  invite: string | null;
}

export interface RoomView {
  id: string;
  name: string;
  state: string;
  viewers: number;
  maxViewers: number;
}

export interface Joined {
  viewerToken: string;
  room: RoomView;
}

const ID_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
const SESSION_KEY = 'vellcast.viewer.';

export function isWatchPath(pathname: string, hash: string): boolean {
  return pathname.startsWith('/watch') || /(^|[#&])room=/.test(hash);
}

/** Lê `room` e `invite` do fragmento; valores fora do formato são ignorados. */
export function parseInvite(hash: string): InviteParams | null {
  const params = new URLSearchParams(hash.replace(/^#/, ''));
  const room = params.get('room');
  if (!room || !ID_PATTERN.test(room)) return null;
  const invite = params.get('invite');
  return { room, invite: invite && ID_PATTERN.test(invite) ? invite : null };
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

export async function joinRoom(room: string, invite: string): Promise<Joined> {
  const res = await fetch(`/api/rooms/${encodeURIComponent(room)}/join`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ invite }),
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
