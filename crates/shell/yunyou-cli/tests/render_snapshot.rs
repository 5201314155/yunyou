//! 渲染快照回归（task-45 第二半，lead 批复方案）：五视觉块字节级 golden。
//!
//! 【覆盖】banner / diff（⋮+束线）/ 落款（全行 dim）/ 工具卡 / 折叠行（束线+计数）。
//! 【确定性】纯函数注入：Profile::Full、set_width(100)、elapsed 固定 Duration、
//! 固定模型串——零时钟零随机零环境依赖（防误报①③）。
//! 【更新流程（bless 纪律）】行为/规范变更 →
//! `YUNYOU_SNAPSHOT_BLESS=1 cargo test -p yunyou-cli --test render_snapshot`
//! 重录 golden → `git diff tests/fixtures/snapshot/` 人工过目 → 同批 commit
//! 且信息写明触发原因（行为变更/规范变更；规范变更先改规范再 bless，顺序锁）。
//! 【层B 说明】演示样例（docs/设计/演示/）的一致性走结构断言与人工重录流程
//! （task-45①），不做字节全等——mock 措辞与人话摘要存在已知演进差，字节
//! 全等=永久假红（方案防误报②）。

use yunyou_cli::render::event::{DetailKind, DiffHunk, TurnEvent};
use yunyou_cli::render::Profile;
use yunyou_cli::render::Renderer;

const DIR: &str = "tests/fixtures/snapshot";
const MODEL: &str = "z-ai/glm-5.3-flash";
const SESSION: &str = "/tmp/snap/sessions/default.jsonl";

/// 渲染→与 golden 比对；`YUNYOU_SNAPSHOT_BLESS=1` 时重录（bless 纪律见模块注）。
fn snapshot<F: FnOnce(&mut Renderer<Vec<u8>>) -> std::io::Result<()>>(name: &str, setup: F) {
    let mut r = Renderer::new(Vec::new(), Profile::Full, true);
    r.set_width(100);
    setup(&mut r).unwrap();
    let out = String::from_utf8(r.into_output()).unwrap();
    std::fs::create_dir_all(DIR).unwrap();
    let path = format!("{DIR}/{name}.snap");
    if std::env::var("YUNYOU_SNAPSHOT_BLESS").as_deref() == Ok("1") {
        std::fs::write(&path, &out).unwrap();
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("golden 缺失 {path}：{err}（YUNYOU_SNAPSHOT_BLESS=1 重录）"));
    assert_eq!(
        out, expected,
        "快照漂移：{name}——若为有意变更，YUNYOU_SNAPSHOT_BLESS=1 重录并在 commit 信息写明触发原因"
    );
}

/// §4.1 启动横幅（大字版；notice=None 基线态，提示行由既有条件测试覆盖）。
#[test]
fn snap_banner() {
    snapshot("banner_full", |r| {
        r.banner(MODEL, SESSION, None, true)
    });
}

/// §4.7/§4.11 工具完成卡 + diff 详情（⋮ 上下文 + 束线连续 + ± 着色）。
#[test]
fn snap_diff() {
    snapshot("diff_full", |r| {
        r.tool_done(&TurnEvent::ToolResult {
            call_id: String::new(),
            name: "edit".into(),
            ok: true,
            summary: "改 crates/yunyou/src/fiber.rs · 替换 1 处".into(),
            detail: Some(DetailKind::Diff {
                added: 2,
                removed: 1,
                hunks: vec![DiffHunk {
                path: None,
                old_start: None,
                    added: vec!["let epoch = self.recompute_epoch(id);".into(), "epoch_gate(epoch)?;".into()],
                    removed: vec!["let epoch = self.epochs[id];".into()],
                    context_lines: 2,
                }],
            }),
            parent_tool_call_id: None,
        })
    });
}

/// §4.12 状态落款行（全行 dim 含 ── lead_in；task-45② 定谳形态）。
#[test]
fn snap_status() {
    snapshot("status_full", |r| {
        r.turn_done(&TurnEvent::TurnDone {
            elapsed: std::time::Duration::from_millis(3271),
            prompt_tokens: Some(1204),
            completion_tokens: Some(8931),
            context_tokens: None,
            context_window: None, // task-27
            model: MODEL.into(),
        })
    });
}

/// §4.7 工具完成卡（bash 成功：✓ + 运行/退出码人话摘要）。
#[test]
fn snap_tool_card() {
    snapshot("tool_card_full", |r| {
        r.tool_done(&TurnEvent::ToolResult {
            call_id: String::new(),
            name: "bash".into(),
            ok: true,
            summary: "运行 cargo test -p yunyou-cli · 退出码 0".into(),
            detail: Some(DetailKind::Bash {
                exit_code: Some(0),
                output: "running 88 tests\ntest result: ok\n".into(),
            }),
            parent_tool_call_id: None,
        })
    });
}

/// §4.9 bash 折叠（左缘束线 + 秋香黄 … + 折叠计数单源句式）。
#[test]
fn snap_fold() {
    let output: String = (1..=12).map(|n| format!("第{n}行\n")).collect();
    snapshot("fold_full", |r| {
        r.tool_done(&TurnEvent::ToolResult {
            call_id: String::new(),
            name: "bash".into(),
            ok: true,
            summary: "运行 cat 会话日志 · 退出码 0".into(),
            detail: Some(DetailKind::Bash { exit_code: Some(0), output }),
            parent_tool_call_id: None,
        })
    });
}
