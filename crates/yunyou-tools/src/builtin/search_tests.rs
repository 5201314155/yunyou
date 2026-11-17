//! search 单元测试外挂（task-72 截断标注+既有 glob/grep/t70 CJK 全集；
//! #[path] 先例，保 search.rs ≤400——家规硬顶）。
use super::*;
use crate::builtin::cwd::CwdService;
use crate::contract::{ToolDefinition, ToolExec};
use std::sync::Arc;

#[cfg(test)]
mod unit {
    use super::*;
    use crate::contract::ToolError;
    use tokio_util::sync::CancellationToken;

    fn exec() -> ToolExec {
        ToolExec { call_id: "t".into(), cancel: CancellationToken::new(), workspace: None }
    }


    /// glob 与 grep 全链路。
    #[tokio::test]
    async fn 检索工具全链路() {
        let dir = std::env::temp_dir().join(format!("yunyou-tools-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn main() {}\n// TODO 检查\n").unwrap();
        std::fs::write(dir.join("src/deep/b.rs"), "struct Demo;\n").unwrap();
        let cwd = Arc::new(CwdService::new(&dir));

        let glob = GlobTool { cwd: cwd.clone() };
        let outcome = glob.execute(serde_json::json!({"pattern": "src/**/*.rs"}), exec()).await;
        match outcome {
            ToolOutcome::Ok(value) => assert_eq!(value["count"], 2),
            other => panic!("glob 应成功: {other:?}"),
        }

        let grep = GrepTool { cwd: cwd.clone() };
        let outcome = grep.execute(serde_json::json!({"pattern": "TODO"}), exec()).await;
        match outcome {
            ToolOutcome::Ok(value) => {
                let matches = value["matches"].as_array().unwrap();
                assert_eq!(matches.len(), 1);
                assert!(matches[0].as_str().unwrap().starts_with("src/a.rs:2:"));
            }
            other => panic!("grep 应成功: {other:?}"),
        }
        // 越界子目录拒绝
        let outcome = grep
            .execute(serde_json::json!({"pattern": "x", "subdir": "../../etc"}), exec())
            .await;
        assert!(matches!(&outcome, ToolOutcome::Err(ToolError { code, .. }) if code == "tool/path-outside-root"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// task-52 全族单义锁：bash cd（pwd 捕获更新）后 glob/grep 仍锚工作区根
    /// ——与 fs 三件同义，检索相对语义不随当前目录漂移。
    #[tokio::test]
    async fn 检索工具根锚定不随当前目录漂移() {
        let dir = std::env::temp_dir()
            .join(format!("yunyou-tools-search-anchor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("src/deep")).unwrap();
        std::fs::write(dir.join("src/a.rs"), "fn main() {}\n// TODO 检查\n").unwrap();
        std::fs::write(dir.join("src/deep/b.rs"), "struct Demo;\n").unwrap();
        let cwd = Arc::new(CwdService::new(&dir));
        // 模拟 bash cd sub：pwd 捕获更新共享视图（bash spawn 基准；检索工具不再消费）
        assert!(cwd.set_captured(dir.join("sub").to_str().unwrap()));

        let glob = GlobTool { cwd: cwd.clone() };
        let outcome = glob.execute(serde_json::json!({"pattern": "src/**/*.rs"}), exec()).await;
        match outcome {
            ToolOutcome::Ok(value) => {
                assert_eq!(value["count"], 2, "glob 应锚根（锚 cwd 则 sub 下无 src 计 0）");
            }
            other => panic!("glob 应成功: {other:?}"),
        }
        let grep = GrepTool { cwd: cwd.clone() };
        let outcome = grep.execute(serde_json::json!({"pattern": "TODO"}), exec()).await;
        match outcome {
            ToolOutcome::Ok(value) => assert_eq!(value["count"], 1, "grep 应锚根"),
            other => panic!("grep 应成功: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

mod t70_cjk_tests {
    //! task-70③：glob/grep CJK 文件名/内容用例（五工具族锁之三、之四）。
    use super::*;
    use crate::builtin::cwd::CwdService;
    use crate::contract::{ToolDefinition, ToolExec};
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn exec() -> ToolExec {
        ToolExec { call_id: "t70".into(), cancel: CancellationToken::new(), workspace: None }
    }

    #[tokio::test]
    async fn cjk文件名glob与grep() {
        let home = std::env::temp_dir().join(format!("t70s-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("docs/设计")).unwrap();
        std::fs::write(home.join("docs/设计/视觉评审席-20261116.md"), "挑刺要点：中文正文\n").unwrap();
        let cwd = Arc::new(CwdService::new(&home));
        let glob = GlobTool { cwd: cwd.clone() };
        if let ToolOutcome::Ok(v) = glob
            .execute(serde_json::json!({"pattern": "docs/设计/*.md"}), exec())
            .await
        {
            let hits = v["matches"].as_array().map(|a| a.len()).unwrap_or(0);
            assert_eq!(hits, 1, "CJK 目录 glob 应命中: {v:?}");
        } else {
            panic!("CJK glob 应成功");
        }
        let grep = GrepTool { cwd };
        if let ToolOutcome::Ok(v) = grep
            .execute(serde_json::json!({"pattern": "挑刺要点", "path": "docs/设计"}), exec())
            .await
        {
            let hits = v["matches"].as_array().map(|a| a.len()).unwrap_or(0);
            assert!(hits >= 1, "CJK 内容 grep 应命中: {v:?}");
        } else {
            panic!("CJK grep 应成功");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn glob截断标注与小结果零回归() {
        let home = std::env::temp_dir().join(format!("t72-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("big")).unwrap();
        for i in 0..251 {
            std::fs::write(home.join(format!("big/f{i:03}.txt")), "x").unwrap();
        }
        let cwd = Arc::new(CwdService::new(&home));
        let glob = GlobTool { cwd: cwd.clone() };
        if let ToolOutcome::Ok(v) = glob
            .execute(serde_json::json!({"pattern": "big/*.txt"}), exec())
            .await
        {
            assert_eq!(v["truncated"].as_bool(), Some(true), "251 文件应标注截断");
            assert_eq!(v["count"].as_i64(), Some(200));
            assert!(v["summary"].as_str().unwrap().contains("已截断"), "{v:?}");
        } else {
            panic!("glob 应成功");
        }
        let glob2 = GlobTool { cwd };
        if let ToolOutcome::Ok(v) = glob2
            .execute(serde_json::json!({"pattern": "big/f000*.txt"}), exec())
            .await
        {
            assert_eq!(v["truncated"].as_bool(), Some(false), "小结果零变化");
        } else {
            panic!("小结果应成功");
        }
        let _ = std::fs::remove_dir_all(&home);
    }
}
