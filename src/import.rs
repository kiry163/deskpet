//! 素材 zip 导入（见 docs/需求规格.md §4）：**递归扫描 webm**（无 manifest 也能导入），
//! 若含 `manifest.yaml` 则解析出**宠物名 / 待机基准 / 动作→状态归属**（见 docs/行为状态机设计.md §11）。
//! 流程：校验（zip 可读、每个 webm 可解析）→ 平铺解压全部 webm 到 `<素材根>/<id>/`
//! （动画名 = 文件名 stem）→ 返回报告（含可选 manifest 数据供注册）。
//! 校验失败不落盘任何内容，解压走临时目录 + 原子替换，避免污染素材根。

use serde::Deserialize;
use std::io::{Cursor, Read, Seek};
use std::path::Path;

use crate::webm::WebM;
use zip::ZipArchive;

/// 导入结果报告。
pub struct ImportReport {
    /// 素材集 id（时间戳生成，净化后；兼作素材子目录名）。
    pub id: String,
    pub display_name: String,
    pub video_count: usize,
    /// 成功校验的动画名（webm 文件名 stem，唯一）。
    pub videos: Vec<String>,
    pub warnings: Vec<String>,
    /// manifest 给出的宠物名（可选；缺省用 id）。
    pub pet_name: Option<String>,
    /// 体型基准（待机）动作名（manifest.idle，可选）。
    pub idle: Option<String>,
    /// manifest 解析出的动作归属（空 = 由调用方用默认值）。
    pub actions: Vec<crate::db::ActionRow>,
    /// 动作 → 状态绑定（空 = 默认归空闲池）。
    pub action_states: Vec<(String, String, f64, bool)>,
}

// ---------------- manifest（可选） ----------------

#[derive(Deserialize, Default)]
struct PetManifest {
    #[serde(default)] pet: Option<PetMeta>,
    #[serde(default)] idle: Option<String>,
    #[serde(default)] actions: Vec<RawAction>,
}

#[derive(Deserialize, Default)]
struct PetMeta {
    #[serde(default)] name: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawAction {
    file: String,
    #[serde(default)] display_name: Option<String>,
    /// state | click | drag
    #[serde(default)] owner: Option<String>,
    #[serde(default)] states: Vec<RawState>,
}

#[derive(Deserialize, Default)]
struct RawState {
    state: String,
    #[serde(default)] weight: Option<f64>,
    #[serde(default)] enabled: Option<bool>,
}

/// 从 manifest 构建动作归属 + 状态绑定 + 待机基准。
fn build_from_manifest(
    m: &PetManifest,
    valid: &[String],
) -> (Vec<crate::db::ActionRow>, Vec<(String, String, f64, bool)>, Option<String>) {
    let mut actions = Vec::new();
    let mut action_states = Vec::new();
    let row_of: std::collections::HashMap<&str, &RawAction> =
        m.actions.iter().map(|a| (a.file.as_str(), a)).collect();

    for name in valid {
        let raw = row_of.get(name.as_str()).copied();
        let owner = raw.and_then(|r| r.owner.clone()).unwrap_or_else(|| "state".to_string());
        let display_name = raw.and_then(|r| r.display_name.clone()).unwrap_or_else(|| name.clone());
        if owner == "click" || owner == "drag" {
            actions.push(crate::db::ActionRow {
                action: name.clone(),
                display_name,
                owner_kind: "interactive".to_string(),
                kind: Some(owner.clone()),
                enabled: true,
            });
            continue;
        }
        actions.push(crate::db::ActionRow {
            action: name.clone(),
            display_name,
            owner_kind: "state".to_string(),
            kind: None,
            enabled: true,
        });
        if let Some(raw) = raw {
            for st in &raw.states {
                action_states.push((
                    name.clone(),
                    st.state.clone(),
                    st.weight.unwrap_or(1.0),
                    st.enabled.unwrap_or(true),
                ));
            }
        }
        if action_states_last_is_empty(&action_states, name) {
            // 未声明归属 → 默认空闲池
            action_states.push((name.clone(), "idle".to_string(), 1.0, true));
        }
    }
    // 待机基准：manifest.idle 合法则用它，否则取第一个动作
    let idle = match m.idle.as_deref() {
        Some(idle) if valid.iter().any(|n| n == idle) => Some(idle.to_string()),
        _ => valid.first().cloned(),
    };
    (actions, action_states, idle)
}

fn action_states_last_is_empty(list: &[(String, String, f64, bool)], name: &str) -> bool {
    !list.iter().any(|(a, _, _, _)| a == name)
}

/// 解析待机基准（体型基准/全身照来源）：
/// 优先「动作绑定到 idle 状态池」的返回顺序，其次第一个 state 类动作，最后第一个视频。
/// 供 `apply_import` 无 manifest.idle 时兜底，与 `build_from_manifest` 的默认归属一致。
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
/// 若该动作的 webm 缺失或取帧失败，返回 Err（调用方用 None 容忍，不阻断导入）。
pub fn generate_full_body(assets_root: &Path, pet_id: &str, idle_action: &str) -> Result<String, String> {
    let src = assets_root.join(pet_id).join(format!("{}.webm", idle_action));
    let bytes = std::fs::read(&src).map_err(|e| format!("读取待机动画失败 {}: {}", src.display(), e))?;
    let png = crate::thumb::extract_png(&bytes).ok_or_else(|| format!("待机动画取帧失败: {}", idle_action))?;
    let dest = assets_root.join(pet_id).join("fullbody.png");
    std::fs::write(&dest, png).map_err(|e| format!("写入全身照失败: {}", e))?;
    log_info!("全身照已生成: {}", dest.display());
    Ok("fullbody.png".to_string())
}

/// 校验 zip 并解压到素材根。`Err` 表示失败（未落盘任何内容）。
pub fn import_zip(zip_bytes: &[u8], assets_root: &std::path::Path) -> Result<ImportReport, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| format!("zip 解析失败: {}", e))?;

    // 1. 递归收集全部 *.webm（忽略目录结构）
    let entries = collect_webm_entries(&mut archive)?;
    if entries.is_empty() {
        return Err("zip 内未找到 webm 文件".to_string());
    }

    // 2. 校验：每个 webm 可解析；同名（stem 冲突）保留先出现者
    let mut warnings = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ok_entries: Vec<(usize, String)> = Vec::new();
    let mut ok_names: Vec<String> = Vec::new();
    for (idx, stem) in &entries {
        if !seen.insert(stem.clone()) {
            warnings.push(format!("{}: 文件名重复（忽略后出现者）", stem));
            continue;
        }
        match read_entry(&mut archive, *idx) {
            Ok(data) => match WebM::parse(&data) {
                Some(_) => {
                    ok_entries.push((*idx, stem.clone()));
                    ok_names.push(stem.clone());
                }
                None => warnings.push(format!("{}: 不是有效的 webm", stem)),
            },
            Err(e) => warnings.push(format!("{}: 读取失败 ({})", stem, e)),
        }
    }
    if ok_entries.is_empty() {
        return Err(format!("全部 {} 个视频校验失败", entries.len()));
    }

    // 3. 生成素材集 id（时间戳，避免与旧素材冲突）
    let id = new_pet_id();

    // 4. 可选 manifest：宠物名 / 待机基准 / 动作归属
    let manifest = read_manifest(&mut archive);
    let (manifest_actions, manifest_states, manifest_idle) = match &manifest {
        Some(m) => build_from_manifest(m, &ok_names),
        None => (Vec::new(), Vec::new(), None),
    };
    if manifest.is_some() {
        warnings.push("读取到 manifest，按清单配置动作归属".to_string());
    }
    let pet_name = manifest.and_then(|m| m.pet.and_then(|p| p.name)).filter(|s| !s.is_empty());

    // 5. 平铺解压 webm → <素材根>/<id>/（临时目录 + 原子替换）
    extract_webm_to(assets_root, &id, &mut archive, &ok_entries, &mut warnings)?;

    Ok(ImportReport {
        display_name: id.clone(),
        id,
        video_count: ok_entries.len(),
        videos: ok_names,
        warnings,
        pet_name,
        idle: manifest_idle,
        actions: manifest_actions,
        action_states: manifest_states,
    })
}

/// 读取 zip 内的 manifest.yaml / manifest.yml（返回 None 表示无）。
fn read_manifest<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Option<PetManifest> {
    for i in 0..archive.len() {
        let entry = archive.by_index(i).ok()?;
        if entry.is_dir() {
            continue;
        }
        let name = entry_name(&entry);
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("manifest.yaml") || lower.ends_with("manifest.yml") {
            let mut buf = Vec::new();
            // 重新拿可读 entry（by_index 已 move）
            drop(entry);
            if let Ok(mut e) = archive.by_index(i) {
                if e.read_to_end(&mut buf).is_ok() {
                    return serde_yaml::from_slice::<PetManifest>(&buf).ok();
                }
            }
            return None;
        }
    }
    None
}

// ---------------- 工具 ----------------

/// 递归收集全部 `*.webm` 条目 → (条目索引, 文件名 stem)。忽略目录与大小写扩展名。
fn collect_webm_entries<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Vec<(usize, String)>, String> {
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 条目读取失败: {}", e))?;
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

fn read_entry<R: Read + Seek>(archive: &mut ZipArchive<R>, idx: usize) -> Result<Vec<u8>, String> {
    let mut entry = archive
        .by_index(idx)
        .map_err(|e| format!("zip 条目读取失败: {}", e))?;
    let mut buf = Vec::new();
    entry
        .read_to_end(&mut buf)
        .map_err(|e| format!("zip 条目读取失败: {}", e))?;
    Ok(buf)
}

/// 只解压校验通过的 webm 到 `<素材根>/.<id>.tmp`，成功后原子替换 `<素材根>/<id>`。
/// 平铺存放（文件名 = stem.webm，动画名即 stem）。任一步失败清理临时目录。
fn extract_webm_to<R: Read + Seek>(
    assets_root: &std::path::Path,
    id: &str,
    archive: &mut ZipArchive<R>,
    entries: &[(usize, String)],
    warnings: &mut Vec<String>,
) -> Result<(), String> {
    let target = assets_root.join(id);
    let tmp = assets_root.join(format!(".{}.tmp", id));
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = std::fs::create_dir_all(&tmp) {
        return Err(format!("创建临时目录失败: {}", e));
    }

    let fill = (|| -> Result<(), String> {
        for (idx, stem) in entries {
            let dest = tmp.join(format!("{}.webm", stem));
            if let Err(e) = read_entry(archive, *idx).and_then(|data| {
                std::fs::write(&dest, data).map_err(|e| format!("写入失败 {}: {}", dest.display(), e))
            }) {
                warnings.push(format!("{}: {}", stem, e));
            }
        }
        Ok(())
    })();
    if let Err(e) = fill {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    let _ = std::fs::remove_dir_all(&target);
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("落位失败 {}: {}", target.display(), e));
    }
    log_info!("素材导入完成: {} ({} 段动画) -> {}", id, entries.len(), target.display());
    Ok(())
}

/// 素材集 id：`pet_<unix 毫秒>`（净化；时间戳保证唯一）。
fn new_pet_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
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
    fn manifest_parses_and_builds_actions() {
        let yaml = r#"
pet:
  name: 蓝发女仆
idle: 待机呼吸休闲
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
        let m: PetManifest = serde_yaml::from_str(yaml).expect("manifest 应可解析");
        assert_eq!(m.pet.as_ref().and_then(|p| p.name.as_deref()), Some("蓝发女仆"));
        assert_eq!(m.idle.as_deref(), Some("待机呼吸休闲"));

        let valid = vec!["待机呼吸休闲".to_string(), "开心蹦跳".to_string(), "挥手打招呼".to_string(), "未声明".to_string()];
        let (actions, states, idle) = build_from_manifest(&m, &valid);

        assert_eq!(idle.as_deref(), Some("待机呼吸休闲"));
        // 交互类
        let click = actions.iter().find(|a| a.action == "挥手打招呼").unwrap();
        assert_eq!(click.owner_kind, "interactive");
        assert_eq!(click.kind.as_deref(), Some("click"));
        // 状态类
        let state_row = actions.iter().find(|a| a.action == "开心蹦跳").unwrap();
        assert_eq!(state_row.owner_kind, "state");
        assert!(states.iter().any(|(a, s, w, _)| a == "开心蹦跳" && s == "active" && *w == 0.8));
        // 未声明 → 默认空闲池
        assert!(states.iter().any(|(a, s, _, _)| a == "未声明" && s == "idle"));
        // manifest 未覆盖的 待机呼吸休闲 也有 idle 绑定（build_from_manifest 给它补了）
        assert!(states.iter().any(|(a, s, _, _)| a == "待机呼吸休闲" && s == "idle"));
    }
}
