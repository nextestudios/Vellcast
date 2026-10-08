import { useCallback, useEffect, useRef, useState } from 'react';
import { Connection, type ConnectionStatus } from './connection';
import { initDiscord, type DiscordContext } from './discord';
import { GamepadMonitor, type GamepadReport } from './gamepad';
import { Player } from './player';
import type { MetricsSnapshot } from './metrics';

interface StreamInfo {
  id: number;
  name: string;
  width: number;
  height: number;
  fps: number;
  codec: string;
  hasAudio: boolean;
  videoBitrateBps: number;
}

const METRICS_INTERVAL_MS = 500;

const fmt = (v: number | null | undefined, digits = 0) => (v == null ? '—' : v.toFixed(digits));

export function App() {
  const [discord, setDiscord] = useState<DiscordContext | null>(null);
  const [streams, setStreams] = useState<StreamInfo[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [status, setStatus] = useState<{ s: ConnectionStatus; detail?: string } | null>(null);
  const [metrics, setMetrics] = useState<MetricsSnapshot | null>(null);
  const [gamepad, setGamepad] = useState<GamepadReport | null>(null);
  const [volume, setVolume] = useState(0.8);
  const [copied, setCopied] = useState<string | null>(null);

  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const playerRef = useRef<Player | null>(null);
  const connRef = useRef<Connection | null>(null);
  // Após o primeiro gesto do usuário, novas transmissões já iniciam com áudio.
  const audioWantedRef = useRef(false);

  useEffect(() => {
    void initDiscord().then(setDiscord);
    fetch('/api/streams')
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`))))
      .then((list: StreamInfo[]) => setStreams(list))
      .catch((e: Error) => setListError(e.message));
    const monitor = new GamepadMonitor(setGamepad);
    return () => monitor.dispose();
  }, []);

  // Conecta à transmissão selecionada; desmonta tudo ao trocar ou sair.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!selected || !canvas) return;
    let conn: Connection | null = null;
    const player = new Player(canvas, {
      serverToLocalMs: (us) => conn?.serverToLocalMs(us) ?? null,
      requestKeyframe: () => conn?.requestKeyframe(),
    });
    player.setVolume(volume);
    if (audioWantedRef.current) void player.enableAudio();
    conn = new Connection(selected, {
      onPacket: (p) => player.handle(p),
      onConnected: () => player.reset(),
      onStatus: (s, detail) => setStatus({ s, detail }),
    });
    playerRef.current = player;
    connRef.current = conn;

    const timer = window.setInterval(() => {
      if (document.hidden) return; // Sem atualizações de UI com a Activity oculta.
      const st = player.stats;
      const init = player.videoInit;
      setMetrics({
        stream: selected,
        codec: init?.codec ?? null,
        resolution: init ? `${init.codedWidth}x${init.codedHeight}@${init.framerate}` : null,
        videoSupported: player.videoSupported,
        audioSupported: player.audioSupported,
        receivedFps: st.received.rate(),
        decodedFps: st.decoded.rate(),
        framesDecoded: st.framesDecoded,
        framesDropped: st.framesDropped,
        decodeErrors: st.decodeErrors,
        decodeQueue: player.decodeQueue(),
        bitrateKbps: (st.bytes.sumPerSecond() * 8) / 1000,
        latencyMs: player.latencySummary(),
        rttMs: conn?.rttMs() ?? null,
        clockOffsetMs: conn?.clockOffsetMs() ?? null,
        audioState: player.audioState(),
        avSkewMs: player.avSkewMs(),
        audioUnderruns: st.audioUnderruns,
        reconnects: conn?.reconnects ?? 0,
        keyframeRequests: st.keyframeRequests,
        userAgent: navigator.userAgent,
      });
    }, METRICS_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
      conn?.close();
      player.close();
      playerRef.current = null;
      connRef.current = null;
      setMetrics(null);
      setStatus(null);
    };
    // O volume é aplicado separadamente; não deve recriar a conexão.
  }, [selected]);

  const enableAudio = useCallback(() => {
    audioWantedRef.current = true;
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
    else el.requestFullscreen().catch(() => setCopied('Tela cheia bloqueada neste ambiente'));
  };

  const copyReport = async () => {
    const report = JSON.stringify({ at: new Date().toISOString(), discord, metrics, gamepad }, null, 2);
    console.info('[vellcast] relatório', report);
    try {
      await navigator.clipboard.writeText(report);
      setCopied('Relatório copiado');
    } catch {
      setCopied('Área de transferência bloqueada — relatório registrado no console');
    }
  };

  const lat = metrics?.latencyMs;

  return (
    <div className="app">
      <header className="topbar">
        <span className="brand">
          <span className="brand-mark" aria-hidden />
          Vellcast
        </span>
        <span className="tag">ETAPA 0 · validação</span>
        <span className="spacer" />
        <DiscordBadge ctx={discord} />
      </header>

      <aside className="rooms" aria-label="Transmissões">
        <h2>Transmissões de teste</h2>
        {listError && <p className="error">Falha ao listar: {listError}</p>}
        {!streams && !listError && <p className="muted">Carregando…</p>}
        <ul>
          {streams?.map((s) => (
            <li key={s.id}>
              <button
                className={`room ${selected === s.name ? 'selected' : ''}`}
                aria-pressed={selected === s.name}
                onClick={() => setSelected(selected === s.name ? null : s.name)}
              >
                <span className="room-name">{s.name}</span>
                <span className="room-meta">
                  {s.width}×{s.height} · {s.fps} fps · {(s.videoBitrateBps / 1e6).toFixed(1)} Mbps
                  {s.hasAudio ? ' · áudio' : ''}
                </span>
              </button>
            </li>
          ))}
        </ul>
        <p className="muted small">Mídia sintética gerada por scripts/generate-test-media.sh.</p>
      </aside>

      <main className="center">
        <div className="stage" ref={stageRef}>
          <canvas ref={canvasRef} className={selected ? '' : 'hidden'} />
          {!selected && <p className="placeholder">Selecione uma transmissão para assistir.</p>}
          {status && status.s !== 'conectado' && selected && (
            <div className="overlay">
              {status.s}
              {status.detail ? ` — ${status.detail}` : ''}
            </div>
          )}
        </div>
        <div className="controls">
          <button onClick={enableAudio} disabled={!selected}>
            Ativar áudio
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
          <button onClick={fullscreen} disabled={!selected}>
            Tela cheia
          </button>
          <button onClick={() => void copyReport()}>Copiar relatório</button>
          {copied && <span className="muted small">{copied}</span>}
        </div>
      </main>

      <section className="diagnostics" aria-label="Diagnóstico">
        <h2>Métricas</h2>
        <dl>
          <Row k="Codec" v={metrics ? `${metrics.codec ?? '—'} (${supportLabel(metrics.videoSupported)})` : '—'} />
          <Row k="Resolução" v={metrics?.resolution ?? '—'} />
          <Row k="FPS recebido / decodificado" v={`${fmt(metrics?.receivedFps)} / ${fmt(metrics?.decodedFps)}`} />
          <Row k="Bitrate" v={`${fmt(metrics?.bitrateKbps)} kbps`} />
          <Row k="Latência média / p95" v={`${fmt(lat?.mean)} / ${fmt(lat?.p95)} ms`} />
          <Row k="RTT" v={`${fmt(metrics?.rttMs, 1)} ms`} />
          <Row k="Quadros descartados" v={String(metrics?.framesDropped ?? '—')} />
          <Row k="Fila do decoder" v={String(metrics?.decodeQueue ?? '—')} />
          <Row k="Erros de decodificação" v={String(metrics?.decodeErrors ?? '—')} />
          <Row k="Áudio" v={metrics ? `${metrics.audioState} (${supportLabel(metrics.audioSupported)})` : '—'} />
          <Row k="A/V skew" v={`${fmt(metrics?.avSkewMs)} ms`} />
          <Row k="Underruns de áudio" v={String(metrics?.audioUnderruns ?? '—')} />
          <Row k="Reconexões" v={String(metrics?.reconnects ?? '—')} />
        </dl>
        <p className="muted small">
          Latência = envio no servidor → quadro decodificado. Não inclui captura nem codificação.
        </p>

        <h2>Controle (Gamepad API)</h2>
        <dl>
          <Row k="API disponível" v={gamepad ? (gamepad.apiAvailable ? 'sim' : 'não') : '—'} />
          <Row
            k="Permissions Policy"
            v={gamepad?.policyAllowed == null ? 'não informado' : gamepad.policyAllowed ? 'permitido' : 'bloqueado'}
          />
          <Row k="Eventos de conexão" v={String(gamepad?.events ?? 0)} />
        </dl>
        {gamepad?.pads.length ? (
          <ul className="pads">
            {gamepad.pads.map((p) => (
              <li key={p.index}>
                #{p.index} {p.id} ({p.mapping}) — botões: {p.pressed.join(', ') || 'nenhum'}
              </li>
            ))}
          </ul>
        ) : (
          <p className="muted small">Nenhum controle detectado. Pressione um botão do controle com a Activity em foco.</p>
        )}
      </section>

      <footer className="statusbar">
        <span className={`dot ${status?.s === 'conectado' ? 'ok' : status ? 'warn' : ''}`} aria-hidden />
        <span>{status ? status.s : 'ocioso'}</span>
        <span className="spacer" />
        <span className="hint">
          <kbd>A</kbd> confirmar · <kbd>B</kbd> voltar · <kbd>D-pad</kbd> navegar · <kbd>Tab</kbd> teclado
        </span>
      </footer>
    </div>
  );
}

function supportLabel(v: boolean | null): string {
  return v === null ? 'verificando' : v ? 'suportado' : 'não suportado';
}

function Row({ k, v }: { k: string; v: string }) {
  return (
    <>
      <dt>{k}</dt>
      <dd>{v}</dd>
    </>
  );
}

function DiscordBadge({ ctx }: { ctx: DiscordContext | null }) {
  if (!ctx) return <span className="badge">Discord: verificando…</span>;
  if (ctx.mode === 'standalone') return <span className="badge">Modo navegador (fora do Discord)</span>;
  if (ctx.error) return <span className="badge error">Discord SDK: {ctx.error}</span>;
  return (
    <span className="badge ok" title={`instância ${ctx.instanceId}`}>
      Discord SDK pronto · {ctx.platform}
    </span>
  );
}
