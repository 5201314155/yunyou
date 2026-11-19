//! 零内核依赖 CI 断言（skills 下沉批，lead 派单①）：`cargo tree -e normal`
//! 输出不得出现禁运内核/业务 crate——技能纯函数面零依赖（连 serde 都不引）。

#![allow(clippy::expect_used)] // 观察期口径：expect 失败=断言红，语义等价

use std::process::Command;

/// 禁运清单（内核与业务组件 crate 全集，SDK 先例同款照抄）。
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
    "yunyou-plugin-sdk",
    "yunyou-plugin-proto",
];

#[test]
fn 零内核依赖() {
    let output = Command::new(env!("CARGO"))
        .args(["tree", "-e", "normal"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("cargo tree 应可执行");
    let tree = String::from_utf8_lossy(&output.stdout);
    // 包名 token 精确比对（行首包名段；substring 会把自身 yunyou-skills 误判）
    for line in tree.lines() {
        let package = line.trim().split([' ', '\t']).next().unwrap_or("");
        assert!(
            !BANNED.contains(&package),
            "禁运依赖 {package} 出现在依赖树（零依赖纪律破）：\n{tree}"
        );
    }
}
