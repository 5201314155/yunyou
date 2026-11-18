//! 伪 CLI UI 演示壳（业主令「输入 yunyou 就可以看效果」的离线回放版）。
//!
//! 目的：不配 key、不跑真实对话，一条命令回放 B 线渲染的全部设计效果
//! （设计规范 docs/设计/CLI终端显示设计规范.md §4 场景）。
//!
//! 铁律：**直接调用真实渲染函数族**（[`yunyou_cli::render`] 公共 API），
//! 不手写任何 ANSI——demo 调真函数＝效果与实现永不漂移（上会话 bash mock
//! 与实现漂移的教训）。数据全假（无真实 key/敏感路径），延时假造。
//!
//! 运行：
//! - `cargo run -p yunyou-cli --example demo`（演示节奏）
//! - `YUNYOU_DEMO_SPEED=fast cargo run -p yunyou-cli --example demo`
//!   （快进：跳过全部中间帧——spinner 帧/思考活动行计数帧，一瞬只出终态画面；
//!   SSH 弱终端回放时逐帧重绘会滚动堆积成糊屏，快进档正是为回放场景开的）
//!
//! 档位：复用渲染层单源探测 [`Profile::detect_from`]——`TERM=dumb` 或
//! `YUNYOU_PLAIN=1` → Plain（色彩+符号全退化），`NO_COLOR=1` → NoColor，
//! 其余 Full。演示壳据此随终端能力出帧：渲染自查基准的「dumb 场景」由
//! 此拿到真 Plain 档证据（此前固定 Full，dumb 腿等于 Full 窄幅回放，
//! Plain 档无器具可验）。is_tty 恒传 true：演示壳定位是「重定向到文件
//! 保留真实字节流」的效果回放（管道非 TTY 不降档，否则回放样例自身
//! 变无色档），非 TTY 判定交由产品入口（run.rs）承担。interactive=true：
//! 演示 spinner 与活动行重绘（Plain 档下 spinner 走 ASCII 帧、`\r\e[K`
//! 结构性重绘序列保留——那是动画机构不是色彩泄漏，与产品 Plain 档一致）。

use std::io::Write;
use std::time::Duration;
use yunyou_cli::render::{tool, tree, DetailKind, DiffHunk, Profile, Renderer, TurnEvent};

/// 帧间冲刷：spinner 的 `\r\e[K` 原位序列不带换行，不冲刷则滞留行缓冲（REPL
/// 观察者同款要求，见 observe.rs 驱动循环；repl.rs:81 同款 flush 惯用法）。
macro_rules! beat {
    ($r:expr) => {
        $r.parts().0.flush()?
    };
}

/// 演示延时节奏：`YUNYOU_DEMO_SPEED=fast` 归零（快进跑完全部场景）。
enum Speed {
    /// 演示节奏：spinner 80ms/帧（对齐观察者驱动节奏）、流式行间、段间缓冲。
    Show,
    /// 快进：全部延时归零，一次性出完整画面。
    Fast,
}

impl Speed {
    fn from_env() -> Self {
        if std::env::var("YUNYOU_DEMO_SPEED").ok().as_deref() == Some("fast") {
            Self::Fast
        } else {
            Self::Show
        }
    }

    fn sleep(&self, ms: u64) {
        if matches!(self, Self::Show) {
            std::thread::sleep(Duration::from_millis(ms));
        }
    }

    /// 中间帧是否值得渲染（任务30 二期⑤）：快进档 spinner 帧与思考活动行
    /// 计数帧全跳过，只出终态画面（完成卡/冻结单行）——逐帧重绘是糊屏来源。
    fn frames(&self) -> bool {
        matches!(self, Self::Show)
    }
}

fn main() -> std::io::Result<()> {
    let mut speed = Speed::from_env();
    // 档位自适应：探测逻辑复用渲染层单源（terminal.rs Profile::detect_from），
    // 演示壳不重复实现；is_tty 恒传 true（保留重定向回放语义，见模块注释）。
    // interactive=true：演示 spinner 与活动行重绘（非交互档会静默暂态帧，
    // Renderer 契约：mod.rs:30-33）。StdoutLock 'static 整体 move 进 Renderer。
    let mut r = Renderer::new(
        std::io::stdout().lock(),
        Profile::detect_from(true, &|key| std::env::var(key).ok()),
        true,
    );

    // ① §4.1 启动横幅（品牌标 + 模型 + 会话目录 + 提示；数据全假）
    r.banner("GLM-4.7（演示数据）", "~/.yunyou/sessions/20261114-demo（演示）", None, true)?;
    speed.sleep(400);

    // ② §4.2 输入提示符 + §4.3 用户消息回显
    r.prompt()?;
    beat!(r);
    speed.sleep(500);
    r.user_echo("帮我看下项目结构，然后把 README 标题改成中文？")?;
    speed.sleep(400);

    // ③ 思考流收敛（任务 30-②：默认不倾倒原文——活动行「│（思考中… N 字）」
    //    计数原位重绘，块终冻结「│ 想了 X.Xs · N 字」）。
    //    快进档（任务30 二期⑤）：计数帧全跳过，合并为一次喂入——活动行只
    //    重绘 1 帧即到块终，冻结单行照出（终态不丢）。
    if speed.frames() {
        r.reasoning_delta("用户想先浏览项目结构，再修改 README 标题。")?;
        beat!(r);
        speed.sleep(150);
        r.reasoning_delta("先跑一次目录列举，看清楚 crate 划分。")?;
        beat!(r);
        speed.sleep(150);
        r.reasoning_delta("然后读 README 当前标题，再做最小改动。")?;
        beat!(r);
        speed.sleep(150);
    } else {
        r.reasoning_delta("（快进档合并）想先看结构，再改 README 标题。")?;
        beat!(r);
    }

    // ④ §4.5 流式正文（按行喂入＝满行落屏的真实观感；覆盖 markdown 行级＋
    //    行内层（任务 30-①）：列表 / 行内代码 / **粗体** / _斜体_ / H2 / 中文长句折行）
    let script = [
        "项目结构已经看清楚了：",
        "- `crates/yunyou`：内核（fiber 装卸 / 事件总线 / effect 逆序撤销）",
        "- `crates/shell/yunyou-cli`：终端外壳，你正在看的这些效果就是它渲染的",
        "",
        "## 结论",
        "**关键改动只有一处**——README 标题改为中文，其余保持原样（这一段演示 *行内粗体* 与 _行内斜体_ 的着色与去定界符）。",
        "接下来把 README 标题改为中文——这里刻意放一个包含很多中文词汇的超长句子用于演示终端窄窗口下的自动折行场景与满行才落屏的流式语义，这一行明显超过常规 80 列宽度，落定后由终端完成折行且永不回改。",
    ];
    for line in script {
        r.text_delta(line)?;
        r.text_delta("\n")?;
        beat!(r);
        speed.sleep(120);
    }
    speed.sleep(300);

    // ⑤ 工具卡片（§4.6 进行中 spinner → §4.7 完成卡 → §4.9-4.11 详情展开）
    // 5a. bash：Pending spinner（8 帧）→ Ok + 输出折叠摘要
    spin(&mut r, &mut speed, "bash", "ls crates/ && wc -l *.rs", 8)?;
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "demo-bash-1".into(),
        name: "bash".into(),
        ok: true,
        summary: "exit 0 · 1.2s".into(),
        detail: Some(DetailKind::Bash {
            exit_code: Some(0),
            output: "yunyou\nyunyou-agent-loop\nyunyou-cli\nyunyou-config\nyunyou-credentials\n".into(),
        }),
        parent_tool_call_id: None,
    })?;
    beat!(r);
    speed.sleep(500);

    // 5b. read：结果先到＝无 spinner 快路径直接落定（不假装忙碌）→ 行号预览 + 折叠
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "demo-read-1".into(),
        name: "read".into(),
        ok: true,
        summary: "crates/shell/yunyou-cli/src/repl.rs · 12 行".into(),
        detail: Some(DetailKind::Read {
            path: "crates/shell/yunyou-cli/src/repl.rs（演示数据）".into(),
            from: 1,
            lines: (1..=12)
                .map(|no| format!("// 演示预览第 {no} 行：REPL 交互循环与渲染接线"))
                .collect(),
        }),
        parent_tool_call_id: None,
    })?;
    beat!(r);
    speed.sleep(500);

    // 5c. edit：spinner → Ok + diff 着色（+/− 前缀，上下文折叠）
    spin(&mut r, &mut speed, "edit", "README.md", 5)?;
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "demo-edit-1".into(),
        name: "edit".into(),
        ok: true,
        summary: "README.md · +1 −1".into(),
        detail: Some(DetailKind::Diff {
            added: 1,
            removed: 1,
            hunks: vec![DiffHunk {
                path: None,
                old_start: None,
                removed: vec!["# YunYou Agent".into()],
                added: vec!["# 云悠 YunYou".into()],
                context_lines: 2,
            }],
        }),
        parent_tool_call_id: None,
    })?;
    beat!(r);
    speed.sleep(500);

    // 5d. 错误态：path-outside-root 红错（错误不能藏——完成卡 ✗ 绯红）
    spin(&mut r, &mut speed, "read", "/etc/passwd", 4)?;
    r.tool_done(&TurnEvent::ToolResult {
        call_id: "demo-read-2".into(),
        name: "read".into(),
        ok: false,
        summary: "path-outside-root: 路径越界，仅允许项目根内文件（演示数据）".into(),
        detail: None,
        parent_tool_call_id: None,
    })?;
    beat!(r);
    speed.sleep(500);

    // ⑥ 树状缩进（§4.8：并行/嵌套调用的 ├─ └─ 缩进）。
    // 说明：树归集接线层（repl 按 parent 关联拼缩进）尚未落地，此处为
    // `tree::indent_prefix` + `tool::tool_done` 两个真函数的组合预演——
    // 未来接线层即按此组合拼装，demo 不另造样式。
    {
        let (stream_out, profile) = r.parts();
        let child = |summary: &str| TurnEvent::ToolResult {
            call_id: format!("demo-tree-{summary}"),
            name: "read".into(),
            ok: true,
            summary: summary.into(),
            detail: None,
            parent_tool_call_id: Some("demo-bash-1".into()),
        };
        write!(stream_out, "{}", tree::indent_prefix(profile, 1, false))?;
        tool::tool_done(stream_out, profile, &child("crates/yunyou/Cargo.toml · 3 行"), 80, 0)?;
        write!(stream_out, "{}", tree::indent_prefix(profile, 1, true))?;
        tool::tool_done(stream_out, profile, &child("README.md · 1 行"), 80, 0)?;
    }
    beat!(r);
    speed.sleep(500);

    // ⑦ §4.12 会话落款行（耗时 / ↑↓ tok / 模型；本地聚合假数据）
    r.turn_done(&TurnEvent::TurnDone {
        elapsed: Duration::from_millis(3200),
        prompt_tokens: Some(1234),
        completion_tokens: Some(567),
        context_tokens: Some(84_232), // task-26：水位主位（端点侧测量）
        context_window: None, // task-27
        model: "GLM-4.7（演示数据）".into(),
    })?;
    speed.sleep(400);

    // ⑧ 收尾：落定未满行缓冲 + 活动行清理防御，再给结束提示（烟灰说明，非规范块）
    r.finish()?;
    let (out, profile) = r.parts();
    writeln!(
        out,
        "{}—— 演示结束（全部为假数据回放；真实交互请运行 `yunyou`，需配置模型 key）{}",
        profile.dim(),
        profile.reset()
    )?;
    out.flush()
}

/// 工具进行中的 spinner 序列（§4.6：`tool_call_frame` 单帧原位重绘，
/// 80ms/帧对齐观察者驱动节奏）。快进档零帧：跳过全部 spinner 帧，直接进
/// 完成卡（任务30 二期⑤——逐帧重绘是 SSH 回放糊屏的本体，完成卡即终态）。
fn spin(
    r: &mut Renderer<std::io::StdoutLock<'static>>,
    speed: &mut Speed,
    name: &str,
    summary: &str,
    frames: usize,
) -> std::io::Result<()> {
    if !speed.frames() {
        return Ok(());
    }
    r.tool_call_frame(name, summary, 0, Some(0))?;
    for frame in 1..frames {
        beat!(r);
        speed.sleep(80);
        // 耗时=帧数×80ms 换算（秒级取整；与 observe.rs 驱动同律）
        r.tool_call_frame(name, summary, frame, Some(frame as u64 * 80 / 1000))?;
    }
    beat!(r);
    speed.sleep(80);
    Ok(())
}
