# 打包 Windows 发布 zip：deskpet.exe（仅二进制）+ LICENSE + 使用说明。
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

# 1. 前端（web/dist 缺失或源码更新时重建，产物内嵌进二进制）
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

# 3. 组装临时目录（仅二进制 + LICENSE + 说明；不含素材）
$name = "deskpet-v$version-windows-x64"
$stage = Join-Path $repo "target\pack\$name"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item (Join-Path $repo 'target\release\deskpet.exe') $stage
Copy-Item (Join-Path $repo 'LICENSE') $stage

$readme = @"
deskpet 桌宠 v$version（Windows x64）

发布物仅二进制：素材不随包分发，首次运行后请经控制台导入素材包。

快速开始：
1. 双击 deskpet.exe 启动；
2. 托盘图标 → 菜单「打开控制台」，浏览器打开管理界面
   （地址也可读 %APPDATA%\deskpet\control.json 中的 url）；
3. 控制台「导入」页上传素材 zip 包（zip 根 = manifest.json + videos/），
   校验通过后自动解压到素材根并热加载（无需重启）。

退出：托盘图标右键菜单 → 退出。
日志：%APPDATA%\deskpet\logs\deskpet.log（超 1MB 自动滚动为 .old）
配置：%APPDATA%\deskpet\config.json（assets_dir / character 可覆盖）
自启：菜单「开机自启」写入 HKCU\...\Run

素材规范：zip 根目录即角色包 —— manifest.json + videos/（VP9+alpha webm），
详见项目 docs/需求规格.md §3。

本软件以 MIT 许可发布（见 LICENSE）。
动画与交互设计源自 ianlike-ui/dsh-pet-standalone（MIT）、PC2005-cloud/dsh-pet（MIT）
与 MerZlin/dsh-pet-indesktop，特此致谢。
"@
Set-Content -Path (Join-Path $stage 'README.txt') -Value $readme -Encoding UTF8

# 4. 压缩
$zip = Join-Path $repo "target\$name.zip"
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path $stage -DestinationPath $zip
Remove-Item $stage -Recurse -Force
Write-Host "打包完成: $zip"
