// Integração com o Discord Embedded App SDK.
//
// Dentro do Discord (URL com `frame_id`): `ready()` → `authorize` (código
// OAuth2) → o relay troca o código no servidor (`/api/discord/token`, o client
// secret nunca chega ao navegador) → `authenticate`. O relay devolve uma sessão
// Vellcast curta, usada para parear a instância com uma sala e listar as
// transmissões. Fora do Discord a página roda em modo navegador.
//
// Requisições usam caminhos relativos: o URL Mapping `/` da aplicação aponta
// para o relay (docs/DISCORD_INTEGRATION.md).

export interface DiscordContext {
  mode: 'discord' | 'standalone';
  platform?: string;
  instanceId?: string;
  guildId?: string | null;
  channelId?: string | null;
  error?: string;
}

export interface DiscordUser {
  id: string;
  username: string;
}

export interface DiscordSession {
  instanceId: string;
  /** Sessão Vellcast (Bearer) emitida pelo relay. */
  sessionToken: string;
  user: DiscordUser;
  platform: string;
}

export interface StreamEntry {
  roomId: string;
  name: string;
  state: 'waiting' | 'live' | 'paused' | 'closed' | string;
  viewers: number;
  viewerToken: string;
}

export class DiscordApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string | null,
    message: string,
  ) {
    super(message);
  }
}

export function insideDiscord(): boolean {
  return new URLSearchParams(location.search).has('frame_id');
}

function clientId(): string | undefined {
  return import.meta.env.VITE_DISCORD_CLIENT_ID as string | undefined;
}

/** Só valida que o SDK inicializa (página de validação da ETAPA 0). */
export async function initDiscord(): Promise<DiscordContext> {
  if (!insideDiscord()) return { mode: 'standalone' };
  const id = clientId();
  if (!id) return { mode: 'discord', error: 'VITE_DISCORD_CLIENT_ID não definido no build' };
  try {
    // Import dinâmico: o SDK só é carregado dentro do Discord.
    const { DiscordSDK } = await import('@discord/embedded-app-sdk');
    const sdk = new DiscordSDK(id);
    await sdk.ready();
    return {
      mode: 'discord',
      platform: sdk.platform,
      instanceId: sdk.instanceId,
      guildId: sdk.guildId,
      channelId: sdk.channelId,
    };
  } catch (e) {
    return { mode: 'discord', error: e instanceof Error ? e.message : String(e) };
  }
}

async function postJson<T>(path: string, body: unknown, session?: string): Promise<T> {
  const res = await fetch(path, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      ...(session ? { authorization: `Bearer ${session}` } : {}),
    },
    body: JSON.stringify(body),
  });
  return parse<T>(res);
}

async function parse<T>(res: Response): Promise<T> {
  if (res.ok) return (await res.json()) as T;
  let code: string | null = null;
  let message = `HTTP ${res.status}`;
  try {
    const err = (await res.json()) as { code?: string; message?: string };
    code = err.code ?? null;
    message = err.message ?? message;
  } catch {
    // corpo sem JSON
  }
  throw new DiscordApiError(res.status, code, message);
}

/** Inicializa o SDK, autentica o usuário e obtém a sessão Vellcast. */
export async function connectDiscord(): Promise<DiscordSession> {
  const id = clientId();
  if (!id) throw new Error('Esta versão da Activity foi publicada sem VITE_DISCORD_CLIENT_ID.');
  const { DiscordSDK } = await import('@discord/embedded-app-sdk');
  const sdk = new DiscordSDK(id);
  await sdk.ready();
  const { code } = await sdk.commands.authorize({
    client_id: id,
    response_type: 'code',
    state: '',
    prompt: 'none',
    // Escopo mínimo: só identificar o usuário.
    scope: ['identify'],
  });
  const token = await postJson<{ accessToken: string; sessionToken: string; user: DiscordUser }>(
    '/api/discord/token',
    { code },
  );
  const auth = await sdk.commands.authenticate({ access_token: token.accessToken });
  if (!auth) throw new Error('O Discord recusou a autenticação.');
  return {
    instanceId: sdk.instanceId,
    sessionToken: token.sessionToken,
    user: token.user,
    platform: sdk.platform,
  };
}

/** Vincula a instância da Activity à sala do código mostrado no desktop. */
export async function pairInstance(session: DiscordSession, code: string): Promise<StreamEntry[]> {
  const res = await postJson<{ streams: StreamEntry[] }>(
    '/api/discord/pair',
    { code, instanceId: session.instanceId },
    session.sessionToken,
  );
  return res.streams;
}

/** Transmissões já vinculadas a esta instância (com tokens de espectador novos). */
export async function listStreams(session: DiscordSession): Promise<StreamEntry[]> {
  const res = await fetch(`/api/discord/instances/${encodeURIComponent(session.instanceId)}/streams`, {
    headers: { authorization: `Bearer ${session.sessionToken}` },
  });
  return (await parse<{ streams: StreamEntry[] }>(res)).streams;
}

/** Normaliza o código digitado (maiúsculas, sem espaço/hífen); mesmo critério do relay. */
export function normalizeCode(input: string): string {
  return input.replace(/[\s-]/g, '').toUpperCase();
}

/** Alfabeto dos códigos de pareamento (sem 0/O, 1/I/L). */
export const CODE_ALPHABET = 'ABCDEFGHJKMNPQRSTUVWXYZ23456789';
export const CODE_LENGTH = 8;

export function friendlyDiscordError(e: unknown): string {
  if (e instanceof DiscordApiError) {
    if (e.status === 503) return 'Este servidor do Vellcast ainda não está configurado para o Discord.';
    if (e.status === 429) return 'Muitas tentativas. Aguarde alguns segundos e tente de novo.';
    if (e.code === 'forbidden') return 'Código inválido ou expirado. Gere um novo no app do Vellcast (Salas → Parear com o Discord).';
    if (e.code === 'unauthorized') return 'Sessão expirada. Feche e abra a Activity de novo.';
    if (e.code === 'room_full') return 'Esta Activity já tem salas demais vinculadas.';
    return e.message;
  }
  return e instanceof Error ? e.message : String(e);
}
