//! Fila de capacidade 1 ("o mais recente vence") entre a captura e o encoder.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

struct State<T> {
    item: Option<T>,
    closed: bool,
    wake: bool,
}

/// Guarda no máximo um item. Um `put` substitui o item pendente, que é
/// devolvido ao chamador (e descartado), garantindo memória limitada e que o
/// encoder sempre receba o quadro mais novo.
pub struct LatestSlot<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

impl<T> Default for LatestSlot<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LatestSlot<T> {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                item: None,
                closed: false,
                wake: false,
            }),
            ready: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Coloca um item; devolve o item substituído, se havia um.
    pub fn put(&self, item: T) -> Option<T> {
        let mut s = self.lock();
        if s.closed {
            return Some(item);
        }
        let old = s.item.replace(item);
        drop(s);
        self.ready.notify_one();
        old
    }

    /// Espera até `timeout` por um item. Retorna `None` em timeout, ao ser
    /// acordado por [`LatestSlot::wake`] ou depois de [`LatestSlot::close`].
    pub fn take(&self, timeout: Duration) -> Option<T> {
        let mut s = self.lock();
        if s.item.is_none() && !s.closed && !s.wake {
            s = self
                .ready
                .wait_timeout_while(s, timeout, |s| s.item.is_none() && !s.closed && !s.wake)
                .map(|(g, _)| g)
                .unwrap_or_else(|p| p.into_inner().0);
        }
        s.wake = false;
        s.item.take()
    }

    /// Acorda quem está esperando em [`LatestSlot::take`] (ex.: pedido de parada).
    pub fn wake(&self) {
        self.lock().wake = true;
        self.ready.notify_all();
    }

    /// Fecha o slot: novos itens são recusados e o item pendente é descartado.
    pub fn close(&self) {
        let mut s = self.lock();
        s.closed = true;
        s.item = None;
        drop(s);
        self.ready.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn latest_wins() {
        let slot = LatestSlot::new();
        assert_eq!(slot.put(1), None);
        assert_eq!(slot.put(2), Some(1));
        assert_eq!(slot.take(Duration::from_millis(1)), Some(2));
        assert_eq!(slot.take(Duration::from_millis(1)), None);
    }

    #[test]
    fn take_waits_for_producer() {
        let slot = Arc::new(LatestSlot::new());
        let producer = Arc::clone(&slot);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            producer.put(7);
        });
        assert_eq!(slot.take(Duration::from_secs(2)), Some(7));
        t.join().expect("thread");
    }

    #[test]
    fn wake_and_close_interrupt_waiting() {
        let slot = Arc::new(LatestSlot::<u32>::new());
        let other = Arc::clone(&slot);
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            other.wake();
        });
        let start = std::time::Instant::now();
        assert_eq!(slot.take(Duration::from_secs(5)), None);
        assert!(start.elapsed() < Duration::from_secs(2));
        t.join().expect("thread");

        slot.close();
        assert!(slot.is_closed());
        assert_eq!(slot.put(1), Some(1));
        assert_eq!(slot.take(Duration::from_secs(5)), None);
    }
}
