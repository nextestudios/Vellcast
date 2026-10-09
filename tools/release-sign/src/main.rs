//! Assinatura dos manifestos de atualização (S-012, ADR-046).
//!
//! ```text
//! vellcast-release-sign keygen
//!     imprime a chave privada (base64, guarde como segredo UPDATE_SIGNING_KEY)
//!     e a chave pública (base64, embutida em apps/desktop/src/updater.rs)
//! vellcast-release-sign manifest <versão> <url-https-do-instalador> <arquivo-do-instalador> [notas]
//!     imprime o manifesto JSON com o SHA-256 do instalador
//! UPDATE_SIGNING_KEY=<base64> vellcast-release-sign sign <manifesto.json>
//!     grava <manifesto.json>.sig (assinatura Ed25519 em base64 dos bytes exatos)
//! vellcast-release-sign verify <manifesto.json> <chave-pública-base64>
//! ```

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};

fn main() {
    if let Err(e) = run() {
        eprintln!("erro: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let mut seed = [0u8; 32];
            getrandom::fill(&mut seed).map_err(|e| e.to_string())?;
            let key = SigningKey::from_bytes(&seed);
            println!("privada (UPDATE_SIGNING_KEY): {}", STANDARD.encode(seed));
            println!("pública: {}", STANDARD.encode(key.verifying_key().to_bytes()));
            Ok(())
        }
        Some("manifest") if args.len() >= 4 => {
            let data = std::fs::read(&args[3]).map_err(|e| e.to_string())?;
            let sha: String = Sha256::digest(&data)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if !args[2].starts_with("https://") {
                return Err("a URL do instalador precisa ser HTTPS".into());
            }
            let manifest = serde_json::json!({
                "version": args[1].trim_start_matches('v'),
                "url": args[2],
                "sha256": sha,
                "notes": args.get(4).cloned().unwrap_or_default(),
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?
            );
            Ok(())
        }
        Some("sign") if args.len() >= 2 => {
            let secret = std::env::var("UPDATE_SIGNING_KEY")
                .map_err(|_| "defina UPDATE_SIGNING_KEY".to_string())?;
            let seed: [u8; 32] = STANDARD
                .decode(secret.trim())
                .map_err(|_| "UPDATE_SIGNING_KEY não é base64".to_string())?
                .try_into()
                .map_err(|_| "UPDATE_SIGNING_KEY precisa ter 32 bytes".to_string())?;
            let key = SigningKey::from_bytes(&seed);
            let body = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
            let sig = STANDARD.encode(key.sign(&body).to_bytes());
            std::fs::write(format!("{}.sig", args[1]), sig).map_err(|e| e.to_string())?;
            println!("assinado: {}.sig", args[1]);
            Ok(())
        }
        Some("verify") if args.len() >= 3 => {
            let body = std::fs::read(&args[1]).map_err(|e| e.to_string())?;
            let sig = std::fs::read_to_string(format!("{}.sig", args[1]))
                .map_err(|e| e.to_string())?;
            let public: [u8; 32] = STANDARD
                .decode(args[2].trim())
                .map_err(|_| "chave pública inválida".to_string())?
                .try_into()
                .map_err(|_| "chave pública precisa ter 32 bytes".to_string())?;
            let sig: [u8; 64] = STANDARD
                .decode(sig.trim())
                .map_err(|_| "assinatura inválida".to_string())?
                .try_into()
                .map_err(|_| "assinatura precisa ter 64 bytes".to_string())?;
            VerifyingKey::from_bytes(&public)
                .map_err(|_| "chave pública inválida".to_string())?
                .verify(&body, &Signature::from_bytes(&sig))
                .map_err(|_| "assinatura NÃO confere".to_string())?;
            println!("assinatura válida");
            Ok(())
        }
        _ => Err(
            "uso: keygen | manifest <versão> <url> <arquivo> [notas] | sign <manifesto> | verify <manifesto> <chave-pública>"
                .into(),
        ),
    }
}
