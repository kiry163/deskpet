# 素材目录

## videos/（源素材）

- 51 段 VP9+alpha 透明 webm（640×360，24fps，10s/段，合计约 35MB），来自上游
  [ianlike-ui/dsh-pet-standalone](https://github.com/ianlike-ui/dsh-pet-standalone)（MIT）
- 素材动画、动画链行为模型、交互设计源自
  [PC2005-cloud/dsh-pet](https://github.com/PC2005-cloud/dsh-pet)（MIT）与
  [MerZlin/dsh-pet-indesktop](https://github.com/MerZlin/dsh-pet-indesktop)，特此声明并致谢
- **本目录不入 Git**（体积考虑）；获取方式：
  - Windows：`powershell -ExecutionPolicy Bypass -File tools\fetch-webm-assets.ps1`
  - macOS / Linux：`sh tools/fetch-webm-assets.sh`
  - 或手动从上游仓库复制 `assets/videos/*.webm`
- 构建时由 `build.rs` 打包为 `assets.pak`（`cargo:rerun-if-changed` 跟踪素材目录）

## frames/（帧序列产物，已废弃）

- 旧版 Tauri 前端使用的 WebP 帧序列（约 440MB，由已移除的 `tools/webm2frames` 生成）
- 原生实现直接解码 webm，不再需要帧序列；本目录不入 Git
