//! SQLite 数据层：素材集（桌宠）注册、动作配置、程序级设置、导入记录。
//!
//! - 数据库为独立 `.db` 文件（不内嵌二进制），首次运行无库时自动创建并建表；
//! - schema 用 `PRAGMA user_version` 做版本化迁移（每版结构变更 +1，追加迁移段）；
//! - 主线程独占访问（控制服务 API 经 mpsc 转主线程执行，不跨线程共享连接）。
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

/// 桌宠（素材集）注册行。
#[derive(Clone, Debug)]
pub struct PetRow {
    pub id: String,
    pub display_name: String,
    pub source: String,
    pub imported_at: i64,
    pub builtin: bool,
}

/// 动作配置行（最终模型：交互 or 状态池）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ActionRow {
    pub action: String,       // key = webm 文件名 stem
    pub display_name: String, // 显示名（用户可编辑）
    pub owner_kind: String,   // 'interactive' | 'state'
    pub kind: Option<String>, // interactive → 'click'/'drag'；state → None
    pub enabled: bool,
}

/// 宠物锚点（跨动画共享归一化基准）：`scale = TARGET_H / h_ref`。
#[derive(Clone, Debug)]
pub struct PetAnchor {
    pub pet_id: String,
    pub scale: f64,
    pub h_ref: f64,
    pub source_w: i64,
    pub source_h: i64,
}

pub struct Db {
    pub conn: Connection,
    pub path: PathBuf,
}

/// 建表 schema（最终模型）。无历史用户/无数据迁移，直接一次性建表；下次启动幂等。
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS pets (
  id               TEXT PRIMARY KEY,
  display_name     TEXT NOT NULL,
  idle_action      TEXT,
  full_body_image  TEXT,
  source           TEXT NOT NULL,
  imported_at      INTEGER NOT NULL,
  builtin          INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS pet_actions (
  pet_id       TEXT NOT NULL REFERENCES pets(id) ON DELETE CASCADE,
  action       TEXT NOT NULL,
  display_name TEXT NOT NULL,
  owner_kind   TEXT NOT NULL,
  kind         TEXT,
  enabled      INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (pet_id, action)
);
CREATE TABLE IF NOT EXISTS action_states (
  pet_id   TEXT NOT NULL REFERENCES pets(id) ON DELETE CASCADE,
  action   TEXT NOT NULL,
  state_id TEXT NOT NULL,
  weight   REAL NOT NULL DEFAULT 1.0,
  enabled  INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (pet_id, action, state_id)
);
CREATE TABLE IF NOT EXISTS convert_jobs (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  pet_id      TEXT NOT NULL,
  src_path    TEXT NOT NULL,
  status      TEXT NOT NULL,
  progress    REAL NOT NULL DEFAULT 0,
  error       TEXT,
  created_at  INTEGER NOT NULL,
  finished_at INTEGER
);
CREATE TABLE IF NOT EXISTS pet_anchors (
  pet_id    TEXT PRIMARY KEY REFERENCES pets(id) ON DELETE CASCADE,
  scale     REAL NOT NULL,
  h_ref     REAL NOT NULL,
  source_w  INTEGER NOT NULL,
  source_h  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS pet_import_jobs (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  pet_id         TEXT NOT NULL,
  pet_name       TEXT,
  total          INTEGER NOT NULL,
  done           INTEGER NOT NULL DEFAULT 0,
  failed         INTEGER NOT NULL DEFAULT 0,
  status         TEXT NOT NULL,
  current_action TEXT,
  error          TEXT,
  created_at     INTEGER NOT NULL,
  finished_at    INTEGER
);
CREATE TABLE IF NOT EXISTS app_settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS import_log (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  pet_id     TEXT,
  file_name  TEXT NOT NULL,
  result     TEXT NOT NULL,
  detail     TEXT,
  created_at INTEGER NOT NULL
);
"#;

fn db_err(e: rusqlite::Error) -> String {
    format!("数据库操作失败: {}", e)
}

impl Db {
    /// 打开（不存在则创建）数据库并建表（幂等）。
    pub fn open(path: &Path) -> Result<Db, String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建数据库目录失败 {}: {}", parent.display(), e))?;
            }
        }
        let conn = Connection::open(path).map_err(|e| format!("打开数据库失败 {}: {}", path.display(), e))?;
        let db = Db { conn, path: path.to_path_buf() };
        db.migrate()?;
        log_info!("数据库就绪: {}", path.display());
        Ok(db)
    }

    /// 打开内存库（数据库文件不可用时兜底，保证桌宠可用；配置不持久）。
    pub fn open_in_memory() -> Db {
        let conn = Connection::open_in_memory().expect("内存数据库打开失败");
        let db = Db { conn, path: PathBuf::from(":memory:") };
        if let Err(e) = db.migrate() {
            log_error!("内存库建表失败: {}", e);
        }
        db
    }

    fn migrate(&self) -> Result<(), String> {
        self.conn
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(db_err)?;
        self.conn.execute_batch(SCHEMA).map_err(db_err)?;
        // 播种默认状态集合（无则写；用户已有修改时不覆盖）。
        let _ = self.seed_default_behavior();
        Ok(())
    }

    /// 若 app_settings 无 `behavior`（状态集合）则写入默认值（behavior::default_states）。
    fn seed_default_behavior(&self) -> Result<(), String> {
        if self.get_setting("behavior")?.is_none() {
            let v = serde_json::to_string(&crate::behavior::default_states()).unwrap_or_default();
            self.set_setting("behavior", &v)?;
            log_info!("已播种默认状态集合 (behavior)");
        }
        Ok(())
    }

    // ---------------- pets ----------------

    pub fn insert_pet(&self, id: &str, display_name: &str, source: &str) -> Result<(), String> {
        let now = now_secs();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO pets (id, display_name, source, imported_at) VALUES (?1, ?2, ?3, ?4)",
                params![id, display_name, source, now],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn list_pets(&self) -> Result<Vec<PetRow>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, display_name, source, imported_at, builtin FROM pets ORDER BY imported_at")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok(PetRow {
                    id: r.get(0)?,
                    display_name: r.get(1)?,
                    source: r.get(2)?,
                    imported_at: r.get(3)?,
                    builtin: r.get::<_, i64>(4)? != 0,
                })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    pub fn get_pet(&self, id: &str) -> Result<Option<PetRow>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, display_name, source, imported_at, builtin FROM pets WHERE id = ?1")
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![id], |r| {
                Ok(PetRow {
                    id: r.get(0)?,
                    display_name: r.get(1)?,
                    source: r.get(2)?,
                    imported_at: r.get(3)?,
                    builtin: r.get::<_, i64>(4)? != 0,
                })
            })
            .map_err(db_err)?;
        rows.next().transpose().map_err(db_err)
    }

    pub fn update_display_name(&self, id: &str, display_name: &str) -> Result<(), String> {
        self.conn
            .execute("UPDATE pets SET display_name = ?1 WHERE id = ?2", params![display_name, id])
            .map_err(db_err)?;
        Ok(())
    }

    /// 删除桌宠注册（pet_actions 级联删除；素材文件由调用方决定是否删除）。
    pub fn delete_pet(&self, id: &str) -> Result<(), String> {
        self.conn.execute("DELETE FROM pets WHERE id = ?1", params![id]).map_err(db_err)?;
        Ok(())
    }

    // ---------------- pet_actions ----------------

    pub fn list_actions(&self, pet_id: &str) -> Result<Vec<ActionRow>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT action, display_name, owner_kind, kind, enabled FROM pet_actions WHERE pet_id = ?1")
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![pet_id], |r| {
                Ok(ActionRow {
                    action: r.get(0)?,
                    display_name: r.get(1)?,
                    owner_kind: r.get(2)?,
                    kind: r.get(3)?,
                    enabled: r.get::<_, i64>(4)? != 0,
                })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// 批量替换动作配置（导入/整表重置用）：事务内先删后插 pet_actions 与 action_states。
    pub fn replace_actions(
        &mut self,
        pet_id: &str,
        actions: &[ActionRow],
        action_states: &[(String, String, f64, bool)],
    ) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM pet_actions WHERE pet_id = ?1", params![pet_id]).map_err(db_err)?;
        tx.execute("DELETE FROM action_states WHERE pet_id = ?1", params![pet_id]).map_err(db_err)?;
        for a in actions {
            tx.execute(
                "INSERT OR REPLACE INTO pet_actions (pet_id, action, display_name, owner_kind, kind, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![pet_id, a.action, a.display_name, a.owner_kind, a.kind, a.enabled as i64],
            )
            .map_err(db_err)?;
        }
        for (action, state_id, weight, enabled) in action_states {
            tx.execute(
                "INSERT OR REPLACE INTO action_states (pet_id, action, state_id, weight, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![pet_id, action, state_id, weight, *enabled as i64],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(())
    }

    /// 单条更新（管理端逐条保存，含归属与状态绑定）。
    pub fn upsert_action(
        &self,
        pet_id: &str,
        row: &ActionRow,
        action_states: &[(String, f64, bool)],
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO pet_actions (pet_id, action, display_name, owner_kind, kind, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![pet_id, row.action, row.display_name, row.owner_kind, row.kind, row.enabled as i64],
            )
            .map_err(db_err)?;
        self.conn
            .execute("DELETE FROM action_states WHERE pet_id = ?1 AND action = ?2", params![pet_id, row.action])
            .map_err(db_err)?;
        for (state_id, weight, enabled) in action_states {
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO action_states (pet_id, action, state_id, weight, enabled)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![pet_id, row.action, state_id, weight, *enabled as i64],
                )
                .map_err(db_err)?;
        }
        Ok(())
    }

    // ---------------- app_settings ----------------

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row("SELECT value FROM app_settings WHERE key = ?1", params![key], |r| r.get(0))
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(db_err(other)),
            })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn all_settings(&self) -> Result<HashMap<String, String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT key, value FROM app_settings")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(db_err)?
            .collect::<Result<HashMap<_, _>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    // ---------------- import_log ----------------

    pub fn log_import(&self, pet_id: Option<&str>, file_name: &str, result: &str, detail: &str) {
        let _ = self.conn.execute(
            "INSERT INTO import_log (pet_id, file_name, result, detail, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![pet_id, file_name, result, detail, now_secs()],
        );
    }

    // ---------------- behavior：状态集合（程序级，app_settings["behavior"]） ----------------

    /// 读取状态集合（缺省用默认值）。用户编辑后以库中值为准。
    pub fn get_behavior_states(&self) -> Result<Vec<crate::behavior::StateDef>, String> {
        let raw = self.get_setting("behavior")?;
        match raw {
            Some(s) => serde_json::from_str::<Vec<crate::behavior::StateDef>>(&s)
                .map_err(|e| format!("解析状态集合失败: {}", e)),
            None => Ok(crate::behavior::default_states()),
        }
    }

    pub fn set_behavior_states(&self, states: &[crate::behavior::StateDef]) -> Result<(), String> {
        let v = serde_json::to_string(states).map_err(|e| format!("序列化状态集合失败: {}", e))?;
        self.set_setting("behavior", &v)
    }

    // ---------------- pets 基线（体型基准 + 全身照） ----------------

    /// 记录宠物体型基准（待机动作）与全身照（自动由待机取帧）。
    pub fn set_pet_baseline(&self, id: &str, idle_action: &str, full_body_image: Option<&str>) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE pets SET idle_action = ?1, full_body_image = ?2 WHERE id = ?3",
                params![idle_action, full_body_image, id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// 读取宠物基线记录。
    pub fn pet_baseline(&self, id: &str) -> Result<(Option<String>, Option<String>), String> {
        let mut stmt = self
            .conn
            .prepare("SELECT idle_action, full_body_image FROM pets WHERE id = ?1")
            .map_err(db_err)?;
        let mut rows = stmt.query_map(params![id], |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?))).map_err(db_err)?;
        match rows.next().transpose().map_err(db_err)? {
            Some(v) => Ok(v),
            None => Ok((None, None)),
        }
    }

    // ---------------- action_states（动作 → 状态多选，按状态各自权重） ----------------

    /// 列出该宠物全部「动作→状态」绑定。
    pub fn list_action_states(&self, pet_id: &str) -> Result<Vec<(String, String, f64, bool)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT action, state_id, weight, enabled FROM action_states WHERE pet_id = ?1")
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![pet_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, bool>(3)?))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// 替换某宠物的「动作→状态」绑定（事务内先删后插）。
    pub fn replace_action_states(&mut self, pet_id: &str, rows: &[(String, String, f64, bool)]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM action_states WHERE pet_id = ?1", params![pet_id]).map_err(db_err)?;
        for (action, state_id, weight, enabled) in rows {
            tx.execute(
                "INSERT OR REPLACE INTO action_states (pet_id, action, state_id, weight, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![pet_id, action, state_id, weight, *enabled as i64],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(())
    }

    // ---------------- convert_jobs（异步转换作业） ----------------

    pub fn insert_convert_job(&self, pet_id: &str, src_path: &str) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO convert_jobs (pet_id, src_path, status, progress, created_at) VALUES (?1, ?2, 'queued', 0, ?3)",
                params![pet_id, src_path, now_secs()],
            )
            .map_err(db_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_convert_job(&self, id: i64, status: &str, progress: f64, error: Option<&str>) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE convert_jobs SET status=?1, progress=?2, error=?3, finished_at=?4 WHERE id=?5",
                params![status, progress, error, if status == "done" || status == "error" { Some(now_secs()) } else { None }, id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn list_convert_jobs(&self, pet_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, src_path, status, progress, error, created_at FROM convert_jobs WHERE pet_id = ?1 ORDER BY id")
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![pet_id], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "src": r.get::<_, String>(1)?,
                    "status": r.get::<_, String>(2)?,
                    "progress": r.get::<_, f64>(3)?,
                    "error": r.get::<_, Option<String>>(4)?,
                    "created_at": r.get::<_, i64>(5)?,
                }))
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    // ---------------- pet_anchors（宠物级归一化锚点） ----------------

    pub fn set_pet_anchor(&self, a: &PetAnchor) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO pet_anchors (pet_id, scale, h_ref, source_w, source_h) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![a.pet_id, a.scale, a.h_ref, a.source_w, a.source_h],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn get_pet_anchor(&self, pet_id: &str) -> Result<Option<PetAnchor>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT pet_id, scale, h_ref, source_w, source_h FROM pet_anchors WHERE pet_id = ?1")
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![pet_id], |r| {
                Ok(PetAnchor {
                    pet_id: r.get(0)?,
                    scale: r.get(1)?,
                    h_ref: r.get(2)?,
                    source_w: r.get(3)?,
                    source_h: r.get(4)?,
                })
            })
            .map_err(db_err)?;
        rows.next().transpose().map_err(db_err)
    }

    // ---------------- pet_import_jobs（视频包 → 新建整只宠） ----------------

    pub fn insert_pet_import_job(&self, pet_id: &str, pet_name: &str, total: usize) -> Result<i64, String> {
        self.conn
            .execute(
                "INSERT INTO pet_import_jobs (pet_id, pet_name, total, status, created_at) VALUES (?1, ?2, ?3, 'running', ?4)",
                params![pet_id, pet_name, total as i64, now_secs()],
            )
            .map_err(db_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_pet_import_job(&self, id: i64, status: &str, done: usize, failed: usize, current_action: Option<&str>, error: Option<&str>) -> Result<(), String> {
        let finished = if status == "done" || status == "error" { Some(now_secs()) } else { None };
        self.conn
            .execute(
                "UPDATE pet_import_jobs SET status=?1, done=?2, failed=?3, current_action=?4, error=?5, finished_at=?6 WHERE id=?7",
                params![status, done as i64, failed as i64, current_action, error, finished, id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn get_pet_import_job(&self, id: i64) -> Result<serde_json::Value, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, pet_id, pet_name, total, done, failed, status, current_action, error, created_at FROM pet_import_jobs WHERE id = ?1")
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![id], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "pet_id": r.get::<_, String>(1)?,
                    "pet_name": r.get::<_, Option<String>>(2)?,
                    "total": r.get::<_, i64>(3)?,
                    "done": r.get::<_, i64>(4)?,
                    "failed": r.get::<_, i64>(5)?,
                    "status": r.get::<_, String>(6)?,
                    "current_action": r.get::<_, Option<String>>(7)?,
                    "error": r.get::<_, Option<String>>(8)?,
                    "created_at": r.get::<_, i64>(9)?,
                }))
            })
            .map_err(db_err)?;
        match rows.next().transpose().map_err(db_err)? {
            Some(v) => Ok(v),
            None => Err(format!("作业不存在: {}", id)),
        }
    }
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
