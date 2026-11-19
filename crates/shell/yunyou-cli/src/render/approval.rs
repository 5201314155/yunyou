//! 权限对话框 TUI 呈现器（批 7 骨架，设计稿 件一·附/规格 B 消费面；lead 批
//! 2026-11-19：只消费 yun-suo 已入库的 ApprovalUiRegistry/ApprovalUiService，
//! 只消费不定义）。
//!
//! 职责：①三型审批对象 → 对话框文本（纯函数可测）；②键 → 决定映射（走
//! keybinding Confirmation context 解析——键位单源，交互矩阵 A2 落地首例）；
//! ③三型 [`ApprovalPresenter`] 实现（注册表按型分派，一型一注册）。
//!
//! 降级纪律（规格 B6）：非 TTY/读源故障 → Err → 内核 fail-closed Deny（人话
//! 指路），对话框永不假答；EOF 同 fail-closed。
//!
//! 已知限界（登记协调票）：呈现期间 repl 的回合期 Esc 监视臂（repl.rs:261）
//! 与本呈现器并发读 InputSource 全局通道——Esc 臂可能抢走对话框键字节（触发
//! 回合取消，present 经 GATE_TIMEOUT fail-closed 收场）；收口需 repl 域
//! 「对话框期间挂起 Esc 臂」配合，另行协调。

use crate::keybinding::{default_registry, Context, KeyCombo, Registry, Action};
use std::io::{IsTerminal, Write};
use yunyou::Result;
use yunyou_agent_loop::{ApprovalDecision, ApprovalSubject};

/// 束线（detail::rail 同形；本地平写避免跨件改私有面——形态三符三义纪律同源）。
fn rail(profile: crate::render::terminal::Profile) -> &'static str {
    match profile {
        crate::render::terminal::Profile::Plain => "|",
        _ => "▎",
    }
}

/// 审批对象 owned 快照（呈现期拷贝；`BoxFut<'_>` 生命周期约束下 subject
/// 借用不能进 Future——present 装箱前由此拷出，字段一一对应内核三型）。
pub enum ApprovalSnapshot {
    ToolCall {
        name: String,
        arguments: String,
    },
    FileReview {
        path: String,
        preview: String,
    },
    PluginPermission {
        plugin_id: String,
        requested: Vec<String>,
        purpose: String,
    },
}

impl From<&ApprovalSubject<'_>> for ApprovalSnapshot {
    fn from(subject: &ApprovalSubject<'_>) -> Self {
        match subject {
            ApprovalSubject::ToolCall(view) => ApprovalSnapshot::ToolCall {
                name: view.name.to_string(),
                arguments: view.arguments.to_string(),
            },
            ApprovalSubject::FileReview { path, preview } => ApprovalSnapshot::FileReview {
                path: (*path).to_string(),
                preview: (*preview).to_string(),
            },
            ApprovalSubject::PluginPermission { plugin_id, requested, purpose } => {
                ApprovalSnapshot::PluginPermission {
                    plugin_id: (*plugin_id).to_string(),
                    requested: requested.to_vec(),
                    purpose: (*purpose).to_string(),
                }
            }
        }
    }
}

/// 对话框文本（纯函数）：完成卡式形态（§4.7 同族）——请求标题+对象摘要+
/// 三段式选项行（规格 B1：是 / 是-本会话允许 / 否；键位最小集 y/n/a+Esc=否
/// 见规格 B5）。返回逐行（尾无换行；落定由呈现循环承担）。
pub fn dialog_text(profile: crate::render::terminal::Profile, subject: &ApprovalSnapshot) -> Vec<String> {
    let brand = profile.color(crate::render::terminal::SGR_BRAND_PRIMARY_256);
    let dim = profile.color(crate::render::terminal::SGR_TEXT_SECONDARY_256);
    let reset = profile.reset();
    let bold = profile.bold();
    let mut lines = Vec::new();
    lines.push(format!(
        "  {bold}{brand}⚠ 审批请求{reset} {dim}—— y=是 · a=本会话允许 · n/Esc=否{reset}"
    ));
    match subject {
        ApprovalSnapshot::ToolCall { name, arguments } => {
            lines.push(format!("    {dim}工具 {reset}{bold}{brand}{name}{reset}"));
            if !arguments.is_empty() {
                lines.push(format!("    {dim}{}{reset}", super::tool::clip_width(arguments, 96)));
            }
        }
        ApprovalSnapshot::FileReview { path, preview } => {
            lines.push(format!("    {dim}文件 {reset}{bold}{path}{reset}"));
            for line in preview.lines().take(8) {
                lines.push(format!("    {dim}{}{reset}{dim}{line}{reset}", rail(profile)));
            }
        }
        ApprovalSnapshot::PluginPermission { plugin_id, requested, purpose } => {
            lines.push(format!("    {dim}插件 {reset}{bold}{plugin_id}{reset} {dim}申请能力面：{reset}"));
            for item in requested.iter().take(8) {
                lines.push(format!("    {dim}{}{reset}{dim}• {item}{reset}", rail(profile)));
            }
            if !purpose.is_empty() {
                lines.push(format!("    {dim}用途：{purpose}{reset}"));
            }
        }
    }
    lines
}

/// 键 → 决定（纯函数）：走 keybinding Confirmation context 解析（键位单源
/// ——交互矩阵 A2 落地首例；未注册键=None 继续等待）。a=本会话允许：决定
/// 面与 y 同为 Approved，「记住」策略落库归呈现器/策略侧（内核注释同款），
/// 策略缝另票。
pub fn decision_from_action(action: Action) -> Option<ApprovalDecision> {
    match action {
        Action::ConfirmYes => Some(ApprovalDecision::Approved),
        // a=本会话允许：ApprovedAlways{rule}——rule 文本=对话框对象的规则粒度
        // 骨架（v1 无 suggestions 数据，规则=「本会话放行」通用粒度；suggestions
        // 档位（命令前缀/路径级）随批 7 二段）。持久化经内核 PermissionPersistence
        // 缝（gate.rs S5），呈现器只出决定不落库。
        Action::ConfirmAlways => {
            Some(ApprovalDecision::ApprovedAlways { rule: "session:allow".to_string() })
        }
        Action::ConfirmNo => Some(ApprovalDecision::Denied { message: "用户拒绝".to_string() }),
        _ => None,
    }
}

/// 单键 → 决定（Confirmation context 解析封装；决定集外键=None 继续等待）。
fn decide_key(registry: &Registry, byte: u8) -> Option<ApprovalDecision> {
    let key = match byte {
        b'\r' | b'\n' => "enter",
        0x1b => "escape",
        other => match (other as char).to_ascii_lowercase() {
            c if c.is_ascii_alphanumeric() => return decide_named(registry, &str::from_utf8(&[other]).unwrap_or("")),
            _ => return None,
        },
    };
    decide_named(registry, key)
}

fn decide_named(registry: &Registry, key: &str) -> Option<ApprovalDecision> {
    registry.resolve(Context::Confirmation, &KeyCombo::parse(key)).and_then(decision_from_action)
}

/// 呈现核心（三型共用）：渲染对话框+异步读决定。out/read 注入可测；
/// read=None=EOF → fail-closed Deny（规格 B6）。
async fn present_core<W: Write>(
    registry: &Registry,
    out: &mut W,
    profile: crate::render::terminal::Profile,
    subject: &ApprovalSnapshot,
    read_byte: &mut (impl AsyncReadByte),
) -> std::io::Result<ApprovalDecision> {
    for line in dialog_text(profile, subject) {
        writeln!(out, "{line}")?;
    }
    out.flush()?;
    // 读循环：决定集外键继续等（对话框模态语义）。
    loop {
        match read_byte.read_byte().await? {
            Some(byte) => {
                if let Some(decision) = decide_key(registry, byte) {
                    return Ok(decision);
                }
            }
            None => {
                return Ok(ApprovalDecision::Denied {
                    message: "输入源关闭（EOF），按拒绝处理（fail-closed）".to_string(),
                });
            }
        }
    }
}

/// 异步单键读取抽象（真实=InputSource；测试=预置字节序）。
trait AsyncReadByte {
    fn read_byte(&mut self) -> impl std::future::Future<Output = std::io::Result<Option<u8>>> + Send;
}

struct InputSourceReader;

impl AsyncReadByte for InputSourceReader {
    async fn read_byte(&mut self) -> std::io::Result<Option<u8>> {
        crate::input_source::InputSource::global().read_channel_byte().await
    }
}

/// 三型呈现器共用体（渲染出口=stdout 直写，逐行落定形态与渲染层一致）。
struct Core {
    registry: Registry,
}

impl Core {
    fn new() -> Self {
        Self { registry: default_registry() }
    }

    async fn present(&self, snapshot: ApprovalSnapshot) -> Result<ApprovalDecision> {
        let mut out = std::io::stdout();
        let profile = crate::render::terminal::Profile::detect(std::io::stdout().is_terminal());
        let mut reader = InputSourceReader;
        present_core(&self.registry, &mut out, profile, &snapshot, &mut reader)
            .await
            .map_err(|err| {
                yunyou::CordisError::new(
                    yunyou::ErrorKind::Internal,
                    format!("审批对话框呈现失败（{}），按拒绝处理（fail-closed）", err),
                )
            })
    }
}

macro_rules! tui_presenter {
    ($name:ident, $kind:expr) => {
        /// TUI 呈现器（一型一注册；同型 fail-loud 由注册表把关）。
        pub struct $name {
            core: Core,
        }

        impl Default for $name {
            fn default() -> Self {
                Self { core: Core::new() }
            }
        }

        impl yunyou_agent_loop::ApprovalPresenter for $name {
            fn subject(&self) -> yunyou_agent_loop::ApprovalSubjectKind {
                $kind
            }

            fn present(
                &self,
                subject: ApprovalSubject<'_>,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ApprovalDecision>> + Send + '_>> {
                // subject 借用不能进 BoxFut<'_>（trait 返回生命周期只挂 &self）——
                // 装箱前拷出 owned 快照，快照所有权随 async 块移动（机械对齐现态，
                // 呈现语义不变；yun-suo 回归复核点）。
                let snapshot = ApprovalSnapshot::from(&subject);
                Box::pin(self.core.present(snapshot))
            }
        }
    };
}

tui_presenter!(ToolCallApprovalPresenter, yunyou_agent_loop::ApprovalSubjectKind::ToolCall);
tui_presenter!(FileReviewApprovalPresenter, yunyou_agent_loop::ApprovalSubjectKind::FileReview);
tui_presenter!(
    PluginPermissionApprovalPresenter,
    yunyou_agent_loop::ApprovalSubjectKind::PluginPermission
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::terminal::Profile;
    use yunyou_agent_loop::ToolCallView;

    /// 对话框文本三型形态锁：标题+对象摘要+选项行（规格 B1 键位提示）。
    #[test]
    fn dialog_三型文本() {
        let view = ToolCallView { name: "bash", arguments: "rm -rf /" };
        let lines = dialog_text(Profile::Full, &ApprovalSnapshot::from(&ApprovalSubject::ToolCall(&view)));
        assert!(lines[0].contains("审批请求") && lines[0].contains("y=是"), "{lines:?}");
        // 工具名与参数摘要分两行落盘（名称行+摘要行），逐行断言不并单行
        assert!(lines.iter().any(|l| l.contains("bash")));
        assert!(lines.iter().any(|l| l.contains("rm -rf /")));

        let fr = dialog_text(
            Profile::Full,
            &ApprovalSnapshot::from(&ApprovalSubject::FileReview { path: "a.rs", preview: "fn a() {}\nfn b() {}" }),
        );
        // 路径与「文件」标签间有 SGR 着色段（Full 档）——断言只核路径连排
        assert!(fr.iter().any(|l| l.contains("a.rs")));
        assert!(fr.iter().filter(|l| l.contains("▎")).count() == 2, "预览逐行束线：{fr:?}");

        let pp = dialog_text(
            Profile::Full,
            &ApprovalSnapshot::from(&ApprovalSubject::PluginPermission {
                plugin_id: "demo",
                requested: &["network".to_string()],
                purpose: "联网",
            }),
        );

        // 申请项行（• network）与用途行（联网）分两行落盘——逐行断言不并单行
        assert!(pp.iter().any(|l| l.contains("network")));
        assert!(pp.iter().any(|l| l.contains("联网")));
    }

    /// 键→决定锁：y/enter/a=Approved，n/Esc=Denied（人话），决定集外键=None。
    #[test]
    fn 决定映射() {
        let reg = default_registry();
        assert!(matches!(decide_key(&reg, b'y'), Some(ApprovalDecision::Approved)));
        assert!(matches!(decide_key(&reg, b'\r'), Some(ApprovalDecision::Approved)));
        // a=本会话允许：ApprovedAlways{rule}（与内核 ApprovalDecision 三变体
        // 对齐；rule 文本=decision_from_action 现态 "session:allow"）
        assert!(matches!(
            decide_key(&reg, b'a'),
            Some(ApprovalDecision::ApprovedAlways { .. })
        ));
        assert!(matches!(
            decide_key(&reg, b'n'),
            Some(ApprovalDecision::Denied { .. })
        ));
        assert!(matches!(
            decide_key(&reg, 0x1b),
            Some(ApprovalDecision::Denied { .. })
        ));
        assert!(decide_key(&reg, b'x').is_none(), "决定集外键继续等待");
    }

    /// 呈现核心锁：预置字节序→决定；EOF→fail-closed Deny（规格 B6）。
    #[tokio::test]
    async fn 呈现核心_读序与降级() {
        struct Script(Vec<Option<u8>>);
        impl AsyncReadByte for Script {
            async fn read_byte(&mut self) -> std::io::Result<Option<u8>> {
                Ok(if self.0.is_empty() { None } else { self.0.remove(0) })
            }
        }
        let view = ToolCallView { name: "bash", arguments: "ls" };
        let mut out: Vec<u8> = Vec::new();
        // x（集外忽略）→ y（决定）
        let mut script = Script(vec![Some(b'x'), Some(b'y')]);
        let d = present_core(&default_registry(), &mut out, Profile::Full, &ApprovalSnapshot::from(&ApprovalSubject::ToolCall(&view)), &mut script)
            .await
            .unwrap();
        assert!(matches!(d, ApprovalDecision::Approved));
        assert!(String::from_utf8(out.clone()).unwrap().contains("审批请求"));
        // EOF → fail-closed Deny
        let mut eof = Script(vec![]);
        let d = present_core(&default_registry(), &mut out, Profile::Full, &ApprovalSnapshot::from(&ApprovalSubject::ToolCall(&view)), &mut eof)
            .await
            .unwrap();
        assert!(matches!(d, ApprovalDecision::Denied { .. }));
    }
}
