//! bash 工具孤儿收割测试（task-14：挑刺 P1-1+云垒 P2-9 的进程树断言）。
//!
//! 收割契约：`process_group(0)` 后子进程自为组长，取消/正常完成/上层放弃
//! （abort future→GroupReaper drop）三路径全组 SIGKILL——组内孙进程死透
//! 不复用孤儿。断言形态=命令内 `echo $! > pidfile` 记后台孙 pid，工具
//! 返回后轮询 `kill(pid, 0)` 至 ESRCH（死透=彻底消失，非僵尸）。
//!
//! 防挂死护栏：每场景外层 `tokio::time::timeout`——修复前的行为是工具
//! 永挂（遗留组员持管道 read_to_end 永不 EOF），护栏让该回归以「超时
//! panic」显式失败而非测试卡死。

#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use yunyou_tools::builtin::bash::BashTool;
use yunyou_tools::builtin::cwd::CwdService;
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};

fn exec(cancel: tokio_util::sync::CancellationToken) -> ToolExec {
    ToolExec { call_id: "orphan".into(), cancel, workspace: None }
}

fn tool(tag: &str) -> (BashTool, PathBuf) {
    let dir = std::env::temp_dir().join(format!("yunyou-tools-orphan-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    (BashTool { cwd: Arc::new(CwdService::new(&dir)), enforcer: None , tasks: None, background_after_secs: 60 }, dir)
}

/// pidfile 里记的后台孙进程已死透（kill(pid,0)→ESRCH），3s 轮询上限。
fn assert_pid_dead(pidfile: &std::path::Path, what: &str) {
    let raw = std::fs::read_to_string(pidfile).unwrap_or_else(|e| panic!("{what} pidfile 缺失: {e}"));
    let pid: i32 = raw.trim().parse().unwrap_or_else(|e| panic!("{what} pid 解析失败({raw:?}): {e}"));
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if !alive {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what} 未被收割：pid={pid} 仍存活（killpg 组收割未生效）"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 场景①取消路径：`sleep 30 & wait` 前台 bash 挂等后台孙，工具态取消——
/// bash 与 sleep 同组，取消必须连孙一起收割（云勘原孤儿场景）。
#[tokio::test]
async fn 取消路径孙进程死透() {
    let (tool, dir) = tool("cancel");
    let pidfile = dir.join("sun.pid");
    // pidfile 落工作区根内相对路径（cwd=workspace root）：绝对 /tmp 重定向
    // 会被 D-17-1 门禁 fail-closed 拦截（测试第一版实证），相对路径门禁放行
    let cmd = "sleep 30 & echo $! > sun.pid; wait".to_string();
    let cancel = tokio_util::sync::CancellationToken::new();
    let task = tokio::spawn(tool.execute(serde_json::json!({"cmd": cmd}), exec(cancel.clone())));
    tokio::time::sleep(Duration::from_millis(300)).await;
    cancel.cancel();
    // timeout 外层=Elapsed（不挂死护栏），内层=JoinError（任务不应 panic）
    let outcome = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("取消后工具应返回（不挂死）")
        .expect("工具任务不应 panic");
    match &outcome {
        ToolOutcome::Err(err) => assert_eq!(err.code, "tool/cancelled", "{outcome:?}"),
        other => panic!("应取消: {other:?}"),
    }
    assert_pid_dead(&pidfile, "取消路径孙进程");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 场景②正常完成路径：bash 提交后台后立即退出（exit 0），sleep 成为
/// 持管道的遗留组员——正常完成也必须收割（组长已死 killpg 命中组员）。
#[tokio::test]
async fn 正常完成后台遗留死透() {
    let (tool, dir) = tool("done");
    let pidfile = dir.join("sun.pid");
    let cmd = "sleep 30 & echo $! > sun.pid; echo done".to_string();
    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        tool.execute(serde_json::json!({"cmd": cmd}), exec(tokio_util::sync::CancellationToken::new())),
    )
    .await
    .expect("正常完成应返回（遗留组员持管道不致挂死=P2-9 治标生效）");
    match &outcome {
        ToolOutcome::Ok(value) => {
            assert_eq!(value["exit_code"], 0, "落款完整: {value:?}");
            assert!(value["output"].as_str().unwrap().contains("done"));
        }
        other => panic!("应成功: {other:?}"),
    }
    assert_pid_dead(&pidfile, "正常完成路径后台遗留");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 场景③上层放弃路径（drop/超时的模拟形态）：abort 执行 future——
/// GroupReaper 随 future drop 收割全组（kill_on_drop 直子盲区的补位验证）。
#[tokio::test]
async fn abort_future全组收割() {
    let (tool, dir) = tool("abort");
    let pidfile = dir.join("sun.pid");
    let cmd = "sleep 30 & echo $! > sun.pid; wait".to_string();
    let handle = tokio::spawn(
        tool.execute(serde_json::json!({"cmd": cmd}), exec(tokio_util::sync::CancellationToken::new())),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle.abort();
    let _ = handle.await;
    assert_pid_dead(&pidfile, "abort/drop 路径孙进程");
    let _ = std::fs::remove_dir_all(&dir);
}

/// 场景④ executor 接线级（task-14 R2，云勘真机打回项）：execute_tool 超时
/// 分支必须给工具发信号——修复前超时只返回 Err 不 cancel，bash 的 killpg
/// 挂点够不着=「执行超时」上屏后进程树双活到自然结束。修复=每工具子令牌
/// （child_token 派生）+超时分支显式 cancel，再返回 tool/timeout。
#[tokio::test(flavor = "multi_thread")]
async fn executor超时触发工具组收割() {
    let (bash, dir) = tool("exec-timeout");
    let cmd = "sleep 30 & echo $! > sun.pid; wait".to_string();
    let outcome = yunyou_tools::executor::execute_tool(
        std::sync::Arc::new(bash),
        serde_json::json!({"cmd": cmd}),
        "exec-timeout".into(),
        tokio_util::sync::CancellationToken::new(),
        Some(Duration::from_millis(800)), // 工具超时远小于 sleep 30
    )
    .await;
    match &outcome {
        ToolOutcome::Err(err) => assert_eq!(err.code, "tool/timeout", "{outcome:?}"),
        other => panic!("应超时: {other:?}"),
    }
    // 超时上屏的同时进程树必须死透：child.cancel() → bash 取消分支 killpg
    assert_pid_dead(&dir.join("sun.pid"), "executor 超时路径孙进程");
    let _ = std::fs::remove_dir_all(&dir);
}
