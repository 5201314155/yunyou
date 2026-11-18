//! 人设分源测试（task-6 persona.md 既有回归 + task-25 AGENTS.md 四级链）。
//! Managed 层路径=/etc/yunyou（root 域，测试不触碰——以「不存在=跳过」为常态路径
//! 覆盖；构造法注入受阻于常量路径，链序与拼接用 user/project/local 三层实测）。

use super::*;
use yunyou_system_prompt::SystemPrompt;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// pid 隔离临时目录。
fn temp_dir(tag: &str) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("yunyou-persona-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn write(dir: &Path, rel: &str, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().ok_or("缺父目录")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// task-6 既有回归：persona.md 两级优先与空文件回落（行为不变）。
#[test]
fn persona_md_两级优先与空文件回落() -> TestResult {
    let root = temp_dir("prio")?;
    assert_eq!(resolve_persona_md(&root, &root.join("home")), None);
    write(&root.join("home"), "persona.md", "home 人设")?;
    assert_eq!(resolve_persona_md(&root, &root.join("home")).as_deref(), Some("home 人设"));
    write(&root, ".yunyou/persona.md", "project 人设")?;
    assert_eq!(resolve_persona_md(&root, &root.join("home")).as_deref(), Some("project 人设"));
    // 空文件=不存在
    write(&root, ".yunyou/persona.md", "  \n")?;
    assert_eq!(resolve_persona_md(&root, &root.join("home")).as_deref(), Some("home 人设"));
    Ok(())
}

/// task-25 验收①：四层各放不同内容 → 链序拼接 Managed→User→Project→Local，
/// 后层文本在前层之后（后层为准的拼接表达）；每层带 stub 锚。
#[test]
fn 四层链序拼接与锚() -> TestResult {
    let root = temp_dir("chain")?;
    let home = root.join("home");
    write(&home, "AGENTS.md", "用户全局规则")?;
    write(&root, "AGENTS.md", "项目规则")?;
    write(&root, ".yunyou/local/AGENTS.md", "本地私货")?;
    let layers = load_agents_chain(&root, &home);
    // /etc/yunyou 常态不存在 → 三层命中，层序 user→project→local
    assert_eq!(
        layers.iter().map(|l| l.kind).collect::<Vec<_>>(),
        vec!["user", "project", "local"],
        "层序：{:?}", layers.iter().map(|l| l.kind).collect::<Vec<_>>()
    );
    let text = compose_agents_text(&layers);
    let (u, p, l) = (
        text.find("[agents:user]").ok_or("user 锚")?,
        text.find("[agents:project]").ok_or("project 锚")?,
        text.find("[agents:local]").ok_or("local 锚")?,
    );
    assert!(u < p && p < l, "层序拼接：{text}");
    assert!(text.contains("用户全局规则") && text.contains("项目规则") && text.contains("本地私货"));
    Ok(())
}

/// task-25「比 claude 好」①：同规则多层重复 → 人话冲突尾注点名「后层为准」；
/// 标题行不视为冲突。
#[test]
fn 冲突检测尾注_标题不算冲突() -> TestResult {
    let root = temp_dir("conflict")?;
    let home = root.join("home");
    write(&home, "AGENTS.md", "# 规约\n回答要短。\n禁改库。")?;
    write(&root, "AGENTS.md", "# 规约\n禁改库。\n项目特有规则。")?;
    let layers = load_agents_chain(&root, &home);
    let text = compose_agents_text(&layers);
    assert!(text.contains("人设冲突提示") && text.contains("后层为准"), "{text}");
    assert!(text.contains("- 禁改库。"), "重复行点名：{text}");
    assert!(!text.contains("- # 规约"), "标题行不算冲突：{text}");
    assert!(!text.contains("- 回答要短。"), "单层行不点名：{text}");
    Ok(())
}

/// task-25 验收②：优先级写死=AGENTS 链（仅 user 层在）> persona.md > toml。
#[test]
fn 优先级_agents链压过persona压过toml() -> TestResult {
    let root = temp_dir("prio2")?;
    let home = root.join("home");
    // 全空 → None（回落 toml）
    assert_eq!(resolve_persona_text(&root, &home), None);
    // 只有 persona.md → persona 生效
    write(&home, "persona.md", "persona 人设")?;
    assert_eq!(resolve_persona_text(&root, &home).as_deref(), Some("persona 人设"));
    // AGENTS 链出现（user 层）→ shadow persona
    write(&home, "AGENTS.md", "agents 全局规则")?;
    let text = resolve_persona_text(&root, &home).ok_or("应命中")?;
    assert!(text.starts_with("[agents:user]"), "{text}");
    assert!(text.contains("agents 全局规则") && !text.contains("persona 人设"), "{text}");
    // local 层（最低层）存在时也在链内——链整体压 persona
    write(&root, ".yunyou/local/AGENTS.md", "本地微调")?;
    let text = resolve_persona_text(&root, &home).ok_or("应命中")?;
    assert!(text.contains("本地微调") && !text.contains("persona 人设"), "{text}");
    Ok(())
}

/// 空层文件=跳过不进链（trim 后判空）。
#[test]
fn 空层文件跳过() -> TestResult {
    let root = temp_dir("emptylayer")?;
    let home = root.join("home");
    write(&home, "AGENTS.md", "  \n")?;
    write(&root, "AGENTS.md", "项目规则")?;
    let layers = load_agents_chain(&root, &home);
    assert_eq!(layers.iter().map(|l| l.kind).collect::<Vec<_>>(), vec!["project"]);
    Ok(())
}

/// md 切段与装配联动（task-6 既有回归保留）。
#[test]
fn md人设切段进装配() -> TestResult {
    let md = "你是一个严谨的助手。\n# 红线\n永不删除用户数据。";
    let sections = yunyou_system_prompt::persona_md::split_md_sections(md);
    assert_eq!(sections.len(), 2);
    let prompt = yunyou_system_prompt::DefaultSystemPrompt::with_persona(md.to_string())?;
    let text = yunyou_system_prompt::SystemPrompt::compose(&prompt, "", &[]);
    let first = text.find("你是一个严谨的助手。").ok_or("段一应出现")?;
    let second = text.find("# 红线").ok_or("段二应出现")?;
    assert!(first < second, "段序应保文件顺序：{text}");
    Ok(())
}
