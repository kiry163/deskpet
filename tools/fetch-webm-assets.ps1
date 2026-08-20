# 从上游 dsh-pet-standalone 获取 51 个 webm 素材到仓库 assets/videos。
# 用法（仓库根）：
#   powershell -ExecutionPolicy Bypass -File tools\fetch-webm-assets.ps1
$ErrorActionPreference = 'Stop'

$repo = Split-Path -Parent $PSScriptRoot   # tools/ 的父目录 = 仓库根
$dest = Join-Path $repo 'assets\videos'
New-Item -ItemType Directory -Force -Path $dest | Out-Null

$tmp = Join-Path $env:TEMP 'dsh-pet-standalone-assets'
if (Test-Path $tmp) { Remove-Item $tmp -Recurse -Force }

Write-Host '克隆上游仓库（浅克隆）...'
git clone --depth 1 https://github.com/ianlike-ui/dsh-pet-standalone $tmp
if ($LASTEXITCODE -ne 0) { throw 'git clone 失败' }

Copy-Item (Join-Path $tmp 'assets\videos\*.webm') $dest -Force
Remove-Item $tmp -Recurse -Force

$count = (Get-ChildItem $dest -Filter *.webm).Count
Write-Host "完成：assets\videos 现有 $count 个 webm。"
