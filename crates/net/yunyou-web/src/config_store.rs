//! 配置修订写面（W5 批，yunyou-web 内自管——lead 2026-11-19 裁决②，零跨界）：
//! revision 乐观锁 + 原子写回 yunyou.toml + 写前深度校验。
//!
//! # 语义（契约 规格.md §3）
//! - revision 单源=本进程计数器（从 1 起，单调递增；**不承诺跨重启**——重启后
//!   重置，前端以 system/restarted 事件作废缓存）；
//! - 写流程：读文件文本 → toml 改段 → **load_from_str 深度校验**（deny_unknown_fields
//!   拼写防呆，失败=原文件不动）→ 临时文件+rename 原子落盘（600 权限，参考
//!   yunyou-credentials write_secret_file 先例）→ revision+1；
//! - 并发：std 锁串行（配置写=低频管理面，锁内含文件 IO 同 credentials store 律）。

use yunyou_config::{load_from_str, DshConfig};
use std::path::PathBuf;
use std::sync::Mutex;

/// 修订写状态（进程级单源）。
pub(crate) struct ConfigStore {
    /// 配置文件路径（yunyou.toml）。
    path: PathBuf,
    inner: Mutex<u64>,
}

impl ConfigStore {
    /// 构造（revision 从 1 起；path=配置文件落点）。
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path, inner: Mutex::new(1) }
    }

    /// 当前 revision（sections/form 投影逐段携带）。
    pub(crate) fn revision(&self) -> u64 {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    /// 配置文件路径（credentials/list 的 store 探测与 form 取值用）。
    pub(crate) fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// 修订写校验：revision 相等才放行（不等=Err 现值——响应 config/revision-conflict）。
    pub(crate) fn check_revision(&self, presented: u64) -> Result<(), u64> {
        let current = self.revision();
        if current == presented {
            Ok(())
        } else {
            Err(current)
        }
    }

    /// 段写入：改段→深度校验→原子落盘→revision+1。返回新 revision。
    ///
    /// # 校验语义（deny_unknown_fields 拼写防呆）
    /// 段值合并进现文档后必须能整体反序列化为 DshConfig；失败=原文件不动，
    /// Err(非法键路径清单)——details.keys（契约 config/validation）。
    pub(crate) fn write_section(&self, section: &str, value: &serde_json::Value) -> Result<u64, Vec<String>> {
        let original = std::fs::read_to_string(&self.path).unwrap_or_default();
        let mut doc: toml::Value = toml::from_str(&original)
            .map_err(|_| vec![format!("<文档级>（现有 yunyou.toml 解析失败，拒绝盲写）")])?;
        let section_value = toml::Value::try_from(value)
            .map_err(|_| vec![format!("{section}（段值不可序列化为 toml）")])?;
        doc[section.to_lowercase().as_str()] = section_value;
        let text = toml::to_string_pretty(&doc)
            .map_err(|_| vec![format!("{section}（文档序列化失败）")])?;
        // 深度校验：deny_unknown_fields 全集（失败=原文件不动，Err 非法键路径）
        if let Err(err) = load_from_str(&text) {
            return Err(validation_keys(&err));
        }
        atomic_write(&self.path, &text)
            .map_err(|err| vec![format!("{section}（落盘失败：{err}）)")])?;
        let mut revision = self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        *revision += 1;
        Ok(*revision)
    }
}

/// 校验错误 → 非法键路径清单（聚合格式「invalid config:」逐行剥离取键）。
fn validation_keys(err: &yunyou_config::ConfigError) -> Vec<String> {
    match err {
        yunyou_config::ConfigError::Validation(_) => {
            // ValidationError 展示格式=多行「invalid config:」+键路径；解析取行
            err.to_string()
                .lines()
                .filter_map(|line| {
                    let line = line.trim();
                    line.strip_prefix("- ").map(str::to_string)
                })
                .collect()
        }
        other => vec![other.to_string()],
    }
}

/// 原子写（600 权限：临时文件+rename；参考 yunyou-credentials write_secret_file 先例）。
fn atomic_write(path: &std::path::Path, text: &str) -> Result<(), std::io::Error> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(text.as_bytes())?;
        file.flush()?;
    }
    std::fs::rename(&tmp, path)
}

/// 段 schema（sections/form 投影源）：schemars 全档 derive，逐段取 properties。
pub(crate) fn section_schema(section: &str) -> Option<serde_json::Value> {
    let schema = serde_json::to_value(schemars::schema_for!(DshConfig)).ok()?;
    let section_schema = schema["properties"][section.to_lowercase().as_str()].clone();
    (section_schema.is_object()).then_some(section_schema)
}

/// 已知段清单（契约 §1.1：顶层九段+plugins 容器段，DshConfig 实测冻结）。
pub(crate) const KNOWN_SECTIONS: &[&str] = &[
    "llm", "session", "system_prompt", "tools", "workspace", "agent_loop", "retry",
    "credentials", "sandbox", "plugins",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 测试用临时配置文件（预置 llm 段）。
    fn store_with(content: &str) -> (ConfigStore, PathBuf) {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("yunyou-web-cfg-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("yunyou.toml");
        std::fs::write(&path, content).expect("预置配置");
        (ConfigStore::new(path.clone()), path)
    }

    /// revision 乐观锁：初值 1、写成功 +1、错版本 Err 现值。
    #[test]
    fn revision锁与递增() {
        let (store, path) = store_with("[llm]\ntimeout_secs = 30\n");
        assert_eq!(store.revision(), 1);
        store.check_revision(1).expect("同版本放行");
        assert_eq!(store.check_revision(2).unwrap_err(), 1, "错版本=Err 现值");
        let next = store.write_section("llm", &json!({"timeout_secs": 60})).expect("写成功");
        assert_eq!(next, 2);
        assert_eq!(store.revision(), 2);
        let text = std::fs::read_to_string(&path).expect("读回");
        assert!(text.contains("timeout_secs = 60"), "{text}");
    }

    /// 深度校验：未知键拒绝（deny_unknown_fields，原文件不动）+ 已知键改写落盘。
    #[test]
    fn 深度校验拒绝未知键() {
        let (store, path) = store_with("[llm]\ntimeout_secs = 30\n");
        let original = std::fs::read_to_string(&path).unwrap();
        let err = store.write_section("llm", &json!({"no_such_key": 1})).unwrap_err();
        assert_eq!(err, vec!["llm.no_such_key".to_string()], "非法键路径：{err:?}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original, "原文件不动");
    }

    /// 原子写权限（unix）：落盘文件 600。
    #[cfg(unix)]
    #[test]
    fn 原子写600权限() {
        let (store, path) = store_with("[llm]\ntimeout_secs = 30\n");
        store.write_section("session", &json!({"path": "sessions/default.jsonl"})).expect("写成功");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "落盘 600：{mode:o}");
    }

    /// 段 schema 与已知段清单：九段+plugins 可投影；未知段 None。
    #[test]
    fn 段schema与清单() {
        for section in KNOWN_SECTIONS {
            assert!(section_schema(section).is_some(), "{section} 应有 schema");
        }
        assert!(section_schema("no_such").is_none());
    }
}
