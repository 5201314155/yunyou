//! 零内核依赖 CI 断言（task-10 硬验收，lead 批复①）：`cargo tree -e normal`
//! 输出不得出现禁运内核包——插件作者依赖面=SDK+proto（契约冻结面）而已。
//! yunyou-util 当前实际不在 proto 依赖树内；若未来进入（白名单成员）亦放行。

// 测试面豁免（K-14 观察期口径；expect 失败=断言红，语义等价）
#![allow(clippy::expect_used)]

use std::process::Command;

/// 禁运清单（lead 批复 2026-11-18）：内核与业务组件 crate 全集。
const BANNED: &[&str] = &[
    "yunyou",
    "yunyou-tools",
    "yunyou-agent-loop",
    "yunyou-config",
    "yunyou-commands",
    "yunyou-session",
    "yunyou-team",
    "yunyou-gateway",
    "yunyou-cli",
    "yunyou-system-prompt",
    "yunyou-llm",
    "yunyou-sandbox",
    "yunyou-credentials",
    "yunyou-web",
    "yunyou-ipc-types",
];

/// 白名单放行（工具/契约层，非内核；前缀精确比较防 yunyou vs yunyou-util 误伤）。
const ALLOWED: &[&str] = &["yunyou-util", "yunyou-plugin-proto", "yunyou-plugin-sdk"];

/// 从 cargo tree 行提取包名（行形如 `├── yunyou-plugin-proto v0.1.0`）。
fn package_name(line: &str) -> &str {
    let name = line
        .trim_start_matches(|c: char| matches!(c, '├' | '└' | '─' | '│' | ' ') || c.is_whitespace());
    name.split_whitespace().next().unwrap_or("")
}

#[test]
fn sdk依赖树零内核包() {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml");
    let output = Command::new("cargo")
        .args(["tree", "-e", "normal", "--manifest-path", manifest])
        .output()
        .expect("cargo tree 应可执行（CI/本机均有 cargo）");
    let tree = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(output.status.success(), "cargo tree 失败：{tree}");
    for banned in BANNED {
        let hit = tree.lines().any(|line| {
            let pkg = package_name(line);
            pkg == *banned && !ALLOWED.contains(&pkg)
        });
        assert!(
            !hit,
            "SDK 依赖树出现禁运内核包 {banned}（零内核依赖断言失败）\n{tree}"
        );
    }
}
