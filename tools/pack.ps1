# 打包 Windows 发布 zip：deskpet.exe + 素材 + LICENSE + 使用说明。
# 用法（仓库根）：
#   powershell -ExecutionPolicy Bypass -File tools\pack.ps1
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# 版本（从 Cargo.toml）
$m = Select-String -Path (Join-Path $repo 'Cargo.toml') -Pattern '^version = "(.+)"'
if (-not $m) { throw '无法从 Cargo.toml 读取版本' }
$version = $m.Matches[0].Groups[1].Value

# 1. 构建 release
Write-Host '构建 release...'
$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path $cargo)) { throw "找不到 cargo: $cargo" }
# VPX_LIB_DIR 缺省探测（vcpkg 常见位置）
if (-not $env:VPX_LIB_DIR) {
    foreach ($c in @(
        "$env:USERPROFILE\AppData\Local\Temp\vcpkg\installed\x64-windows\lib",
        "$env:USERPROFILE\vcpkg\installed\x64-windows\lib",
        'C:\vcpkg\installed\x64-windows\lib'
    )) {
        if (Test-Path (Join-Path $c 'vpx.lib')) { $env:VPX_LIB_DIR = $c; break }
    }
}
Push-Location $repo
& $cargo build --release
if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'cargo build 失败' }
Pop-Location

# 2. 组装临时目录
$name = "deskpet-v$version-windows-x64"
$stage = Join-Path $repo "target\pack\$name"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item (Join-Path $repo 'target\release\deskpet.exe') $stage
Copy-Item (Join-Path $repo 'assets') (Join-Path $stage 'assets') -Recurse
Copy-Item (Join-Path $repo 'LICENSE') $stage

$readme = @"
deskpet 桌宠 v$version（Windows x64）

运行：双击 deskpet.exe，或命令行运行。
退出：托盘图标（左键单击切换显示/隐藏，右键菜单 → 退出）。
日志：%APPDATA%\deskpet\logs\deskpet.log（超 1MB 自动滚动为 .old）
配置：%APPDATA%\deskpet\config.json（assets_dir / character 可指定素材位置与角色）
素材：assets/ 目录与软件分离，可整体替换或自定义（结构见 assets/README.md）

本软件以 MIT 许可发布（见 LICENSE）。
素材来自 ianlike-ui/dsh-pet-standalone（MIT）；动画与交互设计源自
PC2005-cloud/dsh-pet（MIT）与 MerZlin/dsh-pet-indesktop，特此致谢。
"@
Set-Content -Path (Join-Path $stage 'README.txt') -Value $readme -Encoding UTF8

# 3. 压缩
$zip = Join-Path $repo "target\$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $stage -DestinationPath $zip
Remove-Item $stage -Recurse -Force
Write-Host "打包完成: $zip"
