// Verificação da Gamepad API dentro da Activity e navegação por foco.
//
// O polling só roda enquanto houver controle conectado (sem rAF ocioso).

export interface GamepadReport {
  apiAvailable: boolean;
  /** Resultado da Permissions Policy para `gamepad`, quando o navegador expõe a consulta. */
  policyAllowed: boolean | null;
  pads: { index: number; id: string; mapping: string; pressed: number[] }[];
  events: number;
}

type PolicyDocument = Document & {
  featurePolicy?: { allowsFeature(feature: string): boolean };
  permissionsPolicy?: { allowsFeature(feature: string): boolean };
};

const REPEAT_DELAY_MS = 350;
const REPEAT_RATE_MS = 120;
const AXIS_THRESHOLD = 0.5;

// Mapeamento padrão (https://w3c.github.io/gamepad/#remapping)
const BUTTON = { A: 0, B: 1, UP: 12, DOWN: 13, LEFT: 14, RIGHT: 15 } as const;

function policyAllowed(): boolean | null {
  const doc = document as PolicyDocument;
  const policy = doc.permissionsPolicy ?? doc.featurePolicy;
  try {
    return policy ? policy.allowsFeature('gamepad') : null;
  } catch {
    return null;
  }
}

function focusables(): HTMLElement[] {
  return Array.from(
    document.querySelectorAll<HTMLElement>('button:not([disabled]), select, input, [tabindex="0"]'),
  ).filter((el) => el.offsetParent !== null);
}

function moveFocus(delta: number): void {
  const list = focusables();
  if (list.length === 0) return;
  const i = list.indexOf(document.activeElement as HTMLElement);
  const next = list[(i + delta + list.length) % list.length];
  next.focus();
}

export class GamepadMonitor {
  private raf = 0;
  private events = 0;
  private held = new Map<string, number>();
  private lastReport = '';

  constructor(private readonly onChange: (r: GamepadReport) => void) {
    window.addEventListener('gamepadconnected', this.onConnect);
    window.addEventListener('gamepaddisconnected', this.onConnect);
    this.report();
  }

  private onConnect = (): void => {
    this.events++;
    this.report();
    const any = this.pads().length > 0;
    if (any && !this.raf) this.raf = requestAnimationFrame(this.poll);
  };

  private pads(): Gamepad[] {
    if (!('getGamepads' in navigator)) return [];
    try {
      return navigator.getGamepads().filter((p): p is Gamepad => p !== null);
    } catch {
      // Bloqueado pela Permissions Policy do iframe.
      return [];
    }
  }

  /** Notifica a UI apenas quando algo mudou, evitando re-render a cada quadro. */
  private report(): void {
    const r: GamepadReport = {
      apiAvailable: 'getGamepads' in navigator,
      policyAllowed: policyAllowed(),
      pads: this.pads().map((p) => ({
        index: p.index,
        id: p.id,
        mapping: p.mapping || 'não padrão',
        pressed: p.buttons.flatMap((b, i) => (b.pressed ? [i] : [])),
      })),
      events: this.events,
    };
    const key = JSON.stringify(r);
    if (key === this.lastReport) return;
    this.lastReport = key;
    this.onChange(r);
  }

  /** Dispara a ação na borda de pressionar e repete enquanto segurado. */
  private repeat(key: string, active: boolean, now: number, action: () => void): void {
    if (!active) {
      this.held.delete(key);
      return;
    }
    const next = this.held.get(key);
    if (next === undefined) {
      action();
      this.held.set(key, now + REPEAT_DELAY_MS);
    } else if (now >= next) {
      action();
      this.held.set(key, now + REPEAT_RATE_MS);
    }
  }

  private poll = (now: number): void => {
    const pads = this.pads();
    if (pads.length === 0) {
      this.raf = 0;
      this.report();
      return;
    }
    // Navegação só com a Activity em foco; nunca captura em segundo plano.
    if (document.hasFocus()) {
      for (const p of pads) {
        const b = (i: number) => p.buttons[i]?.pressed ?? false;
        const y = p.axes[1] ?? 0;
        const x = p.axes[0] ?? 0;
        this.repeat(`${p.index}:next`, b(BUTTON.DOWN) || b(BUTTON.RIGHT) || y > AXIS_THRESHOLD || x > AXIS_THRESHOLD, now, () => moveFocus(1));
        this.repeat(`${p.index}:prev`, b(BUTTON.UP) || b(BUTTON.LEFT) || y < -AXIS_THRESHOLD || x < -AXIS_THRESHOLD, now, () => moveFocus(-1));
        this.repeat(`${p.index}:a`, b(BUTTON.A), now, () => (document.activeElement as HTMLElement | null)?.click());
        this.repeat(`${p.index}:b`, b(BUTTON.B), now, () => (document.activeElement as HTMLElement | null)?.blur());
      }
    }
    this.report();
    this.raf = requestAnimationFrame(this.poll);
  };

  dispose(): void {
    cancelAnimationFrame(this.raf);
    window.removeEventListener('gamepadconnected', this.onConnect);
    window.removeEventListener('gamepaddisconnected', this.onConnect);
  }
}
