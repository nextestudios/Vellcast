//! Controle de ritmo dos quadros capturados (puro).

/// Aceita quadros na taxa-alvo, mesmo que a fonte produza mais (ex.: 144 Hz → 60 FPS).
///
/// Usa uma grade de horários ideais com meia janela de tolerância, o que mantém a
/// média na taxa-alvo sem acumular atraso; se a fonte ficar parada, a grade é
/// reancorada no próximo quadro.
#[derive(Debug, Clone)]
pub struct FramePacer {
    interval: i64,
    next_due: Option<i64>,
}

impl FramePacer {
    pub fn new(fps: u32) -> Self {
        Self {
            interval: 10_000_000 / i64::from(fps.max(1)),
            next_due: None,
        }
    }

    /// `time_hns`: timestamp do quadro em unidades de 100 ns.
    pub fn accept(&mut self, time_hns: i64) -> bool {
        let Some(due) = self.next_due else {
            self.next_due = Some(time_hns + self.interval);
            return true;
        };
        if time_hns + self.interval / 2 < due {
            return false;
        }
        let mut next = due + self.interval;
        // Fonte parada ou atrasada: reancora para não aceitar uma rajada depois.
        if next < time_hns {
            next = time_hns + self.interval;
        }
        self.next_due = Some(next);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accepted(fps: u32, source_hz: f64, seconds: f64) -> usize {
        let mut pacer = FramePacer::new(fps);
        let step = 10_000_000.0 / source_hz;
        let frames = (seconds * source_hz) as usize;
        (0..frames)
            .filter(|i| pacer.accept((*i as f64 * step) as i64))
            .count()
    }

    #[test]
    fn caps_high_refresh_sources() {
        let n = accepted(60, 144.0, 10.0);
        assert!((595..=605).contains(&n), "{n}");
        let n = accepted(30, 60.0, 10.0);
        assert!((295..=305).contains(&n), "{n}");
    }

    #[test]
    fn passes_matching_rate() {
        let n = accepted(60, 60.0, 10.0);
        assert!((598..=600).contains(&n), "{n}");
    }

    #[test]
    fn passes_slower_sources_untouched() {
        assert_eq!(accepted(60, 30.0, 10.0), 300);
    }

    #[test]
    fn reanchors_after_idle_without_burst() {
        let mut pacer = FramePacer::new(60);
        assert!(pacer.accept(0));
        // Fonte parada por 1 s (janela sem mudanças): o próximo quadro passa,
        // mas o seguinte, 1 ms depois, não.
        assert!(pacer.accept(10_000_000));
        assert!(!pacer.accept(10_010_000));
    }
}
