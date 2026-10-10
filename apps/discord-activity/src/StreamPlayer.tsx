import { useCallback, useEffect, useRef, useState } from 'react';
import { Connection, wsUrl, type ConnectionStatus } from './connection';
import { Player } from './player';
import type { ErrorCode, Presence } from './protocol';
import { friendlyError } from './watch';

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

export const STATUS_LABEL: Record<ConnectionStatus, string> = {
  conectando: 'Conectando…',
  conectado: 'Ao vivo',
  reconectando: 'Reconectando…',
  aguardando: 'Aguardando transmissão',
  encerrado: 'Desconectado',
};

const fmt = (v: number | null | undefined, digits = 0) => (v == null ? '—' : v.toFixed(digits));

export interface StreamPlayerProps {
  /** Token de espectador; trocar o valor reconecta com o novo token. */
  token: string;
  /** Mensagem fixa exibida sobre o vídeo (ex.: falha antes de conectar). */
  notice?: string | null;
  /** Erro fatal do relay (token expirado, sala encerrada…). */
  onFatal?(code: ErrorCode): void;
  onStatus?(status: ConnectionStatus): void;
  /** Participantes ativos da sala (mensagem `Presence` do relay). */
  onPresence?(presence: Presence): void;
}

/** Vídeo + áudio de uma transmissão do relay, com controles e estatísticas. */
export function StreamPlayer({ token, notice, onFatal, onStatus, onPresence }: StreamPlayerProps) {
  const [status, setStatus] = useState<{ s: ConnectionStatus; detail?: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);
  const [showStats, setShowStats] = useState(false);
  const [audioOn, setAudioOn] = useState(false);
  const [volume, setVolume] = useState(0.8);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<Player | null>(null);
  const volumeRef = useRef(volume);
  const audioRef = useRef(audioOn);
  const callbacks = useRef({ onFatal, onStatus, onPresence });
  callbacks.current = { onFatal, onStatus, onPresence };

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !token) return;
    let conn: Connection | null = null;
    const player = new Player(canvas, {
      serverToLocalMs: (us) => conn?.serverToLocalMs(us) ?? null,
      requestKeyframe: () => conn?.requestKeyframe(),
    });
    player.setVolume(volumeRef.current);
    if (audioRef.current) void player.enableAudio();
    playerRef.current = player;
    setError(null);
    conn = new Connection(
      { url: wsUrl(), viewerToken: token },
      {
        onPacket: (p) => player.handle(p),
        onConnected: () => {
          setError(null);
          player.reset();
        },
        onStatus: (s, detail) => {
          setStatus({ s, detail });
          callbacks.current.onStatus?.(s);
        },
        onError: (code, _message, fatal) => {
          if (!fatal) return;
          setError(friendlyError(code));
          callbacks.current.onFatal?.(code);
        },
        onPresence: (presence) => callbacks.current.onPresence?.(presence),
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
  }, [token]);

  const enableAudio = useCallback(() => {
    setAudioOn(true);
    audioRef.current = true;
    void playerRef.current?.enableAudio();
  }, []);

  const changeVolume = (v: number) => {
    setVolume(v);
    volumeRef.current = v;
    playerRef.current?.setVolume(v);
  };

  const fullscreen = () => {
    const el = stageRef.current;
    if (!el) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else el.requestFullscreen().catch(() => setError('Tela cheia bloqueada neste ambiente.'));
  };

  const overlay =
    notice ??
    error ??
    (status && status.s !== 'conectado'
      ? `${STATUS_LABEL[status.s]}${status.detail ? ` — ${status.detail}` : ''}`
      : null);
  const unsupported = stats?.videoSupported === false;

  return (
    <>
      <div className="stage" ref={stageRef}>
        <canvas ref={canvasRef} />
        {overlay && <div className={`watch-overlay ${notice || error ? 'error' : ''}`}>{overlay}</div>}
        {unsupported && (
          <div className="watch-overlay error">
            Este ambiente não decodifica H.264 via WebCodecs. Use o Discord para desktop ou um navegador baseado em
            Chromium.
          </div>
        )}
      </div>

      <div className="controls">
        <button onClick={enableAudio} disabled={!token || audioOn}>
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
        <button onClick={fullscreen} disabled={!token}>
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
    </>
  );
}
