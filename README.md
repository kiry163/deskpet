# deskpet 桌宠（原生实现）

单 exe、零运行时依赖的桌面宠物：直接用 libvpx 解码 51 段 VP9+alpha webm
并逐像素合成渲染。

- Windows：Win32 原生（`CreateWindowExW` + `UpdateLayeredWindow` 透明窗口、托盘、注册表自启）——**已实现并验证**
- macOS：AppKit 原生（`NSWindow` 透明 + `NSView` 逐帧 `drawRect` 渲染、`NSStatusItem` 托盘、
  LaunchAgent 自启）——**已实现并真机验证（2025）**

> **声明与致谢**：本项目 fork 自
> [ianlike-ui/dsh-pet-standalone](https://github.com/ianlike-ui/dsh-pet-standalone)（MIT）。
> 素材动画、动画链行为模型、交互设计均来自
> [dsh-pet](https://github.com/PC2005-cloud/dsh-pet) 与
> [dsh-pet-indesktop](https://github.com/MerZlin/dsh-pet-indesktop)。
> 特此声明并致谢。裁剪/改造点见下文「与上游差异」。本项目以 MIT 许可发布，见 [LICENSE](LICENSE)。

## 功能集

- 单只桌宠，51 段动画：待机 / 转向 / 移动 / 点击回应 / 拖拽反馈 / 随机动作
- 透明逐像素窗口（Windows：`UpdateLayeredWindow`；macOS：`CGBitmapContext`→`CGImage` 逐帧绘制）
- 鼠标穿透（Windows：`WM_NCHITTEST` 按像素 alpha；macOS：`ignoresMouseEvents` 按光标位置逐 tick 切换）
- 点击回应：待机时点击随机播放 3 种回应动画
- 拖拽：Windows `SetCapture` / macOS AppKit 按下期间隐式捕获 + 全局坐标跟手
- 托盘右键菜单：回到右下角 / 窗口置顶 / 不移动 / 开机自启 / 大小（50% 72% 85% 100%）/ 显示隐藏 / 退出
  （桌宠身上不再弹右键菜单，全部集中到托盘）
- **浏览器控制台**（托盘「打开控制台」→ `http://127.0.0.1:<port>/`，前端编译期内嵌）：
  状态面板 + 指令面板（say 气泡 / play / move / 退出）、zip 素材导入、外观配置（热生效）
- **zip 素材导入**：控制台导入素材包（校验 → 解压到素材根 → 热加载，不重启）；
  发布物仅二进制，素材不随包分发（docs/需求规格.md §3）
- 鼠标悬停桌宠 → 手型光标，提示可交互（点击回应 / 拖拽）——**仅 Windows**（macOS 因逐 tick
  穿透切换，窗口服务器不支持光标样式，悬停手型已放弃）
- 开机自启：Windows `HKCU\...\Run`；macOS `~/Library/LaunchAgents/com.kiry.deskpet.plist`
- 配置：Windows `%APPDATA%\deskpet\config.json`；macOS `~/Library/Application Support/deskpet/config.json`
  （首次运行自动从旧版 Tauri 应用配置迁移）

## 构建

### Windows

前置：Rust MSVC 工具链 + libvpx 静态库。

1. **libvpx**（推荐 vcpkg）：`vcpkg install libvpx:x64-windows`，构建时设
   `set VPX_LIB_DIR=C:\path\to\vcpkg\installed\x64-windows\lib`。
   或按上游方式编译出 `vendor_libvpx/x64/Release/vpxmd.lib`。
2. **素材**（开发期）：从上游 `ianlike-ui/dsh-pet-standalone` 的 `assets/videos/` 复制 51 个 webm 到
   本仓库 `assets/videos/`。素材与软件分离：运行时从素材根目录加载
   （解析顺序：配置 `assets_dir` > 环境变量 `DESKPET_ASSETS_DIR` > 配置目录 `assets/` >
   exe 旁 `assets/` > 当前目录 `assets/`）。
   **运行时**：发布物不含素材，首次运行后经控制台导入素材 zip 包（见「控制台」）。
3. **前端**（可选，缺失时控制台用内嵌占位页）：`cd web && npm install && npm run build`。
4. `cargo build --release` → `target\release\deskpet.exe`（约 1.5MB，不含素材）。

### macOS

前置：Rust 工具链（`rustup target add aarch64-apple-darwin`）+ libvpx。

1. **libvpx**：`brew install libvpx`（或 vcpkg；build.rs 自动探测
   `VPX_LIB_DIR` > `VCPKG_ROOT/installed/*-osx/lib` > `/opt/homebrew/lib` > `/usr/local/lib`）。
2. **素材**（开发期）：从上游 `ianlike-ui/dsh-pet-standalone` 的 `assets/videos/` 复制到
   `assets/videos/`，或设 `DESKPET_ASSETS_DIR` 指向素材根目录。
   **运行时**：发布物不含素材，首次运行后经控制台导入素材 zip 包（见「控制台」）。
3. **前端**（可选，缺失时控制台用内嵌占位页）：`cd web && npm install && npm run build`，
   产物编译期内嵌进二进制；开发期免重编译可用 `DESKPET_CONSOLE_DIR=web/dist`。
4. 在 Mac 上：`cargo build --release`，运行 `target/release/deskpet`。

> 跨平台类型检查（无需 Mac）：`DESKPET_ALLOW_NO_LIBVPX=1 cargo check --target aarch64-apple-darwin`

> crates.io 不可达时（本机实测），cargo 命令需带 rsproxy 镜像参数：
> `--config 'source.crates-io.replace-with="rsproxy-sparse"' --config 'source.rsproxy-sparse.registry="sparse+https://rsproxy.cn/index/"'`

### macOS 真机验证（2025 完成）

- 穿透：`ignoresMouseEvents` 按光标位置逐 tick 切换（窗口服务器层真穿透）——真机验证通过
- `CGContextDrawImage` 正立渲染、Retina 坐标换算、状态栏菜单——真机验证通过
- 修复：`CGBitmapContextCreate` 字节序常量非法 → 渲染全透明 + 托盘图标缺失；macOS ld
  优先 dylib → build.rs 改传 `libvpx.a` 绝对路径强制静态链接
- 未完整验证：LaunchAgent 自启；macOS 悬停手型光标不支持（穿透机制限制，已放弃）

## 运行

```bat
target\release\deskpet.exe
```

退出：托盘/状态栏图标 → 退出。

## 日志

写入 `<配置目录>/logs/deskpet.log`（Windows `%APPDATA%\deskpet\logs\`；
macOS `~/Library/Application Support/deskpet/logs/`）。单文件超过 1MB 自动滚动为
`deskpet.log.old`（磁盘占用 ≤2MB）。级别由环境变量 `DESKPET_LOG` 控制
（`off|error|warn|info|debug`，默认 `info`）。

## 配置

`config.json`（路径见上）：

```json
{
  "rx": 0.83,
  "ry": 0.62,
  "facing_right": true,
  "scale": 0.5,
  "always_on_top": true,
  "no_move": false,
  "assets_dir": null,
  "character": null
}
```

`rx/ry` 为工作区内归一化位置（0..1，相对于主屏工作区），`null` 表示右下角默认位。
`autostart` 不持久化在配置中，以注册表/LaunchAgent 为准（避免与系统启动项管理漂移）。
`assets_dir`（素材根目录，`null` = 自动解析：环境变量 `DESKPET_ASSETS_DIR` > 配置目录
`assets/` > exe 旁 `assets/` > 当前目录 `assets/`）与 `character`（角色子目录名，
`null` = 自动检测 `assets/` 下第一个含 `videos/` 或 `manifest.json` 的子目录；
兼容 `assets/` 本身即角色目录）。配置也可在控制台「配置」页修改（热生效并落盘）。

## 控制台

托盘/状态栏菜单「打开控制台」→ 系统默认浏览器打开 `http://127.0.0.1:<port>/`
（实际端口写 `<配置目录>/control.json`，默认 18686）。前端编译期内嵌在二进制里，
`web/dist` 缺失时回退内嵌占位页（仍可导入素材）。

- **状态**：当前动作/位置/朝向/缩放/可见性 + 指令面板（say 气泡 / play 动作 / move / 退出）
- **导入**：拖拽上传素材 zip 包（zip 根 = `manifest.json` + `videos/`），校验 → 解压到
  素材根 → 热加载（不重启），导入角色自动成为当前角色
- **配置**：缩放 / 置顶 / 朝向 / 可见性 / 不移动，改动即时生效并写入 config.json

对外 JSON API（Agent / 脚本直接调用）：`GET /api/state`、`GET|PATCH /api/config`、
`POST /api/pet/{say,play,move,set_state}`、`POST /api/import`、`POST /api/quit`；
响应统一 `{ok, data?, error?}`，详见 docs/需求规格.md §5。

## 与上游差异

- 裁剪：多角色（外部形象目录）、多桌宠（生成/删除）、右键"播放任意动画"子菜单
- 配置：改为单宠物结构 + 从旧版 Tauri 应用迁移；位置用归一化 `rx/ry`
- 自启注册表值名：`deskpet`（上游为 `dsh-pet-standalone`）
- 构建：libvpx 链接改为 build.rs 自动探测（`vpx.lib`/`vpxmd.lib`，vcpkg 优先）
- 素材：与软件分离（exe 不内嵌素材，~1.5MB），运行时从目录加载
  （`assets_dir`/`character` 配置 + manifest.json 分类 + 子目录/关键词兜底）；
  支持自定义素材，见 docs/需求规格.md
- 架构：平台中立核心（`pet.rs`/`state.rs`/`clip.rs`/`webm.rs`）+ 平台后端
  （`win32.rs` / `macos.rs`）；菜单为数据驱动（`menu.rs`）
- **macOS 后端为全新编写**（上游无原生 macOS 实现；参考行为：动画链/交互与 Win32 版一致）
- 功能保留：点击回应、拖拽、置顶、不移动、自启、4 档大小、回到右下角、托盘；
  新增：托盘右键完整菜单、悬停手型光标（仅 Windows）

## 验证工具

```bat
cargo run --release --bin decode_check
```

解析全部 51 段 webm、解码首帧、检查 alpha 分布与解码性能。

