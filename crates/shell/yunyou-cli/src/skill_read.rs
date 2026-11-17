//! skill_read 按需加载工具（池 task-13②）：模型判相关→读技能全文进本轮。
//!
//! # 渐进披露链
//! 目录段（[`crate::skills::register_skill_catalog`]）常驻 name+描述元数据；
//! 模型判断某个技能相关时经本工具读**全文**（剥 frontmatter 后正文）——全文只进
//! 本轮 ToolResult，不常驻系统提示。
//!
//! # 正文用完即弃（lead 裁「预留锚」方案）
//! 输出首行带 `[skill:{name}]` 标记：正式的「历史轮 skill_read 结果折叠」由
//! task-11 组装前钩子缝（yun-ya）落地后的策略卡实现（对带标记的历史结果做
//! StubToolResult）；本卡只留识别锚，**不硬编码剥离策略**（主板论：缝归主板，
//! 策略是卡）。
//!
//! # 容错（lead 批注 b）
//! 不存在的技能名=人话报错+附当前可用清单（fail-loud 防模型瞎猜名后空转）；
//! 路径安全：name 过命令名守卫（`^[a-z][a-z0-9_-]*$`），拼进双槽目录内解析，
//! 无路径穿越面。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_util::BoxFut;

/// skill_read 工具（双槽：cwd/.yunyou/skills 优先，cwd/.dsh/skills 过渡）。
pub struct SkillReadTool {
    /// 双槽目录（项目根锚定；构造期解析，进程内稳定）。
    slots: (PathBuf, PathBuf),
}

impl SkillReadTool {
    /// 从项目根构造（同 [`crate::skills::register_skill_commands`] 的双槽口径）。
    pub fn from_cwd(cwd: &Path) -> Self {
        Self { slots: (cwd.join(".yunyou").join("skills"), cwd.join(".dsh").join("skills")) }
    }

    /// 按名解析技能文件：先 .yunyou 槽后 .dsh 槽（同名 .yunyou 胜，与目录段
    /// 扫描同规则）；返回剥 frontmatter 后正文。
    fn resolve(&self, name: &str) -> Option<String> {
        for dir in [&self.slots.0, &self.slots.1] {
            let path = dir.join(format!("{name}.md"));
            if let Ok(text) = std::fs::read_to_string(&path) {
                let (_, body) = crate::skills::split_frontmatter(&text);
                return Some(body.to_string());
            }
        }
        None
    }

    /// 当前可用技能名清单（报错附带的指路面；复用目录扫描，项目根=双槽祖父目录）。
    fn available_names(&self) -> Vec<String> {
        let project_root = self.slots.0.parent().and_then(Path::parent).unwrap_or(Path::new("."));
        crate::skills::scan_skill_catalog(project_root).into_iter().map(|e| e.name).collect()
    }
}

impl ToolDefinition for SkillReadTool {
    fn name(&self) -> &str {
        "skill_read"
    }

    fn description(&self) -> &str {
        "按名读取一个技能的完整正文（目录里只列名字与描述；写插件/查能力等场景先读 plugins-seams）。只在本技能与当前任务相关时调用。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "技能名（目录段列出的名，如 plugins-seams）"}
            },
            "required": ["name"]
        })
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {"type": "string", "description": "技能名"},
                "content": {"type": "string", "description": "技能全文（首行 [skill:名] 为折叠锚标记）"}
            },
            "required": ["skill", "content"]
        }))
    }

    fn is_concurrency_safe(&self) -> bool {
        true // 纯读
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let tool = Arc::new(SkillReadTool { slots: self.slots.clone() });
        Box::pin(async move {
            let Some(name) = args.get("name").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 name 参数");
            };
            if !crate::skills::valid_name(name) {
                return ToolOutcome::err_h(
                    "tool/args",
                    format!("技能名 {name} 非法（需 ^[a-z][a-z0-9_-]*$）"),
                    "只使用目录段里列出的技能名",
                );
            }
            match tool.resolve(name) {
                Some(body) => ToolOutcome::ok(serde_json::json!({
                    "skill": name,
                    // 首行折叠锚（task-13③ 预留；正式剥离归钩子缝策略卡）
                    "content": format!("[skill:{name}]\n{body}"),
                })),
                None => {
                    let names = tool.available_names();
                    let list = if names.is_empty() {
                        "（当前无可用技能）".to_string()
                    } else {
                        names.join(", ")
                    };
                    ToolOutcome::err_h(
                        "tool/not-found",
                        format!("技能 {name} 不存在"),
                        format!("可用技能：{list}"),
                    )
                }
            }
        })
    }
}

/// 注册进工具注册表（壳侧职责；Disposer 无 Drop 副作用，进程生命周期随根卸载）。
/// 返回是否注册成功（观测用）。
pub fn register_skill_read_tool(root: &yunyou::Context, cwd: &Path) -> bool {
    let Ok(registry) = root.get::<dyn yunyou_tools::RegistryService>(
        yunyou::ServiceId::of::<dyn yunyou_tools::RegistryService>(),
    ) else {
        eprintln!("[技能] tools 注册表未就绪，skill_read 注册跳过");
        return false;
    };
    match Arc::clone(&registry).register_tool(Arc::new(SkillReadTool::from_cwd(cwd))) {
        Ok(_) => true,
        Err(err) => {
            eprintln!("[技能] skill_read 注册失败（跳过）：{err}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_util::sync::CancellationToken;

    fn exec() -> ToolExec {
        ToolExec { call_id: "t".into(), cancel: CancellationToken::new(), workspace: None }
    }

    async fn run(tool: &SkillReadTool, name: &str) -> ToolOutcome {
        tool.execute(serde_json::json!({"name": name}), exec()).await
    }

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn setup_dir(tag: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dir = std::env::temp_dir().join(format!("yunyou-skill-read-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yunyou/skills"))?;
        std::fs::create_dir_all(dir.join(".dsh/skills"))?;
        Ok(dir)
    }

    /// 按需加载：命中返回 [skill:名] 锚+剥 frontmatter 正文；.yunyou 同名胜。
    #[tokio::test]
    async fn 命中返回锚与正文_同名项目槽胜() -> TestResult {
        let dir = setup_dir("hit")?;
        std::fs::write(
            dir.join(".yunyou/skills/deploy.md"),
            "---\nalias: 部署\n---\n# 部署指南\n步骤一。",
        )?;
        std::fs::write(dir.join(".dsh/skills/deploy.md"), "# 旧版\n旧正文")?;
        let tool = SkillReadTool::from_cwd(&dir);
        let ToolOutcome::Ok(value) = run(&tool, "deploy").await else { panic!("应命中") };
        let content = value["content"].as_str().ok_or("content 应为串")?;
        assert!(content.starts_with("[skill:deploy]\n"), "首行折叠锚：{content}");
        assert!(
            content.contains("# 部署指南") && content.contains("步骤一"),
            "正文剥离 frontmatter：{content}"
        );
        assert!(
            !content.contains("旧正文") && !content.contains("alias"),
            "同名项目槽胜且元数据不进正文"
        );
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// 不存在=人话报错+可用清单；非法名=直接拒。
    #[tokio::test]
    async fn 未知名报错附清单_非法名拒() -> TestResult {
        let dir = setup_dir("miss")?;
        std::fs::write(dir.join(".yunyou/skills/plugins-seams.md"), "# 插件缝地图\n现有缝……")?;
        let tool = SkillReadTool::from_cwd(&dir);
        let ToolOutcome::Err(err) = run(&tool, "no-such-skill").await else {
            panic!("未知名应报错")
        };
        assert!(err.message.contains("不存在"), "{err:?}");
        assert!(
            err.hint.as_deref().is_some_and(|h| h.contains("plugins-seams")),
            "清单应含可用名：{err:?}"
        );
        let ToolOutcome::Err(err) = run(&tool, "../etc/passwd").await else {
            panic!("非法名应拒")
        };
        assert!(err.message.contains("非法"), "{err:?}");
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
