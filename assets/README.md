# 素材目录

## videos/（源素材）
- 51 段 VP9+alpha 透明 webm，640×360，24fps，10s/段，共 35MB
- 来源：https://github.com/ianlike-ui/dsh-pet-standalone
- 素材动画、动画链行为模型、交互设计源自 https://github.com/PC2005-cloud/dsh-pet（MIT）
  与 https://github.com/MerZlin/dsh-pet-indesktop，特此声明并致谢。
- 本目录不入 Git（体积考虑）；如需复现转换，先从上游仓库获取。

## frames/（转帧产物）
- 51 段动画 → WebP 帧序列（12fps，无损），每动画一个子目录，帧文件 `f00000.webp` 起
- 由 `tools/webm2frames/` 转换生成，命令：
  ```bash
  cd tools/webm2frames && cargo build --release
  ./convert_all.sh ../../assets/videos ../../assets/frames
  ```
- `meta.json`：动画元数据清单（帧数/时长/体积/总计）
- 本目录不入 Git（约 440MB）；作为素材包外置分发或首次运行下载。
