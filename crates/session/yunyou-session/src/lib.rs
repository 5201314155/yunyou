//! 会话域（对标 TS 版 yunyou-session 的最小裁剪，方案 §2.5）。
//!
//! - 事件模型：user/assistant/tool_call/tool_result/system 五类（一期闭环词汇）；
//! - 存储：JSONL 追加写（一行一个事件，行序即事件序，对标 TS 的持久化事件日志）；
//! - 恢复：`load_all` 逐行重放；**未知事件类型跳过不失败**（对齐 TS
//!   `SessionEvent.ignorable` 兼容机制——新版本写入的事件不应让旧版读取崩溃）；
//! - 服务面：[`SessionStore`] trait（object-safe）由 [`SessionPlugin`] 注册到内核。
//!
//! # 契约
//! - 追加写是**只追加**的（append-only）：已落盘的行永不改写，崩溃安全；
//! - `load_all` 返回的事件顺序与落盘顺序一致；
//! - 全部实现 `Send + Sync`（方案 §2.3）。

pub mod events;
pub mod jsonl;
pub mod paths;
pub mod plugin;

/// 详情预览直出行数（跨 crate 单源，task-22 尾件，lead 裁方案甲）：折叠详情
/// 「先渲染前 N 行再折叠」的 N——agent-loop [`crate::fold`]（8KB 降级预览）与
/// cli render detail（bash/read/8KB 三路详情块）是同一展示语义的两个消费方，
/// 分叉即漂移面（原双侧同批同值+双测试锁死，上移后单源单测）。
pub const PREVIEW_LINES: usize = 10;
/// 详情折叠触发阈值（task-22 尾件随 PREVIEW_LINES 同域集中，lead 裁）：超此
/// 行数才折叠，≤ 此行数直接全量展示；现消费方仅 cli render detail（单消费方
/// 随批集中防「一半上一半留」的参数族切裂），侧测试锁值不变。
pub const FOLD_THRESHOLD: usize = 10;

#[cfg(test)]
mod preview_lines_tests {
    /// 单源值锁：预览行数与折叠阈值=10（任务30 四期 D-A 业主定值；改动此处
    /// 必须同批走渲染自查基准 demo 回放+双侧测试复核，见 fold/detail 历史注释）。
    #[test]
    fn 预览行数与折叠阈值单源值() {
        assert_eq!(super::PREVIEW_LINES, 10);
        assert_eq!(super::FOLD_THRESHOLD, 10);
    }
}

/// 跨进程会话排他锁（task-36①，方案 A）：flock 封装，由 jsonl::JsonlStore 消费。
mod lock;

pub use events::SessionEvent;
pub use jsonl::JsonlStore;
pub use paths::sanitize_project_dir;
pub use lock::ExclusiveLock;
pub use plugin::SessionPlugin;

use yunyou::Result;
use yunyou_util::BoxFut;

/// 会话存储服务契约（消费方：agent-loop；实现：JsonlStore，未来可换 SQLite）。
pub trait SessionStore: Send + Sync {
    /// 追加一个事件（落盘成功才返回 Ok）。
    ///
    /// # 后置条件
    /// 返回 Ok 后事件已持久化；进程崩溃不丢该行。
    fn append(&self, event: &SessionEvent) -> BoxFut<'static, Result<()>>;

    /// 读取全部事件（重启恢复）。未知类型行跳过（见 crate 文档）。
    fn load_all(&self) -> BoxFut<'static, Result<Vec<SessionEvent>>>;

    /// 续聊升级（task-74 v5）：回放装载（只读）后续聊进入对话态前调用——
    /// 确保写路径可用（fail-loud，升级失败当场报）。默认 no-op（无升级
    /// 语义的实现零负担；JsonlStore 实现见 jsonl.rs）。
    fn ensure_writable(&self) -> BoxFut<'static, Result<()>> {
        Box::pin(async move { Ok(()) })
    }

    /// 会话文件路径（task-11② 投影缝）：策略插件（如 tool-archive）从会话
    /// 文件推导 side-store 落点——默认 None（无路径语义的实现零负担；
    /// 消费方对 None 必须诚实降级，不猜路径）。JsonlStore 实现返回自身
    /// jsonl 路径（父目录即会话目录）。
    fn path(&self) -> Option<&std::path::Path> {
        None
    }
}
