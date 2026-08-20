# deskpet 控制台（web/）

浏览器管理前端（M1：状态 / 导入 / 配置三页）。技术栈：Vite + React + TypeScript。

## 构建（产物内嵌进二进制）

```bash
npm install   # 走 npmmirror 镜像（.npmrc）
npm run build # 产出 web/dist
```

`build.rs` 会在 `cargo build` 时把 `web/dist` 全部文件以 `include_bytes!` 资源表
内嵌进二进制（缺失时控制台回退到内嵌占位页）。因此**改完前端要重新 `cargo build`**
才能生效。

## 开发

- `npm run dev`：Vite dev server（默认 5173 端口）；
- 免重编译调试：启动桌宠时设 `DESKPET_CONSOLE_DIR=web/dist`（绝对路径），
  桌宠 HTTP 服务会直接从该目录读静态资源（优先于内嵌），改完前端 `npm run build`
  刷新浏览器即可，无需重编 Rust。
- 控制台地址：托盘菜单「打开控制台」，或读 `<配置目录>/control.json` 里的 `url`。

## 页面

| 页 | 内容 |
|---|---|
| 状态 | 当前状态面板（动作/位置/朝向/缩放/可见性/置顶/不移动）+ 指令面板（say/play/move/退出）+ 指令日志 |
| 导入 | zip 素材包拖拽导入（校验/解压/热加载），显示角色 id/显示名/视频数/警告 |
| 配置 | 外观表单（缩放/置顶/朝向/可见性/不移动，即时热生效）+ 当前角色与素材根 + 原始 config.json |

## 备注

- API 响应统一 `{ok, data?, error?}`，见 `src/api.ts`；
- 页面全部通过桌宠内嵌 HTTP 服务（127.0.0.1）访问，无外部依赖；
- 行为策略表单（动作概率/移动参数/模式预设）属 M2。
