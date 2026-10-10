# Code signing policy / Política de assinatura de código

**English.** Vellcast releases are **not code-signed yet**. Windows SmartScreen may warn when you run the installer or
the portable executable. Every release is built by the public GitHub Actions workflow
(`.github/workflows/release.yml`) from a tag on `main`, and ships `SHA256SUMS.txt` with the SHA-256 of both downloads
plus a `vellcast-update.json` manifest signed (Ed25519) with the project's update key, which lives only in a GitHub
Actions secret. The installed app only runs an update whose manifest signature and SHA-256 match. We will never ask you
to disable Windows protections or to install a root certificate. The release workflow is ready to sign (Authenticode,
with a timestamp) the executables and the installer as soon as the maintainer configures a signing certificate
(`WINDOWS_CERT_PFX_BASE64` and `WINDOWS_CERT_PASSWORD`) in the `release` GitHub Actions environment, which can require
manual approval. Only tag builds are signed, from CI, and the certificate is never stored in this repository.

- **Committers and reviewers:** @nextestudios (every change goes through a pull request and CI)
- **Approvers:** @nextestudios (approves every release)
- Signed binaries will carry `Vellcast` as product name and the release version as product version.
- Privacy: [PRIVACY.md](PRIVACY.md).

**Português.** As releases do Vellcast **ainda não têm assinatura de código**. O SmartScreen do Windows pode avisar ao
abrir o instalador ou o executável portátil. Toda release é gerada pelo workflow público do GitHub Actions
(`.github/workflows/release.yml`) a partir de uma tag na `main` e inclui o `SHA256SUMS.txt` com o SHA-256 dos dois
downloads e um manifesto `vellcast-update.json` assinado (Ed25519) com a chave de atualização do projeto, que existe
apenas num secret do GitHub Actions. O app instalado só executa uma atualização cuja assinatura e SHA-256 confiram.
Nunca pediremos para desativar proteções do Windows nem instalar certificados raiz. O workflow de release já está pronto
para assinar (Authenticode, com carimbo de tempo) os executáveis e o instalador assim que o mantenedor configurar um
certificado (`WINDOWS_CERT_PFX_BASE64` e `WINDOWS_CERT_PASSWORD`) no ambiente `release` do GitHub Actions, que pode
exigir aprovação manual. Só builds de tag são assinados, pela CI, e o certificado nunca fica neste repositório.
