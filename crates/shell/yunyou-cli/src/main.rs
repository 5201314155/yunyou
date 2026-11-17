//! yunyou 二进制入口（薄壳）：参数解析 + 运行时构建 + 总控调度，逻辑全在 lib。

use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    // 沙箱 helper 自唤起分派（task-24，设计 §3.4 C 案）：arg0 哨兵命中即应用
    // Landlock+seccomp 后 execvp 用户命令（永不返回）；未命中=正常 CLI 路径。
    // 必须先于 clap 解析——finder 隐患防线：helper 参数集（--policy-json 等）
    // 不进 clap 参数面，哨兵分支不可达=策略喂 clap 的事故形态被入口卡死。
    if let Some(code) = yunyou_sandbox::helper::try_helper(&std::env::args().collect::<Vec<_>>()) {
        return code;
    }
    // task-38②：clap 原生报错为英文，与全中文 UI 混排——参数错误给中文一行+
    // 指引（详情保留 clap 原文以定位具体参数）；帮助/版本请求原样放行。
    let command = match yunyou_cli::cli::Cli::try_parse() {
        Ok(command) => command,
        Err(err) => {
            use clap::error::ErrorKind as ClapKind;
            if matches!(err.kind(), ClapKind::DisplayHelp | ClapKind::DisplayVersion) {
                err.exit();
            }
            eprintln!("参数错误：无法解析命令行参数（yunyou --help 查看用法）");
            eprintln!("{err}");
            return ExitCode::from(2);
        }
    };
    // 多线程调度（方案 §2.3）：enable_all 补齐 I/O 与信号驱动
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("yunyou：tokio 运行时构建失败：{err}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(yunyou_cli::run(command));
    // task-61 v2（云点裁决③）：runtime.shutdown_timeout(500ms)——系统性兜底
    // 所有异步 IO 的退出竞态（不只 Ctrl-C 落盘）：SessionStore append 的 tokio
    // 缓冲 flush 在瞬时 exit 130 竞态下可能未落介质；shutdown_timeout 让
    // pending 的 spawn_blocking/异步写有窗口完成（正常退出无 pending=无感，
    // 不破「秒退」判据）。drop 前显式 shutdown 而非 implicit drop（后者无限等）。
    runtime.shutdown_timeout(std::time::Duration::from_millis(500));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        // task-59a：取消走 130 惯例（SIGINT=128+2，POSIX）——取消消息已由
        // 流内单点打印（双打印去重），此处静默给码；失败仍 1。
        Err(yunyou_cli::AppError::Interrupted) => ExitCode::from(130),
        Err(err) => {
            eprintln!("yunyou：{err}");
            ExitCode::FAILURE
        }
    }
}
