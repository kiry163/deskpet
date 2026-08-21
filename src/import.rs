//! 素材 zip 导入（见 docs/需求规格.md §4）：**仅扫描 webm**，无 manifest / videos 结构要求。
//! 流程：校验（zip 可读、每个 webm 可解析）→ 平铺解压全部 webm 到 `<素材根>/<id>/`
//! （动画名 = 文件名 stem）→ 返回报告（动画清单供注册默认动作配置）。
//! 校验失败不落盘任何内容，解压走临时目录 + 原子替换，避免污染素材根。

use std::io::{Cursor, Read, Seek};

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

    // 4. 平铺解压 webm → <素材根>/<id>/（临时目录 + 原子替换）
    extract_webm_to(assets_root, &id, &mut archive, &ok_entries, &mut warnings)?;

    Ok(ImportReport {
        display_name: id.clone(),
        id,
        video_count: ok_entries.len(),
        videos: ok_names,
        warnings,
    })
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
