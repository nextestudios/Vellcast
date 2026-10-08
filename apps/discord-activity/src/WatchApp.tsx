import { useCallback, useEffect, useRef, useState } from 'react';
import { Connection, wsUrl, type ConnectionStatus } from './connection';
import { GamepadMonitor } from './gamepad';
import { Player } from './player';
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

interface Stats {
  resolution: string | null;
  decodedFps: number;
  bitrateKbps: number;
  latencyMs: number | null;
  rttMs: number | null;
  dropped: number;
  videoSupported: boolean | null;
  audioSupported: boolean | null;
}

const STATUS_LABEL: Record<ConnectionStatus, string> = {
  conectando: 'Conectando…',
  conectado: 'Ao vivo',
  reconectando: 'Reconectando…',
  aguardando: 'Aguardando transmissão',
  encerrado: 'Desconectado',
};

const fmt = (v: number | null | undefined, digits = 0) => (v == null ? '—' : v.toFixed(digits));

/** Player para quem recebeu um link de convite (`/watch#room=..&invite=..`). */
export function WatchApp() {
  const [roomId] = useState(() => parseInvite(location.hash)?.room ?? null);
  const [room, setRoom] = useState<RoomView | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: 'joining' });
  const [status, setStatus] = useState<{ s: ConnectionStatus; detail?: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [showStats, setShowStats] = useState(false);
  const [audioOn, setAudioOn] = useState(false);
  const [volume, setVolume] = useState(0.8);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<Player | null>(null);

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

  useEffect(() => {
    const canvas = canvasRef.current;
    if (phase.kind !== 'ready' || !canvas || !roomId) return;
    let conn: Connection | null = null;
    const player = new Player(canvas, {
      serverToLocalMs: (us) => conn?.serverToLocalMs(us) ?? null,
      requestKeyframe: () => conn?.requestKeyframe(),
    });
    player.setVolume(volume);
    playerRef.current = player;
    conn = new Connection(
      { url: wsUrl(), viewerToken: phase.token },
      {
        onPacket: (p) => player.handle(p),
        onConnected: () => {
          setError(null);
          player.reset();
        },
        onStatus: (s, detail) => setStatus({ s, detail }),
        onError: (code, _message, fatal) => {
          if (!fatal) return;
          if (code === 'unauthorized' || code === 'room_closed' || code === 'room_not_found') {
            saveViewerToken(roomId, null);
          }
          setError(friendlyError(code));
        },
      },
    );

    const timer = window.setInterval(() => {
      if (document.hidden) return;
      const st = player.stats;
      const init = player.videoInit;
      setStats({
        resolution: init ? `${init.codedWidth}×${init.codedHeight} @ ${init.framerate}` : null,
        decodedFps: st.decoded.rate(),
        bitrateKbps: (st.bytes.sumPerSecond() * 8) / 1000,
        latencyMs: player.latencySummary().p95,
        rttMs: conn?.rttMs() ?? null,
        dropped: st.framesDropped,
        videoSupported: player.videoSupported,
        audioSupported: player.audioSupported,
      });
    }, 1000);

    return () => {
      window.clearInterval(timer);
      conn?.close();
      player.close();
      playerRef.current = null;
    };
    // O volume é aplicado separadamente; não deve recriar a conexão.
  }, [phase, roomId]);

  const enableAudio = useCallback(() => {
    setAudioOn(true);
    void playerRef.current?.enableAudio();
  }, []);

  const changeVolume = (v: number) => {
    setVolume(v);
    playerRef.current?.setVolume(v);
  };

  const fullscreen = () => {
    const el = stageRef.current;
    if (!el) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else el.requestFullscreen().catch(() => setError('Tela cheia bloqueada neste navegador.'));
  };

  const live = status?.s === 'conectado';
  const overlay =
    phase.kind === 'failed'
      ? phase.message
      : error ??
        (phase.kind === 'joining'
          ? 'Entrando na sala…'
          : status && status.s !== 'conectado'
            ? `${STATUS_LABEL[status.s]}${status.detail ? ` — ${status.detail}` : ''}`
            : null);
  const unsupported = stats?.videoSupported === false;

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
          {phase.kind === 'failed' || error ? 'Indisponível' : status ? STATUS_LABEL[status.s] : 'Conectando…'}
        </span>
      </header>

      <main className="watch-main">
        <div className="stage" ref={stageRef}>
          <canvas ref={canvasRef} />
          {overlay && <div className={`watch-overlay ${phase.kind === 'failed' || error ? 'error' : ''}`}>{overlay}</div>}
          {unsupported && (
            <div className="watch-overlay error">
              Este navegador não decodifica H.264 via WebCodecs. Use Chrome, Edge ou outro navegador baseado em Chromium.
            </div>
          )}
        </div>

        <div className="controls">
          <button onClick={enableAudio} disabled={phase.kind !== 'ready' || audioOn}>
            {audioOn ? 'Áudio ativado' : 'Ativar áudio'}
          </button>
          <label className="volume">
            Volume
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={volume}
              onChange={(e) => changeVolume(Number(e.target.value))}
            />
          </label>
          <button onClick={fullscreen} disabled={phase.kind !== 'ready'}>
            Tela cheia
          </button>
          <button onClick={() => setShowStats((v) => !v)} aria-pressed={showStats}>
            Estatísticas
          </button>
        </div>

        {showStats && (
          <dl className="watch-stats">
            <dt>Resolução</dt>
            <dd>{stats?.resolution ?? '—'}</dd>
            <dt>FPS</dt>
            <dd>{fmt(stats?.decodedFps)}</dd>
            <dt>Bitrate</dt>
            <dd>{fmt(stats?.bitrateKbps)} kbps</dd>
            <dt>Atraso (p95)</dt>
            <dd>{fmt(stats?.latencyMs)} ms</dd>
            <dt>RTT</dt>
            <dd>{fmt(stats?.rttMs, 1)} ms</dd>
            <dt>Quadros descartados</dt>
            <dd>{stats?.dropped ?? '—'}</dd>
            <dt>Áudio</dt>
            <dd>{stats?.audioSupported === false ? 'decoder indisponível' : audioOn ? 'ativo' : 'desativado'}</dd>
          </dl>
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
