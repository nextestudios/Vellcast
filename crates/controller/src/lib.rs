//! Entrada por gamepad do Vellcast.
//!
//! Converte a entrada bruta do GilRs em ações semânticas (docs/CONTROLLER_SUPPORT.md),
//! com deadzone configurável e repetição controlada. A lógica de mapeamento,
//! deadzone e repetição é independente de hardware e coberta por testes; apenas
//! [`spawn`] toca no dispositivo.
//!
//! Regra de privacidade/jogo: enquanto o aplicativo não estiver em foco
//! ([`ControllerHandle::set_active`]), nenhuma ação é emitida — o controle
//! continua pertencendo ao jogo.

mod mapping;
mod repeat;
mod stick;
mod worker;

pub use mapping::{Button, map_button};
pub use repeat::Repeater;
pub use stick::{Direction, StickState};
pub use worker::{ControllerHandle, SpawnError, spawn};

use std::time::Duration;

/// Ações semânticas da interface (spec §13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Up,
    Down,
    Left,
    Right,
    /// Xbox A / PlayStation Cruz.
    Confirm,
    /// Xbox B / PlayStation Círculo.
    Back,
    /// Xbox X / PlayStation Quadrado.
    Secondary,
    /// Xbox Y / PlayStation Triângulo.
    Options,
    /// LB / L1.
    PrevSection,
    /// RB / R1.
    NextSection,
    /// LT / L2.
    PrevTab,
    /// RT / R2.
    NextTab,
    /// Analógico direito para cima.
    ScrollUp,
    /// Analógico direito para baixo.
    ScrollDown,
    /// Start / Options.
    Menu,
    /// Select / Create.
    QuickActions,
}

impl Action {
    /// Ações que se repetem enquanto o botão é mantido pressionado.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Action::Up
                | Action::Down
                | Action::Left
                | Action::Right
                | Action::ScrollUp
                | Action::ScrollDown
        )
    }
}

/// Eventos entregues à aplicação.
#[derive(Debug, Clone, PartialEq)]
pub enum ControllerEvent {
    Action(Action),
    Connected { id: usize, name: String },
    Disconnected { id: usize },
}

/// Configuração de entrada, alterável em tempo de execução.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputConfig {
    /// Deadzone radial dos analógicos, de 0.0 a 0.9.
    pub deadzone: f32,
    /// Atraso antes da primeira repetição.
    pub repeat_delay: Duration,
    /// Intervalo entre repetições.
    pub repeat_interval: Duration,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            deadzone: 0.2,
            repeat_delay: Duration::from_millis(400),
            repeat_interval: Duration::from_millis(110),
        }
    }
}

impl InputConfig {
    /// Garante valores dentro de limites seguros.
    pub fn sanitized(self) -> Self {
        Self {
            deadzone: if self.deadzone.is_finite() {
                self.deadzone.clamp(0.0, 0.9)
            } else {
                0.2
            },
            repeat_delay: self
                .repeat_delay
                .clamp(Duration::from_millis(150), Duration::from_millis(1500)),
            repeat_interval: self
                .repeat_interval
                .clamp(Duration::from_millis(40), Duration::from_millis(1000)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_out_of_range_config() {
        let c = InputConfig {
            deadzone: 5.0,
            repeat_delay: Duration::from_millis(1),
            repeat_interval: Duration::from_secs(60),
        }
        .sanitized();
        assert_eq!(c.deadzone, 0.9);
        assert_eq!(c.repeat_delay, Duration::from_millis(150));
        assert_eq!(c.repeat_interval, Duration::from_millis(1000));
        let nan = InputConfig {
            deadzone: f32::NAN,
            ..InputConfig::default()
        }
        .sanitized();
        assert_eq!(nan.deadzone, 0.2);
    }

    #[test]
    fn only_navigation_and_scroll_repeat() {
        assert!(Action::Down.repeats());
        assert!(Action::ScrollUp.repeats());
        assert!(!Action::Confirm.repeats());
        assert!(!Action::Back.repeats());
        assert!(!Action::NextSection.repeats());
    }
}
