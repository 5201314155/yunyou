//! 装配决策纯函数组（插槽规格与段值面，方案 §2.4/§2.5）：
//! `host_slot_support`（宿主插槽支持集）/`enabled_of_with_file`（enable 判定·装载器形态）/
//! `check_slot_versions`（插槽版本比对）/`merge_segment`（段值合并）。
//! 从 bundled.rs 按职责拆出（清单=注册表域，决策=本文件域），行数红线内聚。

use crate::AppError;
use crate::bundled::{BundledEntry, BUNDLED_PLUGINS};
use yunyou::ConfigValue;
use yunyou_config::model::PluginEntry;
use yunyou_config::DshConfig;
use serde_json::Value;
use std::collections::BTreeMap;

/// 宿主插槽规格支持集（方案 §2.5「宿主装载前校验」的比对源）：
/// 注册表全部条目的 `contracts` 并集，name → 最大版本号 N。
/// bundled 同代内嵌契约（随二进制分发，禁用只影响启用态不影响宿主能力集）；
/// market 级二期才需要独立的宿主版本表。
pub(crate) fn host_slot_support() -> BTreeMap<String, u32> {
    let mut support: BTreeMap<String, u32> = BTreeMap::new();
    for entry in BUNDLED_PLUGINS {
        for spec in (entry.manifest)().contracts {
            // 格式合法性由 validate_manifest 负责（contracts 项 name.vN）；
            // 此处对合法项取 name 与版本号，畸形项跳过（校验层已报，不重复报）。
            if let Some((name, ver)) = spec.rsplit_once('.') {
                if let Some(n) = ver.strip_prefix('v').and_then(|d| d.parse::<u32>().ok()) {
                    let slot = support.entry(name.to_string()).or_insert(n);
                    if n > *slot {
                        *slot = n;
                    }
                }
            }
        }
    }
    support
}

/// enable 判定·装载器 L1 形态（K2）：`[plugins.<id>] enable` 显式值仍最高优先，
/// 其次 plugins/ 目录 manifest 文件的 `auto_enable`（落盘声明投影覆盖代码缺省，
/// 设计 §3），最后回落 bundled 代码内 `auto_enable`。
/// （原 `enabled_of` 两参形态已被本函数 `file_auto=None` 分支超集替代，删除。）
pub(crate) fn enabled_of_with_file(
    entry: &BundledEntry,
    config: &DshConfig,
    file_auto: Option<bool>,
) -> bool {
    config
        .plugins
        .entries
        .get(entry.id)
        .and_then(|e| e.enable)
        .or(file_auto)
        .unwrap_or_else(|| (entry.manifest)().auto_enable)
}

/// 插槽版本比对（方案 §2.5）：启用插件 `contracts` 逐项对宿主支持集校验，
/// 宿主无该插槽或版本高于支持集 → 报「插槽版本不符」，拒装不静默降级
///（对齐官方 plugin-manager 的兼容性评估语义）。
pub(crate) fn check_slot_versions(
    id: &str,
    contracts: &[String],
    support: &BTreeMap<String, u32>,
) -> Vec<String> {
    let mut issues = Vec::new();
    for spec in contracts {
        // 畸形项跳过：格式合法性由 validate_manifest 负责（已报「格式非法」）。
        let Some((name, ver)) = spec.rsplit_once('.') else { continue };
        let Some(n) = ver.strip_prefix('v').and_then(|d| d.parse::<u32>().ok()) else {
            continue;
        };
        match support.get(name) {
            None => issues.push(format!(
                "插槽版本不符：插件 {id} 声明 {spec}，宿主无插槽 {name}"
            )),
            Some(&host) if n > host => issues.push(format!(
                "插槽版本不符：插件 {id} 声明 {spec}，宿主支持 {name}.v{host}"
            )),
            _ => {}
        }
    }
    issues
}

/// 段值合并（方案 §2.4 第 2 步「config: 段值」）：顶层已知段为基
///（存量五段配置零改动可用），`[plugins.<id>.config]` 浅合并叠加、本键优先
///（新机制插件无顶层段时的唯一配置入口）。config 非 object（非 null）→ 拒装
/// fail-loud：插件段值必须是表，畸形类型静默丢弃会埋配置漂移。
///
/// # 浅合并边界（S2 评审 P2-1，明示防误用）
/// **嵌套表键＝整键替换，不做深合并**：`[plugins.<id>.config]` 写
/// `nested = {...}` 会整体替换顶层段同名嵌套键，其兄弟键随替换丢失
/// （base={"nested":{"a","b"}} 叠加 {"nested":{"b"}} → 结果只有 b）。
/// 存量五段无嵌套表字段，实害为零；S5 凭据/S8 settings 接入嵌套段值前，
/// 需要深合并语义时在本函数升级并同步补测试。
///
/// 对称性（S2 评审 P2-2）：叠加源（config）非表拒装；基（顶层段值）非表
/// 且叠加源非空 → 同样拒装，不静默吞掉叠加配置。
pub(crate) fn merge_segment(
    raw: &Value,
    segment: &str,
    entry: Option<&PluginEntry>,
) -> Result<ConfigValue, AppError> {
    let mut base = raw
        .get(segment)
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    if let Some(cfg) = entry {
        if !cfg.config.is_null() {
            let extra = cfg.config.as_object().ok_or_else(|| {
                AppError::Assemble(format!(
                    "插件 {segment} 的 [plugins] 段值 config 必须是表（object），实际为 {cfg:?}"
                ))
            })?;
            match base.as_object_mut() {
                Some(base_obj) => {
                    for (key, value) in extra {
                        base_obj.insert(key.clone(), value.clone());
                    }
                }
                // 基非表且叠加源非空 → 拒装（与 config 非表对称，不静默吞配置）
                None if !extra.is_empty() => {
                    return Err(AppError::Assemble(format!(
                        "插件 {segment} 的顶层段值必须是表（object）才能叠加 [plugins] 段值，实际为 {base:?}"
                    )));
                }
                None => {}
            }
        }
    }
    Ok(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 宿主插槽支持集：全部插件 contracts 并集（agent-loop 的消费契约、
    /// credentials 的双契约、任务29 的 commands 契约都在列；S10 retry 挂
    /// llm.v1 并集不变，不新增键）。
    #[test]
    fn 宿主插槽支持集覆盖八契约() {
        let support = host_slot_support();
        for name in ["llm", "session", "system-prompt", "tools", "agent-loop", "credentials", "settings", "commands", "sandbox"] {
            assert_eq!(support.get(name), Some(&1), "插槽 {name} 应支持 v1：{support:?}");
        }
        // 第 10 卡=yunyou-sandbox（task-24 沙箱接线；contracts=sandbox.v1 新插槽）
        // 10 基线（九卡+commands+K-7 team）；今晚新挂四件（tool-archive/permission/
        // memory/scheduler 投递）+2 净增=12（P0 定位后机械同步）
        assert_eq!(BUNDLED_PLUGINS.len(), 13, "十三 bundled 齐（十二+memory.v1 记忆卡）");
    }

    /// 插槽版本比对（方案 §2.5）：宿主无该插槽、或声明版本高于支持集 →
    /// 报「插槽版本不符」；等于/低于支持集放行；畸形项跳过（校验层已报）。
    #[test]
    fn 插槽版本比对_违例报不符_合规放行() {
        let support = host_slot_support();
        // 合规：v1 等于支持集
        assert!(check_slot_versions("p", &["tools.v1".into()], &support).is_empty());
        // 违例 a：宿主无该插槽
        let issues = check_slot_versions("p", &["ghost.v1".into()], &support);
        assert!(issues.iter().any(|s| s.contains("插槽版本不符") && s.contains("ghost")), "{issues:?}");
        // 违例 b：版本高于支持集
        let issues = check_slot_versions("p", &["tools.v2".into()], &support);
        assert!(issues.iter().any(|s| s.contains("tools.v2") && s.contains("v1")), "{issues:?}");
        // 畸形项跳过（格式非法由 validate_manifest 报，不在此重复）
        assert!(check_slot_versions("p", &["tools".into()], &support).is_empty());
    }

    /// enable 判定：显式值优先，缺省回落 auto_enable；未知插件 id 不影响其他条目。
    #[test]
    fn enable判定_显式优先_缺省回落() {
        let config = DshConfig::default();
        for entry in BUNDLED_PLUGINS {
            assert!(enabled_of_with_file(entry, &config, None), "空 [plugins] 表应全走 auto_enable");
        }
        let mut config = DshConfig::default();
        config
            .plugins
            .entries
            .insert("yunyou-tools".into(), yunyou_config::model::PluginEntry {
                enable: Some(false),
                ..Default::default()
            });
        let tools = BUNDLED_PLUGINS.iter().find(|e| e.id == "yunyou-tools").unwrap();
        assert!(!enabled_of_with_file(tools, &config, None), "显式 false 应覆盖 auto_enable");
        let llm = BUNDLED_PLUGINS.iter().find(|e| e.id == "yunyou-llm-openai").unwrap();
        assert!(enabled_of_with_file(llm, &config, None), "未列出的插件不受影响");
    }

    /// 段值合并：顶层段为基、config 叠加优先；无顶层段的新插件 config 即全量；
    /// 非 object 的 config 拒装 fail-loud。
    #[test]
    fn 段值合并_叠加优先_畸形拒装() {
        let raw = serde_json::json!({ "tools": { "root_dir": "." } });
        // 叠加：config 覆盖顶层段同名键
        let entry = PluginEntry {
            enable: None,
            config: serde_json::json!({ "root_dir": "/tmp/overlay" }),
        };
        let merged = merge_segment(&raw, "tools", Some(&entry)).expect("object 叠加应成功");
        assert_eq!(merged["root_dir"], "/tmp/overlay", "config 应覆盖顶层段");
        // 无顶层段：config 即全量段值
        let merged = merge_segment(&raw, "compaction", Some(&entry)).expect("无段也应成功");
        assert_eq!(merged["root_dir"], "/tmp/overlay", "无顶层段时 config 即段值");
        // config 缺省（null）：纯顶层段透传
        let merged = merge_segment(&raw, "tools", None).expect("None 应成功");
        assert_eq!(merged["root_dir"], ".", "缺省应纯透传顶层段");
        // 畸形：config 非表 → 拒装
        let bad = PluginEntry { enable: None, config: serde_json::json!(true) };
        let err = merge_segment(&raw, "tools", Some(&bad)).expect_err("非 object 应拒装");
        assert!(err.to_string().contains("必须是表"), "{err}");
        // 嵌套键=整键替换不深合并（评审 P2-1 语义锁）：兄弟键随整键替换丢失
        let nested_raw = serde_json::json!({"tools": {"root_dir": ".", "nested": {"a": 1, "b": 2}}});
        let nested = PluginEntry { enable: None, config: serde_json::json!({"nested": {"b": 9}}) };
        let merged = merge_segment(&nested_raw, "tools", Some(&nested)).expect("嵌套叠加应成功");
        assert_eq!(
            merged["nested"],
            serde_json::json!({"b": 9}),
            "嵌套键=整键替换（rustdoc 明示的浅合并语义）"
        );
        assert!(
            merged["nested"].get("a").is_none(),
            "兄弟键 a 随整键替换丢失＝明示行为，测试锁死防误读为深合并"
        );
        // 基非表且叠加源非空 → 拒装对称（评审 P2-2）
        let scalar_raw = serde_json::json!({"weird": "not-a-table"});
        let overlay = PluginEntry { enable: None, config: serde_json::json!({"k": 1}) };
        let err = merge_segment(&scalar_raw, "weird", Some(&overlay))
            .expect_err("基非表应与 config 非表对称拒装");
        assert!(err.to_string().contains("必须是表"), "{err}");
        // 基非表但叠加源为空表 → 无键可叠，无害放行
        let empty_overlay = PluginEntry { enable: None, config: serde_json::json!({}) };
        let merged = merge_segment(&scalar_raw, "weird", Some(&empty_overlay)).expect("空叠加应放行");
        assert_eq!(merged, serde_json::json!("not-a-table"), "空叠加不改动基值");
    }

    /// 装载位序检查（云镜 S10 复核 P2-S1，D-3）：`yunyou-retry` 必须先于
    /// `yunyou-agent-loop` 装载。生产侧 setup 期容错 get 一次定型（S10 设计
    /// D11 偏差：禁用 retry 不得瘫痪主循环），清单若被重排把 retry 挪到主
    /// 循环之后，集成测试不红而重试静默失效——顺序在清单层锁死；独立成
    /// 纯函数是为了让「乱序注入必红」有可注入的检查单元（防断言恒真）。
    fn 位序检查(ids: &[&str]) -> Result<(), String> {
        let retry = ids.iter().position(|&id| id == "yunyou-retry");
        let agent = ids.iter().position(|&id| id == "yunyou-agent-loop");
        match (retry, agent) {
            (Some(r), Some(a)) if r < a => Ok(()),
            (r, a) => Err(format!(
                "bundled 装载位序被破坏：yunyou-retry 必须先于 yunyou-agent-loop \
                 （重试策略卡先装载、主循环消费 get 一次定型），实际 retry={r:?} agent-loop={a:?}"
            )),
        }
    }

    /// D-3 断言在位：清单实序锁死 retry < agent-loop；重排清单的改动在此红。
    #[test]
    fn 装载位序_retry先于agent_loop() {
        let ids: Vec<&str> = BUNDLED_PLUGINS.iter().map(|e| e.id).collect();
        位序检查(&ids).unwrap();
    }

    /// D-3 乱序注入必红（判别力锁）：把 retry 挪到清单尾部（agent-loop 之后
    /// 的违例形态），检查必须报错——证明上面的实序断言不是恒真。
    #[test]
    fn 装载位序_乱序注入必红() {
        let mut ids: Vec<&str> = BUNDLED_PLUGINS.iter().map(|e| e.id).collect();
        let pos = ids.iter().position(|&id| id == "yunyou-retry").unwrap();
        let retry = ids.remove(pos);
        ids.push(retry);
        let err = 位序检查(&ids).unwrap_err();
        assert!(err.contains("位序被破坏"), "{err}");
    }

}
