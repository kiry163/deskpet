#!/usr/bin/env bash
# 批量转换 dsh-pet-standalone 的 51 段 VP9+alpha webm → WebP 帧序列（12fps）。
# 用法: ./convert_all.sh [源视频目录] [输出目录]
# 素材来源: https://github.com/ianlike-ui/dsh-pet-standalone (MIT, 源自 PC2005-cloud/dsh-pet)
set -euo pipefail

SRC="${1:-$(dirname "$0")/../../assets/videos}"
OUT="${2:-$(dirname "$0")/../../assets/frames}"
BIN="$(dirname "$0")/target/release/webm2frames"

if [ ! -x "$BIN" ]; then
  echo "未找到 $BIN，先执行: cargo build --release" >&2
  exit 1
fi

mkdir -p "$OUT"
total=0
count=0
for f in "$SRC"/*.webm; do
  name=$(basename "$f" .webm)
  dir="$OUT/$name"
  mkdir -p "$dir"
  # 已转换过的跳过（支持断点续跑）
  if ls "$dir"/*.webp >/dev/null 2>&1; then
    echo "[skip] $name"
    continue
  fi
  out=$("$BIN" "$f" "$dir" --fps 12 --format webp)
  size=$(du -sk "$dir" | awk '{print $1}')
  frames=$(ls "$dir" | wc -l | tr -d ' ')
  echo "[ok] $name: $frames 帧, ${size}KB"
  count=$((count + 1))
  total=$((total + size))
done
echo "=== 完成: 转换 $count 个动画, 总大小 ${total}KB ($(echo "scale=1; $total/1024" | bc)MB) ==="
