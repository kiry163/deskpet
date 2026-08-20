#!/usr/bin/env bash
# 打包 macOS 发布 zip：deskpet + 素材 + LICENSE + 使用说明。
# 用法（仓库根）：
#   bash tools/pack.sh            # 构建 release 并打包
#   bash tools/pack.sh --no-build # 跳过构建，仅打包已存在的 target/release/deskpet
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

# 1. 构建 release（默认；--no-build 跳过）
if [ "${1:-}" != "--no-build" ]; then
  echo "构建 release..."
  cargo build --release
fi

bin="target/release/deskpet"
if [ ! -x "$bin" ]; then
  echo "缺少二进制: $bin（请先 cargo build --release）" >&2
  exit 1
fi

# 2. 组装临时目录
name="deskpet-v${version}-macos-${arch}"
stage="target/pack/$name"
rm -rf "$stage"
mkdir -p "$stage"
cp "$bin" "$stage/deskpet"
cp -R assets "$stage/assets"
cp LICENSE "$stage/"

cat > "$stage/README.txt" <<EOF
deskpet 桌宠 v${version}（macOS ${arch}）

运行：双击 deskpet，或在终端执行 ./deskpet。
退出：状态栏图标（左键单击切换显示/隐藏，右键菜单 → 退出）。
日志：~/Library/Application Support/deskpet/logs/deskpet.log（超 1MB 自动滚动为 .old）
配置：~/Library/Application Support/deskpet/config.json（assets_dir / character 可指定素材位置与角色）
自启：菜单"开机自启"写入 ~/Library/LaunchAgents/com.kiry.deskpet.plist
素材：assets/ 目录与软件分离，可整体替换或自定义（约定见 docs/需求规格.md §1）

首次运行如被 Gatekeeper 拦截：右键 deskpet → 打开；
或终端执行 xattr -dr com.apple.quarantine deskpet

本软件以 MIT 许可发布（见 LICENSE）。
素材来自 ianlike-ui/dsh-pet-standalone（MIT）；动画与交互设计源自
PC2005-cloud/dsh-pet（MIT）与 MerZlin/dsh-pet-indesktop，特此致谢。
EOF

# 3. 压缩
zip_path="target/${name}.zip"
rm -f "$zip_path"
(cd target/pack && zip -rq "../${name}.zip" "$name")
rm -rf "$stage"
echo "打包完成: $zip_path"
