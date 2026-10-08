//! Thread dedicada que lê o GilRs e emite [`ControllerEvent`]s.
//!
//! A thread bloqueia em `next_event_blocking` e só acorda por entrada do
//! dispositivo ou pelo próximo prazo de repetição: não há polling ocioso.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use gilrs::{Axis, EventType, Gilrs};

use crate::mapping::{Button, map_button};
use crate::repeat::Repeater;
use crate::stick::{Direction, StickState};
use crate::{Action, ControllerEvent, InputConfig};

#[derive(Debug)]
pub struct SpawnError(pub String);

impl std::fmt::Display for SpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "falha ao iniciar a entrada de gamepad: {}", self.0)
    }
}

impl std::error::Error for SpawnError {}

/// Controle da thread de entrada a partir da aplicação.
#[derive(Clone)]
pub struct ControllerHandle {
    active: Arc<AtomicBool>,
    config: Arc<Mutex<InputConfig>>,
}

impl ControllerHandle {
    /// Liga ou desliga a emissão de ações. Deve refletir o foco da janela:
    /// com o app em segundo plano, a entrada pertence ao jogo.
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
    }

    pub fn set_config(&self, config: InputConfig) {
        *self.config.lock().unwrap_or_else(PoisonError::into_inner) = config.sanitized();
    }
}

/// Inicia a thread de entrada. `on_event` é chamado na thread de entrada;
/// a aplicação é responsável por levar o evento à thread da interface.
pub fn spawn<F>(config: InputConfig, on_event: F) -> Result<ControllerHandle, SpawnError>
where
    F: FnMut(ControllerEvent) + Send + 'static,
{
    let handle = ControllerHandle {
        active: Arc::new(AtomicBool::new(false)),
        config: Arc::new(Mutex::new(config.sanitized())),
    };
    let (ready_tx, ready_rx) = mpsc::channel();
    let shared = handle.clone();
    thread::Builder::new()
        .name("vellcast-gamepad".into())
        .spawn(move || match Gilrs::new() {
            Ok(gilrs) => {
                let _ = ready_tx.send(Ok(()));
                run(gilrs, shared, on_event);
            }
            Err(err) => {
                let _ = ready_tx.send(Err(err.to_string()));
            }
        })
        .map_err(|e| SpawnError(e.to_string()))?;
    match ready_rx.recv() {
        Ok(Ok(())) => Ok(handle),
        Ok(Err(e)) => Err(SpawnError(e)),
        Err(_) => Err(SpawnError("thread de entrada encerrou".into())),
    }
}

fn direction_action(d: Direction) -> Action {
    match d {
        Direction::Up => Action::Up,
        Direction::Down => Action::Down,
        Direction::Left => Action::Left,
        Direction::Right => Action::Right,
    }
}

fn scroll_action(d: Direction) -> Option<Action> {
    match d {
        Direction::Up => Some(Action::ScrollUp),
        Direction::Down => Some(Action::ScrollDown),
        Direction::Left | Direction::Right => None,
    }
}

fn run<F>(mut gilrs: Gilrs, handle: ControllerHandle, mut on_event: F)
where
    F: FnMut(ControllerEvent),
{
    for (id, pad) in gilrs.gamepads() {
        on_event(ControllerEvent::Connected {
            id: usize::from(id),
            name: pad.name().to_owned(),
        });
    }

    let mut repeater = Repeater::default();
    let mut left = StickState::default();
    let mut right = StickState::default();
    let mut was_active = false;

    loop {
        let timeout = repeater
            .next_deadline()
            .map(|d| d.saturating_duration_since(Instant::now()));
        let event = gilrs.next_event_blocking(timeout);
        let config = *handle.config.lock().unwrap_or_else(PoisonError::into_inner);
        let active = handle.active.load(Ordering::Relaxed);
        let now = Instant::now();

        if !active {
            if was_active {
                // Perdeu o foco: nada fica "preso" pressionado.
                repeater.clear();
                left.reset();
                right.reset();
            }
            was_active = false;
        } else {
            was_active = true;
        }

        let press = |action: Action, repeater: &mut Repeater, on_event: &mut F| {
            if repeater.press(action, now, config.repeat_delay) {
                on_event(ControllerEvent::Action(action));
            }
        };

        if let Some(ev) = event {
            match ev.event {
                EventType::Connected => {
                    let name = gilrs.gamepad(ev.id).name().to_owned();
                    on_event(ControllerEvent::Connected {
                        id: usize::from(ev.id),
                        name,
                    });
                }
                EventType::Disconnected => {
                    repeater.clear();
                    left.reset();
                    right.reset();
                    on_event(ControllerEvent::Disconnected {
                        id: usize::from(ev.id),
                    });
                }
                _ if !active => {}
                EventType::ButtonPressed(b, _) => {
                    if let Some(action) = Button::from_gilrs(b).and_then(map_button) {
                        press(action, &mut repeater, &mut on_event);
                    }
                }
                EventType::ButtonReleased(b, _) => {
                    if let Some(action) = Button::from_gilrs(b).and_then(map_button) {
                        repeater.release(action);
                    }
                }
                EventType::AxisChanged(axis, value, _) => {
                    let (released, pressed) = match axis {
                        Axis::LeftStickX => {
                            let (r, p) = left.update(Some(value), None, config.deadzone);
                            (r.map(direction_action), p.map(direction_action))
                        }
                        Axis::LeftStickY => {
                            let (r, p) = left.update(None, Some(value), config.deadzone);
                            (r.map(direction_action), p.map(direction_action))
                        }
                        Axis::RightStickX => {
                            let (r, p) = right.update(Some(value), None, config.deadzone);
                            (r.and_then(scroll_action), p.and_then(scroll_action))
                        }
                        Axis::RightStickY => {
                            let (r, p) = right.update(None, Some(value), config.deadzone);
                            (r.and_then(scroll_action), p.and_then(scroll_action))
                        }
                        _ => (None, None),
                    };
                    if let Some(a) = released {
                        repeater.release(a);
                    }
                    if let Some(a) = pressed {
                        press(a, &mut repeater, &mut on_event);
                    }
                }
                _ => {}
            }
        }

        if active {
            for action in repeater.due(Instant::now(), config.repeat_interval) {
                on_event(ControllerEvent::Action(action));
            }
        }
    }
}
