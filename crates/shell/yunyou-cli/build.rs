//! 构建期把设计 token 单源（docs/设计/tokens.json，云鉴 task-7 产出）生成为
//! `palette.rs`——落实设计规范 §6「渲染模块不得内嵌裸色号，共享色板只从
//! tokens 派生一处」：色值/符号/动效变更只改 tokens.json，代码零改动。

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    // task-26④ 残留修复：溯源段（commit/时间）要求 HEAD 变更即重跑——HEAD 无
    // 文件句柄可 watch，用「不存在哨兵文件」trick 强制每次构建重跑 build.rs
    //（cargo 语义：rerun-if-changed 指向不存在路径=永远视为已变更）。
    // 副作用评估：重跑成本=git rev-parse+date+tokens 解析 ≈50ms/次，可忽略；
    // 收益=--version 溯源在增量构建下不再陈旧（用户看到的 commit 恒准确）。
    // task-26④ 触发修正：不存在哨兵=状态稳定不触发（cargo 变更检测语义），
    // 改 .git 元数据三触发——commit 变更必命中其一（refs 松散/packed/HEAD）。
    // commit 准确=主要诉求；构建时间=次要（无 commit 构建沿用上次时间可接受）。
    for git_meta in [".git/HEAD", ".git/refs/heads", ".git/packed-refs"] {
        let p = PathBuf::from("../../").join(git_meta);
        if p.exists() {
            println!("cargo:rerun-if-changed={}", p.display());
        }
    }
    let tokens = PathBuf::from("../../../docs/设计/tokens.json");
    println!("cargo:rerun-if-changed={}", tokens.display());
    let raw = std::fs::read_to_string(&tokens)
        .unwrap_or_else(|err| panic!("设计 token 单源缺失（{}）：{err}", tokens.display()));
    let value: serde_json::Value =
        serde_json::from_str(&raw).expect("tokens.json 不是合法 JSON");

    let cli = &value["adapters"]["cli"];
    let semantic = &value["semantic"];
    let mut out = String::new();
    out.push_str("// 由 build.rs 从 docs/设计/tokens.json 自动生成——勿手改；改 tokens.json 后重新构建。\n");

    // ① 语义色 → SGR 序列（256 色主案 + 16 色降级）
    let colors = cli["color"].as_object().expect("adapters.cli.color 缺失");
    for (key, spec) in colors {
        let name = const_name(key);
        let default = spec["sgr256"].is_null();
        let sgr256 = if default { "39" } else { spec["sgr256"].as_str().expect("sgr256 应为字符串") };
        let sgr16 = spec["sgr16"].as_str().expect("sgr16 应为字符串");
        let _ = writeln!(out, "pub const SGR_{name}_256: &str = \"{sgr256}\";");
        let _ = writeln!(out, "pub const SGR_{name}_16: &str = \"{sgr16}\";");
    }

    // ② 语义符号（YUNYOU_PLAIN 的 ASCII 降级在运行时按规范 §3 表处理）
    let symbols = semantic["symbol"].as_object().expect("semantic.symbol 缺失");
    for (key, spec) in symbols {
        let name = const_name(key);
        match spec.as_str() {
            Some(text) => {
                let _ = writeln!(out, "pub const SYM_{name}: &str = \"{text}\";");
            }
            None => panic!("semantic.symbol.{key} 应为字符串"),
        }
    }

    // ③ 动效：spinner 帧序列与帧间隔（CLI 唯一动效）
    let frames: Vec<String> = semantic["symbol"]["spinner.frames"]
        .as_str()
        .expect("spinner.frames 应为字符串")
        .chars()
        .map(|c| format!("\"{c}\""))
        .collect();
    let _ = writeln!(out, "pub const SPINNER_FRAMES: [&str; {}] = [{}];", frames.len(), frames.join(", "));
    let interval = cli["motion"]["spinner.interval.ms"].as_u64().unwrap_or(80);
    let _ = writeln!(out, "pub const SPINNER_INTERVAL_MS: u64 = {interval};");

    // ④ ASCII 降级符号表（规范 §3 右列；YUNYOU_PLAIN / TERM=dumb 时整体替换）
    out.push_str("pub mod ascii {\n");
    for (plain, unicode) in [
        ("SUCCESS", "SYM_SUCCESS"),
        ("FAILURE", "SYM_FAILURE"),
        ("PENDING", "SYM_PENDING"),
        ("FOLD", "SYM_FOLD"),
        ("PROMPT", "SYM_PROMPT"),
        ("BRAND", "SYM_BRAND"),
        ("BULLET", "SYM_BULLET"),
        ("QUOTE", "SYM_QUOTE"),
        ("TREE_BRANCH", "SYM_TREE_BRANCH"),
        ("TREE_CORNER", "SYM_TREE_CORNER"),
        ("TREE_PIPE", "SYM_TREE_PIPE"),
        ("TREE_DASH", "SYM_TREE_DASH"),
        // task-35②+65⑨ D3 四符号收编落地单点（thinking.prefix/fence.dot/
        // diff.minus/cancel.dash——stream/thinking/tool/detail 四处手写分流
        // 退役，SYM_* 单源）。
        ("FENCE_DOT", "SYM_FENCE_DOT"),
        ("THINKING_PREFIX", "SYM_THINKING_PREFIX"),
        ("DIFF_MINUS", "SYM_DIFF_MINUS"),
        ("CANCEL_DASH", "SYM_CANCEL_DASH"),
    ] {
        let _ = writeln!(out, "    pub const {plain}: &str = \"{}\";", ascii_of(unicode));
    }
    out.push_str("    pub const SPINNER_FRAMES: [&str; 4] = [\"|\", \"/\", \"-\", \"\\\\\"];\n");
    out.push_str("}\n");

    // ⑤ 构建溯源（task-26④）：commit 与构建时间进编译期环境，--version 输出
    // 走此单源（PATH 真身随主树构建漂移——来源指示=普通用户可分辨「跑的是哪」）。
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:warning=[build.rs-debug] commit={commit} cwd={}", std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
    let build_time = std::process::Command::new("date")
        .arg("-u")
        .arg("+%Y-%m-%d %H:%M UTC")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    // task-26④ 正解（2026-11-17）：版本行改运行时读取——OUT_DIR include! 与
    // rustc-env 均非 crate 重编触发（cargo 双限制实证），const 路径必陈旧；
    // 运行时 fs 读 OUT_DIR/version.txt=每次 --version 拿到当次构建值（恒新鲜）。
    // 注意：version.txt 独立文件（非 palette.rs——palette 被 include! 编译，
    // 裸文本进去=语法炸，初版误写教训）。
    // 双轨：rustc-env（env 变更=cargo 原生 crate 重编触发——OUT_DIR include! 无
    // 触发力的 cargo 限制绕行）——cli.rs 侧 option_env! 消费。
    println!(
        "cargo:rustc-env=YUNYOU_VERSION_LINE={} · 构建 {commit} · {build_time}",
        env!("CARGO_PKG_VERSION")
    );

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    std::fs::write(out_dir.join("palette.rs"), out).expect("写 palette.rs 失败");
    // task-26④：version.txt 独立写（运行时读取源——cli.rs version_line()）
    std::fs::write(
        out_dir.join("version.txt"),
        format!("{} · 构建 {commit} · {build_time}", env!("CARGO_PKG_VERSION")),
    )
    .expect("写 version.txt 失败");
}

/// 键名 → 常量名：`brand.primary` → `BRAND_PRIMARY`，`tree.branch` → `TREE_BRANCH`。
fn const_name(key: &str) -> String {
    key.to_uppercase().replace(['.', '-'], "_")
}

/// ASCII 降级表（规范 §3：✓→v ✗→x ❯→> ☁→YunYou 树→|`- 等）。
fn ascii_of(unicode_const: &str) -> &'static str {
    match unicode_const {
        "SYM_SUCCESS" => "v",
        "SYM_FAILURE" => "x",
        "SYM_PENDING" => "o",
        "SYM_FOLD" => "...",
        "SYM_THINKING_PREFIX" => "|",
        "SYM_DIFF_MINUS" => "-",
        "SYM_CANCEL_DASH" => "-",
        "SYM_PROMPT" => ">",
        "SYM_BRAND" => "YunYou",
        "SYM_BULLET" => "-",
        "SYM_QUOTE" => "|",
        "SYM_TREE_BRANCH" => "|--",
        "SYM_TREE_CORNER" => " `-",
        "SYM_TREE_PIPE" => "|",
        "SYM_TREE_DASH" => "-",
        "SYM_FENCE_DOT" => ".",
        other => panic!("未知符号常量：{other}"),
    }
}
