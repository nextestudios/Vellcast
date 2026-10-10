//! Controles rápidos durante a transmissão (issue #5).
//!
//! Lógica pura, testável em qualquer SO:
//!
//! - quais opções de resolução, FPS e bitrate o Vellcast e o plano oferecem
//!   ([`resolution_options`], [`fps_options`], [`BITRATE_RANGE_KBPS`]);
//! - o que muda sem interromper a imagem e o que exige reiniciar captura e
//!   encoder ([`plan_video_change`]).
//!
//! Regras:
//!
//! | Mudança | Como é aplicada |
//! |---|---|
//! | Preferência de conteúdo (nitidez × movimento) | ao vivo (modo da adaptação) |
//! | Bitrate | ao vivo, se o encoder aceitou mudar o bitrate; senão reinício |
//! | Fonte (tela/janela) | reinício de captura e encoder |
//! | Resolução ou FPS | reinício do encoder (parâmetros fixos no Media Foundation) |
//! | Áudio do jogo, microfone, mudo | ao vivo (thread de áudio) |
//!
//! O reinício mantém a conexão com a sala: os espectadores veem a imagem
//! congelar por um instante e o player se reconfigura no próximo keyframe.

use crate::{Adaptation, Quality, SourceId};

/// Preferência de conteúdo mostrada ao usuário.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContentPreference {
    /// Vídeo e jogos: mantém o FPS e reduz o bitrate quando a rede aperta.
    #[default]
    Motion,
    /// Texto e código: troca FPS por nitidez ([`Adaptation::Clarity`]).
    Clarity,
}

impl ContentPreference {
    /// Modo de adaptação para esta preferência. `motion_mode` é o modo do perfil
    /// escolhido antes de transmitir (equilibrado ou competitivo).
    pub fn adaptation(self, motion_mode: Adaptation) -> Adaptation {
        match self {
            Self::Clarity => Adaptation::Clarity,
            Self::Motion if motion_mode == Adaptation::Clarity => Adaptation::Balanced,
            Self::Motion => motion_mode,
        }
    }

    pub fn from_adaptation(mode: Adaptation) -> Self {
        if mode == Adaptation::Clarity {
            Self::Clarity
        } else {
            Self::Motion
        }
    }
}

/// Limites de qualidade do plano (entitlement da Vellcast Cloud).
///
/// O plano gratuito nunca fica abaixo de 1080p60 (W-014): limites menores
/// vindos de um token são ignorados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanLimits {
    pub max_height: u32,
    pub max_fps: u32,
}

impl PlanLimits {
    pub const FREE: Self = Self {
        max_height: 1080,
        max_fps: 60,
    };

    /// Limites a partir de um token de entitlement (`max_height`, `max_fps`).
    pub fn from_entitlement(max_height: u32, max_fps: u32) -> Self {
        Self {
            max_height: max_height.max(Self::FREE.max_height),
            max_fps: max_fps.max(Self::FREE.max_fps),
        }
    }
}

impl Default for PlanLimits {
    fn default() -> Self {
        Self::FREE
    }
}

/// Por que uma opção aparece, mas não pode ser escolhida.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// O plano atual não inclui.
    Plan,
    /// O plano inclui, mas o pipeline do Vellcast ainda não suporta.
    Engine,
}

fn availability(value: u32, plan_max: u32, engine_max: u32) -> Option<Unavailable> {
    if value > plan_max {
        Some(Unavailable::Plan)
    } else if value > engine_max {
        Some(Unavailable::Engine)
    } else {
        None
    }
}

/// Alturas oferecidas (16:9).
pub const RESOLUTION_HEIGHTS: [u32; 5] = [720, 900, 1080, 1440, 2160];
/// Taxas de quadros oferecidas.
pub const FPS_CHOICES: [u32; 3] = [30, 60, 120];
/// Faixa e passo do bitrate escolhido pelo usuário (mesmos da tela Transmitir).
pub const BITRATE_RANGE_KBPS: (u32, u32) = (1000, Quality::MAX_BITRATE_KBPS);
pub const BITRATE_STEP_KBPS: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolutionOption {
    pub width: u32,
    pub height: u32,
    pub unavailable: Option<Unavailable>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FpsOption {
    pub fps: u32,
    pub unavailable: Option<Unavailable>,
}

/// Resoluções com a disponibilidade real (plano e pipeline).
pub fn resolution_options(plan: PlanLimits) -> Vec<ResolutionOption> {
    RESOLUTION_HEIGHTS
        .iter()
        .map(|&height| ResolutionOption {
            width: height * 16 / 9,
            height,
            unavailable: availability(height, plan.max_height, Quality::MAX_HEIGHT),
        })
        .collect()
}

/// Taxas de quadros com a disponibilidade real (plano e pipeline).
pub fn fps_options(plan: PlanLimits) -> Vec<FpsOption> {
    FPS_CHOICES
        .iter()
        .map(|&fps| FpsOption {
            fps,
            unavailable: availability(fps, plan.max_fps, Quality::MAX_FPS),
        })
        .collect()
}

/// Bitrate escolhido arredondado ao passo e limitado à faixa.
pub fn snap_bitrate(kbps: u32) -> u32 {
    let (min, max) = BITRATE_RANGE_KBPS;
    let snapped = (kbps + BITRATE_STEP_KBPS / 2) / BITRATE_STEP_KBPS * BITRATE_STEP_KBPS;
    snapped.clamp(min, max)
}

/// O que importa do vídeo para decidir como aplicar uma mudança.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSettings {
    pub source: SourceId,
    pub max_height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub adaptation: Adaptation,
}

/// Motivo de um reinício de captura/encoder (mostrado antes de confirmar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartReason {
    Source,
    Resolution,
    FrameRate,
    /// O encoder recusou mudar o bitrate em tempo real.
    Bitrate,
}

/// Como aplicar a passagem de um [`VideoSettings`] para outro.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChangePlan {
    /// Novo bitrate aplicado ao vivo.
    pub live_bitrate: Option<u32>,
    /// Novo modo de adaptação aplicado ao vivo.
    pub live_adaptation: Option<Adaptation>,
    /// Vazio = nada a reiniciar.
    pub restart: Vec<RestartReason>,
}

impl ChangePlan {
    pub fn is_empty(&self) -> bool {
        self.live_bitrate.is_none() && self.live_adaptation.is_none() && self.restart.is_empty()
    }

    pub fn needs_restart(&self) -> bool {
        !self.restart.is_empty()
    }
}

/// Decide o que muda ao vivo e o que exige reiniciar.
///
/// `live_bitrate` diz se o encoder atual aceita mudar o bitrate em tempo real
/// (`SessionStats::live_bitrate != Some(false)`). Com reinício, o bitrate e o
/// modo novos entram junto no reinício (não aparecem como mudanças ao vivo).
pub fn plan_video_change(
    current: &VideoSettings,
    next: &VideoSettings,
    live_bitrate: bool,
) -> ChangePlan {
    let mut plan = ChangePlan::default();
    if next.source != current.source {
        plan.restart.push(RestartReason::Source);
    }
    if next.max_height != current.max_height {
        plan.restart.push(RestartReason::Resolution);
    }
    if next.fps != current.fps {
        plan.restart.push(RestartReason::FrameRate);
    }
    let bitrate_changed = next.bitrate_kbps != current.bitrate_kbps;
    if bitrate_changed && !live_bitrate {
        plan.restart.push(RestartReason::Bitrate);
    }
    if plan.restart.is_empty() {
        if bitrate_changed {
            plan.live_bitrate = Some(next.bitrate_kbps);
        }
        if next.adaptation != current.adaptation {
            plan.live_adaptation = Some(next.adaptation);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> VideoSettings {
        VideoSettings {
            source: SourceId(1),
            max_height: 1080,
            fps: 60,
            bitrate_kbps: 8000,
            adaptation: Adaptation::Balanced,
        }
    }

    #[test]
    fn free_plan_keeps_1080p60_and_marks_higher_options_unavailable() {
        let res = resolution_options(PlanLimits::FREE);
        let usable: Vec<u32> = res
            .iter()
            .filter(|o| o.unavailable.is_none())
            .map(|o| o.height)
            .collect();
        assert_eq!(usable, vec![720, 900, 1080]);
        assert_eq!(
            res.iter().find(|o| o.height == 1440).map(|o| o.unavailable),
            Some(Some(Unavailable::Plan))
        );
        assert_eq!(res[2].width, 1920);
        let fps: Vec<(u32, Option<Unavailable>)> = fps_options(PlanLimits::FREE)
            .iter()
            .map(|o| (o.fps, o.unavailable))
            .collect();
        assert_eq!(
            fps,
            vec![(30, None), (60, None), (120, Some(Unavailable::Plan))]
        );
    }

    #[test]
    fn paid_plan_above_engine_limit_is_still_not_usable() {
        let pro = PlanLimits::from_entitlement(2160, 120);
        let res = resolution_options(pro);
        assert!(res.iter().all(|o| o.unavailable != Some(Unavailable::Plan)));
        assert_eq!(
            res.iter()
                .filter(|o| o.unavailable == Some(Unavailable::Engine))
                .map(|o| o.height)
                .collect::<Vec<_>>(),
            vec![1440, 2160]
        );
        assert_eq!(fps_options(pro)[2].unavailable, Some(Unavailable::Engine));
    }

    #[test]
    fn entitlement_never_lowers_free_floor() {
        let weird = PlanLimits::from_entitlement(720, 30);
        assert_eq!(weird, PlanLimits::FREE);
        assert!(
            resolution_options(weird)
                .iter()
                .any(|o| o.height == 1080 && o.unavailable.is_none())
        );
    }

    #[test]
    fn bitrate_snaps_to_step_and_range() {
        assert_eq!(snap_bitrate(8_240), 8_000);
        assert_eq!(snap_bitrate(8_250), 8_500);
        assert_eq!(snap_bitrate(0), 1_000);
        assert_eq!(snap_bitrate(99_999), Quality::MAX_BITRATE_KBPS);
    }

    #[test]
    fn bitrate_and_content_preference_apply_live() {
        let next = VideoSettings {
            bitrate_kbps: 12_000,
            adaptation: Adaptation::Clarity,
            ..base()
        };
        let plan = plan_video_change(&base(), &next, true);
        assert!(!plan.needs_restart());
        assert_eq!(plan.live_bitrate, Some(12_000));
        assert_eq!(plan.live_adaptation, Some(Adaptation::Clarity));
    }

    #[test]
    fn source_resolution_and_fps_require_restart() {
        let next = VideoSettings {
            source: SourceId(2),
            max_height: 720,
            fps: 30,
            ..base()
        };
        let plan = plan_video_change(&base(), &next, true);
        assert_eq!(
            plan.restart,
            vec![
                RestartReason::Source,
                RestartReason::Resolution,
                RestartReason::FrameRate
            ]
        );
        assert_eq!(plan.live_bitrate, None);
    }

    #[test]
    fn bitrate_requires_restart_when_encoder_refuses_live_changes() {
        let next = VideoSettings {
            bitrate_kbps: 4000,
            ..base()
        };
        let plan = plan_video_change(&base(), &next, false);
        assert_eq!(plan.restart, vec![RestartReason::Bitrate]);
        assert_eq!(plan.live_bitrate, None);
    }

    #[test]
    fn restart_absorbs_live_changes_and_no_change_is_empty() {
        let next = VideoSettings {
            fps: 30,
            bitrate_kbps: 4000,
            adaptation: Adaptation::Clarity,
            ..base()
        };
        let plan = plan_video_change(&base(), &next, true);
        assert_eq!(plan.restart, vec![RestartReason::FrameRate]);
        assert_eq!(plan.live_bitrate, None);
        assert_eq!(plan.live_adaptation, None);
        assert!(plan_video_change(&base(), &base(), true).is_empty());
    }

    #[test]
    fn content_preference_maps_to_adaptation() {
        assert_eq!(
            ContentPreference::Clarity.adaptation(Adaptation::Competitive),
            Adaptation::Clarity
        );
        assert_eq!(
            ContentPreference::Motion.adaptation(Adaptation::Competitive),
            Adaptation::Competitive
        );
        assert_eq!(
            ContentPreference::Motion.adaptation(Adaptation::Clarity),
            Adaptation::Balanced
        );
        assert_eq!(
            ContentPreference::from_adaptation(Adaptation::Clarity),
            ContentPreference::Clarity
        );
        assert_eq!(
            ContentPreference::from_adaptation(Adaptation::Off),
            ContentPreference::Motion
        );
    }
}
