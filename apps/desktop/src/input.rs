//! Roteamento das ações do controle para a interface.
//!
//! Fora de modais, a navegação usa a cadeia de foco do Slint: Baixo/Cima viram
//! Tab/Shift+Tab, Esquerda/Direita viram setas (ajustam valores) e Confirmar
//! vira Enter. Dentro de modais, as ações vão direto ao modal, que mantém o
//! foco preso (teclado virtual, menu rápido).

use vellcast_controller::Action;

/// Teclas sintéticas enviadas à janela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticKey {
    Tab,
    Backtab,
    Left,
    Right,
    Return,
}

/// Ação dentro de um modal (espelha o enum `NavAction` do Slint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalAction {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Secondary,
    Options,
    Menu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    Key(SyntheticKey),
    Modal(ModalAction),
    /// Volta para a tela anterior do histórico.
    HistoryBack,
    /// Seção anterior/próxima na barra lateral (LB/RB).
    Section(i32),
    /// Aba anterior/próxima da tela atual (LT/RT).
    Tab(i32),
    /// Rolagem da área central: positivo = para baixo.
    Scroll(i32),
    OpenMenu,
    /// Botão sem função nesta etapa.
    Unassigned,
}

pub fn route(action: Action, modal_open: bool) -> Route {
    if modal_open {
        let modal = match action {
            Action::Up => ModalAction::Up,
            Action::Down => ModalAction::Down,
            Action::Left => ModalAction::Left,
            Action::Right => ModalAction::Right,
            Action::Confirm => ModalAction::Confirm,
            Action::Back => ModalAction::Back,
            Action::Secondary => ModalAction::Secondary,
            Action::Options => ModalAction::Options,
            Action::Menu => ModalAction::Menu,
            _ => return Route::Unassigned,
        };
        return Route::Modal(modal);
    }
    match action {
        Action::Up => Route::Key(SyntheticKey::Backtab),
        Action::Down => Route::Key(SyntheticKey::Tab),
        Action::Left => Route::Key(SyntheticKey::Left),
        Action::Right => Route::Key(SyntheticKey::Right),
        Action::Confirm => Route::Key(SyntheticKey::Return),
        Action::Back => Route::HistoryBack,
        Action::PrevSection => Route::Section(-1),
        Action::NextSection => Route::Section(1),
        Action::PrevTab => Route::Tab(-1),
        Action::NextTab => Route::Tab(1),
        Action::ScrollUp => Route::Scroll(-1),
        Action::ScrollDown => Route::Scroll(1),
        Action::Menu => Route::OpenMenu,
        Action::Secondary | Action::Options | Action::QuickActions => Route::Unassigned,
    }
}

/// Histórico de telas para o botão Voltar, com tamanho limitado.
#[derive(Debug, Default)]
pub struct History {
    stack: Vec<i32>,
}

const HISTORY_LIMIT: usize = 32;

impl History {
    pub fn push(&mut self, from: i32, to: i32) {
        if from == to {
            return;
        }
        self.stack.push(from);
        if self.stack.len() > HISTORY_LIMIT {
            self.stack.remove(0);
        }
    }

    pub fn back(&mut self) -> Option<i32> {
        self.stack.pop()
    }
}

/// Próxima seção com volta circular.
pub fn step_section(current: i32, delta: i32, count: i32) -> i32 {
    if count <= 0 {
        return 0;
    }
    (current + delta).rem_euclid(count)
}

/// Remove o último caractere (grafema simples) de uma string.
pub fn pop_char(s: &str) -> String {
    let mut chars = s.chars();
    chars.next_back();
    chars.as_str().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_moves_follow_focus_chain() {
        assert_eq!(route(Action::Down, false), Route::Key(SyntheticKey::Tab));
        assert_eq!(route(Action::Up, false), Route::Key(SyntheticKey::Backtab));
        assert_eq!(
            route(Action::Confirm, false),
            Route::Key(SyntheticKey::Return)
        );
    }

    #[test]
    fn modal_traps_navigation() {
        assert_eq!(route(Action::Down, true), Route::Modal(ModalAction::Down));
        assert_eq!(route(Action::Back, true), Route::Modal(ModalAction::Back));
        // Trocar de seção com um modal aberto não é permitido.
        assert_eq!(route(Action::NextSection, true), Route::Unassigned);
    }

    #[test]
    fn shoulders_switch_sections_and_triggers_tabs() {
        assert_eq!(route(Action::PrevSection, false), Route::Section(-1));
        assert_eq!(route(Action::NextTab, false), Route::Tab(1));
        assert_eq!(route(Action::ScrollDown, false), Route::Scroll(1));
        assert_eq!(route(Action::Menu, false), Route::OpenMenu);
    }

    #[test]
    fn sections_wrap() {
        assert_eq!(step_section(0, -1, 8), 7);
        assert_eq!(step_section(7, 1, 8), 0);
        assert_eq!(step_section(3, 1, 0), 0);
    }

    #[test]
    fn history_goes_back_and_is_bounded() {
        let mut h = History::default();
        h.push(0, 4);
        h.push(4, 4);
        h.push(4, 6);
        assert_eq!(h.back(), Some(4));
        assert_eq!(h.back(), Some(0));
        assert_eq!(h.back(), None);
        for i in 0..100 {
            h.push(i, i + 1);
        }
        assert_eq!(h.stack.len(), HISTORY_LIMIT);
    }

    #[test]
    fn pop_char_handles_multibyte() {
        assert_eq!(pop_char("ação"), "açã");
        assert_eq!(pop_char(""), "");
    }
}
