//! Relógio QPC compartilhado por vídeo e áudio.
//!
//! O WGC entrega `SystemRelativeTime` em unidades de 100 ns derivadas do
//! `QueryPerformanceCounter`; o WASAPI informa posições em ticks brutos de QPC.
//! Estas funções convertem os dois para a mesma base.

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// Valor bruto atual do `QueryPerformanceCounter`.
pub fn qpc_ticks() -> i64 {
    let mut value = 0i64;
    // SAFETY: escrita em variável local; a função não falha no Windows XP ou superior.
    let _ = unsafe { QueryPerformanceCounter(&mut value) };
    value
}

/// Frequência do `QueryPerformanceCounter` (ticks por segundo).
pub fn qpc_frequency() -> i64 {
    let mut value = 0i64;
    // SAFETY: escrita em variável local.
    let _ = unsafe { QueryPerformanceFrequency(&mut value) };
    value.max(1)
}

/// Converte ticks brutos de QPC para unidades de 100 ns.
pub fn ticks_to_hns(ticks: i64, frequency: i64) -> i64 {
    ((i128::from(ticks) * 10_000_000) / i128::from(frequency.max(1))) as i64
}
