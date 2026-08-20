//! 本地 HTTP 控制服务（控制层，见 docs/需求规格.md §5）。
//!
//! 单一通道设计：桌宠进程内嵌 tiny_http，绑定 127.0.0.1，静态托管管理前端 +
//! `/api/*` JSON API。Agent / 脚本 / 浏览器都通过本服务与桌宠交互（不提供
//! CLI / MCP / Hooks 独立通道）。
//!
//! 线程模型：HTTP 线程不直接触碰 App（主线程独占，含窗口句柄），每个请求封装为
//! `ApiRequest` 经 mpsc 命令通道转交主线程（`App::drain_api`，平台 tick 内排空），
//! 响应经每请求独立的 reply channel 返回给 HTTP 线程。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server};

// 控制台前端资源表（build.rs 从 web/dist 生成；未构建时为空表 → 占位页兜底）。
include!(concat!(env!("OUT_DIR"), "/console_assets.rs"));

/// 主线程可执行的 API 操作。
#[derive(Debug)]
pub enum ApiOp {
    /// GET /api/state
    State,
    /// GET /api/config
    GetConfig,
    /// PATCH /api/config（部分字段合并，热生效）
    PatchConfig(Value),
    /// POST /api/pet/play
    Play(String),
    /// POST /api/pet/move（归一化 0..1）
    MoveTo { x: f64, y: f64 },
    /// POST /api/pet/set_state（运行时状态，不落盘）
    SetState(Value),
    /// POST /api/pet/say（气泡 + 时长）
    Say { text: String, duration_ms: Option<u64> },
    /// POST /api/import：素材 zip 已校验并解压落位，通知主线程切换角色并热加载
    ApplyImport { id: String },
    /// POST /api/quit
    Quit,
}

/// 一条 API 请求：操作 + 回复通道。
pub struct ApiRequest {
    pub op: ApiOp,
    pub reply: mpsc::Sender<Value>,
}

/// 控制服务句柄（持有服务器，退出时解除阻塞）。
pub struct ControlServer {
    /// 实际监听端口（默认 18686，冲突时随机；写入 control.json）。
    #[allow(dead_code)]
    pub port: u16,
    /// 控制台 URL（托盘「打开控制台」用）。
    pub url: String,
    server: Arc<Server>,
}

/// 默认端口；被占用时回退随机端口（实际端口写 control.json）。
const DEFAULT_PORT: u16 = 18686;

/// 管理前端未构建时的占位页（web/dist 编译期内嵌后由静态资源替代，见 docs §6.2）。
/// 内置最小可用功能：素材 zip 导入 + API 自检，便于前端工程落地前联调。
const PLACEHOLDER_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>deskpet 控制台</title>
<style>
body{font-family:system-ui,-apple-system,sans-serif;max-width:720px;margin:48px auto;padding:0 24px;color:#333;line-height:1.7}
code{background:#f4f4f4;padding:2px 6px;border-radius:4px;font-size:.92em}
h1{font-size:1.6em}
.card{border:1px solid #e2e2e2;border-radius:10px;padding:16px 20px;margin:16px 0}
.card h2{margin:0 0 10px;font-size:1.1em}
#drop{border:2px dashed #ccc;border-radius:10px;padding:24px;text-align:center;color:#888;cursor:pointer}
#drop.drag{border-color:#4a90d9;background:#f0f7ff}
#result{margin-top:12px;white-space:pre-wrap;font-size:.9em;word-break:break-all}
.ok{color:#1a7f37}.err{color:#c62828}
pre{background:#f8f8f8;border-radius:6px;padding:10px;overflow:auto;font-size:.85em}
</style>
</head>
<body>
<h1>🐱 deskpet 控制台</h1>
<p>管理前端尚未构建（<code>web/dist</code> 缺失，当前为内嵌占位页）。
构建前端：<code>cd web &amp;&amp; npm install &amp;&amp; npm run build</code> 后页面即内嵌进二进制。
占位页已内置 <b>素材导入</b> 与 <b>API 自检</b>，可先联调。</p>

<div class="card">
<h2>导入素材包（zip，根目录 = manifest.json + videos/）</h2>
<div id="drop">点击或拖拽 zip 文件到此处</div>
<input type="file" id="file" accept=".zip,application/zip" style="display:none">
<div id="result"></div>
</div>

<div class="card">
<h2>API 自检</h2>
<pre>GET  /api/state        当前状态（动作/位置/朝向/缩放/可见性）
GET  /api/config       当前配置
PATCH /api/config      修改配置（热生效，写同一 config.json）
POST /api/pet/play     {"action":"待机呼吸休闲"} 播放动作（精确/语义/模糊）
POST /api/pet/move     {"x":0.5,"y":0.5} 移动到归一化位置
POST /api/pet/set_state {"scale":0.5} 运行时状态（不落盘）
POST /api/pet/say      {"text":"你好","duration_ms":3000} 说话气泡
POST /api/import       导入素材 zip（本页上方）
POST /api/quit         退出桌宠</pre>
</div>

<script>
const drop = document.getElementById('drop');
const file = document.getElementById('file');
const result = document.getElementById('result');
drop.onclick = () => file.click();
drop.ondragover = e => { e.preventDefault(); drop.classList.add('drag'); };
drop.ondragleave = () => drop.classList.remove('drag');
drop.ondrop = e => { e.preventDefault(); drop.classList.remove('drag'); if (e.dataTransfer.files[0]) upload(e.dataTransfer.files[0]); };
file.onchange = () => { if (file.files[0]) upload(file.files[0]); };
function upload(f) {
  if (!f.name.toLowerCase().endsWith('.zip')) { show('请选择 .zip 文件', false); return; }
  result.textContent = '导入中… ' + f.name;
  fetch('/api/import', { method: 'POST', headers: { 'Content-Type': 'application/zip' }, body: f })
    .then(r => r.json().catch(() => ({ ok: false, error: 'HTTP ' + r.status })))
    .then(j => show(JSON.stringify(j, null, 2), j.ok))
    .catch(e => show('请求失败: ' + e, false));
}
function show(text, ok) { result.textContent = text; result.className = ok ? 'ok' : 'err'; }
</script>
</body>
</html>"#;

impl ControlServer {
    /// 启动 HTTP 服务：优先默认端口，冲突回退随机端口；实际端口写
    /// `<配置目录>/control.json`（外部程序据此发现端点）。
    pub fn start(
        app_tx: mpsc::Sender<ApiRequest>,
        config_dir: PathBuf,
        assets_root: PathBuf,
    ) -> Option<ControlServer> {
        let server = Server::http(("127.0.0.1", DEFAULT_PORT))
            .or_else(|_| Server::http(("127.0.0.1", 0)))
            .ok()?;
        let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
        let url = format!("http://127.0.0.1:{}", port);
        let _ = write_control_json(&config_dir, port, &url);
        log_info!("控制服务已启动: {} (control.json 已写入)", url);

        let server = Arc::new(server);
        let sv = Arc::clone(&server);
        let _ = thread::spawn(move || {
            for req in sv.incoming_requests() {
                handle_request(req, &app_tx, &assets_root);
            }
            log_info!("控制服务已停止");
        });
        Some(ControlServer { port, url, server })
    }

    /// 解除 recv 阻塞，让服务线程退出（进程退出前调用，避免悬挂日志）。
    pub fn stop(&self) {
        self.server.unblock();
    }
}

// ---------------- 路由 ----------------

fn handle_request(mut req: Request, app_tx: &mpsc::Sender<ApiRequest>, assets_root: &Path) {
    // 仅接受 loopback Host（防 DNS rebinding / 浏览器跨站调用）
    if !host_is_loopback(&req) {
        let _ = req.respond(
            Response::from_string("forbidden".to_string())
                .with_status_code(403),
        );
        return;
    }
    let path = req.url().split('?').next().unwrap_or("").to_string();
    let method = req.method().clone();
    match (method, path.as_str()) {
        (Method::Get, "/api/state") => dispatch(req, app_tx, ApiOp::State),
        (Method::Get, "/api/config") => dispatch(req, app_tx, ApiOp::GetConfig),
        (Method::Patch, "/api/config") => {
            let body = read_body(&mut req);
            let v = serde_json::from_slice(&body).unwrap_or(Value::Null);
            dispatch(req, app_tx, ApiOp::PatchConfig(v));
        }
        (Method::Post, "/api/import") => {
            let body = read_body(&mut req);
            handle_import(req, app_tx, &body, assets_root);
        }
        (Method::Post, "/api/pet/play") => {
            let body = read_body(&mut req);
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let action = v.get("action").and_then(|x| x.as_str()).unwrap_or("").to_string();
            dispatch(req, app_tx, ApiOp::Play(action));
        }
        (Method::Post, "/api/pet/move") => {
            let body = read_body(&mut req);
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let x = v.get("x").and_then(|x| x.as_f64()).unwrap_or(0.5);
            let y = v.get("y").and_then(|y| y.as_f64()).unwrap_or(0.5);
            dispatch(req, app_tx, ApiOp::MoveTo { x, y });
        }
        (Method::Post, "/api/pet/set_state") => {
            let body = read_body(&mut req);
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            dispatch(req, app_tx, ApiOp::SetState(v));
        }
        (Method::Post, "/api/pet/say") => {
            let body = read_body(&mut req);
            let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let duration_ms = v.get("duration_ms").and_then(|x| x.as_u64());
            dispatch(req, app_tx, ApiOp::Say { text, duration_ms });
        }
        (Method::Post, "/api/quit") => dispatch(req, app_tx, ApiOp::Quit),
        // 其余 GET 一律交给管理前端处理（内嵌 / 磁盘覆盖 / 404 兜底），
        // 保证 DESKPET_CONSOLE_DIR 下任意路径可访问
        (Method::Get, path) => serve_console(req, path),
        _ => {
            let _ = req.respond(
                Response::from_string(json!({"ok": false, "error": "not found"}).to_string())
                    .with_status_code(404)
                    .with_header(json_header()),
            );
        }
    }
}

/// POST /api/import：素材 zip 上传（裸 body，Content-Type: application/zip）。
/// HTTP 线程完成校验 + 解压落位，再通知主线程切换角色并热加载。
fn handle_import(req: Request, app_tx: &mpsc::Sender<ApiRequest>, body: &[u8], assets_root: &Path) {
    match crate::import::import_zip(body, assets_root) {
        Err(e) => {
            let _ = req.respond(
                Response::from_string(json!({"ok": false, "error": e}).to_string())
                    .with_status_code(400)
                    .with_header(json_header()),
            );
        }
        Ok(report) => {
            let (tx, rx) = mpsc::channel::<Value>();
            let op = ApiOp::ApplyImport { id: report.id.clone() };
            if app_tx.send(ApiRequest { op, reply: tx }).is_err() {
                let _ = req.respond(
                    Response::from_string(json!({"ok": false, "error": "app not ready"}).to_string())
                        .with_status_code(503)
                        .with_header(json_header()),
                );
                return;
            }
            let apply = rx
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or(json!({"ok": false, "error": "timeout"}));
            let resp = if apply.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                json!({
                    "ok": true,
                    "data": {
                        "id": report.id,
                        "display_name": report.display_name,
                        "videos": report.video_count,
                        "warnings": report.warnings,
                        "character": apply.get("data").and_then(|d| d.get("character")).cloned().unwrap_or(Value::Null),
                    }
                })
            } else {
                json!({"ok": false, "error": apply.get("error").cloned().unwrap_or(json!("应用导入失败"))})
            };
            let _ = req.respond(Response::from_string(resp.to_string()).with_header(json_header()));
        }
    }
}

/// GET 静态资源：管理前端。
///
/// 优先级：`DESKPET_CONSOLE_DIR`（磁盘目录，开发期免重编译调试）> 编译期内嵌
/// （web/dist，见 build.rs `embed_console`）> 占位页兜底（仅 "/"）。
fn serve_console(req: Request, path: &str) {
    // 1) 磁盘覆盖（DESKPET_CONSOLE_DIR 指向 web/dist）
    if let Some(root) = console_dir_override() {
        let rel = if path == "/" {
            "index.html"
        } else {
            path.trim_start_matches('/')
        };
        if let Ok(bytes) = std::fs::read(root.join(rel)) {
            let _ = req.respond(
                Response::from_data(bytes).with_header(content_type_for(path)),
            );
            return;
        }
    }
    // 2) 编译期内嵌资源表
    if let Some(bytes) = embedded_asset(path) {
        let _ = req.respond(
            Response::from_data(bytes.to_vec()).with_header(content_type_for(path)),
        );
        return;
    }
    // 3) 兜底：首页回退占位页；其余资源 404
    if path == "/" {
        let _ = req.respond(
            Response::from_string(PLACEHOLDER_HTML.to_string()).with_header(html_header()),
        );
    } else {
        let _ = req.respond(
            Response::from_string(json!({"ok": false, "error": "not found"}).to_string())
                .with_status_code(404)
                .with_header(json_header()),
        );
    }
}

/// 内嵌资源表查找（URL 路径 → 字节）。
fn embedded_asset(path: &str) -> Option<&'static [u8]> {
    CONSOLE_FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

/// `DESKPET_CONSOLE_DIR` 指向 web/dist 时返回该目录（磁盘覆盖内嵌资源）。
fn console_dir_override() -> Option<PathBuf> {
    let dir = std::env::var("DESKPET_CONSOLE_DIR").ok()?;
    let p = PathBuf::from(dir);
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

/// 按扩展名推断 Content-Type（内嵌 / 磁盘静态资源共用）。
fn content_type_for(path: &str) -> Header {
    let ct: &str = if path == "/" {
        "text/html; charset=utf-8"
    } else {
        match path.rsplit('.').next().unwrap_or("") {
            "html" => "text/html; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "svg" => "image/svg+xml",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "ico" => "image/x-icon",
            "woff" => "font/woff",
            "woff2" => "font/woff2",
            "ttf" => "font/ttf",
            "json" => "application/json; charset=utf-8",
            "map" => "application/json",
            "wasm" => "application/wasm",
            _ => "application/octet-stream",
        }
    };
    Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap()
}

/// 把 API 操作转交主线程并等待响应。
fn dispatch(req: Request, app_tx: &mpsc::Sender<ApiRequest>, op: ApiOp) {
    let (tx, rx) = mpsc::channel::<Value>();
    if app_tx.send(ApiRequest { op, reply: tx }).is_err() {
        let _ = req.respond(
            Response::from_string(json!({"ok": false, "error": "app not ready"}).to_string())
                .with_status_code(503)
                .with_header(json_header()),
        );
        return;
    }
    let resp = match rx.recv_timeout(Duration::from_secs(10)) {
        Ok(v) => v,
        Err(_) => json!({"ok": false, "error": "timeout"}),
    };
    let _ = req.respond(Response::from_string(resp.to_string()).with_header(json_header()));
}

/// 读取请求体（上限 256MB，zip 素材包导入用）。
fn read_body(req: &mut Request) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = req.as_reader().take(256 * 1024 * 1024).read_to_end(&mut buf);
    buf
}

// ---------------- 端口发现 ----------------

fn write_control_json(dir: &PathBuf, port: u16, url: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let data = json!({ "port": port, "url": url });
    std::fs::write(dir.join("control.json"), serde_json::to_string_pretty(&data).unwrap())
}

// ---------------- 工具 ----------------

fn host_is_loopback(req: &Request) -> bool {
    req.headers().iter().any(|h| {
        h.field.equiv("host")
            && {
                let v = h.value.as_str().to_ascii_lowercase();
                v.starts_with("127.0.0.1") || v.starts_with("localhost") || v.starts_with("[::1]")
            }
    })
}

fn json_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap()
}

fn html_header() -> Header {
    Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap()
}
