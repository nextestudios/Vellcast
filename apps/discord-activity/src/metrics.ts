// Métricas do player, agregadas em janelas deslizantes.

export const nowMs = (): number => performance.timeOrigin + performance.now();

/** Amostras com timestamp, mantidas apenas dentro de uma janela de tempo. */
export class Window {
  private samples: { t: number; v: number }[] = [];
  constructor(private readonly spanMs: number) {}

  push(v: number, t = nowMs()): void {
    this.samples.push({ t, v });
    this.trim(t);
  }

  private trim(t: number): void {
    const cutoff = t - this.spanMs;
    let i = 0;
    while (i < this.samples.length && this.samples[i].t < cutoff) i++;
    if (i > 0) this.samples.splice(0, i);
  }

  values(): number[] {
    this.trim(nowMs());
    return this.samples.map((s) => s.v);
  }

  /** Eventos por segundo dentro da janela. */
  rate(): number {
    return (this.values().length * 1000) / this.spanMs;
  }

  sumPerSecond(): number {
    return (this.values().reduce((a, b) => a + b, 0) * 1000) / this.spanMs;
  }
}

export function percentile(values: number[], p: number): number | null {
  if (values.length === 0) return null;
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length))];
}

export function mean(values: number[]): number | null {
  return values.length ? values.reduce((a, b) => a + b, 0) / values.length : null;
}

export interface MetricsSnapshot {
  stream: string;
  codec: string | null;
  resolution: string | null;
  videoSupported: boolean | null;
  audioSupported: boolean | null;
  receivedFps: number;
  decodedFps: number;
  framesDecoded: number;
  framesDropped: number;
  decodeErrors: number;
  decodeQueue: number;
  bitrateKbps: number;
  /** Latência servidor → quadro decodificado (ms), janela de 5 s. */
  latencyMs: { mean: number | null; p95: number | null; last: number | null };
  rttMs: number | null;
  clockOffsetMs: number | null;
  audioState: string;
  avSkewMs: number | null;
  audioUnderruns: number;
  reconnects: number;
  keyframeRequests: number;
  userAgent: string;
}
