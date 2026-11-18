//! memory.v1 记忆缝：四操作工具集 + MEMORY.md 活索引常驻注入（池 task-14）。
//!
//! 设计：docs/设计/memory.v1-记忆缝与工具集设计-20261118.md（lead 裁全批）。
//!
//! # 缝定位
//! 跨会话记忆统一存取插槽：目录约定（项目级 `~/.yunyou/projects/<净化 cwd>/memory/`，
//! sanitize 与会话存储同函数=[`yunyou_session::sanitize_project_dir`]）+ 索引常驻
//! 注入 + 四操作工具面。与 persona（我是谁）/session（过程留痕）/skills（怎么干活）
//! 边界见设计 §六关系表。
//!
//! # 硬顶纪律（设计核心判断）
//! MEMORY.md 索引 200 行 / 25KB 硬顶：超限=写入**拒绝**并人话报「索引已满请整理」，
//! **禁静默截断**——截断丢的是用户记忆，宁可拒绝逼整理。
//!
//! # 作用域（v1 裁定）
//! project 作用域全量落地；global 作用域候审批缝（Ask，yun-suo S1 在途，对象类型
//! =目录级）——v1 对 global 一律人话报错标注候缝，不静默降级。
//!
//! # 注入策略（设计 §四）
//! 唯一注入物=MEMORY.md（带来源标签），复用 [`crate::skills::register_skill_catalog`]
//! 同款目录段机制（harness:memory 段，档位 510=skills 500 与工具段 1000 之间空隙）；
//! 正文/日志不注入（按需工具深读）。注入预算 400 token≈1200 字符（中文 1 字≈1
//! token 从严），超预算截顶+**显式尾注**（不静默——模型知道索引只注入了头部，
//! 全量用 memory_list）。

use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use yunyou::ServiceId;
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_util::BoxFut;

/// 索引硬顶：200 行 / 25KB（memdir.ts:34-38 实证值，设计 §二）。
const INDEX_MAX_LINES: usize = 200;
const INDEX_MAX_BYTES: usize = 25 * 1024;
/// 注入 token 预算（≈字符数从严口径，设计验收④ token ≤400）。
const INJECT_BUDGET_CHARS: usize = 1200;
/// harness:memory 段名与档位（skills=500 的紧邻空隙）。
pub const MEMORY_SECTION_NAME: &str = "harness:memory";
pub const MEMORY_SECTION_ORDER: f64 = 510.0;

/// 记忆存储（作用域根目录解析 + 四操作实现；v1 只开 project 作用域）。
pub struct MemoryStore {
    /// 项目级记忆根：`<home>/projects/<净化 cwd>/memory/`。
    project_root: PathBuf,
    /// 全局级记忆根（v1 只用于报错指路，不读写）。
    global_root: PathBuf,
}

impl MemoryStore {
    /// 从 yunyou-home 与项目 cwd 构造（run.rs 装配后一次，进程内稳定）。
    pub fn new(home: &Path, project_cwd: &Path) -> Self {
        Self {
            project_root: home
                .join("projects")
                .join(yunyou_session::sanitize_project_dir(project_cwd))
                .join("memory"),
            global_root: home.join("memory"),
        }
    }

    /// 项目级记忆根（测试指路用）。
    pub(crate) fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// 索引文件路径。
    fn index_path(&self) -> PathBuf {
        self.project_root.join("MEMORY.md")
    }

    /// 话题文件路径（白名单：只许落在本记忆根内；防逃逸运行时闸）。
    fn topic_path(&self, topic: &str) -> Result<PathBuf, ToolOutcome> {
        if topic.is_empty()
            || topic.contains('/')
            || topic.contains('\\')
            || topic.contains("..")
            || topic.starts_with('.')
        {
            return Err(ToolOutcome::err_h(
                "tool/args",
                format!("话题名 {topic} 非法（禁止路径分隔符/.. /首部点）"),
                "话题用短横线或下划线命名，如 deploy-notes",
            ));
        }
        let path = self.project_root.join(format!("{topic}.md"));
        // canonicalize 前置校验：拼出的路径必须仍在记忆根内（防符号链接等漂移，
        // 父目录不存在时以词法根校验兜底——文件工具同族思路）
        if let Ok(canon) = path.canonicalize() {
            if !canon.starts_with(
                &self.project_root.canonicalize().unwrap_or_else(|_| self.project_root.clone()),
            ) {
                return Err(ToolOutcome::err(
                    "tool/path-outside-root",
                    "解析路径逃出记忆目录白名单，已拒绝",
                ));
            }
        }
        Ok(path)
    }

    /// 读索引行（文件缺失=空索引）。
    fn index_lines(&self) -> Vec<String> {
        std::fs::read_to_string(self.index_path())
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// 索引硬顶校验（追加 1 行后不得超限）。
    fn check_index_headroom(&self, lines: &[String]) -> Result<(), ToolOutcome> {
        let bytes: usize = lines.iter().map(|l| l.len() + 1).sum();
        if lines.len() > INDEX_MAX_LINES || bytes > INDEX_MAX_BYTES {
            return Err(ToolOutcome::err_h(
                "memory/index-full",
                "记忆索引已满请整理（硬顶 200 行/25KB，拒绝写入——静默截断会丢你的记忆）",
                "先调 memory_list 看全量，再对过时条目逐个 memory_forget，或手工整理 MEMORY.md",
            ));
        }
        Ok(())
    }

    /// 四操作①save：写话题文件+维护索引行（同 topic 覆盖正文、索引去重）。
    fn save(&self, topic: &str, content: &str, summary: Option<&str>) -> Result<serde_json::Value, ToolOutcome> {
        let path = self.topic_path(topic)?;
        std::fs::create_dir_all(&self.project_root)
            .map_err(|err| ToolOutcome::err("tool/io", format!("记忆目录创建失败：{err}")))?;
        std::fs::write(&path, content)
            .map_err(|err| ToolOutcome::err("tool/io", format!("话题文件写入失败：{err}")))?;
        // 索引行：已有行保留原摘要（去重语义），无则按 content 首行截 60 字
        let mut lines = self.index_lines();
        let entry_prefix = format!("- [{topic}]({topic}.md)");
        if !lines.iter().any(|l| l.starts_with(&entry_prefix)) {
            let summary = summary.map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| {
                // 摘要=首个非标题非空行（标题行当摘要是哑目录条目）
                let first = content
                    .lines()
                    .map(str::trim)
                    .find(|l| !l.is_empty() && !l.starts_with('#'))
                    .unwrap_or("");
                let cut: String = first.chars().take(60).collect();
                if first.chars().count() > 60 { format!("{cut}…") } else { cut.to_string() }
            });
            lines.push(format!("{entry_prefix} — {summary}"));
            self.check_index_headroom(&lines)?;
            std::fs::write(self.index_path(), format!("{}\n", lines.join("\n")))
                .map_err(|err| ToolOutcome::err("tool/io", format!("索引写入失败：{err}")))?;
        }
        Ok(json!({
            "ok": true,
            "path": path.display().to_string(),
            "index_lines_used": lines.len(),
        }))
    }

    /// 四操作②search：索引行+正文全文匹配（v1 简单 grep，v2 语义检索留接口）。
    fn search(&self, query: &str) -> serde_json::Value {
        let mut hits: Vec<serde_json::Value> = Vec::new();
        for line in self.index_lines() {
            if line.contains(query) {
                hits.push(json!({"index": line}));
            }
        }
        if let Ok(entries) = std::fs::read_dir(&self.project_root) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md") && p != &self.index_path())
                .collect();
            files.sort();
            for path in files {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let matched: Vec<&str> =
                        text.lines().filter(|l| l.contains(query)).take(5).collect();
                    if !matched.is_empty() {
                        if let Some(topic) = path.file_stem().and_then(|s| s.to_str()) {
                            hits.push(json!({"topic": topic, "matches": matched}));
                        }
                    }
                }
            }
        }
        json!({"ok": true, "query": query, "hits": hits})
    }

    /// 四操作③list：索引条目全列。
    fn list(&self) -> serde_json::Value {
        let lines = self.index_lines();
        json!({
            "ok": true,
            "path": self.index_path().display().to_string(),
            "index_lines_used": lines.len(),
            "entries": lines,
        })
    }

    /// 四操作④forget：删话题文件+摘索引行（**唯一删除口**，设计 §三）。
    fn forget(&self, topic: &str) -> Result<serde_json::Value, ToolOutcome> {
        let path = self.topic_path(topic)?;
        let removed_file = std::fs::remove_file(&path).is_ok();
        let mut lines: Vec<String> = self
            .index_lines()
            .into_iter()
            .filter(|l| !l.starts_with(&format!("- [{topic}]({topic}.md)")))
            .collect();
        let removed_line = lines.len() != self.index_lines().len();
        if removed_line {
            std::fs::write(self.index_path(), format!("{}\n", lines.join("\n")))
                .map_err(|err| ToolOutcome::err("tool/io", format!("索引改写失败：{err}")))?;
        }
        if !removed_file && !removed_line {
            let names: Vec<String> = self.list_topics();
            let list = if names.is_empty() { "（当前无记忆条目）".into() } else { names.join(", ") };
            return Err(ToolOutcome::err_h(
                "tool/not-found",
                format!("记忆 {topic} 不存在"),
                format!("可用条目：{list}"),
            ));
        }
        Ok(json!({
            "ok": true,
            "path": path.display().to_string(),
            "index_lines_used": lines.len(),
        }))
    }

    /// 现存话题名清单（指路面）。
    fn list_topics(&self) -> Vec<String> {
        std::fs::read_dir(&self.project_root)
            .map(|entries| {
                let mut names: Vec<String> = entries
                    .flatten()
                    .filter(|e| e.path() != self.index_path())
                    .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str().map(str::to_string)))
                    .collect();
                names.sort();
                names
            })
            .unwrap_or_default()
    }

    /// 注入段文本（设计 §四）：来源标签+索引全文；超预算截顶+显式尾注。
    fn section_text(&self) -> String {
        let Ok(text) = std::fs::read_to_string(self.index_path()) else { return String::new() };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        let header = format!(
            "[记忆索引 · 来源 {}]（正文用 read 深读；增删用 memory_save/memory_forget）",
            self.index_path().display()
        );
        let mut lines: Vec<&str> = trimmed.lines().collect();
        let mut omitted = 0usize;
        while lines.iter().map(|l| l.len() + 1).sum::<usize>() + header.len() > INJECT_BUDGET_CHARS {
            lines.pop();
            omitted += 1;
            if lines.is_empty() {
                break;
            }
        }
        let tail_note = format!("…索引尾部 {omitted} 行超常驻预算未注入（全量用 memory_list）");
        if omitted > 0 {
            lines.push(&tail_note);
        }
        format!("{header}\n{}", lines.join("\n"))
    }
}

/// 注入段注册（复用 skills 目录段机制；空索引不注册）。返回是否注册（观测用）。
pub fn register_memory_section(root: &yunyou::Context, store: &Arc<MemoryStore>) -> bool {
    let text = store.section_text();
    if text.is_empty() {
        return false;
    }
    let Ok(prompt) = root.get::<dyn yunyou_system_prompt::SystemPrompt>(
        ServiceId::of::<dyn yunyou_system_prompt::SystemPrompt>(),
    ) else {
        eprintln!("[记忆] system-prompt 服务未就绪，索引段注册跳过");
        return false;
    };
    let section = yunyou_system_prompt::Section::new(MEMORY_SECTION_NAME, MEMORY_SECTION_ORDER, text);
    match prompt.register_section(section) {
        Ok(_) => true,
        Err(err) => {
            eprintln!("[记忆] 索引段注册失败（跳过）：{err}");
            false
        }
    }
}

/// global 作用域统一报错（v1 候审批缝，不静默降级）。
fn global_pending() -> ToolOutcome {
    ToolOutcome::err_h(
        "memory/scope-pending",
        "global 作用域候审批缝（写入跨项目记忆需用户确认，Ask 审批插件落地后开启）",
        "v1 请用 scope=\"project\"；确需全局记忆可先手工写 ~/.yunyou/memory/MEMORY.md",
    )
}

macro_rules! memory_tool_common {
    ($name:literal, $desc:literal) => {
        fn name(&self) -> &str {
            $name
        }
        fn description(&self) -> &str {
            $desc
        }
    };
}

/// memory_save：写话题文件+维护索引（唯一写入口；forget 之外的删除不存在）。
pub struct MemorySaveTool {
    pub store: Arc<MemoryStore>,
}

impl ToolDefinition for MemorySaveTool {
    fn is_concurrency_safe(&self) -> bool {
        false // 文件写族保守串行
    }
    memory_tool_common!(
        "memory_save",
        "保存一条跨会话记忆：写话题文件并维护活索引（同话题覆盖正文）。用于沉淀结论、用户偏好、项目约定等值得跨会话记住的事实。"
    );
    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {"type": "string", "enum": ["project", "global"], "description": "作用域（v1 仅 project 可用）"},
                "topic": {"type": "string", "description": "话题名（文件名，如 deploy-notes）"},
                "content": {"type": "string", "description": "记忆正文（markdown）"},
                "summary": {"type": "string", "description": "可选：索引行摘要（缺省取正文首行截 60 字）"}
            },
            "required": ["scope", "topic", "content"]
        })
    }
    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "ok": {"type": "boolean"},
                "path": {"type": "string"},
                "index_lines_used": {"type": "integer"},
                "warning": {"type": "string"}
            },
            "required": ["ok", "path", "index_lines_used"]
        }))
    }
    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let store = Arc::clone(&self.store);
        Box::pin(async move {
            if args.get("scope").and_then(|v| v.as_str()) != Some("project") {
                return global_pending();
            }
            let (Some(topic), Some(content)) = (
                args.get("topic").and_then(|v| v.as_str()),
                args.get("content").and_then(|v| v.as_str()),
            ) else {
                return ToolOutcome::err("tool/args", "缺少 topic 或 content 参数");
            };
            let summary = args.get("summary").and_then(|v| v.as_str());
            match store.save(topic, content, summary) {
                Ok(mut value) => {
                    if let Some(obj) = value.as_object_mut() {
                        obj.insert("warning".into(), json!("global 作用域候审批缝，本条已存 project 作用域"));
                    }
                    ToolOutcome::ok(value)
                }
                Err(outcome) => outcome,
            }
        })
    }
}

/// memory_search：索引+正文全文匹配。
pub struct MemorySearchTool {
    pub store: Arc<MemoryStore>,
}

impl ToolDefinition for MemorySearchTool {
    memory_tool_common!("memory_search", "按关键词搜索记忆（索引与话题正文全文匹配）。");
    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {"type": "string", "enum": ["project", "global"]},
                "query": {"type": "string", "description": "关键词"}
            },
            "required": ["scope", "query"]
        })
    }
    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(json!({"type": "object", "properties": {"ok": {"type": "boolean"}, "query": {"type": "string"}, "hits": {"type": "array"}}, "required": ["ok", "query", "hits"]}))
    }
    fn is_concurrency_safe(&self) -> bool {
        true // 纯读
    }
    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let store = Arc::clone(&self.store);
        Box::pin(async move {
            if args.get("scope").and_then(|v| v.as_str()) != Some("project") {
                return global_pending();
            }
            let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 query 参数");
            };
            ToolOutcome::ok(store.search(query))
        })
    }
}

/// memory_list：索引条目全列。
pub struct MemoryListTool {
    pub store: Arc<MemoryStore>,
}

impl ToolDefinition for MemoryListTool {
    memory_tool_common!("memory_list", "列出记忆索引全部条目（名+摘要）。");
    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {"scope": {"type": "string", "enum": ["project", "global"]}},
            "required": ["scope"]
        })
    }
    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(json!({"type": "object", "properties": {"ok": {"type": "boolean"}, "entries": {"type": "array"}, "index_lines_used": {"type": "integer"}}, "required": ["ok", "entries", "index_lines_used"]}))
    }
    fn is_concurrency_safe(&self) -> bool {
        true // 纯读
    }
    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let store = Arc::clone(&self.store);
        Box::pin(async move {
            if args.get("scope").and_then(|v| v.as_str()) != Some("project") {
                return global_pending();
            }
            ToolOutcome::ok(store.list())
        })
    }
}

/// memory_forget：删话题文件+摘索引行（唯一删除口）。
pub struct MemoryForgetTool {
    pub store: Arc<MemoryStore>,
}

impl ToolDefinition for MemoryForgetTool {
    fn is_concurrency_safe(&self) -> bool {
        false // 删除族保守串行
    }
    memory_tool_common!("memory_forget", "删除一条记忆：删话题文件并摘除索引行（唯一的记忆删除操作）。");
    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "scope": {"type": "string", "enum": ["project", "global"]},
                "topic": {"type": "string", "description": "要删除的话题名"}
            },
            "required": ["scope", "topic"]
        })
    }
    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(json!({"type": "object", "properties": {"ok": {"type": "boolean"}, "path": {"type": "string"}, "index_lines_used": {"type": "integer"}}, "required": ["ok", "path", "index_lines_used"]}))
    }
    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let store = Arc::clone(&self.store);
        Box::pin(async move {
            if args.get("scope").and_then(|v| v.as_str()) != Some("project") {
                return global_pending();
            }
            let Some(topic) = args.get("topic").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 topic 参数");
            };
            match store.forget(topic) {
                Ok(value) => ToolOutcome::ok(value),
                Err(outcome) => outcome,
            }
        })
    }
}

/// 注册四工具进工具注册表（壳侧职责；进程生命周期随根卸载）。返回注册数（观测用）。
pub fn register_memory_tools(root: &yunyou::Context, store: Arc<MemoryStore>) -> usize {
    let Ok(registry) = root
        .get::<dyn yunyou_tools::RegistryService>(ServiceId::of::<dyn yunyou_tools::RegistryService>())
    else {
        eprintln!("[记忆] tools 注册表未就绪，记忆工具注册跳过");
        return 0;
    };
    let mut registered = 0usize;
    for tool in [
        Arc::new(MemorySaveTool { store: Arc::clone(&store) }) as Arc<dyn ToolDefinition>,
        Arc::new(MemorySearchTool { store: Arc::clone(&store) }),
        Arc::new(MemoryListTool { store: Arc::clone(&store) }),
        Arc::new(MemoryForgetTool { store }),
    ] {
        match Arc::clone(&registry).register_tool(tool) {
            Ok(_) => registered += 1,
            Err(err) => eprintln!("[记忆] 工具注册失败（跳过该工具）：{err}"),
        }
    }
    registered
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
