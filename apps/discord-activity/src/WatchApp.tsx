import { useEffect, useState } from 'react';
import type { ConnectionStatus } from './connection';
import { GamepadMonitor } from './gamepad';
import { STATUS_LABEL, StreamPlayer } from './StreamPlayer';
import {
  JoinError,
  friendlyError,
  joinRoom,
  loadViewerToken,
  parseInvite,
  saveViewerToken,
  scrubInviteFromUrl,
  type RoomView,
} from './watch';

type Phase =
  | { kind: 'joining' }
  | { kind: 'ready'; token: string }
  | { kind: 'failed'; message: string };

/** Player para quem recebeu um link de convite (`/watch#room=..&invite=..`). */
export function WatchApp() {
  const [roomId] = useState(() => parseInvite(location.hash)?.room ?? null);
  const [room, setRoom] = useState<RoomView | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: 'joining' });
  const [status, setStatus] = useState<ConnectionStatus | null>(null);
  const [fatal, setFatal] = useState(false);

  // Troca o convite (uso único) por um token de espectador desta aba.
  useEffect(() => {
    const params = parseInvite(location.hash);
    if (!params) {
      setPhase({ kind: 'failed', message: 'Link incompleto. Abra o link de convite exatamente como foi enviado.' });
      return;
    }
    const { room: id, invite } = params;
    scrubInviteFromUrl(id);
    if (!invite) {
      const saved = loadViewerToken(id);
      setPhase(
        saved
          ? { kind: 'ready', token: saved }
          : { kind: 'failed', message: 'Este link não tem convite. Peça um novo link a quem está transmitindo.' },
      );
      return;
    }
    let cancelled = false;
    joinRoom(id, invite)
      .then((joined) => {
        if (cancelled) return;
        saveViewerToken(id, joined.viewerToken);
        setRoom(joined.room);
        setPhase({ kind: 'ready', token: joined.viewerToken });
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        // Recarregar a página reaproveita o token já obtido nesta aba.
        const saved = loadViewerToken(id);
        if (saved) {
          setPhase({ kind: 'ready', token: saved });
        } else {
          const message = e instanceof JoinError ? friendlyError(e.code, e.status) : friendlyError(null);
          setPhase({ kind: 'failed', message });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const monitor = new GamepadMonitor(() => undefined);
    return () => monitor.dispose();
  }, []);

  const live = status === 'conectado';
  const notice =
    phase.kind === 'failed' ? phase.message : phase.kind === 'joining' ? 'Entrando na sala…' : null;

  return (
    <div className="watch">
      <header className="topbar">
        <span className="brand">
          <span className="brand-mark" aria-hidden />
          Vellcast
        </span>
        <span className="tag">{room?.name ?? (roomId ? 'Sala privada' : '')}</span>
        <span className="spacer" />
        <span className={`badge ${live ? 'live' : ''}`} role="status">
          {phase.kind === 'failed' || fatal ? 'Indisponível' : status ? STATUS_LABEL[status] : 'Conectando…'}
        </span>
      </header>

      <main className="watch-main">
        <StreamPlayer
          token={phase.kind === 'ready' ? phase.token : ''}
          notice={notice}
          onStatus={setStatus}
          onFatal={(code) => {
            setFatal(true);
            if (roomId && (code === 'unauthorized' || code === 'room_closed' || code === 'room_not_found')) {
              saveViewerToken(roomId, null);
            }
          }}
        />
      </main>

      <footer className="statusbar">
        <span className={`dot ${live ? 'ok' : status ? 'warn' : ''}`} aria-hidden />
        <span>Transmissão privada · nada é gravado</span>
        <span className="spacer" />
        <span className="hint">
          <kbd>A</kbd> confirmar · <kbd>D-pad</kbd> navegar
        </span>
      </footer>
    </div>
  );
}
