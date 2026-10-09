//! Ligação entre a página Sobre e o [`crate::updater`].
//!
//! Tudo acontece por clique do usuário; rede e disco rodam em threads curtas,
//! nunca na thread da interface.

use std::sync::Mutex;

use slint::{ComponentHandle, SharedString};

use crate::settings::Language;
use crate::ui::AppWindow;
use crate::updater::{self, CheckOutcome, Manifest};
use crate::with_state;

/// Manifesto verificado aguardando a confirmação do usuário.
static PENDING: Mutex<Option<Manifest>> = Mutex::new(None);

fn t(pt: &str, en: &str) -> SharedString {
    let english = with_state(|st| st.settings.language == Language::En).unwrap_or(false);
    SharedString::from(if english { en } else { pt })
}

fn set_status(ui: &AppWindow, text: SharedString, error: bool) {
    ui.set_update_status(text);
    ui.set_update_error(error);
}

fn spawn<T: Send + 'static>(
    ui: &AppWindow,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&AppWindow, T) + Send + 'static,
) {
    let weak = ui.as_weak();
    let spawned = std::thread::Builder::new()
        .name("vellcast-update".into())
        .spawn(move || {
            let result = work();
            let _ = weak.upgrade_in_event_loop(move |ui| done(&ui, result));
        });
    if let Err(e) = spawned {
        tracing::error!(%e, "não foi possível criar thread de atualização");
    }
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_check_updates(move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_update_busy(true);
        ui.set_update_available(false);
        set_status(&ui, t("Verificando…", "Checking…"), false);
        spawn(&ui, updater::check, |ui, result| {
            ui.set_update_busy(false);
            match result {
                Ok(CheckOutcome::UpToDate { current }) => {
                    let msg = t(
                        &format!("Você já tem a versão mais recente ({current})."),
                        &format!("You already have the latest version ({current})."),
                    );
                    set_status(ui, msg, false);
                }
                Ok(CheckOutcome::Available(manifest)) => {
                    let msg = t(
                        &format!("Versão {} disponível. {}", manifest.version, manifest.notes),
                        &format!("Version {} available. {}", manifest.version, manifest.notes),
                    );
                    set_status(ui, msg, false);
                    ui.set_update_available(true);
                    if let Ok(mut p) = PENDING.lock() {
                        *p = Some(manifest);
                    }
                }
                Err(e) => {
                    tracing::warn!(%e, "verificação de atualização falhou");
                    set_status(ui, SharedString::from(e.to_string()), true);
                }
            }
        });
    });

    let weak = ui.as_weak();
    ui.on_install_update(move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some(manifest) = PENDING.lock().ok().and_then(|p| p.clone()) else {
            return;
        };
        ui.set_update_busy(true);
        set_status(
            &ui,
            t(
                "Baixando e verificando o instalador…",
                "Downloading and verifying the installer…",
            ),
            false,
        );
        // O clique em "Baixar e instalar" é a confirmação explícita do usuário.
        spawn(
            &ui,
            move || updater::download(&manifest).and_then(|p| updater::launch(&p)),
            |ui, result| {
                ui.set_update_busy(false);
                match result {
                    Ok(()) => {
                        set_status(
                            ui,
                            t(
                                "Instalador verificado e aberto. Feche o Vellcast para concluir.",
                                "Installer verified and opened. Close Vellcast to finish.",
                            ),
                            false,
                        );
                    }
                    Err(e) => set_status(ui, SharedString::from(e.to_string()), true),
                }
            },
        );
    });
}
