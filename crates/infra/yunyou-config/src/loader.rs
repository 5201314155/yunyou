//! yunyou.toml 加载流水线：文本 → TOML 解析 → schema 校验 → 强类型 → env 覆盖。
//!
//! 校验错误聚合复用内核 [`yunyou::config`]（对齐内核 ValidationError 风格，
//! 多问题一次报齐；错误码表统一走内核 ConfigValidation）。

use crate::model::{default_env_overrides, DshConfig};
use yunyou::config::{validate_schema, ValidationEntry, ValidationError};
use serde_json::Value;
use std::fmt;
use std::path::Path;

/// 配置加载错误：文件 I/O / TOML 解析 / schema 校验三类。
#[derive(Debug)]
pub enum ConfigError {
    /// 文件读取失败（含路径，便于定位）。
    Io { path: String, source: std::io::Error },
    /// TOML 文本解析失败（脱敏：只含 message 与行号，不含出错行原文——
    /// 挑刺报告 20261117 F-2：行原文可能带明文 key，会随错误信息进日志）。
    Parse { line: usize, message: String },
    /// 结构校验失败（聚合多条；含「校验通过但反序列化失败」的防御分支）。
    Validation(ValidationError),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "读取配置文件 {path} 失败：{source}"),
            Self::Parse { line, message } => write!(f, "yunyou.toml 解析失败（第 {line} 行附近）：{message}"),
            Self::Validation(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Parse { .. } => None,
            Self::Validation(err) => Some(err),
        }
    }
}

/// 加载并校验配置文本（不含 env 覆盖；供测试与 `load` 复用）。
pub fn load_from_str(text: &str) -> Result<DshConfig, ConfigError> {
    // ① TOML 文本 → 统一配置载体（serde_json::Value）
    // F-2：解析错误只留 message+行号，Display 全文（含源码行原文）不得外透
    let raw: Value = toml::from_str(text).map_err(|err| ConfigError::Parse {
        line: err
            .span()
            .map(|span| text.as_bytes()[..span.start.min(text.len())].iter().filter(|b| **b == b'\n').count() + 1)
            .unwrap_or(1),
        message: err.message().to_string(),
    })?;
    // ② schemars 从模型同源生成 JSON Schema 2020-12，交内核校验器聚合执行
    let schema = serde_json::to_value(schemars::schema_for!(DshConfig))
        .expect("DshConfig 的 schema 生成不应失败（derive 同源）");
    validate_schema(&schema, &raw).map_err(ConfigError::Validation)?;
    // ③ 反序列化为强类型；正常不应失败，失败说明校验器与模型漂移，按校验错误暴露
    let mut config: DshConfig = serde_json::from_value(raw).map_err(|err| {
        ConfigError::Validation(ValidationError::new(vec![ValidationEntry {
            path: String::new(),
            message: format!("校验通过但反序列化失败（校验器与模型漂移）：{err}"),
        }]))
    })?;
    // ④ 环境变量覆盖（注入点见 model::apply_env_overrides）
    default_env_overrides(&mut config);
    Ok(config)
}

/// 从指定路径加载配置（文件必须存在，缺失即 Io 错误——显式路径代表明确意图）。
pub fn load(path: &Path) -> Result<DshConfig, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
        path: path.display().to_string(),
        source,
    })?;
    load_from_str(&text)
}

/// 可选路径加载：`--config` 未传（None）或文件不存在时返回全默认配置。
///
/// 支撑联调验收「一条命令启动」：不写 yunyou.toml 也能跑（默认值 + env 覆盖）；
/// 但路径**显式传入**而文件缺失属于拼写错误，报 Io 错误（fail-loud）。
pub fn load_optional(path: Option<&Path>) -> Result<DshConfig, ConfigError> {
    match path {
        Some(path) if path.exists() => load(path),
        Some(path) => Err(ConfigError::Io {
            path: path.display().to_string(),
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "显式指定的配置文件不存在",
            ),
        }),
        None => {
            let mut config = DshConfig::default();
            default_env_overrides(&mut config);
            Ok(config)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 完整合法配置：各段字段全部解析到位。
    #[test]
    fn 完整配置解析() {
        let text = r#"
[llm]
base_url = "http://example.test/v1"
model = "demo/model"
timeout_secs = 30

[session]
path = "/tmp/yunyou-sessions/chat.jsonl"

[system_prompt]
persona = "你是测试助手"

[tools]
root_dir = "/tmp/yunyou-workdir"

[agent_loop]
persona = "测试人设"
tool_timeout_secs = 10
"#;
        let cfg = load_from_str(text).expect("合法配置应通过");
        assert_eq!(cfg.llm.base_url, "http://example.test/v1");
        assert_eq!(cfg.llm.model, "demo/model");
        assert_eq!(cfg.llm.timeout_secs, 30);
        assert_eq!(cfg.session.path.as_deref(), Some("/tmp/yunyou-sessions/chat.jsonl"));
        assert_eq!(cfg.system_prompt.persona.as_deref(), Some("你是测试助手"));
        assert_eq!(cfg.tools.root_dir, "/tmp/yunyou-workdir");
        assert_eq!(cfg.agent_loop.persona.as_deref(), Some("测试人设"));
        assert_eq!(cfg.agent_loop.tool_timeout_secs, 10);
    }

    /// L-1（挑刺报告 20261117）：[agent_loop] max_steps/token_budget 护栏旋钮
    /// 按文档配置可加载（缺席曾致 deny_unknown_fields 启动硬失败）；默认 None=关。
    #[test]
    fn agent_loop_护栏旋钮可配置() {
        // 默认关：不配即 None（护栏语义不变）
        let cfg = load_from_str("").expect("空配置应通过");
        assert_eq!(cfg.agent_loop.max_steps, None);
        assert_eq!(cfg.agent_loop.token_budget, None);
        // 按文档配置：加载成功且值直达（后续经段透传进插件 config → runner）
        let text = "[agent_loop]\nmax_steps = 5\ntoken_budget = 100000\n";
        let cfg = load_from_str(text).expect("文档口径的护栏配置应能加载");
        assert_eq!(cfg.agent_loop.max_steps, Some(5));
        assert_eq!(cfg.agent_loop.token_budget, Some(100_000));
    }

    /// 空文档与空表：全部落默认值（联调「一条命令启动」的兜底路径）。
    #[test]
    fn 空配置落默认值() {
        let cfg = load_from_str("").expect("空文档应通过");
        assert_eq!(cfg, DshConfig::default());
        let cfg = load_from_str("\n").expect("空表应通过");
        assert_eq!(cfg, DshConfig::default());
    }

    /// TOML 语法错误：报 Parse（脱敏形态）。
    #[test]
    fn 语法错误报_parse() {
        let err = load_from_str("[llm\nbase_url=1").expect_err("语法错误应失败");
        assert!(matches!(err, ConfigError::Parse { .. }), "实际：{err}");
    }

    /// F-2（挑刺报告 20261117）：含明文 key 的坏 toml，错误信息**不得**回显
    /// 出错行原文（key 值），只透 message+行号。
    #[test]
    fn 解析错误不回显明文_key() {
        let secret = "sk-超级机密-9999";
        let text = format!("[llm]\nmodel = \"m\"\napi_key = \"{secret}\"\n[broken\n");
        let err = load_from_str(&text).expect_err("语法错误应失败");
        let rendered = err.to_string();
        assert!(!rendered.contains(secret), "错误信息泄漏明文 key：{rendered}");
        assert!(rendered.contains("第 4 行"), "应含行号锚点：{rendered}");
        assert!(matches!(err, ConfigError::Parse { line: 4, .. }), "实际：{err:?}");
    }

    /// 字段类型错误：走内核 ValidationError 聚合（对齐内核错误风格）。
    #[test]
    fn 类型错误聚合为_validation() {
        let text = "[llm]\ntimeout_secs = \"abc\"\n[agent_loop]\ntool_timeout_secs = true\n";
        let err = load_from_str(text).expect_err("类型错误应失败");
        let ConfigError::Validation(ve) = err else {
            panic!("应报 Validation，实际：{err:?}")
        };
        assert!(ve.entries().len() >= 2, "应聚合多条问题：{ve}");
        assert!(ve.to_string().starts_with("invalid config:"), "展示对齐内核：{ve}");
    }

    /// 未知顶层键：fail-loud（拼写防呆，一期静态裁剪语义）。
    #[test]
    fn 未知顶层键报错() {
        let text = "[llm]\nmodel = \"m\"\n\n[typo_section]\nx = 1\n";
        let err = load_from_str(text).expect_err("未知段应失败");
        assert!(matches!(err, ConfigError::Validation(_)), "实际：{err}");
    }

    /// load_optional：显式路径缺失报 Io；缺路径返回默认。
    #[test]
    fn 可选路径语义() {
        let missing = PathBuf::from("/nonexistent/yunyou/yunyou.toml");
        let err = load_optional(Some(&missing)).expect_err("显式缺失应失败");
        assert!(matches!(err, ConfigError::Io { .. }), "实际：{err}");
        let cfg = load_optional(None).expect("无路径应落默认");
        assert_eq!(cfg, DshConfig::default());
    }

    /// load：临时文件正常读取（写入真实文件系统验证 Io 路径）。
    #[test]
    fn 从文件加载() {
        let dir = std::env::temp_dir().join(format!("yunyou-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("应能建临时目录");
        let path = dir.join("yunyou.toml");
        std::fs::write(&path, "[agent_loop]\ntool_timeout_secs = 8\n").expect("应能写临时文件");
        let cfg = load(&path).expect("应能加载");
        assert_eq!(cfg.agent_loop.tool_timeout_secs, 8);
        std::fs::remove_dir_all(&dir).expect("应能清理临时目录");
    }
}
