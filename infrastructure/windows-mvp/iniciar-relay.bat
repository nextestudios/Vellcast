@echo off
rem Inicia o vellcast-relay deste pacote (servidor de salas e mídia do Vellcast).
rem Na primeira execução gera um segredo aleatório e o guarda em relay.env.
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"

if not exist "relay.env" (
  echo Gerando segredo do servidor em relay.env ...
  powershell -NoProfile -ExecutionPolicy Bypass -Command "$b = New-Object byte[] 32; [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b); 'VELLCAST_RELAY_SECRET=' + (-join ($b | ForEach-Object { $_.ToString('x2') })) | Set-Content -Encoding ascii relay.env"
  if errorlevel 1 (
    echo Nao foi possivel gerar o segredo. Verifique se o PowerShell esta disponivel.
    pause
    exit /b 1
  )
)

for /f "usebackq tokens=1,* delims==" %%a in ("relay.env") do set "%%a=%%b"

if not defined VELLCAST_BIND set "VELLCAST_BIND=0.0.0.0:8788"
set "VELLCAST_STATIC_DIR=%~dp0player"

echo.
echo ============================================================
echo  Vellcast relay
echo ============================================================
echo  Neste PC:          http://127.0.0.1:8788
echo  Na rede local:
powershell -NoProfile -Command "Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } | ForEach-Object { '                    http://' + $_.IPAddress + ':8788' }"
echo.
echo  Para assistir em OUTROS aparelhos o navegador exige HTTPS.
echo  Veja LEIA-ME.txt (tunel HTTPS ou VPS com Dokploy).
echo  Se o Firewall do Windows perguntar, permita o acesso.
echo  Feche esta janela (ou Ctrl+C) para desligar o servidor.
echo ============================================================
echo.

"%~dp0vellcast-relay.exe"
echo.
echo O servidor foi encerrado.
pause
