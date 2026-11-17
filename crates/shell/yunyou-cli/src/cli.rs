//! 命令行参数与 yunyou-home 解析（对标 boot 的宿主参数面）。

use clap::{Parser, Subcommand};
use std::path::PathBuf;


/// task-26④：版本行运行时读取——build.rs 每次构建写 OUT_DIR/version.txt，
/// --version 时 fs 读=恒拿到当次构建值（const/option_env 路径均陈旧，cargo
/// 不追踪 OUT_DIR/rustc-env 为重编触发，已实证）。leak=进程生命周期常驻。
pub fn version_line() -> String {
    let path = concat!(env!("OUT_DIR"), "/version.txt");
    std::fs::read_to_string(path)
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").to_string())
}

/// yunyou 二进制命令行参数。
#[derive(Debug, Parser)]
#[command(
    name = "yunyou",
    version = crate::cli::version_line().leak() as &'static str,
    about = "yunyou——终端里的 AI 对话助手",
    after_help = "示例：
  yunyou                  进入交互对话（每次启动全新会话）；/help 查看命令，/quit 退出
  yunyou --continue       续最近一条会话（对齐 claude -c）
  yunyou --resume         交互选择历史会话恢复（对齐 claude -r；带参=按名/序号/路径）
  yunyou --task 总结这段  单句任务：回答一句后退出（适合脚本调用）
  yunyou web              浏览器界面（终端打印带 token 的访问地址，Ctrl-C 退出）
  yunyou --home <DIR>     指定配置与会话的根目录（缺省 ~/.yunyou）
  yunyou --config <PATH>  指定 yunyou.toml 配置文件路径"
)]
pub struct Cli {
    /// yunyou.toml 配置文件路径。
    /// 缺省时用 `<yunyou-home>/yunyou.toml`：存在则加载，不存在则回退默认配置（环境变量可覆盖）。
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// yunyou-home 根目录（会话与运行标记的落点）。缺省：$YUNYOU_HOME，再缺省 ~/.yunyou。
    #[arg(long, global = true, value_name = "DIR")]
    pub home: Option<PathBuf>,

    /// 单任务模式：处理一句话后退出（每次任务用独立会话文件 task-*.jsonl，
    /// 不延续历史会话；缺省进入交互对话）。
    #[arg(long, value_name = "一句话")]
    pub task: Option<String>,

    /// 恢复历史会话：无参=交互选择（列历史会话挑一条，对齐 claude -r）；
    /// 有参=按会话文件名/路径恢复。与 --continue 互斥；两者都不给=启动全新会话
    /// （task-9：默认新会话化解多开锁冲突）。
    #[arg(long, value_name = "会话", num_args = 0..=1, default_missing_value = "", conflicts_with = "continue_recent")]
    pub resume: Option<Option<String>>,

    /// 续最近一条会话（对齐 claude -c）：取 sessions 目录 mtime 最新的常规会话。
    #[arg(long = "continue")]
    pub continue_recent: bool,

    /// 撞锁时的复制出路：目标会话正被另一实例持有时，复制该会话为新会话再启动
    /// （不改别人正用的文件；对齐 task-9「复制为新会话继续」）。
    #[arg(long, global = true)]
    pub fork: bool,

    /// 输出启动期诊断信息（插件状态表、就绪标记路径；缺省静默）。
    #[arg(long, global = true)]
    pub verbose: bool,

    /// 子命令：`web`=浏览器界面；缺省=终端对话（既有行为不变）。
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// 子命令（web 与终端 REPL 共享 --config/--home/--verbose 全局参）。
#[derive(Debug, Subcommand)]
pub enum Command {
    /// 启动浏览器界面（web 并列前端：同一装配、同一会话、同一对话引擎）。
    Web {
        /// 监听地址 host:port（缺省 127.0.0.1:0 随机端口；LAN 暴露需显式指定，token 鉴权恒开）。
        #[arg(long, value_name = "ADDR")]
        addr: Option<String>,
    },
    /// 插件管理（task-10）：new=脚手架生成 / install=归类落盘。
    Plugin {
        #[command(subcommand)]
        cmd: PluginCmd,
    },
}

/// 插件管理子命令（task-10；实现在 plugin_cmd 模块）。
#[derive(Debug, Subcommand)]
pub enum PluginCmd {
    /// 生成插件脚手架（Cargo.toml+模板 main+manifest.toml+README）。
    New {
        /// 插件名（小写字母/数字/连字符；crate 名与工具 id 同源）。
        name: String,
        /// 分类目录（tools/integrations/workflows/ui；install 自动归类依据）。
        #[arg(long, default_value = "tools", value_name = "类")]
        category: String,
        /// 生成落点（缺省当前目录下 <名>/）。
        #[arg(long, value_name = "DIR")]
        dir: Option<PathBuf>,
    },
    /// 安装插件目录（按 manifest.toml 的 id+category 归类落 <home>/plugins/）。
    Install {
        /// 插件目录（含 manifest.toml 与 cargo build 产物）。
        path: PathBuf,
    },
    /// 装前自查（作者侧；与 install 同源校验+依赖形态/契约版本/撞名预警）。
    Check {
        /// 插件目录（含 manifest.toml 与 src/）。
        path: PathBuf,
    },
}

/// 解析 yunyou-home：`--home` > `$YUNYOU_HOME` > `~/.yunyou`（台账环境事实的约定位置）。
pub fn resolve_home(cli_home: Option<PathBuf>) -> PathBuf {
    if let Some(home) = cli_home {
        return home;
    }
    if let Ok(home) = std::env::var("YUNYOU_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home);
        }
    }
    user_home().join(".yunyou")
}

/// 用户主目录（一期只支持 POSIX 的 HOME；Windows 展开随首个发布平台补）。
fn user_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
