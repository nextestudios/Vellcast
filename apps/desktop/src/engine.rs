//! Thread dona do motor de transmissão (`vellcast-core`).
//!
//! A interface só envia comandos e recebe mensagens; nenhuma chamada de
//! captura, encoder ou rede roda na thread da interface (spec §12). A sessão
//! é criada e destruída dentro desta thread, então não precisa ser `Send`.

use std::sync::mpsc;
use std::thread;

use vellcast_core::{
    AudioInputInfo, EncoderInfo, LiveChange, SessionConfig, SessionEvent, SessionStats, SourceInfo,
    StreamSession,
};

pub enum Command {
    ListSources,
    ListEncoders,
    Start(Box<SessionConfig>),
    Stop,
    Stats,
    /// Mudança durante a transmissão (controles rápidos da tela Ao vivo).
    Apply(LiveChange),
    /// Lista os microfones disponíveis.
    ListAudioInputs,
    Shutdown,
}

#[derive(Debug)]
pub enum EngineMsg {
    Sources(Result<Vec<SourceInfo>, String>),
    Encoders(Result<Vec<EncoderInfo>, String>),
    /// `StreamSession::start` falhou antes de começar.
    StartFailed(String),
    Event(SessionEvent),
    Stats(SessionStats),
    AudioInputs(Result<Vec<AudioInputInfo>, String>),
}

pub struct Engine {
    tx: mpsc::Sender<Command>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Engine {
    /// `deliver` é chamado da thread do motor (ou de threads internas da sessão);
    /// cabe a ele levar a mensagem à interface.
    pub fn spawn(deliver: impl Fn(EngineMsg) + Send + Sync + 'static) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel::<Command>();
        let deliver = std::sync::Arc::new(deliver);
        let thread = thread::Builder::new()
            .name("vellcast-engine".into())
            .spawn(move || run(rx, deliver))?;
        Ok(Self {
            tx,
            thread: Some(thread),
        })
    }

    pub fn send(&self, cmd: Command) {
        if self.tx.send(cmd).is_err() {
            tracing::error!("motor de transmissão indisponível");
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn run(rx: mpsc::Receiver<Command>, deliver: std::sync::Arc<dyn Fn(EngineMsg) + Send + Sync>) {
    let mut session: Option<StreamSession> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::ListSources => {
                deliver(EngineMsg::Sources(
                    vellcast_core::list_sources().map_err(|e| e.to_string()),
                ));
            }
            Command::ListEncoders => {
                deliver(EngineMsg::Encoders(
                    vellcast_core::list_encoders().map_err(|e| e.to_string()),
                ));
            }
            Command::Start(config) => {
                if let Some(old) = session.take() {
                    old.stop();
                }
                let sink = std::sync::Arc::clone(&deliver);
                match StreamSession::start(*config, Box::new(move |e| sink(EngineMsg::Event(e)))) {
                    Ok(s) => session = Some(s),
                    Err(e) => deliver(EngineMsg::StartFailed(e.to_string())),
                }
            }
            Command::Stop => {
                if let Some(s) = session.take() {
                    s.stop();
                }
            }
            Command::Stats => {
                if let Some(s) = &session {
                    deliver(EngineMsg::Stats(s.stats()));
                }
            }
            Command::Apply(change) => {
                if let Some(s) = &session {
                    s.apply(change);
                }
            }
            Command::ListAudioInputs => {
                deliver(EngineMsg::AudioInputs(
                    vellcast_core::list_audio_inputs().map_err(|e| e.to_string()),
                ));
            }
            Command::Shutdown => break,
        }
    }
    if let Some(s) = session.take() {
        s.stop();
    }
}
