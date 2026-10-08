//! Escolha do processo excluído na captura do áudio do sistema.
//!
//! O loopback por processo do Windows aceita **um** processo alvo (com a
//! árvore de filhos). No modo [`crate::AudioTarget::System`] excluímos o
//! Discord, para não retransmitir a chamada de voz; sem Discord aberto,
//! excluímos o próprio Vellcast (que não toca áudio).

/// Executáveis dos clientes Discord para desktop.
pub const DISCORD_EXECUTABLES: [&str; 4] = [
    "discord.exe",
    "discordptb.exe",
    "discordcanary.exe",
    "discorddevelopment.exe",
];

/// Um processo em execução.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent_pid: u32,
    pub exe_name: String,
}

pub fn is_discord(exe_name: &str) -> bool {
    let name = exe_name.to_ascii_lowercase();
    DISCORD_EXECUTABLES.contains(&name.as_str())
}

/// PIDs das raízes das árvores de processos do Discord (processos Discord
/// cujo pai não é um processo Discord), em ordem crescente de PID.
pub fn discord_roots(processes: &[ProcessEntry]) -> Vec<u32> {
    let discord: Vec<&ProcessEntry> = processes
        .iter()
        .filter(|p| is_discord(&p.exe_name))
        .collect();
    let mut roots: Vec<u32> = discord
        .iter()
        .filter(|p| {
            !discord
                .iter()
                .any(|q| q.pid == p.parent_pid && q.pid != p.pid)
        })
        .map(|p| p.pid)
        .collect();
    roots.sort_unstable();
    roots.dedup();
    roots
}

/// Processo a excluir no modo sistema, com aviso quando a exclusão é parcial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exclusion {
    pub pid: u32,
    pub excludes_discord: bool,
    pub warning: Option<String>,
}

pub fn choose_exclusion(processes: &[ProcessEntry], own_pid: u32) -> Exclusion {
    let roots = discord_roots(processes);
    match roots.as_slice() {
        [] => Exclusion {
            pid: own_pid,
            excludes_discord: false,
            warning: None,
        },
        [only] => Exclusion {
            pid: *only,
            excludes_discord: true,
            warning: None,
        },
        [first, ..] => Exclusion {
            pid: *first,
            excludes_discord: true,
            warning: Some(
                "Mais de um cliente Discord está aberto; apenas um foi excluído da captura. \
                 O áudio dos outros pode ser transmitido."
                    .into(),
            ),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent_pid: u32, exe: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            exe_name: exe.into(),
        }
    }

    #[test]
    fn finds_root_of_discord_tree() {
        let procs = [
            p(4, 0, "System"),
            p(100, 50, "explorer.exe"),
            p(200, 100, "Discord.exe"),
            p(201, 200, "Discord.exe"),
            p(202, 200, "Discord.exe"),
            p(300, 100, "game.exe"),
        ];
        assert_eq!(discord_roots(&procs), vec![200]);
        let e = choose_exclusion(&procs, 999);
        assert_eq!(
            e,
            Exclusion {
                pid: 200,
                excludes_discord: true,
                warning: None
            }
        );
    }

    #[test]
    fn excludes_self_without_discord() {
        let procs = [p(300, 100, "game.exe")];
        let e = choose_exclusion(&procs, 999);
        assert_eq!(e.pid, 999);
        assert!(!e.excludes_discord);
    }

    #[test]
    fn warns_with_multiple_clients() {
        let procs = [
            p(500, 1, "DiscordCanary.exe"),
            p(200, 1, "Discord.exe"),
            p(201, 200, "discord.exe"),
        ];
        assert_eq!(discord_roots(&procs), vec![200, 500]);
        let e = choose_exclusion(&procs, 1);
        assert_eq!(e.pid, 200);
        assert!(e.warning.is_some());
    }

    #[test]
    fn orphan_child_counts_as_root() {
        // Filho cujo pai Discord já terminou: vira raiz.
        let procs = [p(201, 200, "Discord.exe")];
        assert_eq!(discord_roots(&procs), vec![201]);
    }

    #[test]
    fn name_matching_is_case_insensitive() {
        assert!(is_discord("DISCORDPTB.EXE"));
        assert!(!is_discord("discord-overlay.exe"));
    }
}
