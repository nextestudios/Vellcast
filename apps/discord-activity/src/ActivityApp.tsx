import { useCallback, useEffect, useState } from 'react';
import type { ConnectionStatus } from './connection';
import {
  CODE_ALPHABET,
  CODE_LENGTH,
  connectDiscord,
  friendlyDiscordError,
  listStreams,
  normalizeCode,
  pairInstance,
  type DiscordSession,
  type StreamEntry,
} from './discord';
import { GamepadMonitor } from './gamepad';
import { STATUS_LABEL, StreamPlayer } from './StreamPlayer';

type Phase =
  | { kind: 'connecting' }
  | { kind: 'failed'; message: string }
  | { kind: 'ready' };

/** Atualiza a lista de transmissões vinculadas enquanto nenhuma está aberta. */
const LIST_REFRESH_MS = 5000;

/** Discord Activity: autentica, pareia a instância com uma sala e reproduz. */
export function ActivityApp() {
  const [phase, setPhase] = useState<Phase>({ kind: 'connecting' });
  const [session, setSession] = useState<DiscordSession | null>(null);
  const [streams, setStreams] = useState<StreamEntry[]>([]);
  const [watching, setWatching] = useState<StreamEntry | null>(null);
  const [code, setCode] = useState('');
  const [pairing, setPairing] = useState(false);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const [status, setStatus] = useState<ConnectionStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    connectDiscord()
      .then(async (s) => {
        if (cancelled) return;
        setSession(s);
        const list = await listStreams(s).catch(() => []);
        if (cancelled) return;
        setStreams(list);
        setPhase({ kind: 'ready' });
      })
      .catch((e: unknown) => {
        if (!cancelled) setPhase({ kind: 'failed', message: friendlyDiscordError(e) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const monitor = new GamepadMonitor(() => undefined);
    return () => monitor.dispose();
  }, []);

  const refresh = useCallback(async () => {
    if (!session) return [];
    try {
      const list = await listStreams(session);
      setStreams(list);
      return list;
    } catch (e) {
      setMessage({ text: friendlyDiscordError(e), error: true });
      return [];
    }
  }, [session]);

  // Mantém a lista atual (salas novas, transmissões que entraram ao vivo).
  useEffect(() => {
    if (phase.kind !== 'ready' || watching) return;
    const timer = window.setInterval(() => {
      if (!document.hidden) void refresh();
    }, LIST_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [phase, watching, refresh]);

  const submitCode = async () => {
    if (!session || pairing) return;
    const normalized = normalizeCode(code);
    if (normalized.length !== CODE_LENGTH) {
      setMessage({ text: `O código tem ${CODE_LENGTH} caracteres.`, error: true });
      return;
    }
    setPairing(true);
    try {
      const list = await pairInstance(session, normalized);
      setStreams(list);
      setCode('');
      setMessage({ text: 'Sala vinculada a esta Activity. Todos neste canal podem assistir.', error: false });
      if (list.length === 1) setWatching(list[0]);
    } catch (e) {
      setMessage({ text: friendlyDiscordError(e), error: true });
    } finally {
      setPairing(false);
    }
  };

  // Token expirado ou sala encerrada: busca a lista de novo (tokens novos).
  const onFatal = async () => {
    const current = watching;
    const list = await refresh();
    const again = current && list.find((s) => s.roomId === current.roomId);
    setWatching(again ?? null);
    if (!again) setMessage({ text: 'A transmissão foi encerrada.', error: false });
  };

  const typeChar = (c: string) => setCode((v) => (normalizeCode(v).length < CODE_LENGTH ? normalizeCode(v) + c : v));

  const live = status === 'conectado';

  return (
    <div className="watch">
      <header className="topbar">
        <span className="brand">
          <span className="brand-mark" aria-hidden />
          Vellcast
        </span>
        <span className="tag">{watching ? watching.name : 'Activity'}</span>
        <span className="spacer" />
        {session && <span className="badge">{session.user.username}</span>}
        {watching && (
          <span className={`badge ${live ? 'live' : ''}`} role="status">
            {status ? STATUS_LABEL[status] : 'Conectando…'}
          </span>
        )}
      </header>

      <main className="watch-main">
        {phase.kind === 'connecting' && <p className="muted activity-panel">Conectando ao Discord…</p>}
        {phase.kind === 'failed' && <p className="error activity-panel">{phase.message}</p>}

        {phase.kind === 'ready' && watching && (
          <>
            <StreamPlayer token={watching.viewerToken} onStatus={setStatus} onFatal={() => void onFatal()} />
            <div className="controls">
              <button
                onClick={() => {
                  setWatching(null);
                  setStatus(null);
                }}
              >
                Voltar às transmissões
              </button>
            </div>
          </>
        )}

        {phase.kind === 'ready' && !watching && (
          <div className="activity-panel">
            <section>
              <h2>Transmissões neste canal</h2>
              {streams.length === 0 ? (
                <p className="muted small">Nenhuma sala vinculada ainda. Digite o código mostrado no app do Vellcast.</p>
              ) : (
                <ul className="activity-streams">
                  {streams.map((s) => (
                    <li key={s.roomId}>
                      <button className="room" onClick={() => setWatching(s)}>
                        <span className="room-name">{s.name}</span>
                        <span className="room-meta">
                          {s.state === 'live' ? 'Ao vivo' : 'Aguardando transmissão'} · {s.viewers} assistindo
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            <section>
              <h2>Parear com o app do Vellcast</h2>
              <p className="muted small">
                No computador de quem transmite: Salas → selecione a sala → <strong>Parear com o Discord</strong>.
              </p>
              <form
                className="pair-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  void submitCode();
                }}
              >
                <input
                  className="pair-input"
                  value={code}
                  onChange={(e) => setCode(e.target.value.toUpperCase().slice(0, CODE_LENGTH + 2))}
                  placeholder="ABCD-2345"
                  autoComplete="off"
                  spellCheck={false}
                  aria-label="Código de pareamento"
                />
                <button type="submit" className="primary" disabled={pairing}>
                  {pairing ? 'Pareando…' : 'Parear'}
                </button>
              </form>
              {/* Teclado do alfabeto do código: navegável por controle (D-pad + A). */}
              <div className="code-keys" role="group" aria-label="Teclado do código">
                {CODE_ALPHABET.split('').map((c) => (
                  <button key={c} type="button" onClick={() => typeChar(c)}>
                    {c}
                  </button>
                ))}
                <button type="button" onClick={() => setCode((v) => normalizeCode(v).slice(0, -1))}>
                  Apagar
                </button>
              </div>
            </section>

            {message && <p className={message.error ? 'error small' : 'muted small'}>{message.text}</p>}
          </div>
        )}
      </main>

      <footer className="statusbar">
        <span className={`dot ${live ? 'ok' : session ? 'warn' : ''}`} aria-hidden />
        <span>Transmissão privada · nada é gravado</span>
        <span className="spacer" />
        <span className="hint">
          <kbd>A</kbd> confirmar · <kbd>D-pad</kbd> navegar
        </span>
      </footer>
    </div>
  );
}
