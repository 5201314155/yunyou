//! 技能→命令注册映射（任务29 C4，D4 增补；设计 §七）。
//!
//! 槽位（D4 裁决双扫描）：项目根 `.yunyou/skills` 优先 + `.dsh/skills` 过渡兼容，
//! 两槽同名 `.yunyou` 胜（扫描顺序=.yunyou 先注册进表，.dsh 同名 register Err
//! 被捕获跳过——fail-loud 缝自带「先到先得」语义，无需额外查重）。
//!
//! 映射规则：文件名（去 .md）=命令名，正则 `^[a-z][a-z0-9_-]*$` 守卫（非法跳过
//! +stderr 点名，不拒装其他技能）；描述=剥 frontmatter 后首个一级标题；
//! frontmatter `alias: 中文名` 可选（剥 frontmatter 后正文=展开文本——元数据不是
//! 内容）。命令归**展开类**（D1 二分）：handler 零副作用返回正文，回填输入框
//! 待用户确认发送——「技能进命令」与「命令不进模型」两约束由此并存。
//!
//! 容错纪律：单文件非法/读取失败/同名冲突只跳过该文件并 stderr 点名，
//! 绝不中断其余技能注册（一处坏文件不瘫全目录）。

use std::path::Path;
use std::sync::Arc;
use yunyou::{Context, ServiceId};
use yunyou_commands::{
    CommandDefinition, CommandKind, CommandOutput, CommandsService,
};
use yunyou_util::BoxFut;

/// 技能命令名守卫（设计 §七）：`^[a-z][a-z0-9_-]*$`。
pub(crate) fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// 剥 frontmatter：文件以 `---` 行开头 → 到下一 `---` 行之间为元数据区。
/// 返回（alias 选项，正文）；无 frontmatter 原文返回。一期只认 `alias:` 键
///（设计 §七：中文名别名，缺省无别名）。
pub(crate) fn split_frontmatter(text: &str) -> (Option<String>, &str) {
    let Some(rest) = text.strip_prefix("---") else { return (None, text) };
    if !rest.starts_with('\n') {
        return (None, text); // 非 frontmatter 开头（如 "---" 出现在正文首）按原文处理
    }
    let Some(end) = rest.find("\n---") else { return (None, text) };
    let meta = &rest[..end];
    let after = rest[end + 4..].strip_prefix('\n').unwrap_or(&rest[end + 4..]);
    let alias = meta
        .lines()
        .find_map(|line| line.strip_prefix("alias:"))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    (alias, after)
}

/// 描述=剥 frontmatter 后首个一级标题（`# 标题`；缺标题的技能跳过——
/// 面板/帮助没有描述的命令是坏目录条目，不装哑命令）。
fn first_heading(text: &str) -> Option<&str> {
    text.lines().find_map(|line| line.strip_prefix("# ")).map(str::trim).filter(|h| !h.is_empty())
}

/// 目录段（harness:skills）段名与档位：人设槽 0 与工具段 1000 之间的空隙
/// （守段位表「新段选空隙」纪律）；段位常量归本模块（技能目录是宿主资源面）。
pub const SKILLS_SECTION_NAME: &str = "harness:skills";
/// 目录段档位。
pub const SKILLS_SECTION_ORDER: f64 = 500.0;
/// 目录段 token 预算（字符口径：中文 1 字≈1 token，2000 字符≈2k token）。
const CATALOG_BUDGET_CHARS: usize = 2000;
/// 单条描述截断上限（字符；描述=first_heading 短句，60 字已够判相关性）。
const CATALOG_DESC_MAX_CHARS: usize = 60;
/// 目录段引导语（lead 批注 a）：模型不用自己猜「先看目录还是先读全文」。
const CATALOG_GUIDE: &str = "以下为可用技能目录（名+描述）。写插件/查能力前先 skill_read plugins-seams；需要某技能详情时用 skill_read 按名读全文。";

/// 技能目录条目：命令名 + 描述（元数据常驻面，正文不进）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub name: String,
    pub description: String,
}

/// 双槽技能目录扫描（task-13①）：.yunyou 优先 + .dsh 过渡，同名 .yunyou 胜
/// （与命令注册同规则）；解析复用 valid_name/split_frontmatter/first_heading，
/// 无标题/非法名跳过（与命令注册同容错纪律）。输出按文件名序稳定排序。
pub fn scan_skill_catalog(cwd: &Path) -> Vec<CatalogEntry> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut entries: Vec<CatalogEntry> = Vec::new();
    for slot_dir in [cwd.join(".yunyou").join("skills"), cwd.join(".dsh").join("skills")] {
        let Ok(dir_entries) = std::fs::read_dir(&slot_dir) else { continue };
        let mut files: Vec<std::path::PathBuf> = dir_entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        files.sort();
        for path in files {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if !valid_name(stem) || !seen.insert(stem.to_string()) {
                continue; // 非法名 / 同名（.yunyou 已收录）跳过
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            let (_, body) = split_frontmatter(&text);
            let Some(description) = first_heading(body) else { continue };
            entries.push(CatalogEntry { name: stem.to_string(), description: description.to_string() });
        }
    }
    entries
}

/// 目录段文本（task-13①，lead 批预算）：引导语+每技能一行 `- name: 描述`；
/// 描述截断 60 字符；段总文本超 2000 字符预算即截断并加尾注计数。
pub fn catalog_section_text(entries: &[CatalogEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let truncate = |text: &str| -> String {
        if text.chars().count() <= CATALOG_DESC_MAX_CHARS {
            text.to_string()
        } else {
            let cut: String = text.chars().take(CATALOG_DESC_MAX_CHARS).collect();
            format!("{cut}…")
        }
    };
    let mut lines: Vec<String> = Vec::new();
    let mut omitted = 0usize;
    for entry in entries {
        let line = format!("- {}: {}", entry.name, truncate(&entry.description));
        let projected_len = lines.iter().map(String::len).sum::<usize>() + line.len() + CATALOG_GUIDE.len();
        if projected_len > CATALOG_BUDGET_CHARS {
            omitted += 1;
            continue;
        }
        lines.push(line);
    }
    if omitted > 0 {
        lines.push(format!("…另有 {omitted} 个技能未列入（/skills 查看全量）"));
    }
    format!("{CATALOG_GUIDE}\n{}", lines.join("\n"))
}

/// 目录段注册（task-13①）：SystemPrompt::register_section 挂 harness:skills 段
/// （注册即 effect；壳侧进程生命周期=根 fiber 生命周期，Disposer 不另行保管——
/// 根卸载时随树逆序清理，无 Drop 副作用）。空目录不注册（零技能零噪音）。
/// 返回是否注册（观测用）。
pub fn register_skill_catalog(root: &Context, cwd: &Path) -> bool {
    let text = catalog_section_text(&scan_skill_catalog(cwd));
    if text.is_empty() {
        return false;
    }
    let Ok(prompt) = root.get::<dyn yunyou_system_prompt::SystemPrompt>(
        ServiceId::of::<dyn yunyou_system_prompt::SystemPrompt>(),
    ) else {
        eprintln!("[技能] system-prompt 服务未就绪，目录段注册跳过");
        return false;
    };
    let section = yunyou_system_prompt::Section::new(
        SKILLS_SECTION_NAME,
        SKILLS_SECTION_ORDER,
        text,
    );
    match prompt.register_section(section) {
        Ok(_) => true,
        Err(err) => {
            eprintln!("[技能] 目录段注册失败（跳过，不影响命令注册）：{err}");
            false
        }
    }
}

/// 双槽扫描注册（REPL 装配后调用一次，壳侧职责——技能是宿主资源不是插件业务）。
/// 返回成功注册数（观测用；调用方按 --verbose 门控输出）。
pub fn register_skill_commands(root: &Context, cwd: &Path) -> usize {
    let Ok(commands) = root.get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
    else {
        eprintln!("[技能] commands 服务未就绪，技能注册跳过");
        return 0;
    };
    // .yunyou 优先进表（胜者先占位），.dsh 过渡槽随后（同名冲突被缝拒绝即跳过）
    let mut registered = 0usize;
    for slot_dir in [cwd.join(".yunyou").join("skills"), cwd.join(".dsh").join("skills")] {
        let Ok(entries) = std::fs::read_dir(&slot_dir) else { continue };
        let mut files: Vec<std::path::PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect();
        files.sort(); // 稳定输出：同目录内注册次序不随 read_dir 顺序漂移
        for path in files {
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            if !valid_name(stem) {
                eprintln!(
                    "[技能] 跳过 {}（命令名需 ^[a-z][a-z0-9_-]*$）",
                    path.display()
                );
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                eprintln!("[技能] 读取失败 {}（跳过）", path.display());
                continue;
            };
            let (alias, body) = split_frontmatter(&text);
            let Some(description) = first_heading(body) else {
                eprintln!("[技能] 跳过 {}（无一级标题可作描述）", path.display());
                continue;
            };
            let body = body.to_string();
            let definition = CommandDefinition {
                name: stem.to_string(),
                aliases: alias.into_iter().collect(),
                description: description.to_string(),
                kind: CommandKind::Expand,
                accepts_args: false,
                handler: Arc::new(move |_inv| {
                    let body = body.clone();
                    Box::pin(async move { Ok(CommandOutput::success(body)) })
                        as BoxFut<'static, yunyou::Result<CommandOutput>>
                }),
            };
            match commands.register(definition) {
                Ok(_) => registered += 1,
                Err(err) => eprintln!(
                    "[技能] /{stem} 注册冲突跳过（同名 .yunyou 槽胜出或既有命令占用）：{err}"
                ),
            }
        }
    }
    registered
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_commands::CommandsPlugin;

    /// 造技能槽测试场：真 CommandsPlugin 装配 + temp 双槽文件。
    async fn setup(dir: &Path) -> Context {
        let root = Context::new();
        let handle = root.plugin(CommandsPlugin, serde_json::json!({}));
        handle.await_ready().await.expect("commands 插件应装载");
        let _ = dir;
        root
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().expect("有父目录")).expect("建槽目录");
        std::fs::write(path, content).expect("写技能文件");
    }

    /// 全链路：合法名注册+frontmatter alias 剥离+正文含标题；非法名跳过；
    /// 双槽同名 .yunyou 胜（.dsh 同名被缝拒）；仅 .dsh 有=过渡兼容注册；
    /// 重复扫描幂等（同名全冲突跳过，计数零增）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 双槽扫描注册与同名优先与非法名守卫() {
        let dir = std::env::temp_dir().join(format!("yunyou-skills-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        write(
            &dir,
            ".yunyou/skills/deploy.md",
            "---\nalias: 部署\n---\n# 部署指南\n按步骤部署服务。",
        );
        write(&dir, ".yunyou/skills/Bad_Name.md", "# 非法名\n正文");
        write(&dir, ".dsh/skills/deploy.md", "# 旧槽版本\n.dsh 正文");
        write(&dir, ".dsh/skills/legacy.md", "# 遗留技能\n过渡兼容正文");
        write(&dir, ".dsh/skills/noheading.md", "没有标题的技能");
        let root = setup(&dir).await;
        let registered = register_skill_commands(&root, &dir);
        assert_eq!(registered, 2, "deploy+legacy 注册，非法名/无标题/同名拒各一：{registered}");
        let commands = root
            .get::<dyn CommandsService>(ServiceId::of::<dyn CommandsService>())
            .expect("commands 服务");
        // .yunyou 胜：deploy 的描述是 .yunyou 版标题，别名来自 frontmatter
        let deploy = commands.find("deploy").expect("deploy 应已注册");
        assert_eq!(deploy.description, "部署指南", "同名时 .yunyou 槽胜出");
        assert_eq!(deploy.aliases, vec!["部署".to_string()], "frontmatter alias 进别名");
        assert!(deploy.kind == CommandKind::Expand, "技能命令归展开类（D1）");
        // 过渡兼容：仅 .dsh 槽有的 legacy 正常注册
        assert!(commands.find("legacy").is_some(), ".dsh 过渡槽应注册");
        // 非法名与无标题不进表
        assert!(commands.find("Bad_Name").is_none(), "非法名守卫");
        assert!(commands.find("noheading").is_none(), "无标题跳过");
        // 幂等：重复扫描全冲突跳过，注册数零增、目录不变
        let again = register_skill_commands(&root, &dir);
        assert_eq!(again, 0, "重复扫描应全冲突跳过");
        assert!(commands.find("deploy").is_some(), "重扫不丢命令");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-13① 目录扫描：双槽同名 .yunyou 胜、非法名/无标题跳过、输出按名稳定序。
    #[test]
    fn 目录扫描_同名优先_非法名跳过() {
        let dir = std::env::temp_dir().join(format!("yunyou-skill-catalog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        write(&dir, ".yunyou/skills/alpha.md", "# Alpha 技能\n正文");
        write(&dir, ".dsh/skills/alpha.md", "# 旧版 Alpha");
        write(&dir, ".dsh/skills/beta.md", "# Beta 技能");
        write(&dir, ".dsh/skills/Bad.md", "# 非法名");
        write(&dir, ".yunyou/skills/nohead.md", "无标题");
        let entries = scan_skill_catalog(&dir);
        assert_eq!(
            entries,
            vec![
                CatalogEntry { name: "alpha".into(), description: "Alpha 技能".into() },
                CatalogEntry { name: "beta".into(), description: "Beta 技能".into() },
            ],
            "同名 .yunyou 胜、非法名/无标题跳过、按名排序"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-13① 目录段文本：引导语+逐行条目；描述截断 60 字符；预算击穿截断+尾注。
    #[test]
    fn 目录段文本_引导语_截断_预算尾注() {
        // 常态：引导语+逐行，含 plugins-seams 指引
        let text = catalog_section_text(&[
            CatalogEntry { name: "plugins-seams".into(), description: "插件缝地图".into() },
            CatalogEntry { name: "deploy".into(), description: "部署指南".into() },
        ]);
        assert!(text.starts_with(CATALOG_GUIDE), "{text}");
        assert!(text.contains("- plugins-seams: 插件缝地图"), "{text}");
        assert!(text.contains("- deploy: 部署指南"), "{text}");
        // 描述超 60 字符截断加 …
        let long = "长".repeat(80);
        let text = catalog_section_text(&[CatalogEntry { name: "longy".into(), description: long }]);
        assert!(text.contains(&"长".repeat(60)) && text.contains("…"), "{text}");
        assert!(!text.contains(&"长".repeat(61)), "不得超 60 字符：{text}");
        // 预算击穿：大量技能 → 尾注计数、总文本不超预算
        let many: Vec<CatalogEntry> = (0..200)
            .map(|i| CatalogEntry {
                name: format!("skill{i:03}"),
                description: "描".repeat(50),
            })
            .collect();
        let text = catalog_section_text(&many);
        assert!(text.chars().count() <= CATALOG_BUDGET_CHARS + 100, "预算护栏：{}", text.chars().count());
        assert!(text.contains("个技能未列入"), "尾注计数：{text}");
        // 空目录=空段（不注册）
        assert_eq!(catalog_section_text(&[]), "");
    }

    /// frontmatter 形态边界：无 alias 键/空 alias 值/无结束 --- 均按原文或无别名处理。
    #[test]
    fn frontmatter剥离边界() {
        let (alias, body) = split_frontmatter("---\nalias: 中文名\n---\n# 标题\n正文");
        assert_eq!(alias.as_deref(), Some("中文名"));
        assert_eq!(body, "# 标题\n正文");
        let (alias, body) = split_frontmatter("---\nother: x\n---\n# 标题");
        assert_eq!(alias, None, "非 alias 键忽略");
        assert_eq!(body, "# 标题");
        let (alias, body) = split_frontmatter("---\nalias: \n---\n# 标题");
        assert_eq!(alias, None, "空 alias 值忽略");
        assert_eq!(body, "# 标题");
        let (alias, body) = split_frontmatter("# 无frontmatter\n---\n正文里的分隔线");
        assert_eq!(alias, None, "非 frontmatter 开头按原文");
        assert_eq!(body, "# 无frontmatter\n---\n正文里的分隔线");
        assert_eq!(first_heading("# 空"), Some("空"));
        assert_eq!(first_heading("没有标题"), None, "无标题 None");
        assert!(!valid_name("Bad"), "大写拒");
        assert!(!valid_name("1abc"), "数字开头拒");
        assert!(!valid_name("a b"), "空格拒");
        assert!(valid_name("a"));
        assert!(valid_name("deploy-server_1"));
    }
}
