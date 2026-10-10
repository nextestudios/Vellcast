# Instala o Vellcast em modo silencioso, confere arquivos, atalho e relay, e desinstala.
# Usado pelos workflows Release (antes de publicar) e Smoke.
#
#   .\scripts\windows\Test-Installed.ps1 -Setup dist\Vellcast-Setup-x64.exe

param([Parameter(Mandatory)] [string] $Setup)

$ErrorActionPreference = "Stop"

$dir = Join-Path $env:RUNNER_TEMP "VellcastTeste"
$p = Start-Process $Setup -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/CURRENTUSER", "/NORESTART", "/DIR=`"$dir`"" -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "instalação falhou: $($p.ExitCode)" }
foreach ($f in "vellcast-desktop.exe", "vellcast-relay.exe", "player\index.html", "LICENSE.txt", "THIRD-PARTY-NOTICES.txt", "unins000.exe") {
    if (-not (Test-Path "$dir\$f")) { throw "arquivo ausente após instalar: $f" }
}
$startMenu = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"
if (-not (Get-ChildItem $startMenu -Recurse -Filter "Vellcast.lnk" -ErrorAction SilentlyContinue)) { throw "atalho do Menu Iniciar não criado" }
if (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name Vellcast -ErrorAction SilentlyContinue) {
    throw "início automático foi ativado sem o usuário pedir"
}

$env:VELLCAST_RELAY_INSECURE_DEV = "1"
$env:VELLCAST_BIND = "127.0.0.1:8789"
$env:VELLCAST_STATIC_DIR = "$dir\player"
$relay = Start-Process "$dir\vellcast-relay.exe" -PassThru -NoNewWindow
try {
    Start-Sleep -Seconds 2
    if ((Invoke-WebRequest -UseBasicParsing http://127.0.0.1:8789/healthz).Content.Trim() -ne "ok") { throw "relay instalado não respondeu" }
} finally { Stop-Process -Id $relay.Id -Force; Start-Sleep -Seconds 1 }

# O app instalado abre e continua aberto (sem fechar sozinho nos primeiros segundos).
$app = Start-Process "$dir\vellcast-desktop.exe" -PassThru
Start-Sleep -Seconds 8
$app.Refresh()
if ($app.HasExited) { throw "o app instalado encerrou ao abrir (código $($app.ExitCode))" }
Stop-Process -Id $app.Id -Force
Start-Sleep -Seconds 2

$u = Start-Process "$dir\unins000.exe" -ArgumentList "/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART" -Wait -PassThru
if ($u.ExitCode -ne 0) { throw "desinstalação falhou: $($u.ExitCode)" }
Start-Sleep -Seconds 3
if (Test-Path "$dir\vellcast-desktop.exe") { throw "arquivos permaneceram após desinstalar" }
if (Get-ChildItem $startMenu -Recurse -Filter "Vellcast.lnk" -ErrorAction SilentlyContinue) { throw "atalho permaneceu" }
Write-Output "instalar, abrir e desinstalar: ok"
