//! Converte analógicos em direções discretas, com deadzone radial e histerese.

/// Direção de um analógico tratada como D-pad virtual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

/// Fração do curso útil (após a deadzone) para ativar uma direção.
const PRESS_THRESHOLD: f32 = 0.5;
/// Fração para soltar; menor que a de ativação para evitar oscilação.
const RELEASE_THRESHOLD: f32 = 0.35;

/// Estado de um analógico. `x` cresce para a direita e `y` para cima,
/// como no GilRs.
#[derive(Debug, Default, Clone, Copy)]
pub struct StickState {
    pub x: f32,
    pub y: f32,
    current: Option<Direction>,
}

impl StickState {
    /// Atualiza um eixo e devolve `(soltou, pressionou)` quando a direção muda.
    pub fn update(
        &mut self,
        x: Option<f32>,
        y: Option<f32>,
        deadzone: f32,
    ) -> (Option<Direction>, Option<Direction>) {
        if let Some(x) = x {
            self.x = x;
        }
        if let Some(y) = y {
            self.y = y;
        }
        let magnitude = (self.x * self.x + self.y * self.y).sqrt();
        let scaled = if magnitude <= deadzone {
            0.0
        } else {
            ((magnitude - deadzone) / (1.0 - deadzone).max(f32::EPSILON)).min(1.0)
        };

        let dominant = if self.x.abs() > self.y.abs() {
            if self.x > 0.0 {
                Direction::Right
            } else {
                Direction::Left
            }
        } else if self.y > 0.0 {
            Direction::Up
        } else {
            Direction::Down
        };

        let next = match self.current {
            Some(dir) if scaled >= RELEASE_THRESHOLD && dir == dominant => Some(dir),
            Some(_) | None if scaled >= PRESS_THRESHOLD => Some(dominant),
            _ => None,
        };
        if next == self.current {
            return (None, None);
        }
        let released = self.current;
        self.current = next;
        (released, next)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_suppresses_drift() {
        let mut s = StickState::default();
        assert_eq!(s.update(Some(0.15), Some(-0.1), 0.2), (None, None));
    }

    #[test]
    fn presses_and_releases_with_hysteresis() {
        let mut s = StickState::default();
        assert_eq!(
            s.update(None, Some(-0.9), 0.2),
            (None, Some(Direction::Down))
        );
        // Entre o limiar de soltura e o de ativação: mantém.
        assert_eq!(s.update(None, Some(-0.55), 0.2), (None, None));
        assert_eq!(
            s.update(None, Some(-0.1), 0.2),
            (Some(Direction::Down), None)
        );
    }

    #[test]
    fn changes_direction_on_dominant_axis() {
        let mut s = StickState::default();
        s.update(Some(0.9), Some(0.0), 0.2);
        assert_eq!(
            s.update(Some(0.1), Some(0.95), 0.2),
            (Some(Direction::Right), Some(Direction::Up))
        );
    }

    #[test]
    fn larger_deadzone_requires_more_travel() {
        let mut s = StickState::default();
        assert_eq!(s.update(Some(0.6), None, 0.4), (None, None));
        assert_eq!(
            s.update(Some(0.75), None, 0.4),
            (None, Some(Direction::Right))
        );
    }
}
