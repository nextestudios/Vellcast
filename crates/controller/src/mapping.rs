//! Mapeamento de botões físicos para ações (spec §13).
//!
//! Os nomes seguem o layout posicional do GilRs: `South` é A no Xbox e Cruz
//! no PlayStation, `East` é B/Círculo, e assim por diante. Isso dá o mesmo
//! comportamento para os dois layouts sem tabelas separadas.

use crate::Action;

/// Botões posicionais, independentes de fabricante.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Button {
    South,
    East,
    West,
    North,
    /// LB / L1.
    LeftBumper,
    /// RB / R1.
    RightBumper,
    /// LT / L2.
    LeftTrigger,
    /// RT / R2.
    RightTrigger,
    Select,
    Start,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    /// Botão Guide/PS: reservado ao sistema, nunca mapeado.
    Mode,
    LeftThumb,
    RightThumb,
}

pub fn map_button(button: Button) -> Option<Action> {
    Some(match button {
        Button::South => Action::Confirm,
        Button::East => Action::Back,
        Button::West => Action::Secondary,
        Button::North => Action::Options,
        Button::LeftBumper => Action::PrevSection,
        Button::RightBumper => Action::NextSection,
        Button::LeftTrigger => Action::PrevTab,
        Button::RightTrigger => Action::NextTab,
        Button::Start => Action::Menu,
        Button::Select => Action::QuickActions,
        Button::DPadUp => Action::Up,
        Button::DPadDown => Action::Down,
        Button::DPadLeft => Action::Left,
        Button::DPadRight => Action::Right,
        Button::Mode | Button::LeftThumb | Button::RightThumb => return None,
    })
}

impl Button {
    pub(crate) fn from_gilrs(b: gilrs::Button) -> Option<Self> {
        use gilrs::Button as G;
        Some(match b {
            G::South => Self::South,
            G::East => Self::East,
            G::West => Self::West,
            G::North => Self::North,
            G::LeftTrigger => Self::LeftBumper,
            G::RightTrigger => Self::RightBumper,
            G::LeftTrigger2 => Self::LeftTrigger,
            G::RightTrigger2 => Self::RightTrigger,
            G::Select => Self::Select,
            G::Start => Self::Start,
            G::DPadUp => Self::DPadUp,
            G::DPadDown => Self::DPadDown,
            G::DPadLeft => Self::DPadLeft,
            G::DPadRight => Self::DPadRight,
            G::Mode => Self::Mode,
            G::LeftThumb => Self::LeftThumb,
            G::RightThumb => Self::RightThumb,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn face_buttons_follow_spec() {
        assert_eq!(map_button(Button::South), Some(Action::Confirm));
        assert_eq!(map_button(Button::East), Some(Action::Back));
        assert_eq!(map_button(Button::West), Some(Action::Secondary));
        assert_eq!(map_button(Button::North), Some(Action::Options));
    }

    #[test]
    fn shoulders_and_triggers_follow_spec() {
        assert_eq!(map_button(Button::LeftBumper), Some(Action::PrevSection));
        assert_eq!(map_button(Button::RightBumper), Some(Action::NextSection));
        assert_eq!(map_button(Button::LeftTrigger), Some(Action::PrevTab));
        assert_eq!(map_button(Button::RightTrigger), Some(Action::NextTab));
        assert_eq!(map_button(Button::Start), Some(Action::Menu));
        assert_eq!(map_button(Button::Select), Some(Action::QuickActions));
    }

    #[test]
    fn system_button_is_never_mapped() {
        assert_eq!(map_button(Button::Mode), None);
    }

    #[test]
    fn gilrs_bumpers_and_triggers_are_not_swapped() {
        // No GilRs, LeftTrigger é o bumper (LB) e LeftTrigger2 é o gatilho (LT).
        assert_eq!(
            Button::from_gilrs(gilrs::Button::LeftTrigger),
            Some(Button::LeftBumper)
        );
        assert_eq!(
            Button::from_gilrs(gilrs::Button::LeftTrigger2),
            Some(Button::LeftTrigger)
        );
    }
}
