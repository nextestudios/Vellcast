//! Repintura completa da janela quando o sistema descarta o conteúdo dela (issue #6).
//!
//! O renderer de software do Slint só redesenha as regiões que mudaram desde o
//! último quadro e confia que o Windows preservou o resto. Quando o sistema
//! descarta a superfície da janela (tela que dorme e acorda, bloqueio de sessão,
//! reinício do DWM ou do driver de vídeo, janela ocultada por fora do Slint), ele
//! manda `WM_PAINT` com uma região inválida, o Slint não encontra nada "sujo" e
//! não pinta nada. Como o backend winit cria a janela transparente, a área vazia
//! mostrava o papel de parede, e só os controles redesenhados depois (o botão
//! "Copiar link") voltavam a aparecer. Detalhes na ADR-063.
//!
//! A correção tem duas partes: a janela nativa passa a ser opaca e, quando o
//! sistema pede repintura (ou a janela volta a ficar visível ou em foco), a
//! janela inteira é marcada para redesenho. Os redesenhos pedidos pelo próprio
//! Slint continuam parciais, então o custo ocioso não muda.

use slint::ComponentHandle;
use slint::winit_030::winit;

use crate::ui::AppWindow;

/// Por que a janela inteira precisa ser redesenhada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reason {
    /// O sistema invalidou a janela (`WM_PAINT` com região inválida).
    SystemInvalidated,
    /// A janela recebeu o foco de novo.
    FocusGained,
    /// A janela deixou de estar encoberta ou minimizada.
    Uncovered,
}

/// Decide se o evento exige redesenhar a janela inteira.
///
/// `system_invalidated` só é consultado em `RedrawRequested`: informa se o
/// pedido veio do sistema (região inválida) e não do próprio Slint.
pub(crate) fn full_repaint_reason(
    event: &winit::event::WindowEvent,
    system_invalidated: impl FnOnce() -> bool,
) -> Option<Reason> {
    use winit::event::WindowEvent;
    match event {
        WindowEvent::RedrawRequested => system_invalidated().then_some(Reason::SystemInvalidated),
        WindowEvent::Focused(true) => Some(Reason::FocusGained),
        WindowEvent::Occluded(false) => Some(Reason::Uncovered),
        _ => None,
    }
}

/// Marca a janela inteira para o próximo quadro.
///
/// O Slint não expõe como invalidar o cache de regiões do renderer de software;
/// a forma pública é mudar uma propriedade lida no desenho de um item do
/// tamanho da janela. O fundo da `Window` alterna entre a cor sólida e um
/// gradiente da mesma cor (pixels idênticos), o que basta para o renderer
/// considerar a janela toda alterada.
pub(crate) fn force_full_repaint(ui: &AppWindow) {
    ui.set_repaint_toggle(!ui.get_repaint_toggle());
    ui.window().request_redraw();
}

/// Trata um evento do winit antes do Slint; devolve o motivo, se houve repintura.
pub(crate) fn on_window_event(
    ui: &AppWindow,
    window: &slint::Window,
    event: &winit::event::WindowEvent,
) -> Option<Reason> {
    let reason = full_repaint_reason(event, || system_invalidated(window))?;
    tracing::debug!(?reason, "repintura completa da janela");
    force_full_repaint(ui);
    Some(reason)
}

/// Seleciona o backend com a janela nativa opaca.
///
/// O backend winit do Slint cria todas as janelas com `with_transparent(true)`,
/// e no Windows o winit chama `DwmEnableBlurBehindWindow`: todo pixel que o app
/// não pintou fica transparente. Com a janela opaca, uma falha de pintura não
/// mostra a área de trabalho por baixo. `SLINT_BACKEND` continua valendo.
pub(crate) fn select_opaque_backend() {
    let selected = slint::BackendSelector::new()
        .with_winit_window_attributes_hook(|attributes| attributes.with_transparent(false))
        .select();
    if let Err(e) = selected {
        tracing::warn!(%e, "não foi possível configurar a janela opaca; usando o padrão");
    }
}

/// `true` quando o `WM_PAINT` em curso tem região inválida, isto é, foi o
/// sistema que pediu a repintura. Os pedidos do Slint chegam pelo winit com
/// `RedrawWindow(RDW_INTERNALPAINT)`, sem região inválida.
#[cfg(windows)]
#[allow(unsafe_code)]
fn system_invalidated(window: &slint::Window) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::GetUpdateRect;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

    window
        .with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => {
                let hwnd = HWND(h.hwnd.get() as *mut core::ffi::c_void);
                // SAFETY: `hwnd` é a janela viva do winit (estamos no callback dela,
                // na thread dona); sem retângulo de saída e sem apagar o fundo.
                unsafe { GetUpdateRect(hwnd, None, false) }.as_bool()
            }
            _ => false,
        })
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn system_invalidated(_window: &slint::Window) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::event::WindowEvent;

    #[test]
    fn redraw_from_system_repaints_everything() {
        assert_eq!(
            full_repaint_reason(&WindowEvent::RedrawRequested, || true),
            Some(Reason::SystemInvalidated)
        );
    }

    #[test]
    fn redraw_requested_by_slint_stays_partial() {
        assert_eq!(
            full_repaint_reason(&WindowEvent::RedrawRequested, || false),
            None
        );
    }

    #[test]
    fn focus_and_uncover_repaint_everything() {
        assert_eq!(
            full_repaint_reason(&WindowEvent::Focused(true), || unreachable!()),
            Some(Reason::FocusGained)
        );
        assert_eq!(
            full_repaint_reason(&WindowEvent::Occluded(false), || unreachable!()),
            Some(Reason::Uncovered)
        );
    }

    #[test]
    fn other_events_do_not_repaint() {
        for event in [
            WindowEvent::Focused(false),
            WindowEvent::Occluded(true),
            WindowEvent::CloseRequested,
        ] {
            assert_eq!(full_repaint_reason(&event, || unreachable!()), None);
        }
    }

    /// Com o renderer de software reaproveitando o buffer (como no Windows), um
    /// quadro sem mudanças não pinta nada; após `force_full_repaint`, a região
    /// redesenhada cobre a janela inteira.
    #[test]
    fn toggle_marks_whole_window_dirty() {
        use std::rc::Rc;

        use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
        use slint::platform::{Platform, WindowAdapter};

        struct TestPlatform(Rc<MinimalSoftwareWindow>);
        impl Platform for TestPlatform {
            fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
                Ok(self.0.clone())
            }
        }

        let window = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(window.clone())))
            .expect("plataforma de teste");
        let ui = AppWindow::new().expect("janela");
        ui.window().set_size(slint::PhysicalSize::new(400, 300));
        ui.show().expect("mostrar");

        let (w, h) = (400usize, 300usize);
        let mut buffer = vec![slint::Rgb8Pixel::default(); w * h];
        let draw = |buffer: &mut [slint::Rgb8Pixel]| {
            let mut covered = None;
            window.draw_if_needed(|r| {
                let size = r.render(buffer, w).bounding_box_size();
                covered = Some((size.width, size.height));
            });
            covered
        };

        assert_eq!(
            draw(&mut buffer),
            Some((400, 300)),
            "primeiro quadro é completo"
        );
        assert_eq!(draw(&mut buffer), None, "sem mudanças, nada é redesenhado");
        let first = buffer.clone();

        force_full_repaint(&ui);
        assert_eq!(
            draw(&mut buffer),
            Some((400, 300)),
            "repintura completa forçada"
        );
        assert_eq!(draw(&mut buffer), None);
        // Os dois estados da alternância produzem os mesmos pixels.
        assert!(buffer == first, "a alternância não pode mudar a imagem");

        force_full_repaint(&ui);
        assert_eq!(draw(&mut buffer), Some((400, 300)));
        assert!(buffer == first);
    }
}
