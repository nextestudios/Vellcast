//! Fila de envio limitada do transmissor.
//!
//! Sob pressão da rede, quadros de vídeo delta são descartados e a fila
//! passa a esperar o próximo keyframe (um delta sem a referência anterior
//! não decodifica). Um keyframe novo torna obsoleto todo vídeo ainda na
//! fila. Áudio descarta os pacotes mais antigos. Nada cresce sem limite.

use std::collections::VecDeque;

use bytes::Bytes;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Control,
    Video { keyframe: bool },
    Audio,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub data: Bytes,
    pub kind: ItemKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Queued,
    /// Descartado; `need_keyframe` indica que o encoder deve gerar um IDR.
    Dropped {
        need_keyframe: bool,
    },
}

#[derive(Debug)]
pub struct SendQueue {
    items: VecDeque<Item>,
    bytes: usize,
    max_items: usize,
    max_bytes: usize,
    drop_until_keyframe: bool,
}

impl SendQueue {
    pub fn new(max_items: usize, max_bytes: usize) -> Self {
        Self {
            items: VecDeque::new(),
            bytes: 0,
            max_items: max_items.max(2),
            max_bytes,
            drop_until_keyframe: false,
        }
    }

    fn full_with(&self, extra: usize) -> bool {
        self.items.len() >= self.max_items || self.bytes + extra > self.max_bytes
    }

    fn remove_where(&mut self, pred: impl Fn(&Item) -> bool) -> usize {
        let before = self.items.len();
        let mut bytes = self.bytes;
        self.items.retain(|i| {
            let drop = pred(i);
            if drop {
                bytes -= i.data.len();
            }
            !drop
        });
        self.bytes = bytes;
        before - self.items.len()
    }

    fn push_back(&mut self, item: Item) {
        self.bytes += item.data.len();
        self.items.push_back(item);
    }

    /// Mensagens de controle (configuração, fim) nunca são descartadas.
    pub fn push_control(&mut self, data: Bytes) {
        self.push_back(Item {
            data,
            kind: ItemKind::Control,
        });
    }

    pub fn push_video(&mut self, data: Bytes, keyframe: bool) -> PushOutcome {
        if keyframe {
            // Vídeo ainda na fila ficou obsoleto: o keyframe é um novo ponto de partida.
            self.remove_where(|i| matches!(i.kind, ItemKind::Video { .. }));
            self.drop_until_keyframe = false;
            while self.full_with(data.len()) && self.remove_oldest_audio() {}
            self.push_back(Item {
                data,
                kind: ItemKind::Video { keyframe: true },
            });
            return PushOutcome::Queued;
        }
        if self.drop_until_keyframe {
            return PushOutcome::Dropped {
                need_keyframe: false,
            };
        }
        if self.full_with(data.len()) {
            self.drop_until_keyframe = true;
            return PushOutcome::Dropped {
                need_keyframe: true,
            };
        }
        self.push_back(Item {
            data,
            kind: ItemKind::Video { keyframe: false },
        });
        PushOutcome::Queued
    }

    pub fn push_audio(&mut self, data: Bytes) -> PushOutcome {
        while self.full_with(data.len()) {
            if !self.remove_oldest_audio() {
                return PushOutcome::Dropped {
                    need_keyframe: false,
                };
            }
        }
        self.push_back(Item {
            data,
            kind: ItemKind::Audio,
        });
        PushOutcome::Queued
    }

    fn remove_oldest_audio(&mut self) -> bool {
        if let Some(pos) = self.items.iter().position(|i| i.kind == ItemKind::Audio)
            && let Some(item) = self.items.remove(pos)
        {
            self.bytes -= item.data.len();
            return true;
        }
        false
    }

    pub fn pop(&mut self) -> Option<Item> {
        let item = self.items.pop_front()?;
        self.bytes -= item.data.len();
        Some(item)
    }

    /// Após reconectar: mídia antiga é descartada e o vídeo recomeça num keyframe.
    pub fn reset_for_reconnect(&mut self) {
        self.items.clear();
        self.bytes = 0;
        self.drop_until_keyframe = true;
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(n: usize) -> Bytes {
        Bytes::from(vec![0u8; n])
    }

    #[test]
    fn drops_deltas_until_keyframe_when_full() {
        let mut q = SendQueue::new(3, 1000);
        assert_eq!(q.push_video(b(10), true), PushOutcome::Queued);
        assert_eq!(q.push_video(b(10), false), PushOutcome::Queued);
        assert_eq!(q.push_video(b(10), false), PushOutcome::Queued);
        assert_eq!(
            q.push_video(b(10), false),
            PushOutcome::Dropped {
                need_keyframe: true
            }
        );
        q.pop();
        // Mesmo com espaço, deltas continuam descartados até o keyframe.
        assert_eq!(
            q.push_video(b(10), false),
            PushOutcome::Dropped {
                need_keyframe: false
            }
        );
        assert_eq!(q.push_video(b(10), true), PushOutcome::Queued);
        assert_eq!(q.len(), 1, "keyframe remove o vídeo obsoleto da fila");
        assert_eq!(q.push_video(b(10), false), PushOutcome::Queued);
    }

    #[test]
    fn byte_limit_is_enforced() {
        let mut q = SendQueue::new(100, 25);
        assert_eq!(q.push_video(b(20), true), PushOutcome::Queued);
        assert!(matches!(
            q.push_video(b(10), false),
            PushOutcome::Dropped { .. }
        ));
        assert!(q.bytes() <= 25);
    }

    #[test]
    fn audio_drops_oldest_and_keyframe_evicts_audio() {
        let mut q = SendQueue::new(3, 1000);
        for n in 1..=3 {
            assert_eq!(q.push_audio(b(n)), PushOutcome::Queued);
        }
        assert_eq!(q.push_audio(b(4)), PushOutcome::Queued);
        let lens: Vec<usize> = std::iter::from_fn(|| q.pop())
            .map(|i| i.data.len())
            .collect();
        assert_eq!(lens, vec![2, 3, 4]);

        for n in 1..=3 {
            q.push_audio(b(n));
        }
        assert_eq!(q.push_video(b(5), true), PushOutcome::Queued);
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn reconnect_waits_for_keyframe() {
        let mut q = SendQueue::new(10, 1000);
        q.push_video(b(1), true);
        q.reset_for_reconnect();
        assert!(q.is_empty());
        assert!(matches!(
            q.push_video(b(1), false),
            PushOutcome::Dropped { .. }
        ));
        assert_eq!(q.push_video(b(1), true), PushOutcome::Queued);
    }

    #[test]
    fn control_messages_are_never_dropped() {
        let mut q = SendQueue::new(2, 10);
        q.push_video(b(10), true);
        q.push_control(b(5));
        q.push_control(b(5));
        assert_eq!(q.len(), 3);
    }
}
