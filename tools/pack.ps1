# 打包 Windows 发布：仅产出单个 deskpet.exe（零依赖、双击即运行，不再打 zip）。
# 素材不随包分发：发布物仅二进制，素材由用户经控制台导入（docs/需求规格.md §3）。
# 用法（仓库根）：
#   powershell -ExecutionPolicy Bypass -File tools\pack.ps1
# 环境变量：
#   CARGO_EXTRA  追加到 cargo build 的参数（如 crates.io 不可达时指定镜像）
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot

# 版本（从 Cargo.toml）
$m = Select-String -Path (Join-Path $repo 'Cargo.toml') -Pattern '^version = "(.+)"'
if (-not $m) { throw '无法从 Cargo.toml 读取版本' }
$version = $m.Matches[0].Groups[1].Value

Push-Location $repo

# 1. 前端（web/dist 缺失时重建，产物内嵌进二进制）
if (-not (Test-Path (Join-Path $repo 'web\dist\index.html'))) {
    Write-Host '构建前端（web/dist 缺失）...'
    Push-Location (Join-Path $repo 'web')
    & npm run build
    if ($LASTEXITCODE -ne 0) { Pop-Location; Pop-Location; throw 'npm run build 失败' }
    Pop-Location
} else {
    Write-Host '前端已是最新（web/dist）'
}

# 2. 构建 release
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
# crates.io 不可达时可用 rsproxy 镜像，例如：
#   $env:CARGO_EXTRA = '--config source.crates-io.replace-with="rsproxy-sparse" --config source.rsproxy-sparse.registry="sparse+https://rsproxy.cn/index/"'
$extra = if ($env:CARGO_EXTRA) { $env:CARGO_EXTRA.Split(' ') } else { @() }
& $cargo build --release @extra
if ($LASTEXITCODE -ne 0) { Pop-Location; throw 'cargo build 失败' }
Pop-Location

# 3. 产出单 exe（拷贝 release 二进制，重命名带版本；不再打 zip）
$exe = Join-Path $repo "target\deskpet-v$version-windows-x64.exe"
Copy-Item (Join-Path $repo 'target\release\deskpet.exe') $exe -Force
Write-Host "打包完成: $exe"
