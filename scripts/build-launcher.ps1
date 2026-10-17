$ErrorActionPreference='Stop'
& "$PSScriptRoot/cargo.ps1" build --bin luminacraft-launcher -j 1
if($LASTEXITCODE -ne 0){exit $LASTEXITCODE}
$workspace=Split-Path $PSScriptRoot -Parent
Copy-Item -LiteralPath (Join-Path $workspace 'target/debug/luminacraft-launcher.exe') -Destination (Join-Path $workspace 'LuminaCraft.exe') -Force
Write-Output '已生成根目录 LuminaCraft.exe，可直接双击启动。'
