//! 人设分源装载（池 task-6 persona.md + task-25 AGENTS.md 四级分源升格）。
//!
//! # AGENTS.md 四级分源（对齐 claude CLAUDE.md 四级，行号锚=挖矿-永久记忆
//! -20261117.md claudemd.ts:1-33/:806-1010；「比 claude 好」三点按池卡 rev3）
//!
//! | 层 | 路径 | 语义 |
//! | --- | --- | --- |
//! | Managed | `/etc/yunyou/AGENTS.md` | 策略级（组织统一下发） |
//! | User | `<yunyou-home>/AGENTS.md`（~/.yunyou） | 用户全局 |
//! | Project | `<cwd>/AGENTS.md` | 项目级（随仓库走） |
//! | Local | `<cwd>/.yunyou/local/AGENTS.md` | 私有（不入库，.gitignore 自理） |
//!
//! - **注入=按层序拼接**（Managed→Local，后层可见前层；「冲突以后层为准」由
//!   拼接顺序天然表达——同主题规则后层文本排后=模型面更近）；
//! - **stub 锚**（比 claude 好②）：每层块首行 `[agents:<层名>]`，钩子缝
//!   （task-11 组装前钩子）落地后可按锚做分层剥离/替换，本卡只留锚不接策略；
//! - **冲突检测**（比 claude 好①）：同一非空规则行在 ≥2 层出现时，段尾人话
//!   尾注点名「以下规则多层重复，后层为准」——不静默；
//! - **同源单源**（比 claude 好③）：工具说明/示例配置以代码 description 为真源
//!   （docs/tools/ 与 README 引用同源，见 docs/tools/README.md 头注）。
//!
//! # 优先级（写死，池卡 rev3）
//!
//! **AGENTS.md 链（任一层非空）> persona.md（project > home）> toml 字符串**。
//! AGENTS 链命中即 shadow persona.md；persona.md 命中即 shadow toml（task-6
//! 语义保留）。空文件=不存在（trim 后判空），读取失败按未命中不阻断启动。
//!
//! `#` 标题切段仍由组件侧 [`yunyou_system_prompt::persona_md::split_md_sections`]
//! 完成；本模块只负责发现/读取/拼接。

use serde_json::Value;
use std::path::{Path, PathBuf};

/// Managed 层固定路径（POSIX 策略级；对照 claude /etc/claude-code/CLAUDE.md）。
pub const AGENTS_MANAGED_PATH: &str = "/etc/yunyou/AGENTS.md";
/// Local 层固定相对路径（.yunyou/local 目录不入库，gitignore 自理）。
pub const AGENTS_LOCAL_REL: &str = ".yunyou/local/AGENTS.md";
/// Project/User 层文件名（各层目录下的固定名）。
pub const AGENTS_FILE: &str = "AGENTS.md";

/// 一个已命中的 AGENTS 层。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentsLayer {
    /// 层名（managed/user/project/local；stub 锚与冲突提示用）。
    pub kind: &'static str,
    pub path: PathBuf,
    pub text: String,
}

/// 读一个层文件：存在且 trim 后非空才命中（空=不存在回落）。
fn read_layer(kind: &'static str, path: PathBuf) -> Option<AgentsLayer> {
    std::fs::read_to_string(&path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .map(|text| AgentsLayer { kind, path, text })
}

/// 四级链加载：按 Managed→User→Project→Local 层序返回全部命中层。
/// Managed 不可读/不存在=常态跳过（策略级缺省是常态，不告警）。
pub fn load_agents_chain(project_root: &Path, home: &Path) -> Vec<AgentsLayer> {
    [
        ("managed", PathBuf::from(AGENTS_MANAGED_PATH)),
        ("user", home.join(AGENTS_FILE)),
        ("project", project_root.join(AGENTS_FILE)),
        ("local", project_root.join(AGENTS_LOCAL_REL)),
    ]
    .into_iter()
    .filter_map(|(kind, path)| read_layer(kind, path))
    .collect()
}

/// 冲突检测：同一 trim 后非空行在 ≥2 层出现 → 返回重复行清单（保首次出现序）。
pub fn detect_conflicts(layers: &[AgentsLayer]) -> Vec<String> {
    let mut first_seen: std::collections::HashMap<&str, &'static str> =
        std::collections::HashMap::new();
    let mut duplicates: Vec<String> = Vec::new();
    for layer in layers {
        for line in layer.text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue; // 标题行天然分层复用，不视为冲突
            }
            match first_seen.get(line) {
                None => {
                    first_seen.insert(line, layer.kind);
                }
                Some(_) if !duplicates.iter().any(|d| d == line) => {
                    duplicates.push(line.to_string());
                }
                Some(_) => {}
            }
        }
    }
    duplicates
}

/// 四级链拼接（注入形态）：每层 `[agents:<层名>]` 锚+内容；冲突尾注人话点名。
pub fn compose_agents_text(layers: &[AgentsLayer]) -> String {
    if layers.is_empty() {
        return String::new();
    }
    let mut blocks: Vec<String> = layers
        .iter()
        .map(|layer| format!("[agents:{}]（来源 {}）\n{}", layer.kind, layer.path.display(), layer.text))
        .collect();
    let conflicts = detect_conflicts(layers);
    if !conflicts.is_empty() {
        blocks.push(format!(
            "⚠ 人设冲突提示：以下规则在多层重复声明，**后层为准**（Managed<User<Project<Local）：\n{}",
            conflicts.iter().map(|c| format!("- {c}")).collect::<Vec<_>>().join("\n")
        ));
    }
    blocks.join("\n\n")
}

/// 旧 persona.md 两级回落（task-6 语义保留）：project .yunyou/persona.md > home
/// persona.md；都未命中/空 → None。
pub fn resolve_persona_md(project_root: &Path, home: &Path) -> Option<String> {
    [project_root.join(PROJECT_PERSONA_REL), home.join(HOME_PERSONA_FILE)]
        .into_iter()
        .find_map(|path| {
            std::fs::read_to_string(&path)
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|text| !text.is_empty())
        })
}

/// 生效人设文本（总解析口，优先级写死）：AGENTS 链 > persona.md > None（toml）。
pub fn resolve_persona_text(project_root: &Path, home: &Path) -> Option<String> {
    let layers = load_agents_chain(project_root, home);
    if !layers.is_empty() {
        return Some(compose_agents_text(&layers));
    }
    resolve_persona_md(project_root, home)
}

/// 装配接线（复用既有 shadow 人设槽，不新增第二机制）：文本命中即把内容写进
/// `agent_loop.persona`——[`crate::assemble::bridge_persona`] 桥接只在槽空时填
/// toml 值，故文件命中即 shadow toml。
pub(crate) fn apply_to_raw(mut raw: Value, project_root: &Path, home: &Path) -> Value {
    if let Some(persona) = resolve_persona_text(project_root, home) {
        raw["agent_loop"]["persona"] = Value::String(persona);
    }
    raw
}

/// 装配用项目根：进程当前工作目录（仓库级人设随项目走；取不到＝无项目级人设）。
pub(crate) fn project_root() -> Option<std::path::PathBuf> {
    std::env::current_dir().ok()
}

/// 项目根 persona 文件的固定相对路径（.yunyou 目录由本约定新引入，与仓库配置共居）。
pub const PROJECT_PERSONA_REL: &str = ".yunyou/persona.md";

/// yunyou-home 下 persona 文件名。
pub const HOME_PERSONA_FILE: &str = "persona.md";

#[cfg(test)]
#[path = "persona_md_tests.rs"]
mod tests;
