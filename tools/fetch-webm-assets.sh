#!/usr/bin/env bash
# 从上游 dsh-pet-standalone 获取 51 个 webm 素材到仓库 assets/videos（macOS / Linux 用）。
# 用法（仓库根）：
#   bash tools/fetch-webm-assets.sh
set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"
DEST="$REPO/assets/videos"
mkdir -p "$DEST"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "克隆上游仓库（浅克隆）..."
git clone --depth 1 https://github.com/ianlike-ui/dsh-pet-standalone "$TMP"

cp "$TMP"/assets/videos/*.webm "$DEST"/
COUNT=$(find "$DEST" -maxdepth 1 -name '*.webm' | wc -l | tr -d ' ')
echo "完成：assets/videos 现有 $COUNT 个 webm。"
