//! allow 规则持久化（task-16 S5「总是允许」）：写回 settings.json
//! [permissions].allow（读改写保其他键、去重幂等、600 原子写）。
//!
//! 设计稿 §五注记兑现：v1=user 源整段 JSON 读改写（**只动 permissions.allow
//! 数组，其余键原样保留**——serde_json Value 层操作）；注释丢弃声明照批注
//! （JSON 无注释，lossless 不涉）；分源合并候配置分源卡（TODO 挂此处）。

use crate::rules::RuleSet;
use std::path::PathBuf;
use yunyou_agent_loop::PermissionPersistence;
use yunyou_util::BoxFut;

/// settings 持久化（permission config settings_path 同源；None=内存空集降级）。
pub struct SettingsPersistence {
    path: Option<PathBuf>,
}

impl SettingsPersistence {
    /// 构造（路径与加载面同源：config settings_path > ~/.yunyou/settings.json）。
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// 追加 allow 规则（读改写；规则形态校验复用 parse_rule——坏规则拒入）。
    pub fn append_allow(&self, rule: &str) -> Result<(), String> {
        let Some(path) = &self.path else {
            return Err("settings 路径不可得（无 HOME 投影）：无法持久化 allow 规则".into());
        };
        // 读现有文件（缺=空对象起步）
        let mut value: serde_json::Value = if path.is_file() {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
            serde_json::from_str(&text)
                .map_err(|e| format!("解析 {} 失败：{e}", path.display()))?
        } else {
            serde_json::json!({})
        };
        // 规则形态校验（parse_rule 单源；拒坏规则入 settings）
        crate::rules::parse_rule(rule, crate::rules::Behavior::Allow)
            .map_err(|e| format!("规则形态不合法：{e}"))?;
        let obj = value
            .as_object_mut()
            .ok_or_else(|| format!("{} 顶层不是对象", path.display()))?;
        let permissions = obj
            .entry("permissions")
            .or_insert_with(|| serde_json::json!({}));
        let perms = permissions
            .as_object_mut()
            .ok_or("[permissions] 段不是对象（settings 结构被手改）")?;
        let allow = perms
            .entry("allow")
            .or_insert_with(|| serde_json::json!([]));
        let list = allow
            .as_array_mut()
            .ok_or("[permissions].allow 不是数组（settings 结构被手改）")?;
        // 去重幂等：已存在（字面相等）不重复写
        if !list.iter().any(|v| v.as_str() == Some(rule)) {
            list.push(serde_json::Value::String(rule.to_string()));
        }
        // 写回（600+临时文件原子 rename，对齐 authorization.rs 惯例）
        let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &text).map_err(|e| format!("写 {} 失败：{e}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, path).map_err(|e| format!("落位 {} 失败：{e}", path.display()))
    }

    /// 重载规则集（写回后 Gate 内存态刷新入口；v1 gate 规则静态——重载候
    /// [permission] 热更卡，此方法占位留缝）。
    pub fn reload_rules(&self) -> Result<RuleSet, String> {
        match &self.path {
            Some(path) => RuleSet::load_user_settings(path),
            None => Ok(RuleSet::default()),
        }
    }
}

impl PermissionPersistence for SettingsPersistence {
    fn persist_allow_rule<'a>(&'a self, rule: &'a str) -> BoxFut<'a, Result<(), String>> {
        Box::pin(async move { self.append_allow(rule) })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // 测试断言惯例（同包既有测试同律）
mod tests {
    use super::*;

    /// 写回三态：新建文件/追加去重幂等/坏规则拒入；其他键保留。
    #[test]
    fn allow写回三态与键保留() {
        let dir = std::env::temp_dir().join(format!("perm-s5-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let p = SettingsPersistence::new(Some(path.clone()));
        // 新建：带其他键起步文件——写回后键保留
        std::fs::write(&path, r#"{"llm":{"model":"m"},"permissions":{"allow":["bash"],"deny":["Bash(rm -rf:*)"]}}"#).unwrap();
        p.append_allow("Bash(cargo:*)").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"llm\""), "其他键应保留：{text}");
        assert!(text.contains("Bash(cargo:*)"));
        // 去重幂等
        p.append_allow("Bash(cargo:*)").unwrap();
        let set = RuleSet::load_user_settings(&path).unwrap();
        // 文件面 2 条 allow（bash+cargo）+compile 并入的标准 allow 预设
        // （STANDARD_ALLOW_PRESET=["web_search"]，79a38c95 批）=3
        assert_eq!(
            set.rules.iter().filter(|r| r.behavior == crate::rules::Behavior::Allow).count(),
            3,
            "文件 2 条+标准预设 1 条"
        );
        // 坏规则拒入
        assert!(p.append_allow("Bash(").is_err());
        // 600 权限
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
