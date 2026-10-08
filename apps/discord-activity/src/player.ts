// Player WebCodecs: H.264 → canvas, Opus → Web Audio.
//
// Política de baixa latência: cada quadro decodificado é desenhado
// imediatamente; o áudio é agendado com um jitter buffer pequeno.
// Se o decoder acumular quadros, os quadros delta são descartados até
// o próximo keyframe (nunca cresce uma fila sem limite).

import { Flags, MessageType, type Packet } from './protocol';
import { Window, mean, nowMs, percentile } from './metrics';

/** Quadros pendentes no decoder acima disso disparam descarte até o próximo keyframe. */
const MAX_DECODE_QUEUE = 4;
/**
 * Ao entrar, o relay envia de uma vez o GOP em cache (até ~1 s de vídeo) para
 * a imagem aparecer na hora. Nessa janela a fila do decoder cresce de propósito
 * e não deve disparar descarte.
 */
const CATCH_UP_WINDOW_MS = 1500;
const CATCH_UP_MAX_QUEUE = 120;
/** Jitter buffer de áudio (s). */
const AUDIO_LEAD_S = 0.06;
/** Áudio agendado além disso é considerado atrasado e o relógio é reiniciado. */
const AUDIO_MAX_AHEAD_S = 0.3;
/** Limite de PTS rastreados para o cálculo de latência. */
const MAX_PENDING_PTS = 120;
/** Reinicializações do decoder após erro antes de desistir. */
const MAX_DECODER_RESTARTS = 20;

interface VideoInit {
  codec: string;
  codedWidth: number;
  codedHeight: number;
  framerate: number;
}

interface AudioInit {
  codec: string;
  sampleRate: number;
  numberOfChannels: number;
}

export interface PlayerHooks {
  /** Converte um timestamp do servidor (µs) para o relógio local (ms). */
  serverToLocalMs(serverUs: number): number | null;
  requestKeyframe(): void;
}

export class Player {
  private video: VideoDecoder | null = null;
  private audio: AudioDecoder | null = null;
  private audioCtx: AudioContext | null = null;
  private gain: GainNode | null = null;
  private nextAudioTime = 0;
  /** (instante do AudioContext, PTS em µs) dos blocos agendados, para medir A/V skew. */
  private audioSchedule: { at: number; ptsUs: number }[] = [];
  private lastVideoPtsUs: number | null = null;
  private waitingKeyframe = true;
  private catchUpUntil = 0;
  private sentAtByPts = new Map<number, number>();
  private ctx2d: CanvasRenderingContext2D | null;
  private volume = 0.8;

  videoInit: VideoInit | null = null;
  videoSupported: boolean | null = null;
  audioSupported: boolean | null = null;

  readonly stats = {
    received: new Window(2000),
    decoded: new Window(2000),
    bytes: new Window(2000),
    latency: new Window(5000),
    lastLatency: null as number | null,
    framesDecoded: 0,
    framesDropped: 0,
    decodeErrors: 0,
    audioUnderruns: 0,
    keyframeRequests: 0,
  };

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly hooks: PlayerHooks,
  ) {
    this.ctx2d = canvas.getContext('2d', { alpha: false, desynchronized: true });
  }

  handle(packet: Packet): void {
    const { header, payload } = packet;
    switch (header.type) {
      case MessageType.VideoInit:
        this.configureVideo(JSON.parse(new TextDecoder().decode(payload)) as VideoInit);
        break;
      case MessageType.AudioInit:
        this.configureAudio(JSON.parse(new TextDecoder().decode(payload)) as AudioInit);
        break;
      case MessageType.VideoFrame:
        this.onVideoFrame(packet);
        break;
      case MessageType.AudioFrame:
        this.onAudioFrame(packet);
        break;
    }
  }

  private configureVideo(init: VideoInit): void {
    this.videoInit = init;
    const config: VideoDecoderConfig = {
      codec: init.codec,
      codedWidth: init.codedWidth,
      codedHeight: init.codedHeight,
      optimizeForLatency: true,
      // Sem `description`: o fluxo é Annex B, com SPS/PPS em cada keyframe.
    };
    if (typeof VideoDecoder === 'undefined') {
      this.videoSupported = false;
      return;
    }
    // A verificação de suporte roda em paralelo para não perder o primeiro keyframe.
    VideoDecoder.isConfigSupported(config).then(
      (s) => (this.videoSupported = s.supported === true),
      () => (this.videoSupported = false),
    );

    this.video?.close();
    this.video = new VideoDecoder({
      output: (frame) => this.renderFrame(frame),
      error: (e) => {
        console.warn('[vellcast] erro no VideoDecoder', e);
        this.stats.decodeErrors++;
        this.video = null;
        // Reconfigura e retoma a partir do próximo keyframe, com limite de tentativas.
        if (this.videoInit && this.videoSupported !== false && this.stats.decodeErrors < MAX_DECODER_RESTARTS) {
          this.configureVideo(this.videoInit);
        }
      },
    });
    this.video.configure(config);
    this.waitingKeyframe = true;
    this.canvas.width = init.codedWidth;
    this.canvas.height = init.codedHeight;
  }

  private onVideoFrame({ header, payload }: Packet): void {
    this.stats.received.push(1);
    this.stats.bytes.push(payload.byteLength);
    const decoder = this.video;
    if (!decoder || decoder.state !== 'configured') return;

    const keyframe = (header.flags & Flags.Keyframe) !== 0;
    if (keyframe && this.waitingKeyframe) this.catchUpUntil = nowMs() + CATCH_UP_WINDOW_MS;
    const limit = nowMs() < this.catchUpUntil ? CATCH_UP_MAX_QUEUE : MAX_DECODE_QUEUE;
    if (!keyframe && decoder.decodeQueueSize > limit) {
      this.waitingKeyframe = true;
      this.hooks.requestKeyframe();
      this.stats.keyframeRequests++;
    }
    if (this.waitingKeyframe) {
      if (!keyframe) {
        this.stats.framesDropped++;
        return;
      }
      this.waitingKeyframe = false;
    }

    this.sentAtByPts.set(header.ptsUs, header.sentAtUs);
    if (this.sentAtByPts.size > MAX_PENDING_PTS) {
      const oldest = this.sentAtByPts.keys().next().value;
      if (oldest !== undefined) this.sentAtByPts.delete(oldest);
    }
    decoder.decode(
      new EncodedVideoChunk({ type: keyframe ? 'key' : 'delta', timestamp: header.ptsUs, data: payload }),
    );
  }

  private renderFrame(frame: VideoFrame): void {
    const now = nowMs();
    const sentAt = this.sentAtByPts.get(frame.timestamp);
    if (sentAt !== undefined) {
      this.sentAtByPts.delete(frame.timestamp);
      const sentLocal = this.hooks.serverToLocalMs(sentAt);
      if (sentLocal !== null) {
        const latency = now - sentLocal;
        this.stats.latency.push(latency, now);
        this.stats.lastLatency = latency;
      }
    }
    if (this.canvas.width !== frame.displayWidth || this.canvas.height !== frame.displayHeight) {
      this.canvas.width = frame.displayWidth;
      this.canvas.height = frame.displayHeight;
    }
    this.ctx2d?.drawImage(frame, 0, 0);
    this.lastVideoPtsUs = frame.timestamp;
    this.stats.decoded.push(1, now);
    this.stats.framesDecoded++;
    frame.close();
  }

  private configureAudio(init: AudioInit): void {
    if (typeof AudioDecoder === 'undefined') {
      this.audioSupported = false;
      return;
    }
    const config: AudioDecoderConfig = {
      codec: init.codec,
      sampleRate: init.sampleRate,
      numberOfChannels: init.numberOfChannels,
    };
    AudioDecoder.isConfigSupported(config).then(
      (s) => (this.audioSupported = s.supported === true),
      () => (this.audioSupported = false),
    );

    this.audio?.close();
    this.audio = new AudioDecoder({
      output: (data) => this.playAudio(data),
      error: (e) => {
        console.warn('[vellcast] erro no AudioDecoder', e);
        this.audio = null;
      },
    });
    this.audio.configure(config);
  }

  private onAudioFrame({ header, payload }: Packet): void {
    // Sem AudioContext ativo (precisa de gesto do usuário) o áudio não é decodificado.
    if (!this.audio || this.audio.state !== 'configured' || this.audioCtx?.state !== 'running') return;
    this.audio.decode(new EncodedAudioChunk({ type: 'key', timestamp: header.ptsUs, data: payload }));
  }

  private playAudio(data: AudioData): void {
    const ctx = this.audioCtx;
    if (!ctx || !this.gain) {
      data.close();
      return;
    }
    const buffer = ctx.createBuffer(data.numberOfChannels, data.numberOfFrames, data.sampleRate);
    for (let ch = 0; ch < data.numberOfChannels; ch++) {
      const plane = new Float32Array(data.numberOfFrames);
      data.copyTo(plane, { planeIndex: ch, format: 'f32-planar' });
      buffer.copyToChannel(plane, ch);
    }
    const ptsUs = data.timestamp;
    data.close();

    const now = ctx.currentTime;
    if (this.nextAudioTime < now) {
      if (this.nextAudioTime > 0) this.stats.audioUnderruns++;
      this.nextAudioTime = now + AUDIO_LEAD_S;
    } else if (this.nextAudioTime - now > AUDIO_MAX_AHEAD_S) {
      // Acúmulo excessivo: descarta este bloco e reancora o relógio.
      this.nextAudioTime = now + AUDIO_LEAD_S;
      return;
    }
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(this.gain);
    source.start(this.nextAudioTime);
    this.audioSchedule.push({ at: this.nextAudioTime, ptsUs });
    if (this.audioSchedule.length > 50) this.audioSchedule.shift();
    this.nextAudioTime += buffer.duration;
  }

  /** Deve ser chamado a partir de um gesto do usuário (política de autoplay). */
  async enableAudio(): Promise<void> {
    if (!this.audioCtx) {
      this.audioCtx = new AudioContext({ sampleRate: 48000, latencyHint: 'interactive' });
      this.gain = this.audioCtx.createGain();
      this.gain.gain.value = this.volume;
      this.gain.connect(this.audioCtx.destination);
    }
    await this.audioCtx.resume();
  }

  setVolume(v: number): void {
    this.volume = v;
    if (this.gain) this.gain.gain.value = v;
  }

  audioState(): string {
    if (this.audioSupported === false) return 'decoder indisponível';
    return this.audioCtx?.state ?? 'desativado';
  }

  /** Diferença entre o PTS do vídeo exibido e o PTS do áudio audível (ms). Positivo = vídeo adiantado. */
  avSkewMs(): number | null {
    const ctx = this.audioCtx;
    if (!ctx || ctx.state !== 'running' || this.lastVideoPtsUs === null) return null;
    const t = ctx.currentTime - (ctx.outputLatency || ctx.baseLatency || 0);
    let playing: { at: number; ptsUs: number } | undefined;
    for (const s of this.audioSchedule) {
      if (s.at <= t) playing = s;
      else break;
    }
    if (!playing) return null;
    const audioPtsUs = playing.ptsUs + (t - playing.at) * 1e6;
    return (this.lastVideoPtsUs - audioPtsUs) / 1000;
  }

  latencySummary() {
    const values = this.stats.latency.values();
    return { mean: mean(values), p95: percentile(values, 95), last: this.stats.lastLatency };
  }

  decodeQueue(): number {
    return this.video?.decodeQueueSize ?? 0;
  }

  /** Descarta o estado de decodificação (nova conexão ou nova transmissão). */
  reset(): void {
    this.video?.close();
    this.audio?.close();
    this.video = null;
    this.audio = null;
    this.sentAtByPts.clear();
    this.audioSchedule = [];
    this.nextAudioTime = 0;
    this.lastVideoPtsUs = null;
    this.waitingKeyframe = true;
  }

  close(): void {
    this.reset();
    void this.audioCtx?.close();
    this.audioCtx = null;
  }
}
