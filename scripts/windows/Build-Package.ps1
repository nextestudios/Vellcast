# Gera o pacote do Vellcast para Windows x64 e o instalador (Inno Setup).
# Usado pelos workflows Release (tags) e Smoke (manual).
#
#   .\scripts\windows\Build-Package.ps1 -Version 0.1.3 [-Sign]
#
# Saída em dist/: Vellcast-MVP-windows-x64/ (pasta do pacote), Vellcast-Setup-x64.exe e SHA256SUMS.txt.
# -Sign assina os executáveis e o instalador com WINDOWS_CERT_PFX_BASE64/WINDOWS_CERT_PASSWORD. O script tira
# esses valores do ambiente antes do cargo, do npm e do ISCC; só o signtool os recebe.

param(
    [Parameter(Mandatory)] [string] $Version,
    [switch] $Sign
)

$ErrorActionPreference = "Stop"
Set-Location (Resolve-Path "$PSScriptRoot\..\..")

$pfxBase64 = $env:WINDOWS_CERT_PFX_BASE64
$pfxPassword = $env:WINDOWS_CERT_PASSWORD
Remove-Item Env:WINDOWS_CERT_PFX_BASE64, Env:WINDOWS_CERT_PASSWORD -ErrorAction SilentlyContinue
if ($Sign -and -not $pfxBase64) { throw "-Sign sem WINDOWS_CERT_PFX_BASE64" }

function Invoke-Checked([scriptblock] $Block, [string] $What) {
    & $Block
    if ($LASTEXITCODE -ne 0) { throw "$What falhou (código $LASTEXITCODE)" }
}

function Set-Signature([string[]] $Files) {
    $pfx = Join-Path $env:RUNNER_TEMP "cert.pfx"
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($pfxBase64))
    try {
        $signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin" -Recurse -Filter signtool.exe |
            Where-Object FullName -like "*x64*" | Select-Object -Last 1
        foreach ($f in $Files) {
            & $signtool.FullName sign /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com /f $pfx /p $pfxPassword $f
            if ($LASTEXITCODE -ne 0) { throw "falha ao assinar $f" }
        }
    } finally { Remove-Item $pfx -ErrorAction SilentlyContinue }
}

Write-Output "== Player web"
Push-Location apps/discord-activity
Invoke-Checked { npm ci } "npm ci"
Invoke-Checked { npm run build } "build do player"
Pop-Location

Write-Output "== Build Release (desktop, relay, release-sign)"
Invoke-Checked { cargo build --release --locked -p vellcast-desktop -p vellcast-relay -p vellcast-release-sign } "cargo build"

Write-Output "== Avisos de licenças de terceiros (Rust e npm)"
$aboutVer = "0.9.1"
$aboutDir = Join-Path $env:RUNNER_TEMP "cargo-about"
New-Item -ItemType Directory -Force $aboutDir | Out-Null
Invoke-WebRequest -UseBasicParsing "https://github.com/EmbarkStudios/cargo-about/releases/download/$aboutVer/cargo-about-$aboutVer-x86_64-pc-windows-msvc.tar.gz" -OutFile "$aboutDir\cargo-about.tgz"
tar -xzf "$aboutDir\cargo-about.tgz" -C $aboutDir
$about = Get-ChildItem $aboutDir -Recurse -Filter cargo-about.exe | Select-Object -First 1
Invoke-Checked { & $about.FullName generate --workspace infrastructure/licenses/about.hbs -o THIRD-PARTY-RUST.txt } "cargo-about"
Push-Location apps/discord-activity
Invoke-Checked { npx --yes license-checker-rseidelsohn@5.0.1 --production --plainVertical --out ../../THIRD-PARTY-NPM.txt } "license-checker"
Pop-Location

Write-Output "== Pacote"
$out = "dist/Vellcast-MVP-windows-x64"
New-Item -ItemType Directory -Force -Path "$out/player" | Out-Null
Copy-Item target/release/vellcast-desktop.exe, target/release/vellcast-relay.exe -Destination $out
Copy-Item -Recurse -Path apps/discord-activity/dist/* -Destination "$out/player"
Copy-Item -Path infrastructure/windows-mvp/* -Destination $out
Copy-Item -Path LICENSE -Destination "$out/LICENSE.txt"
Get-Content infrastructure/windows-mvp/THIRD-PARTY-NOTICES.txt, THIRD-PARTY-RUST.txt, THIRD-PARTY-NPM.txt |
    Set-Content -Encoding utf8 "$out/THIRD-PARTY-NOTICES.txt"
"Vellcast $Version`r`ncommit: $env:GITHUB_SHA`r`nbuild: $(Get-Date -Format o)" | Set-Content -Encoding utf8 "$out/VERSAO.txt"

if ($Sign) { Set-Signature @("$out/vellcast-desktop.exe", "$out/vellcast-relay.exe") }

Write-Output "== Relay do pacote responde e serve o player"
$env:VELLCAST_RELAY_INSECURE_DEV = "1"
$env:VELLCAST_BIND = "127.0.0.1:8788"
$env:VELLCAST_STATIC_DIR = "$out/player"
$relay = Start-Process -FilePath "$out/vellcast-relay.exe" -PassThru -NoNewWindow
try {
    Start-Sleep -Seconds 2
    $health = Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8788/healthz
    if ($health.Content.Trim() -ne "ok") { throw "healthz inesperado: $($health.Content)" }
    $watch = Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8788/watch
    if ($watch.StatusCode -ne 200 -or $watch.Content -notmatch "<div id=`"root`">") { throw "player não servido" }
} finally {
    Stop-Process -Id $relay.Id -Force
    Remove-Item Env:VELLCAST_RELAY_INSECURE_DEV, Env:VELLCAST_BIND, Env:VELLCAST_STATIC_DIR
}

Write-Output "== Instalador (Inno Setup)"
$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
if (-not (Test-Path $iscc)) { Invoke-Checked { choco install innosetup --version=6.6.1 --no-progress -y } "instalar Inno Setup" }
$src = (Resolve-Path $out).Path
$dist = (Resolve-Path dist).Path
Invoke-Checked { & $iscc "/DAppVersion=$Version" "/DSourceDir=$src" "/DOutputDir=$dist" infrastructure/installer/vellcast.iss } "ISCC"

if ($Sign) { Set-Signature @("dist/Vellcast-Setup-x64.exe") }

Get-FileHash dist/Vellcast-Setup-x64.exe -Algorithm SHA256 |
    ForEach-Object { "$($_.Hash.ToLower())  Vellcast-Setup-x64.exe" } | Set-Content -Encoding ascii dist/SHA256SUMS.txt
Get-Content dist/SHA256SUMS.txt
Get-ChildItem dist -File | Format-Table Name, Length
