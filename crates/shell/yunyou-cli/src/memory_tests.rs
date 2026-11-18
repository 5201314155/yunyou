//! memory.v1 行为测试（池 task-14；#[path] 外挂保持 memory.rs 主文件 ≤400——
//! 家规第四条先例=assemble_tests.rs）。验收五条：save→重启→search 命中→
//! forget 无残留→硬顶拒绝。

use super::*;
use tokio_util::sync::CancellationToken;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn exec() -> ToolExec {
    ToolExec { call_id: "t".into(), cancel: CancellationToken::new(), workspace: None }
}

fn store_at(tag: &str) -> (PathBuf, Arc<MemoryStore>) {
    let home = std::env::temp_dir().join(format!("yunyou-memory-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let store = Arc::new(MemoryStore::new(&home, Path::new("/home/wu/agent/yunyou")));
    (home, store)
}

async fn save(store: &Arc<MemoryStore>, topic: &str, content: &str) -> ToolOutcome {
    MemorySaveTool { store: Arc::clone(store) }
        .execute(json!({"scope": "project", "topic": topic, "content": content}), exec())
        .await
}

async fn forget(store: &Arc<MemoryStore>, topic: &str) -> ToolOutcome {
    MemoryForgetTool { store: Arc::clone(store) }
        .execute(json!({"scope": "project", "topic": topic}), exec())
        .await
}

/// 验收①+②：save 落文件+索引行；「重启」=新 MemoryStore 实例重读磁盘（同盘
/// 新实例即重启语义），索引段文本含来源标签与条目。
#[tokio::test]
async fn save_落盘索引_重启后注入可见() -> TestResult {
    let (home, store) = store_at("save");
    let outcome = save(&store, "deploy-notes", "# 部署结论\n先备份再动库。").await;
    let ToolOutcome::Ok(value) = outcome else { panic!("save 应成功：{outcome:?}") };
    assert_eq!(value["ok"], json!(true));
    let path = value["path"].as_str().ok_or("path 缺失")?;
    assert!(path.ends_with("deploy-notes.md"), "{path}");
    assert!(Path::new(path).exists(), "话题文件应落盘");
    assert_eq!(value["index_lines_used"], json!(1), "索引应恰 1 行");
    // 重启：新实例重读磁盘（不共享内存态——本 store 本就是纯磁盘读）
    let restarted = Arc::new(MemoryStore::new(
        &home,
        Path::new("/home/wu/agent/yunyou"),
    ));
    let text = restarted.section_text();
    assert!(text.contains("[记忆索引 · 来源"), "来源标签：{text}");
    assert!(text.contains("- [deploy-notes](deploy-notes.md)"), "索引条目：{text}");
    assert!(text.contains("先备份再动库"), "摘要取正文首行：{text}");
    let _ = std::fs::remove_dir_all(&home);
    Ok(())
}

/// 验收③：search 命中索引与正文；无命中空 hits。
#[tokio::test]
async fn search_命中索引与正文() -> TestResult {
    let (_home, store) = store_at("search");
    save(&store, "deploy-notes", "# 部署结论\n先备份再动库。").await;
    save(&store, "user-prefs", "用户偏好：回答要短。").await;
    let tool = MemorySearchTool { store: Arc::clone(&store) };
    let ToolOutcome::Ok(value) = tool
        .execute(json!({"scope": "project", "query": "备份"}), exec())
        .await
    else {
        panic!("search 应成功")
    };
    let hits = value["hits"].as_array().ok_or("hits 缺失")?;
    assert!(
        hits.iter().any(|h| h["topic"] == json!("deploy-notes")),
        "正文命中：{value}"
    );
    // 无命中：空 hits（ok 仍 true）
    let ToolOutcome::Ok(value) = tool
        .execute(json!({"scope": "project", "query": "绝不存在的词xyz"}), exec())
        .await
    else {
        panic!("search 应成功")
    };
    assert!(value["hits"].as_array().is_some_and(|h| h.is_empty()), "无命中空表：{value}");
    Ok(())
}

/// 验收⑤：forget 删文件+摘索引，无残留；未知名报错附清单。
#[tokio::test]
async fn forget_无残留_未知名报错() -> TestResult {
    let (_home, store) = store_at("forget");
    save(&store, "deploy-notes", "# 部署结论\n先备份再动库。").await;
    save(&store, "user-prefs", "用户偏好：回答要短。").await;
    let ToolOutcome::Ok(value) = forget(&store, "deploy-notes").await else {
        panic!("forget 应成功")
    };
    assert_eq!(value["index_lines_used"], json!(1), "索引应只剩 1 行");
    let text = store.section_text();
    assert!(!text.contains("deploy-notes"), "索引无残留：{text}");
    assert!(!store.index_path().display().to_string().is_empty() && !text.contains("先备份再动库"), "正文摘要无残留：{text}");
    // 未知名：人话报错+清单
    let ToolOutcome::Err(err) = forget(&store, "no-such").await else { panic!("未知名应报错") };
    assert!(err.message.contains("不存在"), "{err:?}");
    assert!(err.hint.as_deref().is_some_and(|h| h.contains("user-prefs")), "{err:?}");
    Ok(())
}

/// 验收②（硬顶拒绝）：索引打满 200 行后再 save=拒绝且人话报「请整理」；
/// 忘记不存在的条目不误删既有行。
#[tokio::test]
async fn 硬顶拒绝_人话报请整理() -> TestResult {
    let (_home, store) = store_at("cap");
    // 打满索引（200 行）
    let mut lines = Vec::new();
    for i in 0..200 {
        lines.push(format!("- [old{i:03}](old{i:03}.md) — 旧条目{i}"));
    }
    std::fs::create_dir_all(store.project_root())?;
    std::fs::write(store.index_path(), format!("{}\n", lines.join("\n")))?;
    // 再 save：拒绝且报「整理」，不静默截断
    let ToolOutcome::Err(err) = save(&store, "new-topic", "新记忆").await else {
        panic!("硬顶应拒绝")
    };
    assert!(err.message.contains("整理"), "{err:?}");
    assert_eq!(err.code, "memory/index-full", "{err:?}");
    // 磁盘索引未被截断（原 200 行原样）
    let on_disk = std::fs::read_to_string(store.index_path())?;
    assert_eq!(on_disk.lines().count(), 200, "禁静默截断");
    // forget 一条旧条目腾位后再 save：成功
    let ToolOutcome::Ok(_) = forget(&store, "old000").await else { panic!("forget 应成功") };
    let ToolOutcome::Ok(_) = save(&store, "new-topic", "新记忆").await else {
        panic!("腾位后 save 应成功")
    };
    Ok(())
}

/// 路径逃逸拒绝：topic 带分隔符/../首部点；global 作用域=候缝人话报错。
#[tokio::test]
async fn 路径逃逸拒_global候缝报错() -> TestResult {
    let (_home, store) = store_at("escape");
    for bad in ["../escape", "a/b", "C:\\temp", ".hidden", ".."] {
        let ToolOutcome::Err(err) = save(&store, bad, "x").await else {
            panic!("逃逸 {bad} 应拒")
        };
        assert!(err.message.contains("非法") || err.message.contains("拒绝"), "{bad}: {err:?}");
    }
    // global：人话候缝
    let tool = MemorySaveTool { store: Arc::clone(&store) };
    let ToolOutcome::Err(err) = tool
        .execute(json!({"scope": "global", "topic": "t", "content": "c"}), exec())
        .await
    else {
        panic!("global 应报候缝")
    };
    assert!(err.message.contains("候审批缝"), "{err:?}");
    Ok(())
}

/// 同 topic 覆盖正文但索引行去重（不重复累积）。
#[tokio::test]
async fn 同话题覆盖_索引去重() -> TestResult {
    let (_home, store) = store_at("dedupe");
    let ToolOutcome::Ok(_) = save(&store, "deploy-notes", "第一版").await else { panic!("首存应成功") };
    let ToolOutcome::Ok(v1) = save(&store, "deploy-notes", "第二版更新").await else {
        panic!("覆盖 save 应成功")
    };
    assert_eq!(v1["index_lines_used"], json!(1), "索引去重不累积");
    let text = std::fs::read_to_string(store.project_root().join("deploy-notes.md"))?;
    assert_eq!(text, "第二版更新", "正文覆盖");
    Ok(())
}
