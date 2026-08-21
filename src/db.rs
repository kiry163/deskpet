//! SQLite 数据层：素材集（桌宠）注册、动作配置、程序级设置、导入记录。
//!
//! - 数据库为独立 `.db` 文件（不内嵌二进制），首次运行无库时自动创建并建表；
//! - schema 用 `PRAGMA user_version` 做版本化迁移（每版结构变更 +1，追加迁移段）；
//! - 主线程独占访问（控制服务 API 经 mpsc 转主线程执行，不跨线程共享连接）。
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

/// 当前 schema 版本。
pub const SCHEMA_VERSION: i32 = 1;

/// 触发类型（与 state.rs 行为模型对齐；无 manifest 后全部由管理端配置）。
pub const TRIGGERS: [&str; 6] = ["click", "drag", "idle", "turn", "move", "idle_act"];

/// 桌宠（素材集）注册行。
#[derive(Clone, Debug)]
pub struct PetRow {
    pub id: String,
    pub display_name: String,
    pub source: String,
    pub imported_at: i64,
    pub builtin: bool,
}

/// 动作配置行。
#[derive(Clone, Debug)]
pub struct ActionRow {
    pub action: String,
    pub trigger: String,
    pub weight: f64,
    pub enabled: bool,
}

pub struct Db {
    pub conn: Connection,
    pub path: PathBuf,
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS pets (
  id          TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  source      TEXT NOT NULL,
  imported_at INTEGER NOT NULL,
  builtin     INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS pet_actions (
  pet_id  TEXT NOT NULL REFERENCES pets(id) ON DELETE CASCADE,
  action  TEXT NOT NULL,
  trigger TEXT NOT NULL,
  weight  REAL NOT NULL DEFAULT 1.0,
  enabled INTEGER NOT NULL DEFAULT 1,
  PRIMARY KEY (pet_id, action)
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
    /// 打开（不存在则创建）数据库并迁移到最新 schema。
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
        log_info!("数据库就绪: {} (schema v{})", path.display(), SCHEMA_VERSION);
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
        let ver: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(db_err)?;
        if ver < 1 {
            self.conn.execute_batch(SCHEMA_V1).map_err(db_err)?;
            self.conn.pragma_update(None, "user_version", 1).map_err(db_err)?;
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
            .prepare("SELECT action, trigger, weight, enabled FROM pet_actions WHERE pet_id = ?1")
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![pet_id], |r| {
                Ok(ActionRow {
                    action: r.get(0)?,
                    trigger: r.get(1)?,
                    weight: r.get(2)?,
                    enabled: r.get::<_, i64>(3)? != 0,
                })
            })
            .map_err(db_err)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// action → (trigger, weight, enabled) 映射（分类构建用）。
    pub fn actions_map(&self, pet_id: &str) -> Result<HashMap<String, (String, f64, bool)>, String> {
        let mut map = HashMap::new();
        for a in self.list_actions(pet_id)? {
            map.insert(a.action, (a.trigger, a.weight, a.enabled));
        }
        Ok(map)
    }

    /// 批量替换动作配置（导入/整表重置用）：事务内先删后插。
    pub fn replace_actions(
        &mut self,
        pet_id: &str,
        actions: &[(String, String, f64, bool)],
    ) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM pet_actions WHERE pet_id = ?1", params![pet_id]).map_err(db_err)?;
        for (action, trigger, weight, enabled) in actions {
            tx.execute(
                "INSERT OR REPLACE INTO pet_actions (pet_id, action, trigger, weight, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![pet_id, action, trigger, weight, *enabled as i64],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(())
    }

    /// 单条更新（管理端逐条保存）。
    pub fn upsert_action(
        &self,
        pet_id: &str,
        action: &str,
        trigger: &str,
        weight: f64,
        enabled: bool,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO pet_actions (pet_id, action, trigger, weight, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![pet_id, action, trigger, weight, enabled as i64],
            )
            .map_err(db_err)?;
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
}

fn now_secs() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
