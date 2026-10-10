//! Entrada em salas e compartilhamento por participantes (issue #4).
//!
//! - Início: "Criar sala" reaproveita a criação da tela Salas; "Entrar com
//!   link" troca um convite do Vellcast por um token de participante e guarda a
//!   sala na mesma lista das salas próprias, marcada como `joined`.
//! - Sala: qualquer membro pode pedir para compartilhar. O botão só leva à
//!   tela Transmitir com a sala escolhida; a captura continua exigindo o
//!   "Iniciar transmissão" explícito (M-024). O relay faz a troca de quem
//!   compartilha, mantendo um único transmissor por sala.
//!
//! Códigos de convite e tokens nunca são registrados em log.

use std::cell::RefCell;

use slint::{ComponentHandle, SharedString};

use crate::actions::{self, background, en, t};
use crate::relay_api::{self, ApiError, InviteLink};
use crate::rooms::SavedRoom;
use crate::ui::AppWindow;
use crate::with_state;

/// Página Transmitir na barra lateral.
const STREAM_PAGE: i32 = 1;
/// Página Salas na barra lateral.
const ROOMS_PAGE: i32 = 2;

pub fn wire(ui: &AppWindow) {
    let w = ui.as_weak();
    ui.on_join_room(move |link| {
        if let Some(ui) = w.upgrade() {
            join_from_link(&ui, link.as_str());
        }
    });
    let w = ui.as_weak();
    ui.on_paste_invite(move || {
        if let Some(ui) = w.upgrade() {
            paste_invite(&ui);
        }
    });
    let w = ui.as_weak();
    ui.on_create_room_from_home(move || {
        if let Some(ui) = w.upgrade() {
            crate::navigate_to(&ui, ROOMS_PAGE);
            let name = ui.get_new_room_name().to_string();
            actions::create_room(&ui, &name);
        }
    });
    let w = ui.as_weak();
    ui.on_refresh_room_status(move |index| {
        if let Some(ui) = w.upgrade() {
            refresh_room_status(&ui, index);
        }
    });
    let w = ui.as_weak();
    ui.on_share_screen(move |index| {
        if let Some(ui) = w.upgrade() {
            share_screen(&ui, index);
        }
    });
    let w = ui.as_weak();
    ui.on_stop_share(move || {
        if let Some(ui) = w.upgrade() {
            actions::stop_stream(&ui, None);
            refresh_room_status(&ui, ui.get_rooms_index());
        }
    });
    let w = ui.as_weak();
    ui.on_watch_room(move |index| {
        if let Some(ui) = w.upgrade() {
            watch_room(&ui, index);
        }
    });
    let w = ui.as_weak();
    ui.on_leave_room(move |index| {
        if let Some(ui) = w.upgrade() {
            leave_room(&ui, index);
        }
    });
}

fn room_at(index: i32) -> Option<SavedRoom> {
    with_state(|st| st.rooms.rooms.get(usize::try_from(index).ok()?).cloned()).flatten()
}

fn set_home_message(ui: &AppWindow, text: &str, error: bool) {
    ui.set_home_message(text.into());
    ui.set_home_message_error(error);
}

/// Salas em que este PC é só participante não aceitam ações de dono.
/// Devolve `true` (e avisa) quando a ação deve ser recusada.
pub fn refuse_owner_action(ui: &AppWindow, room: &SavedRoom) -> bool {
    if room.joined {
        actions::set_rooms_message(
            ui,
            &t(
                "Só quem criou a sala pode gerar convites, parear com o Discord ou encerrá-la.",
                "Only the room creator can create invites, pair with Discord or close it.",
            ),
            true,
        );
    }
    room.joined
}

// ---------------------------------------------------------------------------
// Entrar com link

fn paste_invite(ui: &AppWindow) {
    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
        Ok(text) => {
            let text: String = text.trim().chars().take(1024).collect();
            ui.set_join_link(text.into());
            set_home_message(ui, "", false);
        }
        Err(e) => {
            tracing::warn!(%e, "área de transferência indisponível");
            set_home_message(
                ui,
                &t(
                    "Não foi possível ler a área de transferência. Cole o link no campo.",
                    "Could not read the clipboard. Paste the link into the field.",
                ),
                true,
            );
        }
    }
}

/// Mensagem para falhas ao entrar com um convite.
fn join_error_message(e: &ApiError) -> String {
    match e {
        ApiError::Rejected { status: 403, .. } | ApiError::Unauthorized => t(
            "Convite inválido, expirado ou já usado. Peça um novo link a quem criou a sala.",
            "Invalid, expired or already used invite. Ask the room creator for a new link.",
        )
        .to_string(),
        ApiError::RoomGone => t(
            "Esta sala não existe mais (expirou ou foi encerrada).",
            "This room no longer exists (it expired or was closed).",
        )
        .to_string(),
        other => other.user_message(en()),
    }
}

fn join_from_link(ui: &AppWindow, input: &str) {
    let link = match relay_api::parse_invite_link(input) {
        Ok(link) => link,
        Err(e) => {
            set_home_message(ui, &e.message(en()), true);
            return;
        }
    };
    // Já está na lista (como dono ou participante): abre a sala sem gastar o convite.
    let existing = with_state(|st| {
        st.rooms
            .rooms
            .iter()
            .position(|r| r.id == link.room && r.relay == link.base)
    })
    .flatten();
    if let Some(index) = existing {
        open_room(ui, index as i32);
        ui.set_join_link(SharedString::new());
        set_home_message(ui, "", false);
        actions::set_rooms_message(
            ui,
            &t("Você já está nesta sala.", "You are already in this room."),
            false,
        );
        return;
    }
    ui.set_joining(true);
    set_home_message(ui, &t("Entrando na sala…", "Joining the room…"), false);
    let target = link.clone();
    // O nome de exibição vai assinado no token: aparece na lista de participantes.
    let guest = with_state(|st| st.settings.display_name.clone()).unwrap_or_default();
    background(
        ui,
        move || relay_api::join_room(&target, &guest),
        move |ui, result| {
            ui.set_joining(false);
            match result {
                Ok(joined) => on_joined(ui, &link, joined),
                Err(e) => set_home_message(ui, &join_error_message(&e), true),
            }
        },
    );
}

fn on_joined(ui: &AppWindow, link: &InviteLink, joined: relay_api::Joined) {
    let expires_at = relay_api::token_expiry(&joined.viewer_token)
        .map_or(joined.room.expires_at, |exp| {
            exp.min(joined.room.expires_at)
        });
    let room = SavedRoom {
        id: link.room.clone(),
        name: joined.room.name.clone(),
        relay: link.base.clone(),
        created_at: actions::now_unix(),
        expires_at,
        joined: true,
        viewer_token: joined.viewer_token,
        ..SavedRoom::default()
    };
    with_state(|st| st.rooms.add(room));
    actions::save_rooms();
    actions::refresh_rooms_ui(ui);
    ui.set_join_link(SharedString::new());
    set_home_message(ui, "", false);
    open_room(ui, 0);
    let msg = if en() {
        format!(
            "You joined “{}”. Watch the current share or select Share my screen.",
            joined.room.name
        )
    } else {
        format!(
            "Você entrou em “{}”. Assista a quem está compartilhando ou toque em Compartilhar minha tela.",
            joined.room.name
        )
    };
    actions::set_rooms_message(ui, &msg, false);
}

/// Abre a tela Salas com a sala `index` selecionada.
fn open_room(ui: &AppWindow, index: i32) {
    ui.set_rooms_index(index);
    crate::navigate_to(ui, ROOMS_PAGE);
    refresh_room_status(ui, index);
}

// ---------------------------------------------------------------------------
// Sala: quem compartilha, compartilhar, assistir e sair

/// Texto de estado e indicadores a partir da sala no relay.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ShareStatus {
    text: String,
    /// Há algo para assistir (alguém compartilhando ou reconectando).
    active: bool,
}

fn share_status(view: &relay_api::RoomView, en: bool) -> ShareStatus {
    let name = view
        .sharer_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let watching = if en {
        format!(" · {} watching", view.viewers)
    } else {
        format!(" · {} assistindo", view.viewers)
    };
    match (view.state.as_str(), name, en) {
        ("live", Some(n), false) => ShareStatus {
            text: format!("{n} está compartilhando a tela{watching}"),
            active: true,
        },
        ("live", Some(n), true) => ShareStatus {
            text: format!("{n} is sharing their screen{watching}"),
            active: true,
        },
        ("live", None, false) => ShareStatus {
            text: format!("Alguém está compartilhando a tela{watching}"),
            active: true,
        },
        ("live", None, true) => ShareStatus {
            text: format!("Someone is sharing their screen{watching}"),
            active: true,
        },
        ("paused", _, false) => ShareStatus {
            text: "Quem compartilhava caiu; aguardando a reconexão…".into(),
            active: true,
        },
        ("paused", _, true) => ShareStatus {
            text: "The person sharing dropped; waiting for them to reconnect…".into(),
            active: true,
        },
        (_, _, false) => ShareStatus {
            text: "Ninguém está compartilhando agora.".into(),
            active: false,
        },
        (_, _, true) => ShareStatus {
            text: "Nobody is sharing right now.".into(),
            active: false,
        },
    }
}

thread_local! {
    /// Sala cujo estado está na tela (só a thread da interface usa).
    static STATUS_ROOM: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn sharing_here(room_id: &str) -> bool {
    with_state(|st| {
        st.live
            .as_ref()
            .is_some_and(|l| l.room_id == room_id && l.started.is_some())
    })
    .unwrap_or(false)
}

/// Consulta o relay e atualiza quem compartilha na sala selecionada.
/// Chamado ao abrir/selecionar a sala e por um timer de 5 s só enquanto a
/// tela Salas está aberta (e a janela visível).
fn refresh_room_status(ui: &AppWindow, index: i32) {
    let visible = with_state(|st| st.window_visible).unwrap_or(false);
    let room = room_at(index);
    // Outra sala selecionada: o estado anterior não vale para ela.
    let changed = STATUS_ROOM.with(|last| {
        let id = room.as_ref().map(|r| r.id.clone());
        let changed = *last.borrow() != id;
        *last.borrow_mut() = id;
        changed
    });
    if changed {
        ui.set_room_share_status(SharedString::new());
        ui.set_room_share_active(false);
        ui.set_room_share_mine(false);
    }
    let Some(room) = room else {
        return;
    };
    let mine = sharing_here(&room.id);
    ui.set_room_share_mine(mine);
    if mine {
        ui.set_room_share_status(t(
            "Você está compartilhando a tela nesta sala.",
            "You are sharing your screen in this room.",
        ));
        ui.set_room_share_active(true);
    }
    if !visible {
        return;
    }
    let (relay, id, token) = (
        room.relay.clone(),
        room.id.clone(),
        room.access_token().to_owned(),
    );
    background(
        ui,
        move || relay_api::get_room(&relay, &id, &token),
        move |ui, result| {
            // A seleção pode ter mudado enquanto a consulta rodava.
            let still_selected = room_at(ui.get_rooms_index()).is_some_and(|r| r.id == room.id);
            match result {
                Ok(view) => {
                    if !still_selected || sharing_here(&room.id) {
                        return;
                    }
                    let status = share_status(&view, en());
                    ui.set_room_share_status(status.text.into());
                    ui.set_room_share_active(status.active);
                }
                Err(ApiError::RoomGone) => {
                    actions::forget_room(ui, &room.id);
                    actions::set_rooms_message(ui, &ApiError::RoomGone.user_message(en()), true);
                }
                Err(ApiError::Unauthorized) if room.joined => {
                    actions::forget_room(ui, &room.id);
                    actions::set_rooms_message(
                        ui,
                        &ApiError::ParticipantRevoked.user_message(en()),
                        true,
                    );
                }
                Err(e) => {
                    if still_selected {
                        ui.set_room_share_status(e.user_message(en()).into());
                    }
                }
            }
        },
    );
}

/// "Compartilhar minha tela": leva à escolha explícita do que compartilhar,
/// com esta sala como destino. Nada é capturado até "Iniciar transmissão".
fn share_screen(ui: &AppWindow, index: i32) {
    let Some(room) = room_at(index) else { return };
    let busy = with_state(|st| st.live.is_some()).unwrap_or(false);
    if busy {
        actions::set_rooms_message(
            ui,
            &t(
                "Você já está transmitindo. Pare a transmissão atual antes de compartilhar em outra sala.",
                "You are already streaming. Stop the current stream before sharing in another room.",
            ),
            true,
        );
        return;
    }
    ui.set_stream_room_index(index);
    let someone = ui.get_room_share_active() && !ui.get_room_share_mine();
    let msg = match (someone, en()) {
        (true, false) => format!(
            "Escolha o que compartilhar e toque em Iniciar transmissão. Na sala “{}”, a vez passa para você e quem compartilha agora é avisado.",
            room.name
        ),
        (true, true) => format!(
            "Choose what to share and select Start streaming. In “{}”, it becomes your turn and the current sharer is notified.",
            room.name
        ),
        (false, false) => format!(
            "Escolha o que compartilhar e toque em Iniciar transmissão para compartilhar em “{}”.",
            room.name
        ),
        (false, true) => format!(
            "Choose what to share and select Start streaming to share in “{}”.",
            room.name
        ),
    };
    ui.set_stream_message(msg.into());
    ui.set_stream_message_error(false);
    crate::navigate_to(ui, STREAM_PAGE);
}

/// Assistir pelo player web no navegador (o app não tem player próprio).
fn watch_room(ui: &AppWindow, index: i32) {
    let Some(room) = room_at(index) else { return };
    if room.joined {
        // Token do participante no fragmento: não gasta outro uso do convite.
        match relay_api::watch_link_with_token(&room.relay, &room.id, &room.viewer_token) {
            Ok(link) => open_watch_link(ui, &room.relay, &link),
            Err(e) => actions::set_rooms_message(ui, e.message(en()), true),
        }
        return;
    }
    // Dono: um convite novo para o próprio navegador (o token de dono nunca vai a uma URL).
    let public = with_state(|st| st.settings.public_url.clone()).unwrap_or_default();
    let link_base = if public.trim().is_empty() {
        room.relay.clone()
    } else {
        public
    };
    ui.set_rooms_busy(true);
    let (relay, id, owner) = (
        room.relay.clone(),
        room.id.clone(),
        room.owner_token.clone(),
    );
    background(
        ui,
        move || relay_api::create_invite(&relay, &id, &owner),
        move |ui, result| {
            ui.set_rooms_busy(false);
            match result.and_then(|inv| {
                relay_api::watch_link(&link_base, &room.id, &inv.code).map_err(ApiError::Url)
            }) {
                Ok(link) => open_watch_link(ui, &link_base, &link),
                Err(e) => actions::set_rooms_message(ui, &e.user_message(en()), true),
            }
        },
    );
}

fn open_watch_link(ui: &AppWindow, base: &str, link: &str) {
    match open_in_browser(link) {
        Ok(()) => {
            let (msg, warn) = if relay_api::link_plays_in_browser(base) {
                (
                    t(
                        "O player abriu no navegador.",
                        "The player opened in your browser.",
                    ),
                    false,
                )
            } else {
                (
                    t(
                        "O player abriu no navegador, mas sem HTTPS o vídeo pode não decodificar (WebCodecs exige conexão segura).",
                        "The player opened in your browser, but without HTTPS the video may not decode (WebCodecs needs a secure connection).",
                    ),
                    true,
                )
            };
            actions::set_rooms_message(ui, &msg, warn);
        }
        Err(e) => {
            tracing::warn!(%e, "não foi possível abrir o navegador");
            actions::set_rooms_message(
                ui,
                &t(
                    "Não foi possível abrir o navegador.",
                    "Could not open the browser.",
                ),
                true,
            );
        }
    }
}

/// Abre a URL no navegador padrão, sem passar por um shell.
fn open_in_browser(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32.exe");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    let mut child = cmd.spawn()?;
    // Recolhe o processo auxiliar sem bloquear a interface.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Sair de uma sala em que entrou por convite (para de compartilhar, se for o caso).
fn leave_room(ui: &AppWindow, index: i32) {
    let Some(room) = room_at(index) else { return };
    if !room.joined {
        return;
    }
    let live_here =
        with_state(|st| st.live.as_ref().is_some_and(|l| l.room_id == room.id)).unwrap_or(false);
    if live_here {
        // Libera a vaga de compartilhamento na hora (StreamEnd).
        actions::stop_stream(ui, None);
    }
    actions::forget_room(ui, &room.id);
    ui.set_room_share_status(SharedString::new());
    ui.set_room_share_active(false);
    ui.set_room_share_mine(false);
    let msg = if en() {
        format!(
            "You left “{}”. To come back, use an invite link again.",
            room.name
        )
    } else {
        format!(
            "Você saiu de “{}”. Para voltar, use um link de convite de novo.",
            room.name
        )
    };
    actions::set_rooms_message(ui, &msg, false);
}

/// Token de transmissor para compartilhar na sala: `POST /share` com o token
/// de dono ou de participante. Dono em relay antigo (sem `/share`) usa o
/// token de transmissor comum, sem troca automática.
pub fn publisher_token_for(room: &SavedRoom, display_name: &str) -> Result<String, ApiError> {
    match relay_api::share_token(&room.relay, &room.id, room.access_token(), display_name) {
        Err(ApiError::ShareUnsupported) if !room.joined => {
            relay_api::publisher_token(&room.relay, &room.id, &room.owner_token)
        }
        Err(ApiError::Unauthorized | ApiError::Rejected { status: 403, .. }) if room.joined => {
            Err(ApiError::ParticipantRevoked)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(state: &str, sharer: Option<&str>) -> relay_api::RoomView {
        relay_api::RoomView {
            id: "r".into(),
            name: "Sala".into(),
            state: state.into(),
            viewers: 2,
            max_viewers: 10,
            expires_at: 0,
            sharer_name: sharer.map(Into::into),
        }
    }

    #[test]
    fn status_names_who_is_sharing() {
        let s = share_status(&view("live", Some("Ana")), false);
        assert_eq!(s.text, "Ana está compartilhando a tela · 2 assistindo");
        assert!(s.active);
        assert!(
            share_status(&view("live", Some("Ana")), true)
                .text
                .starts_with("Ana is sharing")
        );
        // Sem nome (ex.: transmissor antigo) ou só espaços: texto genérico.
        assert!(
            share_status(&view("live", Some("  ")), false)
                .text
                .starts_with("Alguém")
        );
        assert!(share_status(&view("paused", None), false).active);
        let idle = share_status(&view("idle", None), false);
        assert!(!idle.active);
        assert_eq!(idle.text, "Ninguém está compartilhando agora.");
    }
}
