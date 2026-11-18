//! K-2-① 验收：contract_read 对示范件跑通+validate 双态（合法/缺字段）。

use std::sync::Arc;
use yunyou_plugin_dev::{contract_read::ContractReadTool, validate::ValidateTool};
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};

fn exec() -> ToolExec {
    ToolExec { call_id: "t".into(), cancel: tokio_util::sync::CancellationToken::new(), workspace: None }
}

/// 临时工作区（tag 隔离：两测试并行跑共享 pid 目录会互删对方夹具——
/// contract_read_hello 实测锚；用完即清）。
fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("plugin-dev-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_read对示范件_契约面结构() {
    let ws = tempfile_dir("read-hello");
    std::fs::create_dir_all(ws.join("plugins/hello")).unwrap();
    std::fs::write(
        ws.join("plugins/hello/manifest.toml"),
        "id = \"hello\"\nversion = \"0.1.0\"\nlevel = \"Bundled\"\ncontracts = [\"tools.v1\"]\nauto_enable = true\n",
    )
    .unwrap();
    let root = Arc::from(ws.clone());
    let tool = ContractReadTool { root };
    let outcome = tool.execute(serde_json::json!({"path": "plugins/hello"}), exec()).await;
    match &outcome {
        ToolOutcome::Ok(v) => {
            assert_eq!(v["id"].as_str(), Some("hello"), "契约面 id=hello：{v:?}");
            assert!(v["contracts"].is_array(), "契约面 contracts 数组：{v:?}");
            assert_eq!(v["auto_enable"], serde_json::json!(true), "auto_enable 布尔：{v:?}");
        }
        other => panic!("contract_read 应 Ok：{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_合法件_valid_true() {
    let ws = tempfile_dir("validate-good");
    std::fs::create_dir_all(ws.join("plugins/good")).unwrap();
    std::fs::write(
        ws.join("plugins/good/manifest.toml"),
        "id = \"good\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\n",
    )
    .unwrap();
    std::fs::write(ws.join("plugins/good/Cargo.toml"), "[package]\nname=\"good\"\nversion=\"0.1.0\"\n").unwrap();
    let root = Arc::from(ws.clone());
    let tool = ValidateTool { root };
    let outcome = tool.execute(serde_json::json!({"path": "plugins/good"}), exec()).await;
    match &outcome {
        ToolOutcome::Ok(v) => {
            assert_eq!(v["valid"], serde_json::json!(true), "合法件 valid：{v:?}");
            assert_eq!(v["issues"].as_array().map(|a| a.len()), Some(0), "零 issues：{v:?}");
        }
        other => panic!("validate 应 Ok：{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&ws);
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_缺字段_issues逐条() {
    let ws = tempfile_dir("validate-bad");
    std::fs::create_dir_all(ws.join("plugins/bad")).unwrap();
    std::fs::write(ws.join("plugins/bad/manifest.toml"), "id = \"bad\"\n").unwrap();
    let root = Arc::from(ws.clone());
    let tool = ValidateTool { root };
    let outcome = tool.execute(serde_json::json!({"path": "plugins/bad"}), exec()).await;
    match &outcome {
        ToolOutcome::Ok(v) => {
            assert_eq!(v["valid"], serde_json::json!(false), "缺字段 valid=false：{v:?}");
            let issues = v["issues"].as_array().expect("issues 数组");
            assert!(issues.iter().any(|i| i.as_str().unwrap_or("").contains("version")), "缺 version 应报：{issues:?}");
            assert!(issues.iter().any(|i| i.as_str().unwrap_or("").contains("contracts")), "缺 contracts 应报：{issues:?}");
        }
        other => panic!("validate 应 Ok：{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&ws);
}
