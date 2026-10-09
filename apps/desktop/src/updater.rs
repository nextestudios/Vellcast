//! Atualizações seguras (S-012).
//!
//! Fluxo, sempre iniciado pelo usuário (nenhum serviço em segundo plano):
//! 1. baixa o manifesto JSON e a assinatura Ed25519 destacada (`.sig`, base64);
//! 2. verifica a assinatura com a chave pública embutida no app;
//! 3. compara a versão com a instalada;
//! 4. se o usuário confirmar, baixa o instalador para a pasta temporária,
//!    confere o SHA-256 do manifesto e só então abre o instalador.
//!
//! O manifesto é publicado pelo workflow de release (`.github/workflows/release.yml`)
//! e assinado com `tools/release-sign`. URL padrão: o último release do GitHub;
//! `VELLCAST_UPDATE_URL` permite apontar para outro servidor.

use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Chave pública Ed25519 (32 bytes, base64) que assina os manifestos de atualização.
/// A chave privada correspondente fica só no segredo `UPDATE_SIGNING_KEY` do repositório.
pub const UPDATE_PUBLIC_KEY: &str = "Bh+Na4Efvg7Yf3UOgKOv2SSGkJiQ32bWJgl5EWn6TJU=";

/// Manifesto padrão: ativo do último release do GitHub.
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/nextestudios/Vellcast/releases/latest/download/vellcast-update.json";

/// Limites de tamanho para não baixar conteúdo inesperado sem fim.
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub version: String,
    /// URL HTTPS do instalador.
    pub url: String,
    /// SHA-256 do instalador em hexadecimal.
    pub sha256: String,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UpdateError {
    #[error("não foi possível consultar atualizações: {0}")]
    Network(String),
    #[error("manifesto de atualização inválido: {0}")]
    BadManifest(String),
    #[error("assinatura do manifesto inválida")]
    BadSignature,
    #[error("o arquivo baixado não confere com o manifesto")]
    HashMismatch,
    #[error("chave de verificação de atualizações não configurada nesta versão")]
    NoKey,
    #[error("{0}")]
    Io(String),
}

/// Resultado de uma verificação.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    UpToDate { current: String },
    Available(Manifest),
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn manifest_url() -> String {
    std::env::var("VELLCAST_UPDATE_URL").unwrap_or_else(|_| DEFAULT_MANIFEST_URL.into())
}

/// Versão semântica simples `MAJOR.MINOR.PATCH` (prefixo `v` e sufixos ignorados).
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let major = parts.next()??;
    let minor = parts.next().unwrap_or(Some(0))?;
    let patch = parts.next().unwrap_or(Some(0))?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

fn verifying_key(public_key_b64: &str) -> Result<VerifyingKey, UpdateError> {
    let bytes = STANDARD
        .decode(public_key_b64.trim())
        .map_err(|_| UpdateError::NoKey)?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| UpdateError::NoKey)?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| UpdateError::NoKey)
}

/// Verifica a assinatura destacada (base64) sobre os bytes exatos do manifesto
/// e interpreta o JSON. Nada do manifesto é usado antes da verificação.
pub fn verify_manifest(
    manifest_bytes: &[u8],
    signature_b64: &str,
    public_key_b64: &str,
) -> Result<Manifest, UpdateError> {
    let key = verifying_key(public_key_b64)?;
    let sig = STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| UpdateError::BadSignature)?;
    let sig: [u8; 64] = sig.try_into().map_err(|_| UpdateError::BadSignature)?;
    key.verify(manifest_bytes, &Signature::from_bytes(&sig))
        .map_err(|_| UpdateError::BadSignature)?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|e| UpdateError::BadManifest(e.to_string()))?;
    if parse_version(&manifest.version).is_none() {
        return Err(UpdateError::BadManifest("versão inválida".into()));
    }
    if !manifest.url.starts_with("https://") {
        return Err(UpdateError::BadManifest(
            "o instalador precisa ser HTTPS".into(),
        ));
    }
    if manifest.sha256.len() != 64 || !manifest.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(UpdateError::BadManifest("SHA-256 inválido".into()));
    }
    Ok(manifest)
}

/// Decide, a partir de um manifesto já verificado, se há versão nova.
pub fn evaluate(manifest: Manifest, current: &str) -> CheckOutcome {
    if is_newer(&manifest.version, current) {
        CheckOutcome::Available(manifest)
    } else {
        CheckOutcome::UpToDate {
            current: current.to_string(),
        }
    }
}

fn get_bytes(url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let mut resp = ureq::get(url)
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    let mut out = Vec::new();
    resp.body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut out)
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    if out.len() as u64 > limit {
        return Err(UpdateError::Network("arquivo maior que o permitido".into()));
    }
    Ok(out)
}

/// Consulta o manifesto (bloqueante; chamar fora da thread da interface).
pub fn check() -> Result<CheckOutcome, UpdateError> {
    let url = manifest_url();
    let manifest = get_bytes(&url, MAX_MANIFEST_BYTES)?;
    let signature = get_bytes(&format!("{url}.sig"), 1024)?;
    let signature = String::from_utf8(signature).map_err(|_| UpdateError::BadSignature)?;
    let manifest = verify_manifest(&manifest, &signature, UPDATE_PUBLIC_KEY)?;
    Ok(evaluate(manifest, current_version()))
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Confere o hash e grava o instalador em `dir`. Retorna o caminho do arquivo.
pub fn store_verified(
    manifest: &Manifest,
    data: &[u8],
    dir: &Path,
) -> Result<PathBuf, UpdateError> {
    if !sha256_hex(data).eq_ignore_ascii_case(&manifest.sha256) {
        return Err(UpdateError::HashMismatch);
    }
    let name = format!(
        "Vellcast-Setup-{}.exe",
        manifest
            .version
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '.')
            .collect::<String>()
    );
    let path = dir.join(name);
    std::fs::write(&path, data).map_err(|e| UpdateError::Io(e.to_string()))?;
    Ok(path)
}

/// Baixa e verifica o instalador (bloqueante). Não executa nada.
pub fn download(manifest: &Manifest) -> Result<PathBuf, UpdateError> {
    let data = get_bytes(&manifest.url, MAX_INSTALLER_BYTES)?;
    store_verified(manifest, &data, &std::env::temp_dir())
}

/// Abre o instalador verificado. Chamado só depois da confirmação do usuário.
pub fn launch(installer: &Path) -> Result<(), UpdateError> {
    std::process::Command::new(installer)
        .spawn()
        .map(|_| ())
        .map_err(|e| UpdateError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn keypair() -> (SigningKey, String) {
        let signing = SigningKey::from_bytes(&[7u8; 32]);
        let public = STANDARD.encode(signing.verifying_key().to_bytes());
        (signing, public)
    }

    fn manifest_json(version: &str, sha: &str) -> Vec<u8> {
        format!(
            r#"{{"version":"{version}","url":"https://example.com/Vellcast-Setup-x64.exe","sha256":"{sha}","notes":"teste"}}"#
        )
        .into_bytes()
    }

    #[test]
    fn valid_signature_is_accepted() {
        let (key, public) = keypair();
        let body = manifest_json("1.2.3", &"a".repeat(64));
        let sig = STANDARD.encode(key.sign(&body).to_bytes());
        let m = verify_manifest(&body, &sig, &public).expect("válido");
        assert_eq!(m.version, "1.2.3");
    }

    #[test]
    fn tampered_manifest_is_rejected() {
        let (key, public) = keypair();
        let body = manifest_json("1.2.3", &"a".repeat(64));
        let sig = STANDARD.encode(key.sign(&body).to_bytes());
        let tampered = manifest_json("9.9.9", &"a".repeat(64));
        assert_eq!(
            verify_manifest(&tampered, &sig, &public),
            Err(UpdateError::BadSignature)
        );
    }

    #[test]
    fn signature_from_other_key_is_rejected() {
        let (_, public) = keypair();
        let other = SigningKey::from_bytes(&[9u8; 32]);
        let body = manifest_json("1.2.3", &"a".repeat(64));
        let sig = STANDARD.encode(other.sign(&body).to_bytes());
        assert_eq!(
            verify_manifest(&body, &sig, &public),
            Err(UpdateError::BadSignature)
        );
        assert_eq!(
            verify_manifest(&body, "não-é-base64", &public),
            Err(UpdateError::BadSignature)
        );
    }

    #[test]
    fn placeholder_key_disables_updates() {
        let (key, _) = keypair();
        let body = manifest_json("1.2.3", &"a".repeat(64));
        let sig = STANDARD.encode(key.sign(&body).to_bytes());
        assert_eq!(
            verify_manifest(&body, &sig, "UPDATE_PUBLIC_KEY_PLACEHOLDER"),
            Err(UpdateError::NoKey)
        );
    }

    #[test]
    fn rejects_insecure_or_malformed_manifests() {
        let (key, public) = keypair();
        let http = br#"{"version":"1.0.0","url":"http://x/y.exe","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#;
        let sig = STANDARD.encode(key.sign(http).to_bytes());
        assert!(matches!(
            verify_manifest(http, &sig, &public),
            Err(UpdateError::BadManifest(_))
        ));
        let bad_hash = manifest_json("1.0.0", "xyz");
        let sig = STANDARD.encode(key.sign(&bad_hash).to_bytes());
        assert!(matches!(
            verify_manifest(&bad_hash, &sig, &public),
            Err(UpdateError::BadManifest(_))
        ));
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("0.1.0", "0.0.1"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("1.10.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
        assert!(!is_newer("lixo", "1.0.0"));
        assert_eq!(parse_version("2.3.4-beta.1"), Some((2, 3, 4)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
    }

    #[test]
    fn evaluates_availability() {
        let m = Manifest {
            version: "9.0.0".into(),
            url: "https://x/y.exe".into(),
            sha256: "a".repeat(64),
            notes: String::new(),
        };
        assert!(matches!(
            evaluate(m.clone(), "0.0.1"),
            CheckOutcome::Available(_)
        ));
        assert!(matches!(
            evaluate(m, "9.0.0"),
            CheckOutcome::UpToDate { .. }
        ));
    }

    #[test]
    fn hash_mismatch_is_rejected_and_match_is_stored() {
        let dir = std::env::temp_dir().join(format!("vellcast-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let data = b"instalador";
        let mut m = Manifest {
            version: "1.0.0".into(),
            url: "https://x/y.exe".into(),
            sha256: "0".repeat(64),
            notes: String::new(),
        };
        assert_eq!(
            store_verified(&m, data, &dir),
            Err(UpdateError::HashMismatch)
        );
        m.sha256 = sha256_hex(data);
        let path = store_verified(&m, data, &dir).expect("gravar");
        assert_eq!(std::fs::read(&path).expect("ler"), data);
        let _ = std::fs::remove_dir_all(dir);
    }
}
