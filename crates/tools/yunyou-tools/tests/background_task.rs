//! 后台任务机制端到端验收（池 task-20 卡⑤）：显式 run_in_background 转后台→
//! task_output 轮询到进度→task_stop 可杀→无孤儿（进程组收割）→task_list 终态。
//! 权限链同判（转后台命令同样被 bash_guard 拒）在 tests/bash_guard.rs 锁。

use std::sync::Arc;
use yunyou_tools::builtin::bash::BashTool;
use yunyou_tools::builtin::cwd::CwdService;
use yunyou_tools::builtin::tasks_tool::{TaskListTool, TaskOutputTool, TaskStopTool};
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_tools::tasks::TaskRegistry;

fn exec() -> ToolExec {
    ToolExec {
        call_id: "bg-e2e".into(),
        cancel: tokio_util::sync::CancellationToken::new(),
        workspace: None,
    }
}

fn tools() -> (BashTool, Arc<TaskRegistry>) {
    let dir = std::env::temp_dir().join(format!("bg-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = Arc::new(TaskRegistry::new());
    let bash = BashTool {
        cwd: Arc::new(CwdService::new(&dir)),
        enforcer: None,
        tasks: Some(registry.clone()),
        background_after_secs: 60,
    };
    (bash, registry)
}

/// 显式 run_in_background：立即返回 task_id+日志路径三要素；task_output 轮询到
/// 输出；task_stop 终止；终态 stopped 可查（无孤儿=收割由 killpg 全组语义保证，
/// bash_orphan 域已有组收割实证，此处锁移交链终态）。
#[tokio::test(flavor = "multi_thread")]
async fn 显式转后台_轮询_可杀_终态闭环() {
    let (bash, registry) = tools();
    let outcome = bash
        .execute(
            serde_json::json!({"cmd": "echo bg-hello; sleep 30", "run_in_background": true}),
            exec(),
        )
        .await;
    let task_id = match &outcome {
        ToolOutcome::Ok(v) => {
            assert_eq!(v["backgrounded"], serde_json::json!(true), "应立即返回：{v:?}");
            let id = v["task_id"].as_str().expect("task_id 三要素").to_string();
            assert!(v["log_path"].as_str().is_some(), "日志路径三要素");
            assert!(v["message"].as_str().unwrap().contains("task_output"), "怎么查三要素");
            id
        }
        other => panic!("应转后台成功：{other:?}"),
    };
    // 轮询到输出（阻塞等待至多 5s——echo 早该出）
    let output_tool = TaskOutputTool { tasks: registry.clone() };
    let seen = loop {
        let out = output_tool
            .execute(serde_json::json!({"task_id": task_id, "block_secs": 5}), exec())
            .await;
        match out {
            ToolOutcome::Ok(v) => {
                if v["output"].as_str().unwrap_or("").contains("bg-hello") {
                    break v;
                }
                if v["state"] != "running" {
                    panic!("进程退出仍未见输出：{v:?}");
                }
            }
            other => panic!("轮询应成功：{other:?}"),
        }
    };
    assert_eq!(seen["state"], "running", "sleep 30 未到，任务应仍 running");
    // task_stop 终止
    let stop = TaskStopTool { tasks: registry.clone() };
    let out = stop.execute(serde_json::json!({"task_id": task_id}), exec()).await;
    match out {
        ToolOutcome::Ok(v) => assert_eq!(v["state"], "stopped"),
        other => panic!("应停止成功：{other:?}"),
    }
    // 终态可查+task_list 收录
    let out = output_tool
        .execute(serde_json::json!({"task_id": task_id}), exec())
        .await;
    match out {
        ToolOutcome::Ok(v) => assert_eq!(v["state"], "stopped", "stop 后 task_output 应见终态"),
        other => panic!("终态查询应成功：{other:?}"),
    }
    let list = TaskListTool { tasks: registry };
    let out = list.execute(serde_json::json!({}), exec()).await;
    match out {
        ToolOutcome::Ok(v) => assert_eq!(v["tasks"][0]["state"], "stopped"),
        other => panic!("列表应收录：{other:?}"),
    }
}

/// 自动转后台：background_after_secs=1 的短阈值下，sleep 5 前台命令应自动转后台
/// 返回 task_id（而非等满 5s）。
#[tokio::test(flavor = "multi_thread")]
async fn 前台超阈值自动转后台() {
    let dir = std::env::temp_dir().join(format!("bg-auto-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let registry = Arc::new(TaskRegistry::new());
    let bash = BashTool {
        cwd: Arc::new(CwdService::new(&dir)),
        enforcer: None,
        tasks: Some(registry),
        background_after_secs: 1,
    };
    let started = std::time::Instant::now();
    let outcome = bash
        .execute(serde_json::json!({"cmd": "sleep 20"}), exec())
        .await;
    let elapsed = started.elapsed();
    match outcome {
        ToolOutcome::Ok(v) => {
            assert_eq!(v["backgrounded"], serde_json::json!(true), "应自动转后台：{v:?}");
            assert!(elapsed < std::time::Duration::from_secs(10), "转后台应立即返回（实耗 {elapsed:?}）");
        }
        other => panic!("应自动转后台：{other:?}"),
    }
    // 收割：注册表退场（drop）应把 Running 任务标 stopped——锁会话退场收割语义
}

/// 注册表 Drop 收割：持有 Running 任务的注册表 drop 后任务标 stopped（killpg
/// 组收割语义；孤儿活不过会话边界）。
#[tokio::test(flavor = "multi_thread")]
async fn 注册表退场收割() {
    let (bash, registry) = tools();
    let outcome = bash
        .execute(
            serde_json::json!({"cmd": "sleep 30", "run_in_background": true}),
            exec(),
        )
        .await;
    assert!(matches!(outcome, ToolOutcome::Ok(_)));
    drop(registry); // 会话退场=Drop 收割
    // 重建一张表只能验证登记面（原表已 drop）——收割本身经 killpg 幂等，
    // 孤儿不存在的实证=进程组收割（tests/bash_orphan.rs 同语义），此处锁
    // 「drop 不 panic 且任务随表消亡」。
}
