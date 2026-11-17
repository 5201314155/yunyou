//! 构建期把设计 token 单源（docs/设计/tokens.json，云鉴 task-7 产出）生成为
//! web 资产注入物：`tokens.css`（CSS 变量，按 tokens.json adapters.web 命名
//! 规则映射）与 `tokens.js`（spinner 帧序列、语义符号等 JS 面常量）。
//!
//! 与 yunyou-cli 的 palette.rs 同机制同纪律：色值/符号/间距/动效变更只改
//! tokens.json，代码零改动；本端适配层只做「翻译」（如 mono → 系统等宽栈），
//! 不新增语义、不私设色值。
//!
//! v2 追加（任务40 rev10 路径）：dist/ 嵌入清单——vite 产物（frontend/apps/web/dist
//! 同步至 crate 根 dist/，随仓库提交）在编译期生成 `dist_manifest.rs`
//! （相对路径 → include_bytes! 静态表），业主运行时零 node。dist/ 缺席或为空
//! 时生成空表（v1 vanilla assets 继续服务）——构建不因前端产物缺席而断。

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    embed_dist();

    let tokens = PathBuf::from("../../../docs/设计/tokens.json");
    println!("cargo:rerun-if-changed={}", tokens.display());
    let raw = std::fs::read_to_string(&tokens)
        .unwrap_or_else(|err| panic!("设计 token 单源缺失（{}）：{err}", tokens.display()));
    let value: serde_json::Value = serde_json::from_str(&raw).expect("tokens.json 不是合法 JSON");
    let semantic = &value["semantic"];

    // ① 语义色 → CSS 变量（adapters.web 规则：brand.primary → --yunyou-color-brand-primary）
    let mut css = String::from(
        "/* 由 yunyou-web build.rs 从 docs/设计/tokens.json 生成——勿手改；改 tokens.json 后重新构建。 */\n:root {\n",
    );
    let colors = semantic["color"].as_object().expect("semantic.color 缺失");
    for (key, spec) in colors {
        let name = key.replace('.', "-");
        match spec["hex"].as_str() {
            Some(hex) => {
                let _ = writeln!(css, "  --yunyou-color-{name}: {hex};");
            }
            // hex=null 语义是「正文不设色」：web 侧用系统前景色继承，不私设替代值
            None => {
                let _ = writeln!(css, "  /* {key}：token 未定值（正文不设色）→ web 继承系统前景色 */");
            }
        }
    }

    // ② 字版 → 尺寸/字重/字族变量（--yunyou-typo-*）
    let typo = semantic["typography"].as_object().expect("semantic.typography 缺失");
    for (key, spec) in typo {
        let name = key.replace('.', "-");
        let web = spec["web"].as_str().unwrap_or_else(|| panic!("typography.{key}.web 缺失"));
        // 两种形态："24px/700"（尺寸/字重）与 "13px mono"（尺寸/字族）
        if let Some((size, weight)) = web.split_once('/') {
            let _ = writeln!(css, "  --yunyou-typo-{name}-size: {size};");
            let _ = writeln!(css, "  --yunyou-typo-{name}-weight: {weight};");
        } else if let Some((size, family)) = web.split_once(' ') {
            let _ = writeln!(css, "  --yunyou-typo-{name}-size: {size};");
            let _ = writeln!(css, "  --yunyou-typo-{name}-family: {};", font_family(family));
        } else {
            panic!("typography.{key}.web 形态不认识：{web}");
        }
    }

    // ③ 间距 → 像素变量（key 本身带 space. 前缀 → --yunyou-space-N）
    let spacing = semantic["spacing"].as_object().expect("semantic.spacing 缺失");
    for (key, spec) in spacing {
        let name = key.replace('.', "-");
        let px = spec["px"].as_u64().unwrap_or_else(|| panic!("spacing.{key}.px 缺失"));
        let _ = writeln!(css, "  --yunyou-{name}: {px}px;");
    }

    // ④ 动效：时长变量 + 统一缓动（各 duration 共用同一曲线，tokens.json 为准）
    let motion = semantic["motion"].as_object().expect("semantic.motion 缺失");
    let mut ease_emitted = false;
    for (key, spec) in motion {
        let name = key.replace('.', "-");
        if let Some(ms) = spec["ms"].as_u64() {
            let _ = writeln!(css, "  --yunyou-{name}: {ms}ms;");
        }
        if let Some(curve) = spec["曲线"].as_str() {
            if !ease_emitted {
                let _ = writeln!(css, "  --yunyou-ease-standard: {curve};");
                ease_emitted = true;
            }
        }
    }
    css.push_str("}\n");

    // ⑤ JS 面常量：spinner 帧序列/帧间隔 + 语义符号（前端从 window 常量取，
    //    不在 HTML/JS 里散写字面量——与 CSS 变量同一「单源派生」纪律）
    let frames: Vec<String> = semantic["symbol"]["spinner.frames"]
        .as_str()
        .expect("spinner.frames 应为字符串")
        .chars()
        .map(|c| format!("\"{c}\""))
        .collect();
    let interval = motion
        .get("spinner.interval")
        .and_then(|spec| spec["ms"].as_u64())
        .unwrap_or(80);
    let mut js = String::from(
        "// 由 yunyou-web build.rs 从 docs/设计/tokens.json 生成——勿手改；改 tokens.json 后重新构建。\n",
    );
    let _ = writeln!(js, "window.YUNYOU_SPINNER_FRAMES = [{}];", frames.join(", "));
    let _ = writeln!(js, "window.YUNYOU_SPINNER_INTERVAL_MS = {interval};");
    let symbols = semantic["symbol"].as_object().expect("semantic.symbol 缺失");
    let mut sym_pairs: Vec<String> = Vec::new();
    for (key, spec) in symbols {
        // spinner.frames 是帧序列非单符号，已单独处理
        if key == "spinner.frames" {
            continue;
        }
        let text = spec.as_str().unwrap_or_else(|| panic!("semantic.symbol.{key} 应为字符串"));
        let name = key.replace('.', "_");
        sym_pairs.push(format!("\"{name}\": \"{text}\""));
    }
    let _ = writeln!(js, "window.YUNYOU_SYM = {{{}}};", sym_pairs.join(", "));

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    std::fs::write(out_dir.join("tokens.css"), css).expect("写 tokens.css 失败");
    std::fs::write(out_dir.join("tokens.js"), js).expect("写 tokens.js 失败");
}

/// 字族名 → 字体栈（端适配翻译：不新增语义；新字族先改 tokens.json 再扩本表）。
fn font_family(name: &str) -> &'static str {
    match name {
        "mono" => "ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', monospace",
        other => panic!("未认识的字族 token：{other}（新增请先改 tokens.json 再扩本表）"),
    }
}

/// dist/ 嵌入清单生成：遍历 crate 根 dist/（vite 产物同步落点），产出
/// `dist_manifest.rs`——`pub static DIST_FILES: &[(&str, &[u8])]`，路径为
/// dist 内相对路径（`/` 前缀），字节为 include_bytes! 静态引用（生成绝对
/// 路径：include 相对路径相对所在文件=OUT_DIR 解析，不可用）。目录缺席/
/// 为空 = 空表（v1 vanilla 回退面继续服务，不报错）。
fn embed_dist() {
    let dist = PathBuf::from("dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if dist.is_dir() {
        walk_dist(&dist, Path::new(""), &mut files);
    }
    files.sort();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR 未设置");
    let mut code = String::from(
        "// 由 yunyou-web build.rs 从 dist/ 生成——勿手改；前端产物变更后重新构建。\n",
    );
    if files.is_empty() {
        code.push_str("// 空=dist/ 缺席或为空：v2 前端未构建，v1 vanilla assets 回退服务。\n");
        code.push_str("pub static DIST_FILES: &[(&str, &[u8])] = &[];\n");
    } else {
        code.push_str("// v2 官方移植前端产物（相对路径 → 字节；完整产物随仓库 dist/ 提交）。\n");
        code.push_str("pub static DIST_FILES: &[(&str, &[u8])] = &[\n");
        for (rel, abs) in &files {
            let _ = writeln!(
                code,
                "    (\"/{rel}\", include_bytes!(r\"{}/{}\")),",
                manifest_dir,
                abs.display()
            );
        }
        code.push_str("];\n");
    }
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR 未设置"));
    std::fs::write(out_dir.join("dist_manifest.rs"), code).expect("写 dist_manifest.rs 失败");
}

/// 先序遍历 dist/ 收集文件（相对路径 + 绝对路径对）。
fn walk_dist(root: &Path, rel: &Path, out: &mut Vec<(String, PathBuf)>) {
    let entries = match std::fs::read_dir(root.join(rel)) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = rel.join(entry.file_name());
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk_dist(root, &path, out);
        } else {
            out.push((path.to_string_lossy().replace('\\', "/"), root.join(&path)));
        }
    }
}
