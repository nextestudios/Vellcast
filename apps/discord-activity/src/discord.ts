// Inicialização do Discord Embedded App SDK.
//
// Fora do Discord (navegador comum) a página roda em modo standalone, o que
// permite testar o player localmente. A autenticação OAuth2 do usuário
// fica para a ETAPA 5; aqui só se valida que o SDK inicializa (ready()).

export interface DiscordContext {
  mode: 'discord' | 'standalone';
  platform?: string;
  instanceId?: string;
  guildId?: string | null;
  channelId?: string | null;
  error?: string;
}

export function insideDiscord(): boolean {
  return new URLSearchParams(location.search).has('frame_id');
}

export async function initDiscord(): Promise<DiscordContext> {
  if (!insideDiscord()) return { mode: 'standalone' };
  const clientId = import.meta.env.VITE_DISCORD_CLIENT_ID as string | undefined;
  if (!clientId) return { mode: 'discord', error: 'VITE_DISCORD_CLIENT_ID não definido no build' };
  try {
    // Import dinâmico: o SDK só é carregado dentro do Discord.
    const { DiscordSDK } = await import('@discord/embedded-app-sdk');
    const sdk = new DiscordSDK(clientId);
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
