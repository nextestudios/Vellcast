//! Adaptação de qualidade durante a transmissão (S-001 e modo competitivo S-002).
//!
//! Lógica pura, sem dependência de plataforma: o pipeline chama
//! [`Adapter::update`] uma vez por segundo com os sinais medidos e aplica a
//! decisão (bitrate no encoder, FPS por descarte de quadros).
//!
//! Política:
//! - **Rede congestionada** (quadros descartados pelo transporte, fila de envio
//!   acima de ~0,5 s de vídeo ou RTT muito acima do mínimo observado): reduz o
//!   bitrate multiplicativamente (×0,7), no máximo uma vez a cada intervalo mínimo.
//! - **Encoder sobrecarregado** (quadros recusados pelo encoder ou tempo de
//!   codificação perto do intervalo entre quadros): reduz o FPS em degraus. No
//!   modo competitivo reage na primeira janela e reduz antes o bitrate; no
//!   equilibrado espera a sobrecarga persistir.
//! - **Recuperação**: depois de um período estável (histerese), aumenta o bitrate
//!   de forma aditiva e, mais tarde, devolve o FPS um degrau por vez.
//! - **Nitidez** (texto e código, issue #5): na rede congestionada reduz o
//!   bitrate **e** um degrau de FPS de uma vez (até 15 FPS), preservando os
//!   bits por quadro, que é o que deixa o texto legível.
//! - Nunca passa da qualidade escolhida pelo usuário nem desce do piso.
//! - O modo e o bitrate máximo podem mudar durante a transmissão
//!   ([`Adapter::set_mode`], [`Adapter::set_max_bitrate`]).
//!
//! Resolução não muda durante a transmissão (exigiria reiniciar o encoder):
//! no modo competitivo, sobrecarga persistente no FPS mínimo gera apenas uma
//! recomendação para reduzir a resolução.

/// Piso de bitrate (kbps).
pub const MIN_BITRATE_KBPS: u32 = 500;
/// Degraus de FPS, do maior para o menor.
const FPS_STEPS: [u32; 5] = [60, 45, 30, 24, 15];

/// Modo de adaptação.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Adaptation {
    /// Bitrate e FPS fixos.
    Off,
    /// Ajusta o bitrate à rede e só reduz o FPS se o encoder ficar sobrecarregado por vários segundos.
    #[default]
    Balanced,
    /// Prioriza o desempenho do jogo: reage na hora à carga do encoder.
    Competitive,
    /// Prioriza a nitidez (texto e código): troca FPS por bits por quadro quando a rede aperta.
    Clarity,
}

impl Adaptation {
    /// FPS mínimo que a adaptação pode atingir neste modo.
    fn fps_floor(self) -> u32 {
        match self {
            Self::Competitive => 24,
            Self::Clarity => 15,
            Self::Off | Self::Balanced => 30,
        }
    }
}

/// Sinais de uma janela de medição (normalmente 1 s).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Signals {
    /// Bytes na fila de envio do transporte.
    pub queue_bytes: usize,
    /// Quadros de vídeo descartados pelo transporte nesta janela.
    pub network_dropped: u64,
    /// Quadros recusados pelo encoder (ocupado) nesta janela.
    pub encoder_dropped: u64,
    /// RTT até o relay, se conhecido.
    pub rtt_ms: Option<f32>,
    /// Tempo médio de codificação de um quadro.
    pub encode_ms: f32,
}

/// Mudança decidida pelo controlador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Change {
    pub bitrate_kbps: u32,
    pub fps: u32,
    pub reason: Reason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NetworkCongestion,
    EncoderOverload,
    Recovery,
    /// Sobrecarga no FPS mínimo: só uma nova transmissão com resolução menor resolve.
    RecommendLowerResolution,
}

impl Reason {
    /// `true` quando a qualidade foi reduzida (a interface deve avisar).
    pub fn is_degradation(self) -> bool {
        !matches!(self, Self::Recovery)
    }
}

/// Parâmetros de tempo do controlador (ms).
#[derive(Debug, Clone, Copy)]
struct Timing {
    /// Intervalo mínimo entre duas reduções.
    min_decrease_interval: u64,
    /// Tempo estável exigido antes de aumentar o bitrate.
    increase_after: u64,
    /// Tempo estável exigido antes de devolver um degrau de FPS.
    fps_restore_after: u64,
    /// Janelas seguidas de sobrecarga do encoder antes de reduzir o FPS.
    overload_windows: u32,
}

const BALANCED: Timing = Timing {
    min_decrease_interval: 2_000,
    increase_after: 5_000,
    fps_restore_after: 15_000,
    overload_windows: 3,
};

const COMPETITIVE: Timing = Timing {
    min_decrease_interval: 1_000,
    increase_after: 10_000,
    fps_restore_after: 30_000,
    overload_windows: 1,
};

#[derive(Debug)]
pub struct Adapter {
    mode: Adaptation,
    max_bitrate: u32,
    max_fps: u32,
    min_fps: u32,
    bitrate: u32,
    fps: u32,
    last_decrease_ms: Option<u64>,
    last_change_ms: u64,
    stable_since_ms: u64,
    overload_streak: u32,
    min_rtt_ms: Option<f32>,
    recommended_resolution: bool,
}

impl Adapter {
    pub fn new(mode: Adaptation, max_bitrate_kbps: u32, max_fps: u32, now_ms: u64) -> Self {
        let max_bitrate = max_bitrate_kbps.max(MIN_BITRATE_KBPS);
        let max_fps = max_fps.max(1);
        // O FPS só desce até 30 no modo equilibrado; no competitivo, até 24; na nitidez, até 15.
        let floor = mode.fps_floor();
        Self {
            mode,
            max_bitrate,
            max_fps,
            min_fps: floor.min(max_fps),
            bitrate: max_bitrate,
            fps: max_fps,
            last_decrease_ms: None,
            last_change_ms: now_ms,
            stable_since_ms: now_ms,
            overload_streak: 0,
            min_rtt_ms: None,
            recommended_resolution: false,
        }
    }

    pub fn bitrate_kbps(&self) -> u32 {
        self.bitrate
    }

    pub fn mode(&self) -> Adaptation {
        self.mode
    }

    pub fn max_bitrate_kbps(&self) -> u32 {
        self.max_bitrate
    }

    /// Troca o modo durante a transmissão (preferência de conteúdo). Mantém o
    /// bitrate atual; o FPS atual sobe ao piso do novo modo, se preciso.
    pub fn set_mode(&mut self, mode: Adaptation, now_ms: u64) {
        self.mode = mode;
        self.min_fps = mode.fps_floor().min(self.max_fps);
        self.fps = self.fps.clamp(self.min_fps, self.max_fps);
        if mode == Adaptation::Off {
            // Sem adaptação valem os valores escolhidos pelo usuário.
            self.bitrate = self.max_bitrate;
            self.fps = self.max_fps;
        }
        self.overload_streak = 0;
        self.recommended_resolution = false;
        self.stable_since_ms = now_ms;
        self.last_change_ms = now_ms;
    }

    /// Novo bitrate escolhido pelo usuário durante a transmissão: vira o teto e
    /// o valor atual (a adaptação volta a reduzir se a rede não aguentar).
    pub fn set_max_bitrate(&mut self, kbps: u32, now_ms: u64) {
        self.max_bitrate = kbps.max(MIN_BITRATE_KBPS);
        self.bitrate = self.max_bitrate;
        self.stable_since_ms = now_ms;
        self.last_change_ms = now_ms;
    }

    pub fn fps(&self) -> u32 {
        self.fps
    }

    fn timing(&self) -> Timing {
        if self.mode == Adaptation::Competitive {
            COMPETITIVE
        } else {
            BALANCED
        }
    }

    /// Fila acima disto (~0,5 s do bitrate atual) indica rede abaixo do bitrate.
    fn queue_limit_bytes(&self) -> usize {
        (self.bitrate as usize * 1000 / 8) / 2
    }

    fn network_congested(&mut self, s: &Signals) -> bool {
        let rtt_spike = match (s.rtt_ms, self.min_rtt_ms) {
            (Some(rtt), Some(min)) => rtt > (min * 3.0).max(min + 150.0),
            _ => false,
        };
        if let Some(rtt) = s.rtt_ms.filter(|r| *r > 0.0) {
            self.min_rtt_ms = Some(self.min_rtt_ms.map_or(rtt, |m| m.min(rtt)));
        }
        s.network_dropped > 0 || s.queue_bytes > self.queue_limit_bytes() || rtt_spike
    }

    fn encoder_overloaded(&self, s: &Signals) -> bool {
        let frame_ms = 1000.0 / self.fps.max(1) as f32;
        s.encoder_dropped > 0 || s.encode_ms > frame_ms * 0.85
    }

    fn can_decrease(&self, now_ms: u64) -> bool {
        self.last_decrease_ms
            .is_none_or(|t| now_ms.saturating_sub(t) >= self.timing().min_decrease_interval)
    }

    fn step_fps_down(&self) -> Option<u32> {
        FPS_STEPS
            .iter()
            .copied()
            .find(|&f| f < self.fps && f >= self.min_fps)
    }

    fn step_fps_up(&self) -> Option<u32> {
        FPS_STEPS
            .iter()
            .rev()
            .copied()
            .find(|&f| f > self.fps && f <= self.max_fps)
            .or((self.fps < self.max_fps).then_some(self.max_fps))
    }

    fn change(&mut self, now_ms: u64, bitrate: u32, fps: u32, reason: Reason) -> Option<Change> {
        let bitrate = bitrate.clamp(MIN_BITRATE_KBPS, self.max_bitrate);
        let fps = fps.clamp(self.min_fps, self.max_fps);
        if bitrate == self.bitrate && fps == self.fps && reason != Reason::RecommendLowerResolution
        {
            return None;
        }
        if reason.is_degradation() {
            self.last_decrease_ms = Some(now_ms);
            self.stable_since_ms = now_ms;
        }
        self.bitrate = bitrate;
        self.fps = fps;
        self.last_change_ms = now_ms;
        Some(Change {
            bitrate_kbps: bitrate,
            fps,
            reason,
        })
    }

    /// Processa uma janela de medição. Retorna a mudança a aplicar, se houver.
    pub fn update(&mut self, now_ms: u64, s: Signals) -> Option<Change> {
        if self.mode == Adaptation::Off {
            return None;
        }
        let timing = self.timing();
        let congested = self.network_congested(&s);
        let overloaded = self.encoder_overloaded(&s);
        self.overload_streak = if overloaded {
            self.overload_streak + 1
        } else {
            0
        };
        // Qualquer sinal de problema reinicia a contagem de estabilidade, mesmo sem
        // mudança (ex.: já no piso): evita subir e descer em rede instável.
        if congested || overloaded {
            self.stable_since_ms = now_ms;
        }

        if congested {
            if !self.can_decrease(now_ms) {
                return None;
            }
            let target = (self.bitrate as f32 * 0.7) as u32;
            // Nitidez: menos quadros com o mesmo tamanho, em vez de quadros borrados.
            let fps = if self.mode == Adaptation::Clarity {
                self.step_fps_down().unwrap_or(self.fps)
            } else {
                self.fps
            };
            return self.change(now_ms, target, fps, Reason::NetworkCongestion);
        }

        if self.overload_streak >= timing.overload_windows {
            if !self.can_decrease(now_ms) {
                return None;
            }
            // Competitivo: primeiro o bitrate (alivia o encoder e a rede), depois o FPS.
            if self.mode == Adaptation::Competitive && self.bitrate > self.max_bitrate * 6 / 10 {
                let target = (self.bitrate as f32 * 0.8) as u32;
                return self.change(now_ms, target, self.fps, Reason::EncoderOverload);
            }
            self.overload_streak = 0;
            if let Some(fps) = self.step_fps_down() {
                return self.change(now_ms, self.bitrate, fps, Reason::EncoderOverload);
            }
            if self.mode == Adaptation::Competitive && !self.recommended_resolution {
                self.recommended_resolution = true;
                return self.change(
                    now_ms,
                    self.bitrate,
                    self.fps,
                    Reason::RecommendLowerResolution,
                );
            }
            return None;
        }

        if overloaded {
            // Sobrecarga ainda não confirmada: não aumenta nada.
            return None;
        }

        // Estável: recuperação lenta, um passo por vez.
        let stable_for = now_ms.saturating_sub(self.stable_since_ms);
        let since_change = now_ms.saturating_sub(self.last_change_ms);
        if self.bitrate < self.max_bitrate
            && stable_for >= timing.increase_after
            && since_change >= timing.increase_after / 2
        {
            let step = (self.max_bitrate / 20).max(100);
            return self.change(now_ms, self.bitrate + step, self.fps, Reason::Recovery);
        }
        if self.fps < self.max_fps
            && stable_for >= timing.fps_restore_after
            && since_change >= timing.increase_after
            && let Some(fps) = self.step_fps_up()
        {
            return self.change(now_ms, self.bitrate, fps, Reason::Recovery);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calm() -> Signals {
        Signals {
            rtt_ms: Some(40.0),
            encode_ms: 4.0,
            ..Signals::default()
        }
    }

    fn congested() -> Signals {
        Signals {
            network_dropped: 3,
            ..calm()
        }
    }

    fn overloaded() -> Signals {
        Signals {
            encoder_dropped: 5,
            encode_ms: 20.0,
            ..calm()
        }
    }

    #[test]
    fn off_never_changes() {
        let mut a = Adapter::new(Adaptation::Off, 8000, 60, 0);
        for t in 1..20 {
            assert_eq!(a.update(t * 1000, congested()), None);
        }
        assert_eq!((a.bitrate_kbps(), a.fps()), (8000, 60));
    }

    #[test]
    fn congestion_decreases_multiplicatively_with_min_interval() {
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        let c = a.update(1000, congested()).expect("reduz");
        assert_eq!(c.bitrate_kbps, 5600);
        assert_eq!(c.reason, Reason::NetworkCongestion);
        // Menos de 2 s depois: não reduz de novo (evita colapso por uma rajada).
        assert_eq!(a.update(2000, congested()), None);
        assert_eq!(
            a.update(3000, congested()).map(|c| c.bitrate_kbps),
            Some(3920)
        );
    }

    #[test]
    fn queue_backlog_counts_as_congestion() {
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        // 8000 kbps → 1 MB/s; 0,5 s = 500 kB.
        let s = Signals {
            queue_bytes: 600_000,
            ..calm()
        };
        assert!(a.update(1000, s).is_some());
    }

    #[test]
    fn rtt_spike_counts_as_congestion() {
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        assert_eq!(a.update(1000, calm()), None);
        let spike = Signals {
            rtt_ms: Some(400.0),
            ..calm()
        };
        assert_eq!(
            a.update(2000, spike).map(|c| c.reason),
            Some(Reason::NetworkCongestion)
        );
    }

    #[test]
    fn never_goes_below_floor_or_above_user_choice() {
        let mut a = Adapter::new(Adaptation::Balanced, 2000, 30, 0);
        for t in 1..60 {
            a.update(t * 2000, congested());
        }
        assert_eq!(a.bitrate_kbps(), MIN_BITRATE_KBPS);
        for t in 60..400 {
            a.update(t * 2000, calm());
        }
        assert_eq!(a.bitrate_kbps(), 2000);
        assert_eq!(a.fps(), 30);
    }

    #[test]
    fn recovery_is_slow_and_additive() {
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        a.update(1000, congested());
        assert_eq!(a.bitrate_kbps(), 5600);
        // Estável por menos de 5 s: nada muda.
        for t in 2..6 {
            assert_eq!(a.update(t * 1000, calm()), None);
        }
        let c = a.update(6000, calm()).expect("aumenta");
        assert_eq!(c.reason, Reason::Recovery);
        assert_eq!(c.bitrate_kbps, 5600 + 400);
    }

    #[test]
    fn hysteresis_prevents_oscillation() {
        // Rede alternando entre ruim e boa a cada segundo: o controlador reduz,
        // mas nunca volta a subir enquanto a instabilidade continua.
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        let mut increases = 0;
        for t in 1..60 {
            let s = if t % 2 == 0 { congested() } else { calm() };
            if let Some(c) = a.update(t * 1000, s)
                && c.reason == Reason::Recovery
            {
                increases += 1;
            }
        }
        assert_eq!(increases, 0);
        assert!(a.bitrate_kbps() < 8000);
    }

    #[test]
    fn balanced_waits_for_persistent_encoder_overload() {
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        assert_eq!(a.update(1000, overloaded()), None);
        assert_eq!(a.update(2000, overloaded()), None);
        let c = a.update(3000, overloaded()).expect("reduz FPS");
        assert_eq!((c.fps, c.reason), (45, Reason::EncoderOverload));
        assert_eq!(
            c.bitrate_kbps, 8000,
            "equilibrado não mexe no bitrate por carga"
        );
    }

    #[test]
    fn competitive_lowers_bitrate_then_fps_then_recommends_resolution() {
        let mut a = Adapter::new(Adaptation::Competitive, 5000, 60, 0);
        let mut reasons = Vec::new();
        for t in 1..20 {
            if let Some(c) = a.update(t * 1000, overloaded()) {
                reasons.push((c.bitrate_kbps, c.fps, c.reason));
            }
        }
        // Primeiro bitrate (×0,8 até 60% do máximo), depois FPS 45 → 30 → 24.
        assert_eq!(reasons[0], (4000, 60, Reason::EncoderOverload));
        assert_eq!(reasons[1], (3200, 60, Reason::EncoderOverload));
        assert_eq!(reasons[2], (2560, 60, Reason::EncoderOverload));
        assert_eq!(reasons[3].1, 45);
        assert_eq!(reasons[4].1, 30);
        assert_eq!(reasons[5].1, 24);
        assert_eq!(reasons[6].2, Reason::RecommendLowerResolution);
        // A recomendação aparece uma vez só.
        assert_eq!(
            reasons
                .iter()
                .filter(|r| r.2 == Reason::RecommendLowerResolution)
                .count(),
            1
        );
    }

    #[test]
    fn fps_is_restored_after_long_stability() {
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        for t in 1..=3 {
            a.update(t * 1000, overloaded());
        }
        assert_eq!(a.fps(), 45);
        let mut restored_at = None;
        for t in 4..40 {
            if let Some(c) = a.update(t * 1000, calm())
                && c.fps == 60
            {
                restored_at = Some(t);
                break;
            }
        }
        let t = restored_at.expect("FPS volta a 60");
        assert!(t >= 18, "só depois de ≥15 s estável (voltou em {t} s)");
    }

    #[test]
    fn thirty_fps_source_is_not_reduced_below_floor_in_balanced() {
        let mut a = Adapter::new(Adaptation::Balanced, 4000, 30, 0);
        for t in 1..10 {
            a.update(t * 1000, overloaded());
        }
        assert_eq!(a.fps(), 30);
    }

    #[test]
    fn clarity_trades_fps_for_bits_per_frame_under_congestion() {
        let mut clarity = Adapter::new(Adaptation::Clarity, 8000, 60, 0);
        let mut motion = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        let c = clarity.update(1000, congested()).expect("reduz");
        let m = motion.update(1000, congested()).expect("reduz");
        assert_eq!((c.bitrate_kbps, c.fps), (5600, 45));
        assert_eq!(
            (m.bitrate_kbps, m.fps),
            (5600, 60),
            "movimento mantém o FPS"
        );
        // Bits por quadro: a nitidez preserva mais que o movimento.
        assert!(c.bitrate_kbps / c.fps > m.bitrate_kbps / m.fps);
        for t in 2..20 {
            clarity.update(t * 1000, congested());
        }
        assert_eq!(clarity.fps(), 15, "piso da nitidez");
        assert_eq!(clarity.bitrate_kbps(), MIN_BITRATE_KBPS);
    }

    #[test]
    fn mode_change_mid_stream_keeps_bitrate_and_respects_new_floor() {
        let mut a = Adapter::new(Adaptation::Clarity, 8000, 60, 0);
        for t in 1..=3 {
            a.update(t * 2000, congested());
        }
        assert_eq!(a.fps(), 24);
        let bitrate = a.bitrate_kbps();
        a.set_mode(Adaptation::Balanced, 10_000);
        assert_eq!(a.mode(), Adaptation::Balanced);
        assert_eq!(a.fps(), 30, "sobe ao piso do equilibrado");
        assert_eq!(a.bitrate_kbps(), bitrate);
        a.set_mode(Adaptation::Off, 11_000);
        assert_eq!((a.bitrate_kbps(), a.fps()), (8000, 60));
        assert_eq!(a.update(12_000, congested()), None);
    }

    #[test]
    fn user_bitrate_change_becomes_new_ceiling() {
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        a.update(1000, congested());
        assert_eq!(a.bitrate_kbps(), 5600);
        a.set_max_bitrate(12_000, 2000);
        assert_eq!((a.bitrate_kbps(), a.max_bitrate_kbps()), (12_000, 12_000));
        a.set_max_bitrate(100, 3000);
        assert_eq!(a.bitrate_kbps(), MIN_BITRATE_KBPS);
        // Recuperação nunca passa do novo teto.
        let mut a = Adapter::new(Adaptation::Balanced, 8000, 60, 0);
        a.set_max_bitrate(3000, 0);
        for t in 1..60 {
            a.update(t * 1000, calm());
        }
        assert_eq!(a.bitrate_kbps(), 3000);
    }
}
