//! `yunyou plugin new/install` 插件管理子命令（task-10，业主令「SDK 一起做了吧」）。
//!
//! - `plugin new <名> --category <类>`：生成脚手架（Cargo.toml+模板 main+manifest.toml
//!   +README，自动含分类目录约定），插件作者接着 `cargo build` 即得独立可执行；
//! - `plugin install <目录>`：按 manifest.toml 的 id+category 归类落盘
//!   `<home>/plugins/<category>/<id>/`（已存在=拒绝不覆盖，人话报错）。
//!
//! manifest v2 权威校验复用 [`crate::market_manifest::parse_and_validate`]（冻结面
//! 不另起炉灶）；本模块只做文件编排与拷贝。

use std::path::Path;

/// 脚手架模板 main.rs（SDK 最小插件全文；插件作者改工具逻辑即可）。
const TEMPLATE_MAIN: &str = r#"//! <描述>：用 yunyou-plugin-sdk 写的独立可执行插件。
//! 文档=SDK crate 的 SDK.md（15 分钟上手）。

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use yunyou_plugin_sdk::{tool, Plugin};

#[derive(Deserialize, JsonSchema)]
struct EchoArgs {
    /// 要回显的文本
    text: String,
}

fn main() -> std::process::ExitCode {
    Plugin::new("__ID__", "0.1.0")
        .category("__CATEGORY__")
        .description("__DESC__")
        .tool(tool("echo", "原样返回输入文本", |args: EchoArgs| {
            Ok(json!({ "echoed": args.text }))
        }))
        .serve()
}
"#;

/// 脚手架 Cargo.toml。
fn template_cargo_toml(name: &str) -> String {
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"

[[bin]]
name = "{name}"
path = "src/main.rs"

[dependencies]
# 过渡态=git 依赖（SDK 随主仓库发布；发布 crates.io 后切版本号形态）
yunyou-plugin-sdk = {{ git = "https://gitee.com/yiyuai/yunyou.git" }}
schemars = "1"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#
    )
}

/// 脚手架 manifest.toml（market v2 形态）。
fn template_manifest(id: &str, category: &str) -> String {
    format!(
        r#"# 落盘 manifest（market v2；字段权威校验=L2 设计 §3.1）
id = "{id}"
version = "0.1.0"
format = 2
level = "market"
contracts = ["tools.v1"]
category = "{category}"
auto_enable = true

[runtime]
type = "subprocess"
entry = ["{id}"]                  # 相对插件目录=cargo build 产物名
request_timeout_ms = 30000

[permissions]
writable = ["$SELF"]             # 缺省=仅插件目录可写
credentials = []                 # MVP 凭据面未开闸，恒空
network = false                  # MVP 网络面恒禁
"#
    )
}

const TEMPLATE_README: &str = r#"# <名>

yunyou-plugin-sdk 插件脚手架。

## 构建

```bash
cargo build --release
```

## 安装

```bash
yunyou plugin install <本目录>
```

文档=SDK crate 的 SDK.md。
"#;

/// `plugin new <名> --category <类> [--dir <落点>]`：生成脚手架目录。
///
/// 返回人话结算句（含后续两步指引）。
pub fn scaffold_new(name: &str, category: &str, dir: Option<&Path>) -> Result<String, String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        || name.starts_with('-')
    {
        return Err(format!(
            "插件名 \"{name}\" 不合法：只允许小写字母/数字/连字符（crate 名与工具 id 同源）"
        ));
    }
    let base = dir.unwrap_or(Path::new(".")).join(name);
    if base.exists() {
        return Err(format!("落点 {} 已存在（不覆盖，换名或换 --dir）", base.display()));
    }
    std::fs::create_dir_all(base.join("src"))
        .map_err(|err| format!("创建目录失败：{err}"))?;
    let main = TEMPLATE_MAIN
        .replace("__ID__", name)
        .replace("__CATEGORY__", category)
        .replace("__DESC__", &format!("{name} 插件"));
    let files: [(&str, String); 5] = [
        ("Cargo.toml", template_cargo_toml(name)),
        ("src/main.rs", main),
        ("manifest.toml", template_manifest(name, category)),
        ("README.md", TEMPLATE_README.replace("<名>", name)),
        (".gitignore", "/target\n".into()),
    ];
    for (rel, content) in files {
        std::fs::write(base.join(rel), content)
            .map_err(|err| format!("写 {rel} 失败：{err}"))?;
    }
    Ok(format!(
        "脚手架已生成：{base}\n下一步：\n  1) cd {base} && cargo build --release\n  2) yunyou plugin install {base}\n（工具逻辑改 src/main.rs；参数 schema 自动进模型面）",
        base = base.display()
    ))
}

/// `plugin install <目录>`：按 manifest 的 id+category 归类落盘。
///
/// 校验链=market_manifest::parse_and_validate 五类权威校验（坏 manifest 拒装）；
/// 目标已存在=拒绝不覆盖（人话报错指路手工处理）。
pub fn install(source_dir: &Path, home: &Path) -> Result<String, String> {
    let manifest_path = source_dir.join("manifest.toml");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|err| format!("读取 {} 失败：{err}", manifest_path.display()))?;
    let manifest = crate::market_manifest::parse_and_validate(&text, source_dir)
        .map_err(|violations| {
            format!(
                "manifest 校验违例（拒装，逐条修复后重试）：\n  {}",
                violations.join("\n  ")
            )
        })?;
    let category = manifest
        .category
        .clone()
        .unwrap_or_else(|| "misc".to_string());
    let target = home.join("plugins").join(&category).join(&manifest.id);
    if target.exists() {
        return Err(format!(
            "落点 {} 已存在（不覆盖；先手工删除旧版再安装，或直接替换目录内文件=升级重启生效）",
            target.display()
        ));
    }
    copy_dir_recursive(source_dir, &target).map_err(|err| format!("拷贝失败：{err}"))?;
    Ok(format!(
        "已安装：{src} → {dst}（category={category}）\n重启云悠即装载（/plugins 可见可开关）",
        src = source_dir.display(),
        dst = target.display(),
    ))
}

/// 内置工具名（撞名预警清单；与 yunyou-tools builtin 清单同源维护——
/// plugin_cmd 测试锁两处一致）。
pub const BUILTIN_TOOL_NAMES: &[&str] = &["bash", "apply_patch", "read", "write", "edit", "glob", "grep"];

/// 分类白名单（目录分类律：tools/integrations/workflows/ui + misc 兜底）。
pub const CATEGORY_WHITELIST: &[&str] = &["tools", "integrations", "workflows", "ui", "misc"];

/// 单项检查结果（✓/✗+人话；对齐 task-35 错误指路纪律）。
struct CheckItem {
    ok: bool,
    name: &'static str,
    detail: String,
    fix: String,
}

/// `plugin check <目录>`：作者装前自查（不等 install 报错）。
///
/// 校验逻辑单源：五类权威校验复用 [`crate::market_manifest::parse_and_validate`]
/// （与 install 同一套函数两处消费——check 供作者、install 供宿主，禁复制）；
/// 另加三项 SDK 时代新增：依赖形态/SDK 契约版本匹配/工具名撞名预警。
/// 返回人话清单（逐项 ✓/✗+修法），全部 ✓ 才建议安装。
pub fn check(source_dir: &Path) -> String {
    let mut items: Vec<CheckItem> = Vec::new();

    // ① manifest 存在+五类权威校验（与 install 同源）
    let manifest_path = source_dir.join("manifest.toml");
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(err) => {
            items.push(CheckItem {
                ok: false,
                name: "manifest.toml 存在且可读",
                detail: format!("读取失败：{err}"),
                fix: "在插件目录根写 manifest.toml（脚手架 yunyou plugin new 可生成模板）".into(),
            });
            return render(items);
        }
    };
    match crate::market_manifest::parse_and_validate(&text, source_dir) {
        Ok(m) => items.push(CheckItem {
            ok: true,
            name: "manifest 五类权威校验（schema/格式版本/runtime/entry/权限面）",
            detail: format!("id={} version={} category={:?}", m.id, m.version, m.category),
            fix: String::new(),
        }),
        Err(violations) => {
            items.push(CheckItem {
                ok: false,
                name: "manifest 五类权威校验",
                detail: violations.join("；"),
                fix: "逐条修复 manifest.toml 后重查（人话违例项已在上面）".into(),
            });
            return render(items);
        }
    }
    let manifest = crate::market_manifest::parse_and_validate(&text, source_dir)
        .expect("上方已验过，此处必 Ok");

    // ② 分类白名单（目录分类律；misc=兜底归档位）
    let category = manifest.category.clone().unwrap_or_else(|| "misc".into());
    let cat_ok = CATEGORY_WHITELIST.contains(&category.as_str());
    items.push(CheckItem {
        ok: cat_ok,
        name: "分类白名单",
        detail: format!("category={category}"),
        fix: if cat_ok { String::new() } else {
            format!("合法分类：{}——改 manifest.toml 的 category 字段", CATEGORY_WHITELIST.join("/"))
        },
    });

    // ③ 依赖形态：Cargo.toml 的 SDK 依赖禁 path 指向内核源码树
    let cargo_path = source_dir.join("Cargo.toml");
    if cargo_path.is_file() {
        let cargo = std::fs::read_to_string(&cargo_path).unwrap_or_default();
        let sdk_line = cargo
            .lines()
            .find(|l| l.trim_start().starts_with("yunyou-plugin-sdk"));
        match sdk_line {
            None => items.push(CheckItem {
                ok: false,
                name: "SDK 依赖声明",
                detail: "Cargo.toml 未声明 yunyou-plugin-sdk 依赖".into(),
                fix: r#"加一行：yunyou-plugin-sdk = { git = "https://gitee.com/yiyuai/yunyou.git" }"#.into(),
            }),
            Some(line) => {
                let is_path = line.contains("{ path") || line.contains("path =");
                let is_git_or_registry = line.contains("git =") || !line.contains('{');
                let ok = !is_path && is_git_or_registry;
                items.push(CheckItem {
                    ok,
                    name: "依赖形态（禁 path 指向内核源码树）",
                    detail: line.trim().to_string(),
                    fix: if ok { String::new() } else {
                        r#"外部开发者无内核源码树：改 yunyou-plugin-sdk = { git = "https://gitee.com/yiyuai/yunyou.git" }（发布 crates.io 后切版本号）"#
                            .into()
                    },
                });
            }
        }
    } else {
        items.push(CheckItem {
            ok: false,
            name: "Cargo.toml 存在",
            detail: "插件目录缺 Cargo.toml".into(),
            fix: "yunyou plugin new 可生成脚手架；或手写（SDK.md 有全文）".into(),
        });
    }

    // ④ SDK 契约版本匹配（SDK.md 声明的当前契约集=tools.v1；SLOT_WHITELIST 单源）
    let contracts_ok = manifest.contracts.iter().all(|c| crate::market_manifest::SLOT_WHITELIST.contains(&c.as_str()))
        && !manifest.contracts.is_empty();
    items.push(CheckItem {
        ok: contracts_ok,
        name: "SDK 契约版本匹配",
        detail: format!(
            "contracts={:?}，当前 SDK 支持集={:?}",
            manifest.contracts,
            crate::market_manifest::SLOT_WHITELIST
        ),
        fix: if contracts_ok { String::new() } else {
            "contracts 只允许声明当前 SDK 支持的插槽（tools.v1）；多声明=宿主拒装".into()
        },
    });

    // ⑤ 工具名撞名预警（与内置工具同名=模型面冲突；预警不阻断）
    let main_rs = std::fs::read_to_string(source_dir.join("src/main.rs")).unwrap_or_default();
    let declared: Vec<&str> = BUILTIN_TOOL_NAMES
        .iter()
        .filter(|n| main_rs.contains(&format!("tool(\"{n}\"")))
        .copied()
        .collect();
    items.push(CheckItem {
        ok: declared.is_empty(),
        name: "工具名与内置工具撞名预警",
        detail: if declared.is_empty() {
            "未发现撞名".into()
        } else {
            format!("与内置工具撞名：{}", declared.join("、"))
        },
        fix: if declared.is_empty() { String::new() } else {
            "改工具名（撞名=模型面歧义，宿主侧注册行为未定义）——如 weather_bash 替代 bash".into()
        },
    });

    render(items)
}

/// 渲染逐项 ✓/✗ 清单（人话修法只对 ✗ 输出）。
fn render(items: Vec<CheckItem>) -> String {
    let mut out = String::from("插件自查结果：
");
    for it in &items {
        let mark = if it.ok { "✓" } else { "✗" };
        out.push_str(&format!("  {mark} {}：{}
", it.name, it.detail));
        if !it.ok && !it.fix.is_empty() {
            out.push_str(&format!("      修法：{}
", it.fix));
        }
    }
    let all_ok = items.iter().all(|i| i.ok);
    if all_ok {
        out.push_str("全部通过——可 yunyou plugin install <本目录> 安装");
    } else {
        out.push_str("存在未通过项——修复后重查（安装会被同等校验拒绝）");
    }
    out
}

/// 递归拷贝目录（跳过 target/ 与 .git/——构建产物不随插件分发）。
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        let to = dst.join(&*name);
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

// 测试面豁免（K-14 观察期口径：tests/ 独立 crate root 不受 deny 罩；断言非 panic 语义）
#[allow(clippy::expect_used, clippy::unwrap_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 脚手架生成与manifest自洽() {
        let dir = std::env::temp_dir().join(format!("yunyou-scaffold-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let msg = scaffold_new("my-probe", "tools", Some(&dir)).expect("脚手架应生成");
        assert!(msg.contains("cargo build"));
        let base = dir.join("my-probe");
        assert!(base.join("src/main.rs").is_file());
        assert!(base.join("manifest.toml").is_file());
        // 生成的 manifest 必须过五类权威校验（自洽：脚手架产物即合法插件声明）
        let text = std::fs::read_to_string(base.join("manifest.toml")).unwrap();
        let m = crate::market_manifest::parse_and_validate(&text, &base)
            .expect("脚手架 manifest 应过校验");
        assert_eq!(m.id, "my-probe");
        assert_eq!(m.category.as_deref(), Some("tools"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 脚手架依赖无path指向内核_git形态() {
        // 业主令（2026-11-18 06:5x「相对路径问题不解决，外部开发者没有内核源码」）：
        // 生成包的 SDK 依赖必须=registry/git 二选一，**严禁 path 指向内核源码树**。
        let dir = std::env::temp_dir().join(format!("yunyou-scaffold-dep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        scaffold_new("dep-probe", "tools", Some(&dir)).expect("脚手架应生成");
        let cargo = std::fs::read_to_string(dir.join("dep-probe/Cargo.toml")).unwrap();
        assert!(cargo.contains(r#"yunyou-plugin-sdk = { git = "https://gitee.com/yiyuai/yunyou.git" }"#),
            "SDK 依赖应为 git 形态，实际：\n{cargo}");
        assert!(!cargo.contains("yunyou-plugin-sdk = { path"), "SDK 依赖禁 path 指向内核源码树（外部开发者无内核源码），实际：\n{cargo}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check四态_好件全绿缺manifest坏分类path依赖撞名() {
        let tmp = std::env::temp_dir().join(format!("yunyou-check-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        // ① 好插件（脚手架产物+真工具名）：全绿
        scaffold_new("good-probe", "tools", Some(&tmp)).expect("脚手架应生成");
        let good = tmp.join("good-probe");
        std::fs::create_dir_all(good.join("src")).unwrap();
        std::fs::write(good.join("src/main.rs"), "fn main() {}").unwrap();
        let report = check(&good);
        assert!(report.contains("全部通过"), "好插件应全绿，实际：\n{report}");
        // ② 缺 manifest：✗ 指路
        let missing = tmp.join("no-manifest");
        std::fs::create_dir_all(&missing).unwrap();
        let report = check(&missing);
        assert!(report.contains("✗") && report.contains("manifest.toml"), "缺 manifest 应报：\n{report}");
        // ③ 坏分类：✗+合法清单指路
        let bad = tmp.join("bad-cat");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            bad.join("manifest.toml"),
            "id = \"bad-cat\"\nversion = \"0.1.0\"\nformat = 2\nlevel = \"market\"\ncontracts = [\"tools.v1\"]\ncategory = \"not-a-cat\"\nauto_enable = true\n\n[runtime]\ntype = \"subprocess\"\nentry = [\"x\"]\n",
        ).unwrap();
        let report = check(&bad);
        assert!(report.contains("not-a-cat") && report.contains("tools/integrations"), "坏分类应指路：\n{report}");
        // ④ path 依赖红+⑤ 撞名预警
        scaffold_new("path-probe", "tools", Some(&tmp)).expect("脚手架应生成");
        let probe = tmp.join("path-probe");
        let cargo = std::fs::read_to_string(probe.join("Cargo.toml")).unwrap()
            .replace(r#"git = "https://gitee.com/yiyuai/yunyou.git""#, r#"path = "../../../yunyou""#);
        std::fs::write(probe.join("Cargo.toml"), cargo).unwrap();
        std::fs::create_dir_all(probe.join("src")).unwrap();
        std::fs::write(probe.join("src/main.rs"), r#"fn main() { let _ = tool("bash", "", |a: EmptyProbe| Ok(serde_json::json!({}))); } struct EmptyProbe; "#).unwrap();
        let report = check(&probe);
        assert!(report.contains("path") && report.contains("gitee"), "path 依赖应红+给 git 改法：\n{report}");
        assert!(report.contains("撞名") && report.contains("bash"), "撞名应预警：\n{report}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn 安装归类落盘与重复拒装() {
        let tmp = std::env::temp_dir().join(format!("yunyou-install-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("probe-src");
        scaffold_new("my-probe", "tools", Some(&tmp)).expect("脚手架应生成");
        std::fs::rename(tmp.join("my-probe"), &src).unwrap();
        let home = tmp.join("home");
        let msg = install(&src, &home).expect("安装应成功");
        assert!(msg.contains("plugins/tools/my-probe"));
        assert!(home.join("plugins/tools/my-probe/manifest.toml").is_file());
        // 重复安装=拒绝不覆盖
        assert!(install(&src, &home).is_err(), "重复安装应拒绝");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn 非法名与坏manifest拒装() {
        assert!(scaffold_new("Bad_Name", "tools", None).is_err());
        let tmp = std::env::temp_dir().join(format!("yunyou-badmanifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let src = tmp.join("bad");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("manifest.toml"), "id = \"bad\"\nformat = 9\n").unwrap();
        let err = install(&src, &tmp.join("home")).expect_err("坏 manifest 应拒装");
        assert!(err.contains("校验违例"), "实际：{err}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
