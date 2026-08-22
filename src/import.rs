//! 素材 zip 导入/导出（见 docs/素材转换与集成方案.md §7）。
//!
//! 两类包：
//! - **宠物 zip（webm 成品，§7.2）**：严格结构（`manifest.yaml` + `fullbody.png` + 全部 `*.webm`），
//!   导入即还原、导出即生成；**缺任一必填项 → 拒绝导入**（不强约束、不宽松回退）。
//! - **视频包（源素材，§7.3）**：仅 `*.mp4`/`*.mov`，**不带 manifest**；导入时转换 + 提取全身照 + 存锚点。
//!
//! 本模块负责两类包的校验/解压/落位，以及宠物 zip 的导出组装。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Cursor, Read, Seek, Write};
use std::path::Path;

use crate::db::{ActionRow, PetAnchor};
use crate::webm::WebM;
use zip::ZipArchive;

/// 导入结果报告。
pub struct ImportReport {
    /// 素材集 id（时间戳生成，净化后；兼作素材子目录名）。
    pub id: String,
    pub display_name: String,
    pub video_count: usize,
    /// 成功校验的动作名（webm 文件名 stem，唯一）。
    pub videos: Vec<String>,
    pub warnings: Vec<String>,
    /// 宠物名（manifest.pet.name；缺省用 id）。
    pub pet_name: Option<String>,
    /// 体型基准（待机）动作名（manifest.idle）。
    pub idle: Option<String>,
    /// manifest 解析出的动作归属。
    pub actions: Vec<crate::db::ActionRow>,
    /// 动作 → 状态绑定。
    pub action_states: Vec<(String, String, f64, bool)>,
    /// 宠物锚点（manifest.anchor；跨动画共享归一化基准）。
    pub anchor: Option<crate::db::PetAnchor>,
    /// 全身照文件名（导入落位后为 `fullbody.png`）。
    pub full_body: Option<String>,
}

// ---------------- 严格 manifest（宠物 zip §7.2 全必填） ----------------

#[derive(Deserialize)]
struct StrictManifest {
    pet: PetMeta,
    idle: String,
    anchor: AnchorMeta,
    full_body: String,
    actions: Vec<RawAction>,
}

#[derive(Deserialize)]
struct PetMeta {
    name: String,
}

#[derive(Deserialize, Clone)]
struct AnchorMeta {
    scale: f64,
    h_ref: f64,
    source_w: i64,
    source_h: i64,
}

#[derive(Deserialize)]
struct RawAction {
    file: String,
    #[serde(default)] display_name: Option<String>,
    /// state | click | drag
    #[serde(default)] owner: Option<String>,
    #[serde(default)] states: Vec<RawState>,
}

#[derive(Deserialize)]
struct RawState {
    state: String,
    #[serde(default)] weight: Option<f64>,
    #[serde(default)] enabled: Option<bool>,
}

// ---------------- 导出 manifest（宠物 zip §7.2） ----------------

#[derive(Serialize)]
struct ExportManifest {
    pet: ExportPet,
    idle: String,
    anchor: ExportAnchor,
    full_body: String,
    actions: Vec<ExportAction>,
}

#[derive(Serialize)]
struct ExportPet {
    name: String,
}

#[derive(Serialize)]
struct ExportAnchor {
    scale: f64,
    h_ref: f64,
    source_w: i64,
    source_h: i64,
}

#[derive(Serialize)]
struct ExportAction {
    file: String,
    display_name: String,
    owner: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    states: Vec<ExportState>,
}

#[derive(Serialize)]
struct ExportState {
    state: String,
    weight: f64,
}

/// 从严格 manifest 构建动作归属 + 状态绑定；返回 (actions, action_states, idle, anchor)。
fn build_from_manifest(m: &StrictManifest) -> (Vec<ActionRow>, Vec<(String, String, f64, bool)>, String, AnchorMeta) {
    let mut actions = Vec::new();
    let mut states = Vec::new();
    for raw in &m.actions {
        let owner = raw.owner.clone().unwrap_or_else(|| "state".to_string());
        let display = raw.display_name.clone().unwrap_or_else(|| raw.file.clone());
        if owner == "click" || owner == "drag" {
            actions.push(ActionRow {
                action: raw.file.clone(),
                display_name: display,
                owner_kind: "interactive".to_string(),
                kind: Some(owner.clone()),
                enabled: true,
            });
        } else {
            actions.push(ActionRow {
                action: raw.file.clone(),
                display_name: display,
                owner_kind: "state".to_string(),
                kind: None,
                enabled: true,
            });
            if raw.states.is_empty() {
                states.push((raw.file.clone(), "idle".to_string(), 1.0, true));
            } else {
                for st in &raw.states {
                    states.push((raw.file.clone(), st.state.clone(), st.weight.unwrap_or(1.0), st.enabled.unwrap_or(true)));
                }
            }
        }
    }
    (actions, states, m.idle.clone(), m.anchor.clone())
}

/// 解析待机基准（体型基准/全身照来源）兜底：优先「动作绑定到 idle 状态池」，其次第一个 state 类动作。
pub fn resolve_idle_from(
    actions: &[crate::db::ActionRow],
    action_states: &[(String, String, f64, bool)],
    videos: &[String],
) -> Option<String> {
    for (a, s, _, _) in action_states {
        if s == "idle" {
            return Some(a.clone());
        }
    }
    for a in actions {
        if a.owner_kind == "state" {
            return Some(a.action.clone());
        }
    }
    videos.first().cloned()
}

/// 从待机动画提取一帧并落盘为 `<素材根>/<pet_id>/fullbody.png`，返回相对文件名。
/// 若该动作的 webm 缺失或取帧失败，返回 Err（调用方用 None 容忍，不阻断建宠）。
pub fn generate_full_body(assets_root: &Path, pet_id: &str, idle_action: &str) -> Result<String, String> {
    let src = assets_root.join(pet_id).join(format!("{}.webm", idle_action));
    let bytes = std::fs::read(&src).map_err(|e| format!("读取待机动画失败 {}: {}", src.display(), e))?;
    let png = crate::thumb::extract_png(&bytes).ok_or_else(|| format!("待机动画取帧失败: {}", idle_action))?;
    let dest = assets_root.join(pet_id).join("fullbody.png");
    std::fs::write(&dest, png).map_err(|e| format!("写入全身照失败: {}", e))?;
    log_info!("全身照已生成: {}", dest.display());
    Ok("fullbody.png".to_string())
}

// ---------------- 宠物 zip（§7.2）严格导入 ----------------

/// 校验宠物 zip 并解压到素材根。`Err` 表示失败（未落盘任何内容）。
/// **严格结构**：必须含 `manifest.yaml`（pet/name、idle、anchor、full_body、actions 全必填）、
/// `fullbody.png`、以及每个 `actions.file` 对应的 webm。缺任一 → 拒绝导入。
pub fn import_zip(zip_bytes: &[u8], assets_root: &Path) -> Result<ImportReport, String> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| format!("zip 解析失败: {}", e))?;

    // 1. manifest 必须存在。
    let manifest = read_strict_manifest(&mut archive)?;

    // 2. 收集 zip 内全部 webm stem、全部文件名（用于锚点/全身照/动作校验）。
    let webm_entries = collect_webm_entries(&mut archive)?;
    let webm_stems: HashSet<String> = webm_entries.iter().map(|(_, s)| s.clone()).collect();
    let all_names = collect_all_names(&mut archive);

    // 3. 逐动作校验：每个 manifest.actions.file 必须有对应 webm。
    let mut warnings = Vec::new();
    for raw in &manifest.actions {
        if !webm_stems.contains(&raw.file) {
            return Err(format!("manifest 引用的动作缺少 webm: {}", raw.file));
        }
    }
    // 4. 校验全身照文件存在（按 basename 匹配，容忍目录层级）。
    if !all_names.iter().any(|n| n.ends_with(&manifest.full_body)) {
        return Err(format!("zip 缺少全身照: {}", manifest.full_body));
    }

    // 5. 校验每个 webm 可解析。
    let mut ok_entries: Vec<(usize, String)> = Vec::new();
    for (idx, stem) in &webm_entries {
        match read_entry(&mut archive, *idx) {
            Ok(data) => match WebM::parse(&data) {
                Some(_) => ok_entries.push((*idx, stem.clone())),
                None => return Err(format!("{}: 不是有效的 webm", stem)),
            },
            Err(e) => return Err(format!("{}: 读取失败 ({})", stem, e)),
        }
    }
    if ok_entries.is_empty() {
        return Err("zip 内未找到可用的 webm".to_string());
    }

    // 6. 生成素材集 id，构建动作归属。
    let id = new_pet_id();
    let (manifest_actions, manifest_states, idle, anchor_meta) = build_from_manifest(&manifest);
    let pet_name = Some(manifest.pet.name.clone()).filter(|s| !s.is_empty());
    warnings.push("按严格 manifest 配置动作归属".to_string());

    // 7. 一次 staged 落位：平铺解压全部 webm + 全身照 → 原子替换 <素材根>/<id>/。
    extract_pet_to(assets_root, &id, &mut archive, &ok_entries, &manifest.full_body, &mut warnings)?;

    let anchor = Some(PetAnchor {
        pet_id: id.clone(),
        scale: anchor_meta.scale,
        h_ref: anchor_meta.h_ref,
        source_w: anchor_meta.source_w,
        source_h: anchor_meta.source_h,
    });

    Ok(ImportReport {
        display_name: id.clone(),
        id,
        video_count: ok_entries.len(),
        videos: ok_entries.iter().map(|(_, s)| s.clone()).collect(),
        warnings,
        pet_name,
        idle: Some(idle),
        actions: manifest_actions,
        action_states: manifest_states,
        anchor,
        full_body: Some("fullbody.png".to_string()),
    })
}

// ---------------- 视频包（§7.3）导入：仅源视频 ----------------

/// 视频包导入结果（仅第一步：校验 + 解压落位，**不入库**）。
pub struct VideoPackageReport {
    pub pet_id: String,
    /// 解压到 `/<pet_id>/<stem>.src.mp4` 的文件名（stem 列表）。
    pub files: Vec<String>,
}

/// 校验视频包 zip（仅 `*.mp4`/`*.mov`，**不带 manifest**）并解压到 `<素材根>/<pet_id>/` 为
/// `<stem>.src.mp4`（.mp4 不参与 webm 扫描，转换后清理）。`Err` 表示失败（未落盘任何内容）。
pub fn import_pet_video_zip(zip_bytes: &[u8], assets_root: &Path) -> Result<VideoPackageReport, String> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| format!("zip 解析失败: {}", e))?;

    let mut entries: Vec<(usize, String)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("zip 条目读取失败: {}", e))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry_name(&entry);
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".mp4") || lower.ends_with(".mov") {
            let base = name.rsplit('/').next().unwrap_or(&name);
            let stem = base.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_else(|| base.to_string());
            if !stem.is_empty() && seen.insert(stem.clone()) {
                entries.push((i, stem));
            }
        }
    }
    if entries.is_empty() {
        return Err("视频包内未找到 .mp4/.mov 源视频".to_string());
    }

    let id = new_pet_id();
    let pet_dir = assets_root.join(&id);
    if let Err(e) = std::fs::create_dir_all(&pet_dir) {
        return Err(format!("创建素材目录失败: {}", e));
    }

    let mut files = Vec::new();
    for (idx, stem) in &entries {
        let data = read_entry(&mut archive, *idx)?;
        let dest = pet_dir.join(format!("{}.src.mp4", stem));
        std::fs::write(&dest, data).map_err(|e| format!("写入 {} 失败: {}", dest.display(), e))?;
        files.push(stem.clone());
    }
    log_info!("视频包解压完成: {} ({} 个源视频) -> {}", id, files.len(), pet_dir.display());
    Ok(VideoPackageReport { pet_id: id, files })
}

// ---------------- 宠物 zip（§7.4）导出 ----------------

/// 把一只宠组装为 §7.2 严格 zip 字节。
/// 需要：pet 名、idle、锚点（必填，否则报错）、全部 webm、全身照、动作归属。
pub fn export_pet_zip(
    assets_root: &Path,
    pet_id: &str,
    name: &str,
    idle: &str,
    anchor: &PetAnchor,
    actions: &[ActionRow],
    action_states: &[(String, String, f64, bool)],
) -> Result<(Vec<u8>, String), String> {
    let pet_dir = assets_root.join(pet_id);
    if !pet_dir.is_dir() {
        return Err(format!("素材目录不存在: {}", pet_id));
    }
    // 全部 webm 以磁盘为准。
    let names_on_disk = crate::assets::scan_webm_names(&pet_dir);
    if names_on_disk.is_empty() {
        return Err(format!("宠物 {} 无 webm 动作", pet_id));
    }
    // 全身照必须存在。
    let fb_path = pet_dir.join("fullbody.png");
    if !fb_path.is_file() {
        return Err(format!("宠物 {} 缺少全身照 fullbody.png", pet_id));
    }
    // 动作归属映射。
    let row_of: std::collections::HashMap<&str, &ActionRow> = actions.iter().map(|a| (a.action.as_str(), a)).collect();
    let mut bm = std::collections::HashMap::new();
    for (action, state_id, weight, _en) in action_states {
        bm.entry(action.as_str()).or_insert_with(Vec::new).push((state_id.clone(), *weight));
    }

    // manifest.actions
    let mut out_actions = Vec::new();
    for n in &names_on_disk {
        let row = row_of.get(n.as_str()).copied();
        let owner = row.map(|r| r.owner_kind.clone()).unwrap_or_else(|| "state".to_string());
        let owner_label = if owner == "interactive" {
            row.and_then(|r| r.kind.clone()).unwrap_or_else(|| "click".to_string())
        } else {
            "state".to_string()
        };
        let display = row.map(|r| r.display_name.clone()).unwrap_or_else(|| n.clone());
        let states = bm.get(n.as_str())
            .map(|list| list.iter().map(|(sid, w)| ExportState { state: sid.clone(), weight: *w }).collect())
            .unwrap_or_default();
        out_actions.push(ExportAction { file: n.clone(), display_name: display, owner: owner_label, states });
    }
    let manifest = ExportManifest {
        pet: ExportPet { name: name.to_string() },
        idle: idle.to_string(),
        anchor: ExportAnchor { scale: anchor.scale, h_ref: anchor.h_ref, source_w: anchor.source_w, source_h: anchor.source_h },
        full_body: "fullbody.png".to_string(),
        actions: out_actions,
    };
    let manifest_yaml = serde_yaml::to_string(&manifest).map_err(|e| format!("序列化 manifest 失败: {}", e))?;

    // 组装 zip（winzip 兼容，UTF-8 文件名）。
    let cursor = Cursor::new(Vec::new());
    let mut zw = zip::ZipWriter::new(cursor);
    let opts = zip::write::SimpleFileOptions::default();
    zw.start_file("manifest.yaml", opts).map_err(|e| format!("写 zip 失败: {}", e))?;
    zw.write_all(manifest_yaml.as_bytes()).map_err(|e| format!("写 manifest 失败: {}", e))?;
    for n in &names_on_disk {
        let p = pet_dir.join(format!("{}.webm", n));
        let bytes = std::fs::read(&p).map_err(|e| format!("读取 {} 失败: {}", p.display(), e))?;
        zw.start_file(format!("{}.webm", n), opts).map_err(|e| format!("写 zip 失败: {}", e))?;
        zw.write_all(&bytes).map_err(|e| format!("写 {} 失败: {}", n, e))?;
    }
    {
        let bytes = std::fs::read(&fb_path).map_err(|e| format!("读取全身照失败: {}", e))?;
        zw.start_file("fullbody.png", opts).map_err(|e| format!("写 zip 失败: {}", e))?;
        zw.write_all(&bytes).map_err(|e| format!("写全身照失败: {}", e))?;
    }
    let inner = zw.finish().map_err(|e| format!("结束 zip 失败: {}", e))?;
    let bytes = inner.into_inner();
    let filename = format!("{}.zip", name.trim());
    Ok((bytes, filename))
}

// ---------------- 读取 zip 条目 ----------------

fn read_strict_manifest<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<StrictManifest, String> {
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("zip 条目读取失败: {}", e))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry_name(&entry);
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("manifest.yaml") || lower.ends_with("manifest.yml") {
            drop(entry);
            if let Ok(mut e) = archive.by_index(i) {
                let mut buf = Vec::new();
                if e.read_to_end(&mut buf).is_ok() {
                    return serde_yaml::from_slice::<StrictManifest>(&buf)
                        .map_err(|err| format!("manifest.yaml 解析失败: {}", err));
                }
            }
            return Err("manifest.yaml 读取失败".to_string());
        }
    }
    Err("宠物 zip 必须包含 manifest.yaml".to_string())
}

/// 递归收集全部 `*.webm` 条目 → (条目索引, 文件名 stem)。
fn collect_webm_entries<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Vec<(usize, String)>, String> {
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| format!("zip 条目读取失败: {}", e))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry_name(&entry);
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".webm") {
            let stem = name.rsplit('/').next().unwrap_or(&name);
            let stem = if stem.len() > 5 { &stem[..stem.len() - 5] } else { stem };
            if !stem.is_empty() {
                out.push((i, stem.to_string()));
            }
        }
    }
    Ok(out)
}

/// 收集 zip 内全部（目录无关的）原始文件名字符串，供全身照等 basename 匹配。
fn collect_all_names<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Vec<String> {
    let mut out = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            if !entry.is_dir() {
                out.push(entry_name(&entry));
            }
        }
    }
    out
}

fn read_entry<R: Read + Seek>(archive: &mut ZipArchive<R>, idx: usize) -> Result<Vec<u8>, String> {
    let mut entry = archive.by_index(idx).map_err(|e| format!("zip 条目读取失败: {}", e))?;
    let mut buf = Vec::new();
    entry.read_to_end(&mut buf).map_err(|e| format!("zip 条目读取失败: {}", e))?;
    Ok(buf)
}

/// 单次 staged 落位：把全部 webm + 指定全身照文件解压到 `<素材根>/<id>/`（原子替换）。
fn extract_pet_to<R: Read + Seek>(
    assets_root: &Path,
    id: &str,
    archive: &mut ZipArchive<R>,
    webm_entries: &[(usize, String)],
    full_body_name: &str,
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    extract_into_stage(assets_root, id, |tmp, _| {
        for (idx, stem) in webm_entries {
            let dest = tmp.join(format!("{}.webm", stem));
            if let Err(e) = read_entry(archive, *idx).and_then(|data| {
                std::fs::write(&dest, data).map_err(|e| format!("写入失败 {}: {}", dest.display(), e))
            }) {
                warnings.push(format!("{}: {}", stem, e));
            }
        }
        // 全身照：匹配 zip 内 basename == full_body_name 的文件。
        let mut found = false;
        for i in 0..archive.len() {
            let entry = archive.by_index(i).map_err(|e| format!("zip 条目读取失败: {}", e))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry_name(&entry);
            if name == full_body_name || name.ends_with(format!("/{}", full_body_name).as_str()) {
                drop(entry);
                let data = read_entry(archive, i)?;
                let dest = tmp.join("fullbody.png");
                std::fs::write(&dest, data).map_err(|e| format!("写入失败 {}: {}", dest.display(), e))?;
                found = true;
                break;
            }
        }
        if !found {
            return Err(format!("zip 内未找到全身照: {}", full_body_name));
        }
        Ok(())
    })
}

/// 在临时目录上执行 `fill`，成功后原子替换 `<素材根>/<id>`。
fn extract_into_stage<F>(assets_root: &Path, id: &str, fill: F) -> Result<(), String>
where
    F: FnOnce(&std::path::Path, &mut Vec<String>) -> Result<(), String>,
{
    let target = assets_root.join(id);
    let tmp = assets_root.join(format!(".{}.tmp", id));
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = std::fs::create_dir_all(&tmp) {
        return Err(format!("创建临时目录失败: {}", e));
    }
    let mut warnings = Vec::new();
    if let Err(e) = fill(&tmp, &mut warnings) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(&target);
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("落位失败 {}: {}", target.display(), e));
    }
    log_info!("素材落位完成: {}", target.display());
    Ok(())
}

/// 素材集 id：`pet_<unix 毫秒>`（净化；时间戳保证唯一）。
fn new_pet_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    format!("pet_{}", ms)
}

/// zip 条目名归一化：优先按原始字节做 UTF-8 解码（兼容未设 UTF-8 标志但名称为
/// UTF-8 的 zip），失败则用 crate 的解码（CP437）；统一 / 分隔、去掉开头的 ./
fn entry_name(entry: &zip::read::ZipFile) -> String {
    let raw = entry.name_raw();
    let s = std::str::from_utf8(raw).unwrap_or_else(|_| entry.name());
    s.replace('\\', "/").trim_start_matches("./").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_manifest_parses_and_builds_actions() {
        let yaml = r#"
pet:
  name: 蓝发女仆
idle: 待机呼吸休闲
anchor:
  scale: 0.2345
  h_ref: 1258
  source_w: 1280
  source_h: 720
full_body: fullbody.png
actions:
  - file: 待机呼吸休闲
    display_name: 待机呼吸休闲
    owner: state
    states:
      - { state: idle, weight: 0.6 }
  - file: 开心蹦跳
    owner: state
    states:
      - { state: active, weight: 0.8 }
  - file: 挥手打招呼
    owner: click
"#;
        let m: StrictManifest = serde_yaml::from_str(yaml).expect("严格 manifest 应可解析");
        assert_eq!(m.pet.name, "蓝发女仆");
        assert_eq!(m.idle, "待机呼吸休闲");
        assert_eq!(m.anchor.scale, 0.2345);
        assert_eq!(m.full_body, "fullbody.png");

        let (actions, states, idle, anchor) = build_from_manifest(&m);
        assert_eq!(idle, "待机呼吸休闲");
        assert_eq!(anchor.h_ref, 1258.0);
        let click = actions.iter().find(|a| a.action == "挥手打招呼").unwrap();
        assert_eq!(click.owner_kind, "interactive");
        assert_eq!(click.kind.as_deref(), Some("click"));
        let state_row = actions.iter().find(|a| a.action == "开心蹦跳").unwrap();
        assert_eq!(state_row.owner_kind, "state");
        assert!(states.iter().any(|(a, s, w, _)| a == "开心蹦跳" && s == "active" && *w == 0.8));
        assert!(states.iter().any(|(a, s, _, _)| a == "待机呼吸休闲" && s == "idle"));
    }

    #[test]
    fn strict_manifest_rejects_missing_fields() {
        // 缺 anchor / full_body → 解析失败
        let yaml = r#"
pet:
  name: 蓝发女仆
idle: 待机
actions:
  - file: 待机
"#;
        assert!(serde_yaml::from_str::<StrictManifest>(yaml).is_err());
    }
}
