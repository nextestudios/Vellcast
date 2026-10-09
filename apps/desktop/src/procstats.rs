//! CPU e memória do próprio processo, mostradas em "Métricas avançadas" (S-007).
//!
//! Amostrado só enquanto a tela Ao vivo está visível (o mesmo ritmo das demais
//! métricas). No Windows usa `GetProcessTimes` e `GetProcessMemoryInfo`; em
//! outros sistemas os valores ficam indisponíveis.

use std::time::{Duration, Instant};

/// Medida do processo; `None` quando o sistema não informa.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ProcessStats {
    /// Uso de CPU do processo em % da máquina inteira (0–100).
    pub cpu_percent: Option<f32>,
    /// Memória privada do processo em MB.
    pub memory_mb: Option<f32>,
}

/// Calcula o uso de CPU entre duas amostras.
///
/// `cpu` é o tempo de CPU (usuário + kernel) gasto pelo processo no intervalo
/// `wall`; o resultado é relativo à máquina inteira (`cores` núcleos).
pub fn cpu_percent(cpu: Duration, wall: Duration, cores: usize) -> Option<f32> {
    if wall.is_zero() || cores == 0 {
        return None;
    }
    let pct = cpu.as_secs_f64() / (wall.as_secs_f64() * cores as f64) * 100.0;
    Some(pct.clamp(0.0, 100.0) as f32)
}

#[derive(Default)]
pub struct ProcessSampler {
    last: Option<(Instant, Duration)>,
}

impl ProcessSampler {
    pub fn sample(&mut self) -> ProcessStats {
        let now = Instant::now();
        let cpu_total = platform::cpu_time();
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let cpu = match (self.last, cpu_total) {
            (Some((at, prev)), Some(total)) => {
                cpu_percent(total.saturating_sub(prev), now.duration_since(at), cores)
            }
            _ => None,
        };
        if let Some(total) = cpu_total {
            self.last = Some((now, total));
        }
        ProcessStats {
            cpu_percent: cpu,
            memory_mb: platform::private_bytes().map(|b| b as f32 / (1024.0 * 1024.0)),
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use std::time::Duration;

    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    fn filetime_100ns(t: FILETIME) -> u64 {
        (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)
    }

    pub fn cpu_time() -> Option<Duration> {
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        // SAFETY: pseudo-handle do próprio processo; ponteiros de saída locais.
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut creation,
                &mut exit,
                &mut kernel,
                &mut user,
            )
        }
        .ok()?;
        let ticks = filetime_100ns(kernel) + filetime_100ns(user);
        Some(Duration::from_nanos(ticks.saturating_mul(100)))
    }

    pub fn private_bytes() -> Option<u64> {
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        // SAFETY: a estrutura estendida começa com PROCESS_MEMORY_COUNTERS e `cb`
        // informa o tamanho real; o pseudo-handle não precisa ser fechado.
        unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                std::ptr::from_mut(&mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
                counters.cb,
            )
        }
        .ok()?;
        Some(counters.PrivateUsage as u64)
    }
}

#[cfg(not(windows))]
mod platform {
    use std::time::Duration;

    pub fn cpu_time() -> Option<Duration> {
        None
    }

    pub fn private_bytes() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent_is_relative_to_all_cores() {
        let one_core_busy = cpu_percent(Duration::from_secs(1), Duration::from_secs(1), 4);
        assert_eq!(one_core_busy, Some(25.0));
        assert_eq!(
            cpu_percent(Duration::from_millis(50), Duration::from_secs(1), 1),
            Some(5.0)
        );
    }

    #[test]
    fn cpu_percent_handles_degenerate_inputs() {
        assert_eq!(cpu_percent(Duration::from_secs(1), Duration::ZERO, 4), None);
        assert_eq!(
            cpu_percent(Duration::from_secs(1), Duration::from_secs(1), 0),
            None
        );
        // Relógios inconsistentes nunca passam de 100%.
        assert_eq!(
            cpu_percent(Duration::from_secs(10), Duration::from_secs(1), 1),
            Some(100.0)
        );
    }

    #[test]
    fn first_sample_has_no_cpu_value() {
        let mut s = ProcessSampler::default();
        assert_eq!(s.sample().cpu_percent, None);
    }
}
