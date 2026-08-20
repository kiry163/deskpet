//! 素材 zip 导入：校验 → 解压落位（见 docs/需求规格.md §3）。
//!
//! 约定：**zip 根即角色包**（`manifest.json` + `videos/` 在根目录），解压到
//! `<素材根>/<manifest.id>/`；校验失败不落盘任何内容，解压走临时目录 + 原子替换，
//! 避免污染素材根。

use std::collections::HashSet;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use serde_json::Value;
use zip::ZipArchive;

/// 导入结果报告。
pub struct ImportReport {
    pub id: String,
    pub display_name: String,
    pub video_count: usize,
    pub warnings: Vec<String>,
}

/// 校验 zip 并解压到素材根。`Err` 表示失败（未落盘任何内容）。
pub fn import_zip(zip_bytes: &[u8], assets_root: &Path) -> Result<ImportReport, String> {
    let mut archive =
        ZipArchive::new(Cursor::new(zip_bytes)).map_err(|e| format!("zip 解析失败: {}", e))?;

    // 1. manifest.json（zip 根）
    let manifest = read_manifest(&mut archive)?;
    let id = manifest
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "manifest.json 缺少 id 字段".to_string())?;
    let id = sanitize_id(id)?;
    let display_name = manifest
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| id.clone());

    // 2. 收集根目录 videos/*.webm（含动作子目录）
    let videos = collect_videos(&mut archive)?;
    if videos.is_empty() {
        return Err("zip 内未找到 videos/*.webm".to_string());
    }

    // 3. 校验：每个 webm 可解析 + 可创建解码器；manifest 动作缺文件 → 兜底提示
    let mut warnings = Vec::new();
    let mut ok = 0;
    for (idx, name) in &videos {
        match read_entry(&mut archive, *idx) {
            Ok(data) => match crate::webm::WebM::parse(&data) {
                Some(wm) => {
                    if crate::clip::ClipDecoder::new(std::rc::Rc::new(wm)).is_some() {
                        ok += 1;
                    } else {
                        warnings.push(format!("{}: 无法创建解码器", name));
                    }
                }
                None => warnings.push(format!("{}: 不是有效的 webm", name)),
            },
            Err(e) => warnings.push(format!("{}: 读取失败 ({})", name, e)),
        }
    }
    if ok == 0 {
        return Err(format!("全部 {} 个视频校验失败", videos.len()));
    }
    check_manifest_actions(&manifest, &videos, &mut warnings);

    // 4. 解压落位：临时目录 → 原子替换 <素材根>/<id>/
    extract_to(assets_root, &id, &mut archive)?;

    Ok(ImportReport { id, display_name, video_count: ok, warnings })
}

// ---------------- manifest ----------------

/// 读取 zip 根的 manifest.json。
fn read_manifest<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Value, String> {
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 条目读取失败: {}", e))?;
        let name = entry_name(&entry);
        if name == "manifest.json" {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .map_err(|e| format!("manifest.json 读取失败: {}", e))?;
            return serde_json::from_str(&text)
                .map_err(|e| format!("manifest.json 解析失败: {}", e));
        }
    }
    Err("zip 根目录缺少 manifest.json".to_string())
}

/// manifest 的 actions 引用的动作名 → 检查 zip 内是否有对应视频（缺则走分类兜底）。
fn check_manifest_actions(manifest: &Value, videos: &[(usize, String)], warnings: &mut Vec<String>) {
    let Some(actions) = manifest.get("actions").and_then(|v| v.as_object()) else {
        return;
    };
    let have: HashSet<&str> = videos.iter().map(|(_, n)| n.as_str()).collect();
    for (cat, v) in actions {
        let names: Vec<&str> = match v {
            Value::String(s) => vec![s.as_str()],
            Value::Array(arr) => arr.iter().filter_map(|x| x.as_str()).collect(),
            _ => Vec::new(),
        };
        for n in names {
            if !have.contains(n) {
                warnings.push(format!("动作 [{}/{}] 缺少对应视频，将走分类兜底", cat, n));
            }
        }
    }
}

// ---------------- zip 工具 ----------------

/// 收集 videos/*.webm 条目 → (条目索引, 文件名 stem)。
fn collect_videos<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Vec<(usize, String)>, String> {
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|e| format!("zip 条目读取失败: {}", e))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry_name(&entry);
        if let Some(rest) = name.strip_prefix("videos/") {
            if rest.ends_with(".webm") {
                let stem = rest.rsplit('/').next().unwrap_or(rest);
                let stem = stem.trim_end_matches(".webm");
                if !stem.is_empty() {
                    out.push((i, stem.to_string()));
                }
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

/// 解压全部条目到 `<素材根>/.<id>.tmp`，成功后原子替换 `<素材根>/<id>`。
/// 任一步失败都会清理临时目录，不污染素材根。
fn extract_to<R: Read + Seek>(
    assets_root: &Path,
    id: &str,
    archive: &mut ZipArchive<R>,
) -> Result<(), String> {
    let target = assets_root.join(id);
    let tmp = assets_root.join(format!(".{}.tmp", id));
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = std::fs::create_dir_all(&tmp) {
        return Err(format!("创建临时目录失败: {}", e));
    }

    let fill = (|| -> Result<(), String> {
        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("zip 条目读取失败: {}", e))?;
            if entry.is_dir() {
                continue;
            }
            let rel = sanitize_entry_path(&entry_name(&entry))?;
            let dest = tmp.join(rel);
            if let Some(parent) = dest.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return Err(format!("创建目录失败 {}: {}", parent.display(), e));
                }
            }
            let mut out = std::fs::File::create(&dest)
                .map_err(|e| format!("写入失败 {}: {}", dest.display(), e))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("写入失败 {}: {}", dest.display(), e))?;
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
    log_info!("素材导入完成: {} -> {}", id, target.display());
    Ok(())
}

// ---------------- 安全 ----------------

/// id 净化：仅保留字母数字与 _-，其余替换为 _；结果为空则报错。
fn sanitize_id(id: &str) -> Result<String, String> {
    let clean: String = id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' })
        .collect();
    if clean.is_empty() {
        return Err("manifest id 无效（净化后为空）".to_string());
    }
    Ok(clean)
}

/// zip 条目路径安全：拒绝穿越（..）与盘符（:），统一 / 分隔。
fn sanitize_entry_path(raw: &str) -> Result<PathBuf, String> {
    let normalized = raw.replace('\\', "/");
    let normalized = normalized.trim_start_matches('/');
    if normalized.is_empty() {
        return Err(format!("zip 条目路径为空: {}", raw));
    }
    for seg in normalized.split('/') {
        if seg == ".." || seg.contains(':') {
            return Err(format!("zip 条目路径不安全: {}", raw));
        }
    }
    Ok(PathBuf::from(normalized))
}

/// zip 条目名归一化：优先按原始字节做 UTF-8 解码（兼容未设 UTF-8 标志但名称为
/// UTF-8 的 zip），失败则用 crate 的解码（CP437）；统一 / 分隔、去掉开头的 ./
fn entry_name(entry: &zip::read::ZipFile) -> String {
    let raw = entry.name_raw();
    let s = std::str::from_utf8(raw).unwrap_or_else(|_| entry.name());
    s.replace('\\', "/").trim_start_matches("./").to_string()
}
