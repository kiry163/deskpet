//! 桌宠应用：单只桌宠（窗口/状态机/交互）+ 托盘 + 菜单分发（平台无关核心）。
//! Windows 消息桥接（WindowCallback）在 #[cfg(windows)] 下实现；macOS 由 macos.rs 直接调用。
#![allow(non_snake_case, dead_code)]

use std::sync::mpsc;

use crate::config::Config;
use crate::control::{ApiOp, ApiRequest};
use crate::pet::{self, Pet};

pub const WM_TRAY: u32 = 0x0400 + 100; // win32 托盘回调消息（与 tray.rs 一致）

pub struct App {
    pub pet: Option<Pet>,
    pub cfg: Config,
    pub quitting: bool,
    /// 无素材时保留的透明窗口（导入后交给 Pet::new）。
    win: Option<crate::platform::PetWindow>,
    /// HTTP 线程 → 主线程的 API 请求队列（平台 tick 内排空）。
    api_rx: Option<mpsc::Receiver<ApiRequest>>,
    /// 主线程 → 转换线程的回报通道（异步作业更新进度/结果经此回传）。
    api_tx: mpsc::Sender<ApiRequest>,
    /// 本地控制服务（HTTP：管理前端 + JSON API）。
    pub console: Option<crate::control::ControlServer>,
}

/// 从数据库构建某宠物的行为池（动作归属 + 状态池权重）。
fn build_pools_for(
    db: &crate::db::Db,
    pet_id: &str,
    role: &crate::assets::RoleAssets,
) -> crate::pet::StatePools {
    let actions = db.list_actions(pet_id).unwrap_or_default();
    let action_states = db.list_action_states(pet_id).unwrap_or_default();
    crate::pet::build_pools(&role.names, &actions, &action_states)
}

impl App {
    pub fn new(api_rx: mpsc::Receiver<ApiRequest>, api_tx: mpsc::Sender<ApiRequest>) -> App {
        let cfg = Config::load();
        let assets_dir = crate::assets::resolve_assets_dir(cfg.sys.assets_dir.as_deref(), &cfg.dir);
        let role = crate::assets::load(&assets_dir, cfg.pet.character.as_deref());
        // 窗口始终创建：无素材时也保留（Windows 托盘/定时器需要 hwnd），导入后交给 Pet。
        let win = crate::platform::PetWindow::create(cfg.pet.always_on_top);
        let mut app = App {
            pet: None,
            cfg,
            quitting: false,
            win: None,
            api_rx: Some(api_rx),
            api_tx,
            console: None,
        };
        match (win, role) {
            (Some(win), Some(role)) => {
                let id = app.cfg.pet.character.clone().unwrap_or_default();
                let pools = build_pools_for(&app.cfg.db, &id, &role);
                let mut pet = Pet::new(win, &role, &app.cfg.pet, pools, &app.cfg.pet.behavior_states);
                pet.restore_position(&app.cfg.pet);
                app.pet = Some(pet);
            }
            (Some(win), None) => {
                // 无素材（或解析失败）：应用照常运行，窗口保留等待控制台导入
                log_warn!("素材加载失败或无素材，等待控制台导入（桌宠未创建）");
                win.start_frame_timer(10);
                app.win = Some(win);
            }
            (None, _) => log_error!("透明窗口创建失败"),
        }
        app
    }

    /// 保存宠物位置与设置。
    pub fn save_position(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.save_position(&mut self.cfg.pet);
            self.cfg.save();
        }
    }

    /// 启动补生成全身照：为「已导入但尚无全身照」的宠物从待机动画取一帧。
    /// 覆盖历史导入（旧二进制未生成全身照）的宠物，保证控制台总能展示全身照。
    pub fn backfill_full_body_images(&mut self) {
        let assets_dir = crate::assets::resolve_assets_dir(
            self.cfg.sys.assets_dir.as_deref(),
            &self.cfg.dir,
        );
        let pets = match self.cfg.db.list_pets() {
            Ok(p) => p,
            Err(e) => {
                log_warn!("读取桌宠列表失败，跳过全身照补生成: {}", e);
                return;
            }
        };
        for p in pets {
            let (idle, fb) = self.cfg.db.pet_baseline(&p.id).unwrap_or((None, None));
            if idle.is_none() || fb.is_some() {
                continue;
            }
            let Some(a) = idle else { continue };
            match crate::import::generate_full_body(&assets_dir, &p.id, &a) {
                Ok(img) => {
                    let _ = self.cfg.db.set_pet_baseline(&p.id, &a, Some(&img));
                    log_info!("已补生成全身照: {} ({})", p.display_name, p.id);
                }
                Err(e) => log_warn!("补生成全身照失败 {}: {}", p.id, e),
            }
        }
    }

    pub fn quit_all(&mut self) {
        self.quitting = true;
        self.save_position();
        if let Some(c) = &self.console {
            c.stop();
        }
        crate::platform::post_quit();
    }

    pub fn toggle_visible(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.toggle_visible();
        }
    }

    #[cfg(windows)]
    pub fn primary_hwnd(&self) -> windows_sys::Win32::Foundation::HWND {
        if let Some(p) = &self.pet {
            return p.win.hwnd;
        }
        self.win.as_ref().map(|w| w.hwnd).unwrap_or(std::ptr::null_mut())
    }

    /// 处理托盘菜单命令。
    pub fn handle_tray_command(&mut self, cmd: usize) {
        log_debug!("执行托盘命令: {}", cmd);
        match cmd {
            crate::tray::TRAY_CONSOLE => self.open_console(),
            crate::tray::TRAY_TOGGLE_VISIBLE => self.toggle_visible(),
            crate::tray::TRAY_AUTOSTART => {
                let on = !crate::autostart::is_enabled();
                crate::autostart::set_enabled(on);
            }
            crate::tray::TRAY_QUIT => self.quit_all(),
            _ => {}
        }
    }

    /// 打开系统默认浏览器访问控制台。
    pub(crate) fn open_console(&self) {
        match &self.console {
            Some(c) => {
                log_info!("打开控制台: {}", c.url);
                crate::platform::open_url(&c.url);
            }
            None => log_warn!("控制服务未启动，无法打开控制台"),
        }
    }

    // ---------------- 平台无关交互入口（win32 消息桥 / macOS 视图回调共用） ----------------

    pub fn on_pet_press(&mut self, cx: i32, cy: i32) {
        if let Some(pet) = &mut self.pet {
            pet.on_press(cx, cy);
        }
    }

    pub fn on_pet_drag(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.on_drag_move();
        }
    }

    pub fn on_pet_release(&mut self) {
        if let Some(pet) = &mut self.pet {
            pet.on_release();
        }
        // 拖拽/点击后保存位置
        self.save_position();
    }

    pub fn on_pet_tick(&mut self) {
        self.drain_api();
        if let Some(pet) = &mut self.pet {
            pet.on_tick();
        }
    }

    // ---------------- 本地 HTTP 控制服务（API） ----------------

    /// 排空 HTTP 线程发来的 API 请求（平台 tick 内调用；无素材时也须 tick）。
    pub fn drain_api(&mut self) {
        // 取出 receiver 再处理，避免 self 同时被不可变（rx）与可变（handle_api）借用
        let rx = match self.api_rx.take() {
            Some(r) => r,
            None => return,
        };
        while let Ok(req) = rx.try_recv() {
            if matches!(req.op, ApiOp::Quit) {
                // 退出顺序：先回复 → 给 HTTP 线程冲刷响应的时间 → 最后触发退出，
                // 避免 terminate/post_quit 同步退出导致客户端收到空回复
                let _ = req.reply.send(serde_json::json!({"ok": true, "data": {"msg": "bye"}}));
                std::thread::sleep(std::time::Duration::from_millis(200));
                self.quit_all();
                continue;
            }
            let resp = self.handle_api(req.op);
            let _ = req.reply.send(resp);
        }
        self.api_rx = Some(rx);
    }

    fn handle_api(&mut self, op: ApiOp) -> serde_json::Value {
        use serde_json::json;
        match op {
            ApiOp::State => self.api_state(),
            ApiOp::GetConfig => self.api_get_config(),
            ApiOp::PatchConfig(v) => self.api_patch_config(v),
            ApiOp::Play(action) => self.api_play(&action),
            ApiOp::MoveTo { x, y } => match &mut self.pet {
                Some(p) => {
                    p.enqueue(crate::pet::PetCommand::MoveTo { x, y });
                    json!({"ok": true, "data": {"move": {"x": x, "y": y}}})
                }
                None => json!({"ok": false, "error": "桌宠未创建（无素材）"}),
            },
            ApiOp::SetState(v) => self.api_set_state(v),
            ApiOp::Say { text, duration_ms } => match &mut self.pet {
                Some(p) => {
                    p.enqueue(crate::pet::PetCommand::Say { text: text.clone(), duration_ms });
                    json!({"ok": true, "data": {"say": text}})
                }
                None => json!({"ok": false, "error": "桌宠未创建（无素材）"}),
            },
            ApiOp::ApplyImport { id, videos, pet_name, idle, actions, action_states } => match self.apply_import(&id, &videos, pet_name.as_deref(), idle.as_deref(), &actions, &action_states) {
                Ok(()) => json!({"ok": true, "data": {"character": id}}),
                Err(e) => json!({"ok": false, "error": e}),
            },
            ApiOp::PetsList => self.api_pets(),
            ApiOp::SwitchPet { id } => match self.switch_pet(&id) {
                Ok(()) => json!({"ok": true, "data": {"current": id}}),
                Err(e) => json!({"ok": false, "error": e}),
            },
            ApiOp::DeletePet { id, delete_files } => match self.delete_pet(&id, delete_files) {
                Ok(()) => json!({"ok": true, "data": {"deleted": id}}),
                Err(e) => json!({"ok": false, "error": e}),
            },
            ApiOp::GetPetActions { id } => match self.pet_actions(&id) {
                Ok(list) => json!({"ok": true, "data": list}),
                Err(e) => json!({"ok": false, "error": e}),
            },
            ApiOp::UpdatePetName { id, name } => match self.update_pet_name(&id, &name) {
                Ok(()) => json!({"ok": true, "data": {"name": name}}),
                Err(e) => json!({"ok": false, "error": e}),
            },
            ApiOp::SubmitConvertJob { pet_id, action, owner, src_path, dest } => match self.submit_convert_job(&pet_id, &action, &owner, &src_path, &dest) {
                Ok(job_id) => json!({"ok": true, "data": {"job_id": job_id, "action": action, "owner": owner, "status": "queued"}}),
                Err(e) => json!({"ok": false, "error": e}),
            },
            ApiOp::ConvertProgress { job_id, progress } => {
                self.convert_progress(job_id, progress);
                json!({"ok": true})
            }
            ApiOp::ConvertDone { job_id, pet_id, action, owner, ok, error } => {
                self.convert_done(job_id, &pet_id, &action, &owner, ok, error);
                json!({"ok": true})
            }
            ApiOp::ConvertJobsList { pet_id } => {
                json!({"ok": true, "data": self.cfg.db.list_convert_jobs(&pet_id).unwrap_or_default()})
            }
            ApiOp::SavePetActions { id, actions, action_states } => match self.save_pet_actions(&id, &actions, &action_states) {
                Ok(()) => json!({"ok": true, "data": {"saved": actions.len()}}),
                Err(e) => json!({"ok": false, "error": e}),
            },
            ApiOp::GetSettings => self.api_get_config(),
            ApiOp::PatchSettings(v) => self.api_patch_config(v),
            ApiOp::GetSystem => self.api_get_system(),
            // Quit 由 drain_api 特判（先回复再退出），此处兜底
            ApiOp::Quit => json!({"ok": true, "data": {"msg": "bye"}}),
        }
    }

    /// 播放动作：解析（精确/语义/模糊）后入队，下个 tick 高优先级执行。
    fn api_play(&mut self, action: &str) -> serde_json::Value {
        use serde_json::json;
        match &mut self.pet {
            Some(p) => match p.resolve_action(action) {
                Some(name) => {
                    p.enqueue(crate::pet::PetCommand::Play(name.clone()));
                    json!({"ok": true, "data": {"played": name}})
                }
                None => json!({"ok": false, "error": format!("未找到动作: {}", action)}),
            },
            None => json!({"ok": false, "error": "桌宠未创建（无素材）"}),
        }
    }

    /// 运行时状态设置（不落盘，区别于 PATCH /api/config）。
    fn api_set_state(&mut self, patch: serde_json::Value) -> serde_json::Value {
        use serde_json::json;
        let o = match patch.as_object() {
            Some(o) => o,
            None => return json!({"ok": false, "error": "body must be object"}),
        };
        let Some(pet) = &mut self.pet else {
            return json!({"ok": false, "error": "桌宠未创建（无素材）"});
        };
        let mut applied: Vec<&str> = Vec::new();
        if let Some(v) = o.get("scale").and_then(|x| x.as_f64()) {
            pet.change_scale(v);
            applied.push("scale");
        }
        if let Some(v) = o.get("always_on_top").and_then(|x| x.as_bool()) {
            pet.set_topmost(v);
            applied.push("always_on_top");
        }
        if let Some(v) = o.get("no_move").and_then(|x| x.as_bool()) {
            pet.set_no_move(v);
            applied.push("no_move");
        }
        if let Some(v) = o.get("facing_right").and_then(|x| x.as_bool()) {
            pet.facing_right = v;
            applied.push("facing_right");
        }
        if let Some(v) = o.get("visible").and_then(|x| x.as_bool()) {
            if v {
                pet.win.show();
                pet.visible = true;
            } else {
                pet.win.hide();
                pet.visible = false;
            }
            applied.push("visible");
        }
        json!({"ok": true, "data": {"applied": applied}})
    }

    /// 应用导入结果：注册桌宠（DB）→ 默认动作配置 → （仅当前无桌宠时）设为当前并加载。
    fn apply_import(
        &mut self,
        id: &str,
        videos: &[String],
        pet_name: Option<&str>,
        idle: Option<&str>,
        manifest_actions: &[crate::db::ActionRow],
        manifest_states: &[(String, String, f64, bool)],
    ) -> Result<(), String> {
        let display_name = pet_name.filter(|s| !s.is_empty()).unwrap_or(id);
        self.cfg.db.insert_pet(id, display_name, "zip").map_err(|e| e)?;
        // 有 manifest 动作归属则用之；否则全部动画默认 state → 空闲池
        if !manifest_actions.is_empty() {
            self.cfg.db.replace_actions(id, manifest_actions, manifest_states).map_err(|e| e)?;
        } else {
            let action_rows: Vec<crate::db::ActionRow> = videos
                .iter()
                .map(|a| crate::db::ActionRow {
                    action: a.clone(),
                    display_name: a.clone(),
                    owner_kind: "state".to_string(),
                    kind: None,
                    enabled: true,
                })
                .collect();
            let action_states: Vec<(String, String, f64, bool)> =
                videos.iter().map(|a| (a.clone(), "idle".to_string(), 1.0, true)).collect();
            self.cfg.db.replace_actions(id, &action_rows, &action_states).map_err(|e| e)?;
        }
        // 体型基准 + 全身照：manifest.idle 优先，否则取空闲池第一个动作；
        // 全身照自动从待机动画取一帧（不上传不改），失败不阻断导入（None 容忍）。
        let idle_action = idle
            .map(|s| s.to_string())
            .or_else(|| crate::import::resolve_idle_from(manifest_actions, manifest_states, videos));
        if let Some(a) = idle_action.as_deref() {
            let assets_dir = crate::assets::resolve_assets_dir(
                self.cfg.sys.assets_dir.as_deref(),
                &self.cfg.dir,
            );
            let img = crate::import::generate_full_body(&assets_dir, id, a).ok();
            let _ = self.cfg.db.set_pet_baseline(id, a, img.as_deref());
        } else {
            let _ = self.cfg.db.set_pet_baseline(id, videos.first().map(String::as_str).unwrap_or(""), None);
        }
        self.cfg.db.log_import(Some(id), id, "ok", "");
        // 不自动切换：仅当当前无桌宠时设为当前（已有桌宠时由管理端手动切换）
        if self.cfg.pet.character.is_none() {
            self.cfg.pet.character = Some(id.to_string());
            self.cfg.save();
            self.reload_assets()?;
        }
        Ok(())
    }

    /// 重新加载素材（导入后 / 角色切换）：pet 存在则热替换，否则用保留窗口创建。
    pub fn reload_assets(&mut self) -> Result<(), String> {
        let assets_dir =
            crate::assets::resolve_assets_dir(self.cfg.sys.assets_dir.as_deref(), &self.cfg.dir);
        let character = self.cfg.pet.character.clone();
        let role = crate::assets::load(&assets_dir, character.as_deref())
            .ok_or_else(|| format!("素材加载失败（{} 下无有效素材集）", assets_dir.display()))?;
        let id = character.clone().unwrap_or_default();
        let states = self.cfg.pet.behavior_states.clone();
        let pools = build_pools_for(&self.cfg.db, &id, &role);
        match &mut self.pet {
            Some(pet) => pet.swap_role(&role, pools, &states),
            None => {
                let win = self.win.take().ok_or_else(|| "无可用窗口".to_string())?;
                let mut pet = Pet::new(win, &role, &self.cfg.pet, pools, &states);
                pet.restore_position(&self.cfg.pet);
                self.pet = Some(pet);
            }
        }
        Ok(())
    }

    // ---------------- 桌宠管理（阶段 2 API） ----------------

    fn api_pets(&self) -> serde_json::Value {
        use serde_json::json;
        let pets = match self.cfg.db.list_pets() {
            Ok(p) => p,
            Err(e) => return json!({"ok": false, "error": e}),
        };
        let current = self.cfg.pet.character.as_deref();
        let assets_dir =
            crate::assets::resolve_assets_dir(self.cfg.sys.assets_dir.as_deref(), &self.cfg.dir);
        let list: Vec<serde_json::Value> = pets
            .iter()
            .map(|p| {
                let video_count = crate::assets::scan_webm_names(&assets_dir.join(&p.id)).len();
                let (idle_action, full_body_image) =
                    self.cfg.db.pet_baseline(&p.id).unwrap_or((None, None));
                json!({
                    "id": p.id,
                    "display_name": p.display_name,
                    "source": p.source,
                    "imported_at": p.imported_at,
                    "builtin": p.builtin,
                    "video_count": video_count,
                    "is_current": current == Some(p.id.as_str()),
                    "idle_action": idle_action,
                    "full_body_image": full_body_image,
                    "fullbody_url": format!("/api/pets/{}/fullbody", p.id),
                })
            })
            .collect();
        json!({"ok": true, "data": list})
    }

    fn switch_pet(&mut self, id: &str) -> Result<(), String> {
        let assets_dir =
            crate::assets::resolve_assets_dir(self.cfg.sys.assets_dir.as_deref(), &self.cfg.dir);
        if crate::assets::load(&assets_dir, Some(id)).is_none() {
            return Err(format!("素材加载失败: {}", id));
        }
        self.cfg.pet.character = Some(id.to_string());
        self.cfg.save();
        self.reload_assets()?;
        log_info!("切换当前桌宠 -> {}", id);
        Ok(())
    }

    fn delete_pet(&mut self, id: &str, delete_files: bool) -> Result<(), String> {
        // 若删除的是当前桌宠：卸载 pet（隐藏窗口，回退到无素材状态）
        if self.cfg.pet.character.as_deref() == Some(id) {
            if let Some(pet) = self.pet.take() {
                let win = pet.win;
                win.hide();
                self.win = Some(win);
            }
            self.cfg.pet.character = None;
            self.cfg.save();
        }
        self.cfg.db.delete_pet(id).map_err(|e| e)?;
        if delete_files {
            let assets_dir =
                crate::assets::resolve_assets_dir(self.cfg.sys.assets_dir.as_deref(), &self.cfg.dir);
            let dir = assets_dir.join(id);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("删除素材目录失败: {}", e))?;
                log_info!("已删除素材目录: {}", dir.display());
            }
        }
        log_info!("已删除桌宠: {}", id);
        Ok(())
    }

    /// 桌宠动作配置：动画清单以文件系统为准 + DB 覆盖（未登记默认 state → 空闲池）。
    fn pet_actions(&self, id: &str) -> Result<Vec<serde_json::Value>, String> {
        use serde_json::json;
        let assets_dir =
            crate::assets::resolve_assets_dir(self.cfg.sys.assets_dir.as_deref(), &self.cfg.dir);
        let names = crate::assets::scan_webm_names(&assets_dir.join(id));
        let rows = self.cfg.db.list_actions(id).unwrap_or_default();
        let row_of: std::collections::HashMap<&str, &crate::db::ActionRow> =
            rows.iter().map(|a| (a.action.as_str(), a)).collect();
        let binds = self.cfg.db.list_action_states(id).unwrap_or_default();
        let mut bind_of: std::collections::HashMap<&str, Vec<(String, f64, bool)>> =
            std::collections::HashMap::new();
        for (action, state_id, weight, enabled) in &binds {
            bind_of.entry(action.as_str()).or_default().push((state_id.clone(), *weight, *enabled));
        }
        let mut out = Vec::new();
        for n in names {
            let row = row_of.get(n.as_str()).copied();
            let states: Vec<serde_json::Value> = bind_of
                .get(n.as_str())
                .map(|v| {
                    v.iter()
                        .map(|(sid, w, en)| json!({"state_id": sid, "weight": w, "enabled": en}))
                        .collect()
                })
                .unwrap_or_default();
            out.push(json!({
                "action": n,
                "display_name": row.map(|r| r.display_name.clone()).unwrap_or_else(|| n.clone()),
                "owner_kind": row.map(|r| r.owner_kind.clone()).unwrap_or_else(|| "state".to_string()),
                "kind": row.and_then(|r| r.kind.clone()),
                "enabled": row.map(|r| r.enabled).unwrap_or(true),
                "states": states,
            }));
        }
        Ok(out)
    }

    fn save_pet_actions(&mut self, id: &str, actions: &[crate::db::ActionRow], action_states: &[(String, String, f64, bool)]) -> Result<(), String> {
        self.cfg.db.replace_actions(id, actions, action_states).map_err(|e| e)?;
        // 若保存的是当前桌宠 → 热生效
        if self.cfg.pet.character.as_deref() == Some(id) {
            self.reload_assets()?;
        }
        Ok(())
    }

    /// 编辑桌宠显示名。
    fn update_pet_name(&mut self, id: &str, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("名称不能为空".to_string());
        }
        self.cfg.db.update_display_name(id, name).map_err(|e| e)?;
        Ok(())
    }

    // ---------------- 异步转换作业（mp4 → webm，见 docs/素材转换与集成方案.md §6.2） ----------------

    /// 提交 mp4 转换作业：写库（queued）→ 后台线程跑 convert::convert_file → 进度/结果回传主线程。
    fn submit_convert_job(&mut self, pet_id: &str, action: &str, owner: &str, src_path: &str, dest: &str) -> Result<i64, String> {
        let assets_dir = crate::assets::resolve_assets_dir(self.cfg.sys.assets_dir.as_deref(), &self.cfg.dir);
        let pet_dir = assets_dir.join(pet_id);
        let _ = std::fs::create_dir_all(&pet_dir);
        let dest_webm = pet_dir.join(format!("{}.webm", dest));
        let job_id = self.cfg.db.insert_convert_job(pet_id, src_path).map_err(|e| e)?;

        let tx = self.api_tx.clone();
        let pet_id = pet_id.to_string();
        let action = action.to_string();
        let owner = owner.to_string();
        let src_path = src_path.to_string();
        let dest_webm_str = dest_webm.to_string_lossy().to_string();

        std::thread::spawn(move || {
            let send = |op: ApiOp| {
                let (rtx, _rrx) = mpsc::channel::<serde_json::Value>();
                let _ = tx.send(ApiRequest { op, reply: rtx });
            };
            send(ApiOp::ConvertProgress { job_id, progress: 0.0 });
            let closure_tx = tx.clone();
            let mut last_prog = -1.0f64;
            let mut last_t = std::time::Instant::now();
            let res = crate::convert::convert_file(
                &src_path,
                &dest_webm_str,
                &Default::default(),
                &mut |prog, msg| {
                    log_info!("转换作业 {}: {}", job_id, msg);
                    let now = std::time::Instant::now();
                    if prog < 1.0 && (prog - last_prog >= 0.02 || last_t.elapsed().as_millis() > 400) {
                        last_prog = prog;
                        last_t = now;
                        let (rtx, _rrx) = mpsc::channel::<serde_json::Value>();
                        let _ = closure_tx.send(ApiRequest { op: ApiOp::ConvertProgress { job_id, progress: prog }, reply: rtx });
                    }
                },
            );
            match res {
                Ok((w, h)) => {
                    send(ApiOp::ConvertProgress { job_id, progress: 1.0 });
                    send(ApiOp::ConvertDone { job_id, pet_id, action, owner, ok: true, error: None });
                    log_info!("转换作业 {} 完成 -> {}x{}: {}", job_id, w, h, dest_webm_str);
                }
                Err(e) => {
                    send(ApiOp::ConvertDone { job_id, pet_id, action, owner, ok: false, error: Some(e.clone()) });
                    log_error!("转换作业 {} 失败: {}", job_id, e);
                }
            }
            // 清理源 mp4（.src.mp4 不参与 webm 扫描）
            let _ = std::fs::remove_file(&src_path);
        });
        Ok(job_id)
    }

    /// 转换进度回调（主线程）：写库 status=running + progress。
    fn convert_progress(&self, job_id: i64, progress: f64) {
        let _ = self.cfg.db.update_convert_job(job_id, "running", progress, None);
    }

    /// 转换完成（主线程）：写库 done/error + 注册动作 + （若为当前桌宠热生效）。
    fn convert_done(&mut self, job_id: i64, pet_id: &str, action: &str, owner: &str, ok: bool, error: Option<String>) {
        if ok {
            let _ = self.cfg.db.update_convert_job(job_id, "done", 1.0, None);
            let owner_kind = if owner == "click" || owner == "drag" { "interactive" } else { "state" };
            let kind = if owner_kind == "interactive" { Some(owner.to_string()) } else { None };
            let row = crate::db::ActionRow {
                action: action.to_string(),
                display_name: action.to_string(),
                owner_kind: owner_kind.to_string(),
                kind,
                enabled: true,
            };
            let states: Vec<(String, f64, bool)> = if owner_kind == "state" {
                vec![("idle".to_string(), 1.0, true)]
            } else {
                vec![]
            };
            let _ = self.cfg.db.upsert_action(pet_id, &row, &states);
            if self.cfg.pet.character.as_deref() == Some(pet_id) {
                let _ = self.reload_assets();
            }
            log_info!("转换作业 {} 完成，已注册动作 {} -> {}", job_id, pet_id, action);
        } else {
            let _ = self.cfg.db.update_convert_job(job_id, "error", 0.0, error.as_deref());
            log_error!("转换作业 {} 失败: {:?}", job_id, error);
        }
    }

    fn api_get_system(&self) -> serde_json::Value {
        use serde_json::json;
        let assets_dir =
            crate::assets::resolve_assets_dir(self.cfg.sys.assets_dir.as_deref(), &self.cfg.dir);
        json!({
            "ok": true,
            "data": {
                "version": env!("CARGO_PKG_VERSION"),
                "os": std::env::consts::OS,
                "port": self.console.as_ref().map(|c| c.port),
                "url": self.console.as_ref().map(|c| c.url.clone()),
                "config_dir": self.cfg.dir,
                "yaml_path": self.cfg.yaml_path,
                "db_path": self.cfg.db.path,
                "assets_dir": assets_dir,
                "console_port": self.cfg.sys.console_port,
                "log_level": self.cfg.sys.log_level,
            }
        })
    }

    fn api_state(&self) -> serde_json::Value {
        use serde_json::json;
        let Some(pet) = &self.pet else {
            return json!({"ok": true, "data": {"pet": null}});
        };
        // get_rect 返回 (left, top, right, bottom)
        let (l, t, r, b) = pet.win.get_rect();
        json!({
            "ok": true,
            "data": {
                "pet": {
                    "anim": pet.cur_anim,
                    "state_id": pet.current_state,
                    "x": l, "y": t, "w": r - l, "h": b - t,
                    "facing_right": pet.facing_right,
                    "scale": pet.scale,
                    "visible": pet.visible,
                    "no_move": pet.no_move,
                    "topmost": pet.win_topmost,
                }
            }
        })
    }

    fn api_get_config(&self) -> serde_json::Value {
        use serde_json::json;
        let v = serde_json::to_value(&self.cfg.pet).unwrap_or(serde_json::Value::Null);
        json!({"ok": true, "data": v})
    }

    /// PATCH /api/config：合并可写字段 → 生效 → 落盘。未知字段忽略。
    fn api_patch_config(&mut self, patch: serde_json::Value) -> serde_json::Value {
        use serde_json::json;
        let o = match patch.as_object() {
            Some(o) => o,
            None => return json!({"ok": false, "error": "body must be object"}),
        };
        let mut applied: Vec<&str> = Vec::new();
        if let Some(v) = o.get("scale").and_then(|x| x.as_f64()) {
            self.cfg.pet.scale = v;
            if let Some(p) = &mut self.pet {
                p.change_scale(v);
            }
            applied.push("scale");
        }
        if let Some(v) = o.get("always_on_top").and_then(|x| x.as_bool()) {
            self.cfg.pet.always_on_top = v;
            if let Some(p) = &mut self.pet {
                p.set_topmost(v);
            }
            applied.push("always_on_top");
        }
        if let Some(v) = o.get("no_move").and_then(|x| x.as_bool()) {
            self.cfg.pet.no_move = v;
            if let Some(p) = &mut self.pet {
                p.set_no_move(v);
            }
            applied.push("no_move");
        }
        if let Some(v) = o.get("facing_right").and_then(|x| x.as_bool()) {
            self.cfg.pet.facing_right = v; // 下一帧渲染即生效（每帧读取）
            applied.push("facing_right");
        }
        if let Some(v) = o.get("visible").and_then(|x| x.as_bool()) {
            if let Some(p) = &mut self.pet {
                if v {
                    p.win.show();
                    p.visible = true;
                } else {
                    p.win.hide();
                    p.visible = false;
                }
            }
            applied.push("visible");
        }
        // 行为（移动 / 缩放 / 状态集合）
        let mut behavior_changed = false;
        if let Some(v) = o.get("move_min_px").and_then(|x| x.as_f64()) {
            self.cfg.pet.move_min_px = v;
            behavior_changed = true;
            applied.push("move_min_px");
        }
        if let Some(v) = o.get("move_max_px").and_then(|x| x.as_f64()) {
            self.cfg.pet.move_max_px = v;
            behavior_changed = true;
            applied.push("move_max_px");
        }
        if let Some(v) = o.get("move_margin_px").and_then(|x| x.as_f64()) {
            self.cfg.pet.move_margin_px = v;
            behavior_changed = true;
            applied.push("move_margin_px");
        }
        if let Some(v) = o.get("scale_steps").and_then(|x| x.as_array()) {
            let steps: Vec<f64> = v.iter().filter_map(|x| x.as_f64()).collect();
            if !steps.is_empty() {
                self.cfg.pet.scale_steps = steps;
                behavior_changed = true;
                applied.push("scale_steps");
            }
        }
        if let Some(v) = o.get("behavior_states").and_then(|x| serde_json::from_value::<Vec<crate::behavior::StateDef>>(x.clone()).ok()) {
            if !v.is_empty() {
                self.cfg.pet.behavior_states = v;
                behavior_changed = true;
                applied.push("behavior_states");
            }
        }
        if behavior_changed {
            self.cfg.pet.normalize_behavior();
            if let Some(p) = &mut self.pet {
                p.behavior = crate::pet::Behavior::from(&self.cfg.pet);
                p.states = self.cfg.pet.behavior_states.clone();
            }
            log_info!(
                "行为参数已更新: 状态集合={} move={}-{}",
                self.cfg.pet.behavior_states.len(),
                self.cfg.pet.move_min_px,
                self.cfg.pet.move_max_px,
            );
        }
        self.cfg.save();
        json!({"ok": true, "data": {"applied": applied}})
    }

    /// 处理宠物右键菜单命令（自启全局命令 + 宠物自身命令）。
    pub fn handle_command(&mut self, cmd: usize) {
        log_debug!("执行菜单命令: {}", cmd);
        if cmd == pet::MID_AUTOSTART {
            let on = !crate::autostart::is_enabled();
            crate::autostart::set_enabled(on);
        } else if let Some(pet) = &mut self.pet {
            pet.apply_command(cmd);
            self.save_position();
        }
    }
}

// ---------------- Windows 消息桥 ----------------

#[cfg(windows)]
mod win32_bridge {
    use super::App;
    use crate::menu::MenuEntry;
    use crate::win32::{self, FRAME_TIMER, WindowCallback};
    use windows_sys::Win32::{
        Foundation::{HWND, LRESULT, LPARAM, WPARAM},
        UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture},
        UI::WindowsAndMessaging::{
            GetWindowLongPtrW, LoadCursorW, PostMessageW, SetCursor, SetForegroundWindow,
            SetWindowLongPtrW, GWL_EXSTYLE, IDC_HAND, WM_LBUTTONDOWN, WM_LBUTTONUP,
            WM_MOUSEMOVE, WM_NCHITTEST, WM_NULL, WM_SETCURSOR, WM_TIMER, WS_EX_NOACTIVATE,
            HTCLIENT, HTTRANSPARENT,
        },
    };

    impl WindowCallback for App {
        fn on_wnd_message(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
            match msg {
                WM_NCHITTEST => {
                    let gx = (lparam & 0xFFFF) as i16 as i32;
                    let gy = ((lparam >> 16) & 0xFFFF) as i16 as i32;
                    let hit = self.pet.as_ref().map(|p| p.hit_at(gx, gy)).unwrap_or(false);
                    Some(if hit { HTCLIENT as LRESULT } else { HTTRANSPARENT as LRESULT })
                }
                WM_LBUTTONDOWN => {
                    // 捕获鼠标：快速拖拽时鼠标移出窗口仍能收到事件
                    unsafe { SetCapture(hwnd) };
                    let (cx, cy) = client_pos(lparam);
                    self.on_pet_press(cx, cy);
                    Some(0)
                }
                WM_MOUSEMOVE => {
                    self.on_pet_drag();
                    Some(0)
                }
                WM_LBUTTONUP => {
                    self.on_pet_release();
                    unsafe { ReleaseCapture() };
                    Some(0)
                }
                // 光标悬停在桌宠身上（不透明像素）→ 手型，提示可交互。
                // 移出到透明区域时 WM_NCHITTEST 返回 HTTRANSPARENT，光标由下层窗口恢复箭头。
                WM_SETCURSOR => {
                    let n_hit = (lparam & 0xFFFF) as u32;
                    if n_hit == HTCLIENT {
                        let (sx, sy) = win32::cursor_pos();
                        let hit = self.pet.as_ref().map(|p| p.hit_at(sx, sy)).unwrap_or(false);
                        if hit {
                            let cursor = unsafe { LoadCursorW(std::ptr::null_mut(), IDC_HAND) };
                            if !cursor.is_null() {
                                unsafe { SetCursor(cursor) };
                            }
                            return Some(0);
                        }
                    }
                    None
                }
                WM_TIMER if (wparam as usize) == FRAME_TIMER => {
                    self.on_pet_tick();
                    Some(0)
                }
                super::WM_TRAY => {
                    // Shell_NotifyIconW 回调：lParam 低字 = 触发鼠标消息
                    match (lparam & 0xFFFF) as u32 {
                        0x0202 => {
                            // WM_LBUTTONUP：左键单击切换显示/隐藏
                            self.toggle_visible();
                        }
                        0x0205 => {
                            // WM_RBUTTONUP：右键弹出完整菜单（桌宠菜单 + 托盘项）
                            self.show_tray_menu();
                        }
                        _ => {}
                    }
                    Some(0)
                }
                _ => None,
            }
        }
    }

    impl App {
        /// 托盘右键菜单：桌宠菜单（角落/置顶/不移动/自启/大小）+ 显示隐藏/退出。
        fn show_tray_menu(&mut self) {
            let hwnd = self.primary_hwnd();
            if hwnd.is_null() {
                return;
            }
            let mut items = self
                .pet
                .as_ref()
                .map(|p| p.context_menu_items())
                .unwrap_or_default();
            items.push(MenuEntry::separator());
            items.push(MenuEntry::item(crate::tray::TRAY_CONSOLE, "打开控制台"));
            items.push(MenuEntry::item(crate::tray::TRAY_TOGGLE_VISIBLE, "显示/隐藏"));
            items.push(MenuEntry::item(crate::tray::TRAY_QUIT, "退出"));
            let (sx, sy) = crate::win32::cursor_pos();
            // NOACTIVATE 窗口弹菜单的经典修复：
            // 1) 临时移除 WS_EX_NOACTIVATE，否则 SetForegroundWindow 无效（窗口不可激活）；
            // 2) SetForegroundWindow 让本窗口成为前台——TrackPopupMenu 的菜单才能成为前台，
            //    否则点击菜单外区域时菜单没有失活事件，不会自动关闭；
            // 3) 菜单返回（已关闭）后恢复 WS_EX_NOACTIVATE，并 PostMessage(WM_NULL)
            //    让系统正确回收焦点。
            let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
            unsafe {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex & !(WS_EX_NOACTIVATE as isize));
                SetForegroundWindow(hwnd);
            }
            let cmd = crate::win32::show_menu_blocking(hwnd, &items, sx, sy);
            unsafe {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex);
                PostMessageW(hwnd, WM_NULL, 0, 0);
            }
            if cmd != 0 {
                if cmd >= crate::tray::TRAY_TOGGLE_VISIBLE {
                    self.handle_tray_command(cmd);
                } else {
                    self.handle_command(cmd);
                }
            }
        }
    }

    fn client_pos(lparam: LPARAM) -> (i32, i32) {
        let x = (lparam & 0xFFFF) as i16 as i32;
        let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
        (x, y)
    }
}
