import { useEffect, useState, type FormEvent } from 'react';
import type { ConnectionStatus } from './connection';
import { GamepadMonitor } from './gamepad';
import type { Participant, Presence } from './protocol';
import { STATUS_LABEL, StreamPlayer } from './StreamPlayer';
import {
  JoinError,
  MAX_GUEST_NAME,
  friendlyError,
  joinRoom,
  loadGuestName,
  loadPendingInvite,
  loadViewerToken,
  parseInvite,
  sanitizeGuestName,
  saveGuestName,
  savePendingInvite,
  saveViewerToken,
  scrubInviteFromUrl,
  type RoomView,
} from './watch';

type Phase =
  /** Convite válido em mãos; aguardando o nome. O convite ainda não foi usado. */
  | { kind: 'name'; invite: string; error: string | null; busy: boolean }
  | { kind: 'ready'; token: string }
  | { kind: 'failed'; message: string };

/** Abaixo desta largura a lista de participantes começa fechada (sobreposta ao vídeo). */
const NARROW_PX = 900;

function initialPhase(id: string | null, invite: string | null, token: string | null = null): Phase {
  if (!id) return { kind: 'failed', message: 'Link incompleto. Abra o link de convite exatamente como foi enviado.' };
  // Aberto pelo app desktop com o token do participante: o nome já está no
  // token (o do app) e nenhum convite é gasto, então não há formulário de nome.
  if (token) return { kind: 'ready', token };
  if (invite) return { kind: 'name', invite, error: null, busy: false };
  const saved = loadViewerToken(id);
  if (saved) return { kind: 'ready', token: saved };
  // Recarregar a página antes de enviar o nome: o convite guardado na aba continua valendo.
  const pending = loadPendingInvite(id);
  if (pending) return { kind: 'name', invite: pending, error: null, busy: false };
  return { kind: 'failed', message: 'Este link não tem convite. Peça um novo link a quem está transmitindo.' };
}

function participantLabel(p: Participant): string {
  return p.name ?? (p.role === 'publisher' ? 'Transmissor' : 'Convidado');
}

/** Player para quem recebeu um link de convite (`/watch#room=..&invite=..`). */
export function WatchApp() {
  const [params] = useState(() => parseInvite(location.hash));
  const roomId = params?.room ?? null;
  const [room, setRoom] = useState<RoomView | null>(null);
  const [phase, setPhase] = useState<Phase>(() => initialPhase(roomId, params?.invite ?? null, params?.token ?? null));
  const [status, setStatus] = useState<ConnectionStatus | null>(null);
  const [fatal, setFatal] = useState(false);
  const [name, setName] = useState(loadGuestName);
  const [presence, setPresence] = useState<Presence | null>(null);
  const [showPeople, setShowPeople] = useState(() => window.innerWidth >= NARROW_PX);
  const [sharer, setSharer] = useState<string | null>(null);

  // O código de convite sai da barra de endereços e do histórico na hora;
  // fica só nesta aba até o convidado enviar o nome.
  useEffect(() => {
    if (!params) return;
    if (params.token) saveViewerToken(params.room, params.token);
    else if (params.invite) savePendingInvite(params.room, params.invite);
    scrubInviteFromUrl(params.room);
  }, [params]);

  useEffect(() => {
    const monitor = new GamepadMonitor(() => undefined);
    return () => monitor.dispose();
  }, []);

  const submitName = (e: FormEvent) => {
    e.preventDefault();
    if (phase.kind !== 'name' || phase.busy || !roomId) return;
    const clean = sanitizeGuestName(name);
    if (!clean) {
      setPhase({ ...phase, error: 'Digite um nome para entrar.' });
      return;
    }
    const invite = phase.invite;
    setPhase({ kind: 'name', invite, error: null, busy: true });
    // Troca o convite (uso único) por um token de espectador desta aba.
    joinRoom(roomId, invite, clean)
      .then((joined) => {
        savePendingInvite(roomId, null);
        saveViewerToken(roomId, joined.viewerToken);
        saveGuestName(clean);
        setRoom(joined.room);
        setPhase({ kind: 'ready', token: joined.viewerToken });
      })
      .catch((err: unknown) => {
        if (err instanceof JoinError && err.status === 400) {
          // Nome recusado pelo relay: o convite não foi consumido.
          setPhase({
            kind: 'name',
            invite,
            busy: false,
            error: `Nome inválido. Use de 1 a ${MAX_GUEST_NAME} caracteres.`,
          });
          return;
        }
        if (err instanceof JoinError && err.status === 429) {
          setPhase({ kind: 'name', invite, busy: false, error: friendlyError('rate_limited') });
          return;
        }
        if (!(err instanceof JoinError)) {
          // Falha de rede: o convite pode não ter chegado ao relay; permite tentar de novo.
          setPhase({ kind: 'name', invite, busy: false, error: friendlyError(null) });
          return;
        }
        // Recarregar a página reaproveita o token já obtido nesta aba.
        const saved = loadViewerToken(roomId);
        if (saved) {
          setPhase({ kind: 'ready', token: saved });
        } else {
          savePendingInvite(roomId, null);
          setPhase({ kind: 'failed', message: friendlyError(err.code, err.status) });
        }
      });
  };

  const live = status === 'conectado';
  const ready = phase.kind === 'ready';
  const notice = phase.kind === 'failed' ? phase.message : null;
  const count = presence?.count ?? 0;
  const badge =
    phase.kind === 'failed' || fatal
      ? 'Indisponível'
      : phase.kind === 'name'
        ? 'Aguardando seu nome'
        : status
          ? STATUS_LABEL[status]
          : 'Conectando…';

  return (
    <div className="watch watch-fit">
      <header className="topbar">
        <span className="brand">
          <span className="brand-mark" aria-hidden />
          Vellcast
        </span>
        <span className="tag">{room?.name ?? (roomId ? 'Sala privada' : '')}</span>
        {live && sharer && <span className="tag">{sharer} está compartilhando</span>}
        <span className="spacer" />
        {ready && (
          <button
            className="people-toggle"
            onClick={() => setShowPeople((v) => !v)}
            aria-expanded={showPeople}
            aria-controls="participants"
          >
            Participantes <span className="count">{count}</span>
          </button>
        )}
        <span className={`badge ${live ? 'live' : ''}`} role="status">
          {badge}
        </span>
      </header>

      <main className={`watch-main ${ready && showPeople ? 'with-people' : ''}`}>
        {phase.kind === 'name' ? (
          <div className="join-screen">
            <form className="join-card" onSubmit={submitName} noValidate>
              <h1>Entrar na sala</h1>
              <label htmlFor="guest-name">Seu nome</label>
              <input
                id="guest-name"
                className="join-input"
                value={name}
                maxLength={MAX_GUEST_NAME}
                autoComplete="nickname"
                autoFocus
                placeholder="Como você quer aparecer"
                aria-invalid={phase.error ? true : undefined}
                aria-describedby="guest-name-help"
                onChange={(e) => setName(e.target.value)}
              />
              <p id="guest-name-help" className="muted small">
                Os outros participantes desta sala verão este nome.
              </p>
              {phase.error && (
                <p className="error small" role="alert">
                  {phase.error}
                </p>
              )}
              <button type="submit" className="primary" disabled={phase.busy || !sanitizeGuestName(name)}>
                {phase.busy ? 'Entrando…' : 'Entrar'}
              </button>
            </form>
          </div>
        ) : (
          <div className="watch-player">
            <StreamPlayer
              token={phase.kind === 'ready' ? phase.token : ''}
              notice={notice}
              onStatus={setStatus}
              onPresence={setPresence}
              onSharer={setSharer}
              onFatal={(code) => {
                setFatal(true);
                setPresence(null);
                if (roomId && (code === 'unauthorized' || code === 'room_closed' || code === 'room_not_found')) {
                  saveViewerToken(roomId, null);
                }
              }}
            />
          </div>
        )}

        {ready && showPeople && (
          <aside id="participants" className="participants" aria-label="Participantes">
            <h2>
              Participantes <span className="count">{count}</span>
            </h2>
            {presence && presence.participants.length > 0 ? (
              <ul>
                {presence.participants.map((p) => (
                  <li key={p.id} className={p.name ? '' : 'anonymous'}>
                    <span className="avatar" aria-hidden>
                      {Array.from(participantLabel(p))[0]?.toUpperCase()}
                    </span>
                    <span className="participant-name" title={participantLabel(p)}>
                      {participantLabel(p)}
                    </span>
                    {p.role === 'publisher' && <span className="role">transmitindo</span>}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="muted small">{live || status ? 'Ninguém na sala ainda.' : 'Conectando…'}</p>
            )}
            {presence && presence.count > presence.participants.length && (
              <p className="muted small">e mais {presence.count - presence.participants.length}</p>
            )}
          </aside>
        )}
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
