# Gera e assina o manifesto de atualização (Ed25519) que o app instalado confere antes de atualizar.
# A chave privada existe só no secret UPDATE_SIGNING_KEY; sem ela o script falha, porque o app recusaria a atualização.
#
#   .\scripts\windows\New-UpdateManifest.ps1 -Tag v0.1.3 -Repository nextestudios/Vellcast

param(
    [Parameter(Mandatory)] [string] $Tag,
    [Parameter(Mandatory)] [string] $Repository
)

$ErrorActionPreference = "Stop"
Set-Location (Resolve-Path "$PSScriptRoot\..\..")

if (-not $env:UPDATE_SIGNING_KEY) { throw "UPDATE_SIGNING_KEY ausente: o manifesto de atualização precisa ser assinado." }
$url = "https://github.com/$Repository/releases/download/$Tag/Vellcast-Setup-x64.exe"
target/release/vellcast-release-sign.exe manifest $Tag $url dist/Vellcast-Setup-x64.exe "Vellcast $Tag" |
    Set-Content -Encoding utf8NoBOM -NoNewline dist/vellcast-update.json
if ($LASTEXITCODE -ne 0) { throw "falha ao gerar o manifesto" }
target/release/vellcast-release-sign.exe sign dist/vellcast-update.json
if ($LASTEXITCODE -ne 0) { throw "falha ao assinar o manifesto" }
Get-Content dist/vellcast-update.json
