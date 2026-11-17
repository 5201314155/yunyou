//! fs 工具单元测试（task-33⑤a 同款 #[path] 外挂：fs_tools.rs 主体逼近
//! 400 硬顶——task-50 四用例+全链路测试外迁，零语义改动；私有项经
//! `use super::*` 同 crate 可达）。

    use super::*;
    use std::path::{Path, PathBuf};
    use tokio_util::sync::CancellationToken;

    fn exec() -> ToolExec {
        ToolExec { call_id: "t".into(), cancel: CancellationToken::new(), workspace: None }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunyou-tools-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// read/write/edit 全链路（临时目录内；共享 CWD 服务注入）。
    #[tokio::test]
    async fn 文件工具全链路() {
        let dir = temp_root("chain");
        std::fs::create_dir_all(dir.join("src")).expect("前置目录");
        let cwd = Arc::new(CwdService::new(&dir));

        let write = WriteTool { cwd: cwd.clone() };
        let outcome = write
            .execute(serde_json::json!({"path": "src/a.txt", "content": "hello 世界"}), exec())
            .await;
        assert!(matches!(outcome, ToolOutcome::Ok(_)), "{outcome:?}");

        let read = ReadTool { cwd: cwd.clone() };
        let outcome = read.execute(serde_json::json!({"path": "src/a.txt"}), exec()).await;
        match outcome {
            ToolOutcome::Ok(value) => {
                let content = value["content"].as_str().unwrap();
                assert!(content.contains("1\thello 世界"));
            }
            other => panic!("应读到: {other:?}"),
        }

        let edit = EditTool { cwd: cwd.clone() };
        let outcome = edit
            .execute(serde_json::json!({"path": "src/a.txt", "old": "hello", "new": "haha"}), exec())
            .await;
        assert!(matches!(outcome, ToolOutcome::Ok(_)));

        // 多匹配拒绝（"a" 在 haha 中出现两次）+ 越界拒绝（带指路 hint）+ 越界写拒绝
        let outcome = edit
            .execute(serde_json::json!({"path": "src/a.txt", "old": "a", "new": "x"}), exec())
            .await;
        assert!(matches!(&outcome, ToolOutcome::Err(err) if err.code == "tool/ambiguous"));
        let outcome = read.execute(serde_json::json!({"path": "../../etc/passwd"}), exec()).await;
        match &outcome {
            ToolOutcome::Err(err) => {
                assert_eq!(err.code, "tool/path-outside-root");
                assert_eq!(err.hint.as_deref(), Some(HINT_PATH_OUTSIDE_ROOT), "越界错误必须带指路 hint");
            }
            other => panic!("应拒绝: {other:?}"),
        }
        let outcome = write
            .execute(serde_json::json!({"path": "/etc/passwd", "content": "x"}), exec())
            .await;
        assert!(matches!(&outcome, ToolOutcome::Err(err) if err.code == "tool/path-outside-root"));
        let _ = std::fs::remove_dir_all(&dir);
    }


// ── task-50 四用例（业主真机实锤：缺目录静默建树+相对路径不透明+结果无绝对路径）──

/// 用例 a：缺父目录写入=拒绝且不建树（业主定调「缺目录就该报错」——模型按
/// 报错自纠先建目录，诚实闭环；拒绝路径上绝不静默 create_dir_all）。
#[tokio::test]
async fn 缺父目录写入拒绝且不建目录() {
    let dir = temp_root("a");
    let cwd = Arc::new(CwdService::new(&dir));
    let write = WriteTool { cwd };
    let outcome = write
        .execute(
            serde_json::json!({"path": "docs/设计/报告.md", "content": "长中文内容"}),
            exec(),
        )
        .await;
    match &outcome {
        ToolOutcome::Err(err) => {
            assert!(err.message.contains("目录不存在"), "人话报错: {err:?}");
            assert!(err.message.contains("报告.md"), "报错含解析后绝对路径: {err:?}");
            assert!(err.message.contains("mkdir"), "报错含行动项: {err:?}");
        }
        other => panic!("缺父目录必须拒绝: {other:?}"),
    }
    assert!(!dir.join("docs").exists(), "拒绝路径不得静默创建目录");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例 b：结果回显绝对路径（写歪当场暴露）+ 人话句（成功写入 X 字节到：<绝对路径>）。
#[tokio::test]
async fn 写入结果回显绝对路径() {
    let dir = temp_root("b");
    std::fs::create_dir_all(dir.join("docs/设计")).unwrap();
    let cwd = Arc::new(CwdService::new(&dir));
    let write = WriteTool { cwd };
    let outcome = write
        .execute(
            serde_json::json!({"path": "docs/设计/报告.md", "content": "正文"}),
            exec(),
        )
        .await;
    match &outcome {
        ToolOutcome::Ok(value) => {
            let path = value["path"].as_str().expect("结果应含 path");
            let path = Path::new(path);
            assert!(path.is_absolute(), "必须回显绝对路径: {path:?}");
            assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("报告.md"));
            assert!(path.starts_with(&dir), "应落在工作区根内: {path:?}");
            assert_eq!(value["written_bytes"], 6, "「正文」= 2 字 × 3 字节 = 6 字节（UTF-8）");
            assert!(
                value["summary"].as_str().unwrap().contains("成功写入"),
                "人话句应在结果中: {value}"
            );
        }
        other => panic!("应成功: {other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例 c：相对路径锚定工作区根、不随 bash 当前目录漂移（rev3 语义改版；
/// 业主原案复测：写 docs/设计/ 子目录，落位=根下而非当前目录下）。
#[tokio::test]
async fn 相对路径锚定工作区根不随当前目录漂移() {
    let dir = temp_root("c");
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let cwd = Arc::new(CwdService::new(&dir));
    // 模拟 bash cd sub：pwd 捕获更新共享视图（既有机制；rev3 后 fs 工具不再消费它）
    assert!(cwd.set_captured(&dir.join("sub").display().to_string()));
    let write = WriteTool { cwd: cwd.clone() };
    // docs/设计 不存在 → 拒绝，且报错路径应为「根/docs/设计/报告.md」而非「sub/docs/…」
    let outcome = write
        .execute(
            serde_json::json!({"path": "docs/设计/报告.md", "content": "x"}),
            exec(),
        )
        .await;
    match &outcome {
        ToolOutcome::Err(err) => {
            assert!(
                err.message.contains(&dir.join("docs").display().to_string()),
                "报错路径应锚定工作区根: {err:?}"
            );
        }
        other => panic!("缺目录应拒绝: {other:?}"),
    }
    // 建目录后写 → 落位根/docs/设计/报告.md；sub 下不得出现副本
    std::fs::create_dir_all(dir.join("docs/设计")).unwrap();
    let outcome = write
        .execute(
            serde_json::json!({"path": "docs/设计/报告.md", "content": "x"}),
            exec(),
        )
        .await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)), "{outcome:?}");
    assert!(dir.join("docs/设计/报告.md").is_file(), "应落位工作区根下");
    assert!(!dir.join("sub/docs").exists(), "不得落位当前目录下");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 用例 d：symlink 越界被运行时 canonicalize 闸拦截（词法检查对符号链接
/// 盲视——「link/escape.txt」无 .. 词法分量，旧闸放行即根外落盘；rev3 的
/// canonicalize 唯一化+starts_with 是强于词法的运行时闸）。
#[cfg(unix)]
#[tokio::test]
async fn 符号链接越界被运行时校验拦截() {
    let dir = temp_root("d");
    let outside = std::env::temp_dir().join(format!("yunyou-outside-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("link")).unwrap();
    let cwd = Arc::new(CwdService::new(&dir));
    let write = WriteTool { cwd };
    let outcome = write
        .execute(
            serde_json::json!({"path": "link/escape.txt", "content": "x"}),
            exec(),
        )
        .await;
    match &outcome {
        ToolOutcome::Err(err) => {
            assert_eq!(err.code, "tool/path-outside-root", "{err:?}");
        }
        other => panic!("symlink 越界必须拒: {other:?}"),
    }
    assert!(!outside.join("escape.txt").exists(), "根外不得落盘");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&outside);
}

/// task-70 取证（P0）：CJK 文件名 write 落盘取证——两变体（ASCII 连字符/
/// 全角连字符）write→stat 对账+目录字节级列举。根因定位后本测试转行为锁。
#[tokio::test]
async fn cjk文件名落盘取证() {
    let home = std::env::temp_dir().join(format!("t70-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("docs/设计")).unwrap();
    let cwd = Arc::new(CwdService::new(&home));
    let write = WriteTool { cwd };
    let content = "x".repeat(100);
    for (tag, name) in [
        ("ascii", "docs/设计/UI挑刺报告6-视觉评审席-20261116.md"),
        ("fullwidth", "docs/设计/UI挑刺报告6－视觉评审席－20261116.md"),
    ] {
        let out = write
            .execute(
                serde_json::json!({ "path": name, "content": content }),
                exec(),
            )
            .await;
        // task-70 行为锁：回显字节数=盘上字节数（对账纪律）+读回一致（落盘事实）
        let full = home.join(name);
        let stat = tokio::fs::metadata(&full).await.unwrap();
        assert_eq!(stat.len(), 100, "{tag} 落盘对账");
        if let ToolOutcome::Ok(v) = &out {
            assert_eq!(v["written_bytes"].as_u64(), Some(100), "{tag} 回显=盘上");
        } else {
            panic!("{tag} 应成功: {out:?}");
        }
        let read_back = std::fs::read_to_string(&full).unwrap();
        assert_eq!(read_back, content, "{tag} 读回一致");
    }
    for entry in std::fs::read_dir(home.join("docs/设计")).unwrap() {
        let e = entry.unwrap();
        println!("dir-entry: {:x?}", e.file_name().as_encoded_bytes());
    }
    let _ = std::fs::remove_dir_all(&home);
}

/// task-70③：CJK 文件名 read/edit 全链（写→读→改→读回，五工具族锁之二）。
#[tokio::test]
async fn cjk文件名read_edit全链() {
    let home = std::env::temp_dir().join(format!("t70re-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let cwd = Arc::new(CwdService::new(&home));
    let name = "docs/设计/视觉评审席-20261116.md";
    std::fs::create_dir_all(home.join("docs/设计")).unwrap();
    std::fs::write(home.join(name), "原始内容\n").unwrap();
    let read = ReadTool { cwd: cwd.clone() };
    if let ToolOutcome::Ok(v) = read
        .execute(serde_json::json!({"path": name}), exec())
        .await
    {
        assert!(v["content"].as_str().unwrap().contains("原始内容"));
    } else {
        panic!("CJK read 应成功");
    }
    let edit = EditTool { cwd };
    if let ToolOutcome::Ok(v) = edit
        .execute(
            serde_json::json!({"path": name, "old": "原始内容", "new": "修订内容"}),
            exec(),
        )
        .await
    {
        assert_eq!(v["replaced"].as_i64(), Some(1));
    } else {
        panic!("CJK edit 应成功");
    }
    assert!(std::fs::read_to_string(home.join(name)).unwrap().contains("修订内容"));
    let _ = std::fs::remove_dir_all(&home);
}
