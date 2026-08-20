#!/usr/bin/env bash
# 打包 macOS 发布 zip：deskpet（仅二进制）+ LICENSE + 使用说明。
# 素材不随包分发：发布物仅二进制，素材由用户经控制台导入（docs/需求规格.md §3）。
# 用法（仓库根）：
#   bash tools/pack.sh            # 构建前端（如需）+ release 并打包
#   bash tools/pack.sh --no-build # 跳过前端与 Rust 构建，仅打包已存在的 target/release/deskpet
# 环境变量：
#   CARGO_EXTRA  追加到 cargo build 的参数（如 crates.io 不可达时指定镜像，见下）
set -euo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo"

# 版本（从 Cargo.toml）
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [ -z "$version" ]; then
  echo "无法从 Cargo.toml 读取版本" >&2
  exit 1
fi

# 架构（Intel → x86_64，Apple Silicon → aarch64）
case "$(uname -m)" in
  x86_64) arch="x86_64" ;;
  arm64)  arch="aarch64" ;;
  *)      arch="$(uname -m)" ;;
esac

# 1. 前端（web/dist 缺失或源码更新时重建，产物内嵌进二进制）
if [ "${1:-}" != "--no-build" ]; then
  need_frontend=0
  if [ ! -f web/dist/index.html ]; then
    need_frontend=1
  elif find web/src web/index.html web/package.json -newer web/dist/index.html -print -quit 2>/dev/null | grep -q .; then
    need_frontend=1
  fi
  if [ "$need_frontend" = "1" ]; then
    echo "构建前端（web/dist 缺失或已过期）..."
    (cd web && npm run build)
  else
    echo "前端已是最新（web/dist）"
  fi
fi

# 2. 构建 release（默认；--no-build 跳过）。
#    crates.io 不可达时可用 rsproxy 镜像，例如：
#    CARGO_EXTRA='--config source.crates-io.replace-with="rsproxy-sparse" --config source.rsproxy-sparse.registry="sparse+https://rsproxy.cn/index/"' bash tools/pack.sh
if [ "${1:-}" != "--no-build" ]; then
  echo "构建 release..."
  # shellcheck disable=SC2086
  cargo build --release $CARGO_EXTRA
fi

bin="target/release/deskpet"
if [ ! -x "$bin" ]; then
  echo "缺少二进制: $bin（请先 cargo build --release）" >&2
  exit 1
fi

# 3. 组装临时目录（仅二进制 + LICENSE + 说明；不含素材）
name="deskpet-v${version}-macos-${arch}"
stage="target/pack/$name"
rm -rf "$stage"
mkdir -p "$stage"
cp "$bin" "$stage/deskpet"
cp LICENSE "$stage/"

cat > "$stage/README.txt" <<EOF
deskpet 桌宠 v${version}（macOS ${arch}）

发布物仅二进制：素材不随包分发，首次运行后请经控制台导入素材包。

快速开始：
1. 双击 deskpet 启动（首次如被 Gatekeeper 拦截：右键 deskpet → 打开；
   或终端执行 xattr -dr com.apple.quarantine deskpet）；
2. 状态栏图标 → 菜单「打开控制台」，浏览器打开管理界面
   （地址也可读 ~/Library/Application Support/deskpet/control.json 中的 url）；
3. 控制台「导入」页上传素材 zip 包（zip 根 = manifest.json + videos/），
   校验通过后自动解压到素材根并热加载（无需重启）。

退出：状态栏图标右键菜单 → 退出。
日志：~/Library/Application Support/deskpet/logs/deskpet.log（超 1MB 自动滚动为 .old）
配置：~/Library/Application Support/deskpet/config.json（assets_dir / character 可覆盖）
自启：菜单「开机自启」写入 ~/Library/LaunchAgents/com.kiry.deskpet.plist

素材规范：zip 根目录即角色包 —— manifest.json + videos/（VP9+alpha webm），
详见项目 docs/需求规格.md §3。

本软件以 MIT 许可发布（见 LICENSE）。
动画与交互设计源自 ianlike-ui/dsh-pet-standalone（MIT）、PC2005-cloud/dsh-pet（MIT）
与 MerZlin/dsh-pet-indesktop，特此致谢。
EOF

# 4. 压缩
zip_path="target/${name}.zip"
rm -f "$zip_path"
(cd target/pack && zip -rq "../${name}.zip" "$name")
rm -rf "$stage"
echo "打包完成: $zip_path"
