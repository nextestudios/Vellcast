# Abre o executável portátil numa pasta vazia e confere que ele continua aberto e guarda os dados em Vellcast_Data.
#
#   .\scripts\windows\Test-Portable.ps1 -Exe dist\Vellcast-Portable-x64.exe

param([Parameter(Mandatory)] [string] $Exe)

$ErrorActionPreference = "Stop"

$dir = Join-Path $env:RUNNER_TEMP "VellcastPortable"
New-Item -ItemType Directory -Force $dir | Out-Null
Copy-Item $Exe $dir
$app = Start-Process (Join-Path $dir (Split-Path $Exe -Leaf)) -WorkingDirectory $dir -PassThru
Start-Sleep -Seconds 8
$app.Refresh()
if ($app.HasExited) { throw "o portátil encerrou ao abrir (código $($app.ExitCode))" }
Stop-Process -Id $app.Id -Force
Start-Sleep -Seconds 1
$log = Join-Path $dir "Vellcast_Data\data\logs\vellcast-desktop.log"
if (-not (Test-Path $log)) { throw "o portátil não gravou o log em Vellcast_Data" }
Write-Output "portátil: abriu e guardou os dados em Vellcast_Data"
