//! Repetição controlada de ações mantidas pressionadas.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::Action;

/// Máquina de estados de repetição: dispara ao pressionar, espera
/// `delay`, depois repete a cada `interval` até soltar.
#[derive(Debug, Default)]
pub struct Repeater {
    held: HashMap<Action, Instant>,
}

impl Repeater {
    /// Registra o pressionamento. Retorna `true` se a ação deve disparar agora
    /// (falso se já estava pressionada, por exemplo D-pad + analógico juntos).
    pub fn press(&mut self, action: Action, now: Instant, delay: Duration) -> bool {
        if self.held.contains_key(&action) {
            return false;
        }
        if action.repeats() {
            self.held.insert(action, now + delay);
        }
        true
    }

    pub fn release(&mut self, action: Action) {
        self.held.remove(&action);
    }

    /// Esquece tudo (perda de foco, desconexão).
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Ações cujo prazo de repetição venceu; reagenda cada uma.
    pub fn due(&mut self, now: Instant, interval: Duration) -> Vec<Action> {
        let mut out = Vec::new();
        for (action, next) in &mut self.held {
            if *next <= now {
                out.push(*action);
                // Evita rajadas após uma pausa longa da thread: no máximo uma por chamada.
                *next = now + interval;
            }
        }
        out.sort_by_key(|a| *a as u8);
        out
    }

    /// Próximo instante em que alguma repetição vence, se houver.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.held.values().min().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: Duration = Duration::from_millis(400);
    const I: Duration = Duration::from_millis(100);

    #[test]
    fn fires_on_press_then_waits_delay_then_repeats() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        assert!(r.press(Action::Down, t0, D));
        assert!(r.due(t0 + Duration::from_millis(399), I).is_empty());
        assert_eq!(r.due(t0 + D, I), vec![Action::Down]);
        assert!(r.due(t0 + D + Duration::from_millis(50), I).is_empty());
        assert_eq!(r.due(t0 + D + I, I), vec![Action::Down]);
    }

    #[test]
    fn release_stops_repetition() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        r.press(Action::Up, t0, D);
        r.release(Action::Up);
        assert!(r.due(t0 + D * 2, I).is_empty());
        assert_eq!(r.next_deadline(), None);
    }

    #[test]
    fn non_repeating_actions_fire_once() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        assert!(r.press(Action::Confirm, t0, D));
        assert!(r.due(t0 + D * 5, I).is_empty());
        assert_eq!(r.next_deadline(), None);
    }

    #[test]
    fn duplicate_press_does_not_fire_twice() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        assert!(r.press(Action::Left, t0, D));
        assert!(!r.press(Action::Left, t0, D));
    }

    #[test]
    fn long_stall_does_not_burst() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        r.press(Action::Down, t0, D);
        assert_eq!(r.due(t0 + Duration::from_secs(10), I).len(), 1);
        assert!(r.due(t0 + Duration::from_secs(10), I).is_empty());
    }

    #[test]
    fn deadline_is_earliest_held() {
        let t0 = Instant::now();
        let mut r = Repeater::default();
        r.press(Action::Down, t0, D);
        r.press(Action::Right, t0 + Duration::from_millis(100), D);
        assert_eq!(r.next_deadline(), Some(t0 + D));
        r.clear();
        assert_eq!(r.next_deadline(), None);
    }
}
