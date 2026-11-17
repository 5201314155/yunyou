//! market 插件 manifest v2 解析与五类拒装校验（L2 设计 §3.1；K-19 刀2 批2）。
//!
//! 加法演进律：基座=ManifestFile 八字段零改动（v2 新增 format/[runtime]/
//! [permissions] 段；bundled 命中分支零感知——serde default 兼容）。
//! **五类违例各有专项人话**（§4.4 fail-loud 全链，逐条报不拖累其他插件）：
//! ①格式版本未知 ②runtime 缺失/type 非法 ③entry 越界（绝对路径/../上跳/
//! 空/出插件目录）④插槽白名单外 ⑤凭据面未开 ⑥网络面未开（MVP 恒拒 true）。

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// market manifest v2 文件形态（manifest.toml 反序列化目标）。
#[derive(Debug, Clone, Deserialize)]
pub struct MarketManifestV2 {
    /// 插件 id（包名；全局唯一与 bundled 命名空间防撞由 discover 层管）。
    pub id: String,
    /// 插件版本。
    pub version: String,
    /// 落盘格式版本；v2 必填 =2（未知值拒装=GatewayIpcFrame 未知 type 同款语义）。
    pub format: u32,
    pub level: String,
    #[serde(default)]
    pub contracts: Vec<String>,
    #[serde(default)]
    pub auto_enable: bool,
    /// 分类目录约定（task-10：plugin install 自动归类落 plugins/<category>/<id>/）。
    #[serde(default)]
    pub category: Option<String>,
    /// 装载形态声明（v2 必填段）。
    pub runtime: RuntimeSection,
    /// 权限声明（缺省全最小：writable=$SELF、credentials 空、network=false）。
    #[serde(default)]
    pub permissions: PermissionsSection,
}

/// [runtime] 段（§3.1）。
#[derive(Debug, Clone, Deserialize)]
pub struct RuntimeSection {
    /// 装载形态：当前唯一合法值 "subprocess"（WASM 未来经此处演进不动结构）。
    pub r#type: String,
    /// 插件入口 argv（相对插件目录解析；禁绝对路径/禁 .. 上跳/禁空）。
    pub entry: Vec<String>,
    /// 单次工具调用超时毫秒（缺省 30000；超时=软失败计 fail-loud）。
    #[serde(default = "default_request_timeout")]
    pub request_timeout_ms: u64,
}

fn default_request_timeout() -> u64 {
    30_000
}

/// [permissions] 段（§3.1；缺省全最小）。
#[derive(Debug, Clone, Deserialize)]
pub struct PermissionsSection {
    /// Landlock 写白名单附加根；$SELF=插件目录；只接受 $SELF/绝对路径。
    #[serde(default = "default_writable")]
    pub writable: Vec<String>,
    /// 凭据名声明（L2.1 生效；MVP 非空=拒装「凭据面未开放」）。
    #[serde(default)]
    pub credentials: Vec<String>,
    /// 网络放行开关（MVP 恒拒 true；L2.1 起按声明放行）。
    #[serde(default)]
    pub network: bool,
}

fn default_writable() -> Vec<String> {
    vec!["$SELF".to_string()]
}

impl Default for PermissionsSection {
    fn default() -> Self {
        Self {
            writable: default_writable(),
            credentials: Vec::new(),
            network: false,
        }
    }
}

/// L2-MVP 允许 market 插件消费的宿主插槽白名单（§3.1 contracts 消费面）。
pub const SLOT_WHITELIST: &[&str] = &["tools.v1"];

/// 解析+五类校验（聚合违例逐条人话；全过=Ok 形态）。
///
/// `plugin_dir`：插件目录（entry 相对解析锚+$SELF 展开）。
pub fn parse_and_validate(
    toml_text: &str,
    plugin_dir: &Path,
) -> Result<MarketManifestV2, Vec<String>> {
    let mut violations: Vec<String> = Vec::new();
    let manifest: MarketManifestV2 = match toml::from_str(toml_text) {
        Ok(m) => m,
        Err(err) => return Err(vec![format!("manifest.toml 解析失败：{err}")]),
    };

    // ①格式版本未知（对齐 GatewayIpcFrame 未知 type 拒绝语义）
    if manifest.format != 2 {
        violations.push(format!(
            "不支持的格式版本 {}（当前支持 2）——请升级 manifest 或降级装载器",
            manifest.format
        ));
    }
    // ②runtime 缺失/type 非法
    if manifest.runtime.r#type != "subprocess" {
        violations.push(format!(
            "装载形态 \"{}\" 不支持（当前唯一合法值 \"subprocess\"；WASM 二期经此处演进）",
            manifest.runtime.r#type
        ));
    }
    // ③entry 越界：空/绝对路径/../上跳/解析后出插件目录
    if manifest.runtime.entry.is_empty() {
        violations.push("runtime.entry 为空（至少一项入口 argv）".into());
    } else {
        for (idx, arg) in manifest.runtime.entry.iter().enumerate() {
            let p = Path::new(arg);
            if p.is_absolute() {
                violations.push(format!(
                    "runtime.entry[{idx}] 是绝对路径（\"{arg}\"）——只允许相对路径或 PATH 命令名"
                ));
                continue;
            }
            if p.components().any(|c| c == std::path::Component::ParentDir) {
                violations.push(format!(
                    "runtime.entry[{idx}] 含 \"..\" 上跳（\"{arg}\"）——禁止越出插件目录"
                ));
                continue;
            }
            // 含分隔符=路径项：解析后必须落在插件目录内（§3.1「可执行文件必须
            // 落在插件目录内」）；无分隔符=PATH 命令名（底册自例 python3 形态，
            // 系统解释器不属插件目录管——spawn 失败=Failed 人话兜底）
            if arg.contains('/') || arg.starts_with("./") {
                let resolved = plugin_dir.join(p);
                if !resolved.starts_with(plugin_dir) {
                    violations.push(format!(
                        "runtime.entry[{idx}] 解析后越出插件目录：{}",
                        resolved.display()
                    ));
                } else if idx == 0 && !resolved.is_file() {
                    violations.push(format!(
                        "runtime.entry[0] 可执行文件不在插件目录内或不存在：{}",
                        resolved.display()
                    ));
                }
            }
        }
    }
    // 超时非法
    if manifest.runtime.request_timeout_ms == 0 {
        violations.push("runtime.request_timeout_ms 必须 >0（0=永不超时不允许）".into());
    }
    // ④插槽白名单外（contracts 消费面）
    for c in &manifest.contracts {
        if !SLOT_WHITELIST.contains(&c.as_str()) {
            violations.push(format!(
                "插槽 {c} 未对进程外插件开放（L2-MVP 允许集=仅 tools.v1）"
            ));
        }
    }
    // ⑤凭据面未开（L2-MVP 整体不开闸）
    if !manifest.permissions.credentials.is_empty() {
        violations.push(
            "凭据面未开放（permissions.credentials 非空——L2-MVP 插件为纯本地计算工具，L2.1 起按声明放行）"
                .into(),
        );
    }
    // ⑥网络面未开（MVP 恒拒 true——先关闸）
    if manifest.permissions.network {
        violations.push(
            "网络面未开放（permissions.network=true 恒拒——L2-MVP 协议词汇无网络面承诺，L2.1 起按声明放行）"
                .into(),
        );
    }
    // permissions.writable 形态校验（$SELF/绝对路径；逐条人话警告不阻断的
    // 「非插件目录内路径」警告面在装配层——此处只拒非法形态）
    for (idx, w) in manifest.permissions.writable.iter().enumerate() {
        if w != "$SELF" && !Path::new(w).is_absolute() {
            violations.push(format!(
                "permissions.writable[{idx}]（\"{w}\"）只接受 $SELF 或绝对路径"
            ));
        }
    }

    if violations.is_empty() {
        Ok(manifest)
    } else {
        Err(violations)
    }
}

/// entry[0] 解析为绝对路径（相对插件目录；已过校验——调用方持 Ok 形态）。
pub fn resolved_entry(manifest: &MarketManifestV2, plugin_dir: &Path) -> PathBuf {
    plugin_dir.join(Path::new(&manifest.runtime.entry[0]))
}

/// $SELF 展开后的 Landlock 可写根集合（插件目录+声明项；tmp 并入由 prepare 层）。
pub fn writable_roots(manifest: &MarketManifestV2, plugin_dir: &Path) -> Vec<PathBuf> {
    manifest
        .permissions
        .writable
        .iter()
        .map(|w| {
            if w == "$SELF" {
                plugin_dir.to_path_buf()
            } else {
                PathBuf::from(w)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK_TOML: &str = r#"
id = "hello"
version = "0.1.0"
format = 2
level = "market"
contracts = ["tools.v1"]
auto_enable = true

[runtime]
type = "subprocess"
entry = ["python3", "main.py"]
request_timeout_ms = 5000

[permissions]
writable = ["$SELF"]
credentials = []
network = false
"#;

    /// 五类违例逐条人话（表驱动——§4.4 fail-loud 全链）。
    #[test]
    fn 五类违例逐条() {
        let dir = std::env::temp_dir().join("k19-mv2-ok");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.py"), "# entry").unwrap(); // entry[0] 存在性校验真实现
        // 全过基线
        assert!(parse_and_validate(OK_TOML, &dir).is_ok(), "合法 manifest 应全过");

        // ①格式版本未知
        let v = parse_and_validate(OK_TOML.replace("format = 2", "format = 3").as_str(), &dir).unwrap_err();
        assert!(v.iter().any(|s| s.contains("不支持的格式版本")), "{v:?}");

        // ②runtime type 非法
        let v = parse_and_validate(OK_TOML.replace("\"subprocess\"", "\"wasm\"").as_str(), &dir).unwrap_err();
        assert!(v.iter().any(|s| s.contains("装载形态")), "{v:?}");

        // ③entry 越界（绝对路径/../空）
        let v = parse_and_validate(OK_TOML.replace("\"python3\"", "\"/usr/bin/python3\"").as_str(), &dir).unwrap_err();
        assert!(v.iter().any(|s| s.contains("绝对路径")), "{v:?}");
        let v = parse_and_validate(OK_TOML.replace("\"main.py\"", "\"../evil.py\"").as_str(), &dir).unwrap_err();
        assert!(v.iter().any(|s| s.contains("..\" 上跳")), "{v:?}");

        // ④插槽白名单外
        let v = parse_and_validate(
            OK_TOML.replace("contracts = [\"tools.v1\"]", "contracts = [\"session.v1\"]").as_str(),
            &dir,
        )
        .unwrap_err();
        assert!(v.iter().any(|s| s.contains("session.v1 未对进程外插件开放")), "{v:?}");

        // ⑤凭据面未开
        let v = parse_and_validate(
            OK_TOML.replace("credentials = []", "credentials = [\"openai_key\"]").as_str(),
            &dir,
        )
        .unwrap_err();
        assert!(v.iter().any(|s| s.contains("凭据面未开放")), "{v:?}");

        // ⑥网络面未开
        let v = parse_and_validate(OK_TOML.replace("network = false", "network = true").as_str(), &dir).unwrap_err();
        assert!(v.iter().any(|s| s.contains("网络面未开放")), "{v:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// $SELF 展开+writable 形态校验（非 $SELF 非绝对=拒）。
    #[test]
    fn writable_形态与_self展开() {
        let dir = std::env::temp_dir().join("k19-mv2-w");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.py"), "# entry").unwrap();
        let m = parse_and_validate(OK_TOML, &dir).unwrap();
        let roots = writable_roots(&m, &dir);
        assert_eq!(roots[0], dir, "$SELF=插件目录");
        let bad = OK_TOML.replace("writable = [\"$SELF\"]", "writable = [\"relative/path\"]");
        let v = parse_and_validate(&bad, &dir).unwrap_err();
        assert!(v.iter().any(|s| s.contains("只接受 $SELF 或绝对路径")), "{v:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
