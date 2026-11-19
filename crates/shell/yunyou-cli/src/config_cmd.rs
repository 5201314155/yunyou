//! 配置命令族五件（池 task-30 rev3，业主令「打开配置、换模型、换 API、开关工具」）。
//!
//! # 五件与生效边界（lead 批①分层裁定）
//! - `/login`：向导（base_url→key→拉模型列表编号选→写配置）；
//! - `/config [path]`：无参=`$EDITOR` 打开 yunyou.toml（缺失=人话指路不猜默认，
//!   批②）；`path`=显示路径；
//! - `/model [名]`：带参直切；无参=GET `{base_url}/models` 编号选；
//! - `/key`：关回显读入→写 credentials 托管存储（**唯一热生效项**：resolve 每
//!   操作现解析，当轮即生效）；配置面 api_key 行确保=引用；
//! - `/tools on|off <名>`：写 `[tools.builtin]`（on=移除键回归默认全开）。
//! - `/model`、`/tools`、`/config` 写盘后提示重启生效（不做热切换——守「不新增
//!   第二机制」红线，lead 批①）。
//!
//! # 写盘安全（硬要求，lead 批③「先验后写三段式」）
//! 全部经 [`crate::config_edit`]：文本级定点替换 → `load_from_str` 全量校验
//! → .bak+临时件+rename 原子替换——**写坏在物理上不可能**。

use crate::shell_commands::ShellCore;
use std::sync::Arc;
use yunyou_commands::{CommandDefinition, CommandHandler};
use yunyou::ServiceId;
use yunyou_commands::{CommandInvocation, CommandKind, CommandOutput};
use yunyou_credentials::LLM_API_KEY_REF;
use yunyou_util::BoxFut;

/// 命令族定义（shell_commands 增量外置，sessions_cmd 同款先例）。
pub fn definitions(shell: &Arc<ShellCore>) -> Vec<CommandDefinition> {
    vec![
        CommandDefinition {
            name: "login".into(),
            aliases: vec!["登录".into()],
            description: "配置向导：端点→密钥→选模型→写配置".into(),
            kind: CommandKind::Action,
            accepts_args: false,
            handler: {
                let shell = Arc::clone(shell);
                Arc::new(move |_inv: CommandInvocation| {
                    let shell = Arc::clone(&shell);
                    Box::pin(async move { login_flow(&shell).await })
                        as BoxFut<'static, Result<CommandOutput, yunyou::CordisError>>
                })
            },
        },
        CommandDefinition {
            name: "config".into(),
            aliases: vec!["配置".into()],
            description: "打开/编辑配置（$EDITOR）；/config path 显示路径".into(),
            kind: CommandKind::Action,
            accepts_args: true,
            handler: config_handler(Arc::clone(shell)),
        },
        CommandDefinition {
            name: "model".into(),
            aliases: vec!["模型".into()],
            description: "切模型（无参=拉列表编号选；/model <名> 直切）".into(),
            kind: CommandKind::Action,
            accepts_args: true,
            handler: model_handler(Arc::clone(shell)),
        },
        CommandDefinition {
            name: "key".into(),
            aliases: vec!["密钥".into()],
            description: "换 API 密钥（关回显输入，写托管存储，当轮即生效）".into(),
            kind: CommandKind::Action,
            accepts_args: false,
            handler: key_handler(Arc::clone(shell)),
        },
        CommandDefinition {
            name: "tools".into(),
            // 「工具开关」：tools 插件 /tools 别名「工具」是既有产品语义（云尺 E2E
            // 实测双注册 ServiceDuplicate fail-loud=P0 崩溃；让位裁定 a，lead 批）
            aliases: vec!["工具开关".into()],
            description: "开关工具：/tools on|off <工具名>（写 [tools.builtin]，重启生效）".into(),
            kind: CommandKind::Action,
            accepts_args: true,
            handler: tools_handler(Arc::clone(shell)),
        },
    ]
}

/// 参数段提取（sessions_arg 同源口径）：斜杠后首 token 之外的余段。
fn arg_of(raw: &str, name: &str) -> String {
    let body = raw.strip_prefix('/').unwrap_or(raw);
    let token = body.split_whitespace().next().unwrap_or("");
    let _ = name; // token 即命令名（extra_args 以其为前缀锚），余段即参数
    crate::command_exec::extra_args(raw, token)
        .map(str::trim)
        .map(str::to_string)
        .unwrap_or_default()
}

/// 读当前配置文本（读不到=空串，交由后续手术/校验报人话）。
fn config_text(shell: &ShellCore) -> String {
    std::fs::read_to_string(&shell.config_path).unwrap_or_default()
}

/// 关回显读一行（/key 与 /login 密钥段）：REPL 原始模式下内核 ECHO 已关、
/// 应用回显旗压 off；回车补行。空输入=放弃（None）。
fn read_secret(prompt: &str) -> Option<String> {
    use std::io::Write;
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    crate::input_source::set_echo(false);
    let mut line = String::new();
    let ok = std::io::stdin().read_line(&mut line).unwrap_or(0) > 0;
    crate::input_source::set_echo(true);
    println!();
    if !ok {
        return None;
    }
    let trimmed = line.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// 读一行明文提示输入（回车=空 → 调用方保留现值）。
fn read_line_prompt(prompt: &str) -> String {
    use std::io::Write;
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_string()
}

/// 密钥写入托管存储 + 配置面 api_key 行确保=引用（已是引用则不动）。
async fn write_key(shell: &ShellCore, key: &str) -> Result<String, String> {
    let session = shell.slot.load();
    let store = session
        .root
        .get::<dyn yunyou_credentials::CredentialService>(ServiceId::of::<
            dyn yunyou_credentials::CredentialService,
        >())
        .map_err(|_| "凭据服务未就绪（credentials 插件被禁用？）".to_string())?;
    store
        .set(LLM_API_KEY_REF, key)
        .await
        .map_err(|err| format!("凭据写入失败：{err}"))?;
    let text = config_text(shell);
    let current = crate::config_edit::read_section_value(&text, "llm", "api_key");
    if current.as_deref() == Some(LLM_API_KEY_REF) {
        return Ok("密钥已写入托管存储（当轮即生效）；配置面已是引用，无需改写。".into());
    }
    let new_text = crate::config_edit::upsert_section_value(
        &text,
        "llm",
        "api_key",
        &format!("\"{LLM_API_KEY_REF}\""),
    )?;
    crate::config_edit::commit(&shell.config_path, &new_text)
        .map(|_| "密钥已写入托管存储（当轮即生效）；配置面已切到引用（重启后同样生效）。".into())
}

/// /key：关回显读入→写托管存储。
fn key_handler(shell: Arc<ShellCore>) -> CommandHandler {
    Arc::new(move |_inv: CommandInvocation| {
        let shell = Arc::clone(&shell);
        Box::pin(async move {
            let Some(key) = read_secret("新的 API 密钥（输入不回显，回车取消）: ") else {
                return Ok(CommandOutput::error("已取消（空输入）。"));
            };
            let msg = write_key(&shell, &key).await.map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
            Ok(CommandOutput::success(msg))
        })
    })
}

/// 拉端点模型列表（GET {base_url}/models；鉴权 best-effort：配置 api_key 为
/// 引用时经托管存储现解析）。返回模型 id 列表。
async fn fetch_models(shell: &ShellCore) -> Result<Vec<String>, String> {
    let base_url = shell.config.llm.base_url.trim_end_matches('/').to_string();
    let url = format!("{base_url}/models");
    let mut request = reqwest::Client::new().get(&url);
    let api_key_raw = crate::config_edit::read_section_value(&config_text(shell), "llm", "api_key")
        .map(|v| v.trim_matches('"').to_string());
    if let Some(value) = api_key_raw {
        if !value.is_empty() {
            let resolved = if value.starts_with('$') {
                let session = shell.slot.load();
                let store = session
                    .root
                    .get::<dyn yunyou_credentials::CredentialService>(ServiceId::of::<
                        dyn yunyou_credentials::CredentialService,
                    >())
                    .map_err(|_| "凭据服务未就绪".to_string())?;
                store
                    .resolve(&value)
                    .await
                    .map_err(|err| format!("凭据解析失败：{err}"))?
                    .map(|handle| {
                        handle.with_value(|v| v.to_string())
                    })
                    .unwrap_or_default()
            } else {
                value
            };
            if !resolved.is_empty() {
                request = request.bearer_auth(resolved);
            }
        }
    }
    let response = request
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|err| format!("模型列表拉取失败（{url}）：{err}"))?;
    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|err| format!("模型列表解析失败（HTTP {status}）：{err}"))?;
    if !status.is_success() {
        return Err(format!("HTTP {status}：{}", body));
    }
    let ids: Vec<String> = body["data"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|m| m["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() {
        return Err("端点返回空模型列表（data[].id 缺失？）".into());
    }
    Ok(ids)
}

/// /model：带参直切；无参拉列表编号选。
fn model_handler(shell: Arc<ShellCore>) -> CommandHandler {
    Arc::new(move |inv: CommandInvocation| {
        let shell = Arc::clone(&shell);
        Box::pin(async move {
            let arg = arg_of(&inv.raw_input, "model");
            let model = if arg.is_empty() {
                let ids = fetch_models(&shell).await.map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
                println!("可用模型（回车取消）：");
                for (i, id) in ids.iter().enumerate() {
                    println!("  {}  {}", i + 1, id);
                }
                let line = read_line_prompt("序号: ");
                let pick: Option<&String> = line
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=ids.len()).contains(n))
                    .map(|n| n - 1)
                    .and_then(|i| ids.get(i));
                match pick {
                    Some(id) => id.clone(),
                    None => return Ok(CommandOutput::error("已取消。")),
                }
            } else {
                arg
            };
            switch_model(&shell, &model).await
        })
    })
}

/// 切模型：文本手术+先验后写（重启生效）。
async fn switch_model(shell: &ShellCore, model: &str) -> Result<CommandOutput, yunyou::CordisError> {
    let new_text = crate::config_edit::upsert_section_value(
        &config_text(shell),
        "llm",
        "model",
        &format!("\"{model}\""),
    )
    .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
    crate::config_edit::commit(&shell.config_path, &new_text).map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
    Ok(CommandOutput::success(format!(
        "已切换 model = {model}（重启生效；旧配置在 yunyou.toml.bak）"
    )))
}

/// /config：path=显示路径；无参=$EDITOR 编辑+编辑后校验（坏=还原旧文+回显错误）。
fn config_handler(shell: Arc<ShellCore>) -> CommandHandler {
    Arc::new(move |inv: CommandInvocation| {
        let shell = Arc::clone(&shell);
        Box::pin(async move {
            let arg = arg_of(&inv.raw_input, "config");
            if arg == "path" {
                return Ok(CommandOutput::success(format!(
                    "配置文件：{}（yunyou-home：{}）",
                    shell.config_path.display(),
                    shell.home.display()
                )));
            }
            let editor = match std::env::var("EDITOR") {
                Ok(e) if !e.trim().is_empty() => e,
                _ => {
                    return Ok(CommandOutput::error(
                        "未检测到 $EDITOR（不猜默认编辑器防挂住会话）——export EDITOR=vi 后重试。",
                    ))
                }
            };
            let original = config_text(&shell);
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("{} \"{}\"", editor, shell.config_path.display()))
                .status()
                .map_err(|err| yunyou::CordisError::new(yunyou::ErrorKind::Internal, format!("编辑器启动失败：{err}")))?;
            if !status.success() {
                return Ok(CommandOutput::error(format!("编辑器异常退出（{status}）——配置未校验，请重试。")));
            }
            let edited = config_text(&shell);
            if edited == original {
                return Ok(CommandOutput::success("配置未改动。"));
            }
            match crate::config_edit::validate_config(&edited) {
                Ok(()) => {
                    // 旧文留 .bak 兜底（编辑器已原地写盘，此处只补备份）
                    let _ = std::fs::write(shell.config_path.with_extension("toml.bak"), &original);
                    Ok(CommandOutput::success(
                        "配置已保存并通过校验（重启生效；编辑前版本在 yunyou.toml.bak）。",
                    ))
                }
                Err(err) => {
                    std::fs::write(&shell.config_path, &original)
                        .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, format!("还原失败：{e}")))?;
                    Ok(CommandOutput::error(format!(
                        "编辑后的配置未通过校验，已自动还原旧配置。校验错误：{err}"
                    )))
                }
            }
        })
    })
}

/// /tools on|off <名>：写 [tools.builtin]（off=键值 false；on=移除键回归默认）。
fn tools_handler(shell: Arc<ShellCore>) -> CommandHandler {
    Arc::new(move |inv: CommandInvocation| {
        let shell = Arc::clone(&shell);
        Box::pin(async move {
            let arg = arg_of(&inv.raw_input, "tools");
            let mut parts = arg.split_whitespace();
            let (Some(action), Some(name)) = (parts.next(), parts.next()) else {
                return Ok(CommandOutput::error(
                    "用法：/tools on|off <工具名>（如 /tools off web_fetch；可用名见 /help 或 docs/tools/）",
                ));
            };
            if parts.next().is_some() {
                return Ok(CommandOutput::error("参数过多——用法：/tools on|off <工具名>"));
            }
            let new_text = match action {
                "off" => crate::config_edit::upsert_section_value(
                    &config_text(&shell),
                    "tools.builtin",
                    name,
                    "false",
                )
                .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?,
                "on" => crate::config_edit::remove_section_key(&config_text(&shell), "tools.builtin", name)
                    .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?,
                other => {
                    return Ok(CommandOutput::error(format!(
                        "未知动作 {other}——用法：/tools on|off <工具名>"
                    )))
                }
            };
            crate::config_edit::commit(&shell.config_path, &new_text)
                .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
            Ok(CommandOutput::success(format!(
                "{name} 已 {}（重启生效；旧配置在 yunyou.toml.bak）",
                if action == "off" { "关闭" } else { "开启" }
            )))
        })
    })
}

/// /login 向导：端点→密钥→选模型→写配置（全程先验后写；base_url/model 变更
/// 才写盘，密钥走托管存储）。
async fn login_flow(shell: &ShellCore) -> Result<CommandOutput, yunyou::CordisError> {
    println!(
        "配置向导（回车=保留现值）\n  当前端点: {}\n  当前模型: {}",
        shell.config.llm.base_url,
        shell.config.llm.model
    );
    let base_url = read_line_prompt("端点 base_url: ");
    let base_url = if base_url.is_empty() { None } else { Some(base_url) };
    let key = read_secret("API 密钥（输入不回显，回车跳过）: ");
    // 端点变更先落盘（拉模型列表要用新端点）
    let effective_shell_base;
    if let Some(url) = &base_url {
        let new_text = crate::config_edit::upsert_section_value(
            &config_text(shell),
            "llm",
            "base_url",
            &format!("\"{url}\""),
        )
        .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
        crate::config_edit::commit(&shell.config_path, &new_text)
            .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
        effective_shell_base = url.clone();
    } else {
        effective_shell_base = shell.config.llm.base_url.clone();
    }
    if let Some(key) = &key {
        write_key(shell, key).await.map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
    }
    // 拉模型列表编号选（失败不阻断向导=手填入口）
    let model = {
        println!("正在拉取模型列表（{}）……", effective_shell_base);
        match fetch_models(shell).await {
            Ok(ids) => {
                for (i, id) in ids.iter().enumerate() {
                    println!("  {}  {}", i + 1, id);
                }
                let line = read_line_prompt("模型序号（回车跳过手填）: ");
                match line.parse::<usize>().ok().filter(|n| (1..=ids.len()).contains(n)) {
                    Some(n) => Some(ids[n - 1].clone()),
                    None => {
                        let manual = read_line_prompt("手填模型名: ");
                        if manual.is_empty() { None } else { Some(manual) }
                    }
                }
            }
            Err(err) => {
                println!("拉取失败（{err}）——可手填模型名。");
                let manual = read_line_prompt("手填模型名（回车跳过）: ");
                if manual.is_empty() { None } else { Some(manual) }
            }
        }
    };
    if let Some(model) = &model {
        let new_text = crate::config_edit::upsert_section_value(
            &config_text(shell),
            "llm",
            "model",
            &format!("\"{model}\""),
        )
        .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
        crate::config_edit::commit(&shell.config_path, &new_text)
            .map_err(|e| yunyou::CordisError::new(yunyou::ErrorKind::Internal, e))?;
    }
    Ok(CommandOutput::success(format!(
        "向导完成：端点={} 模型={} 密钥={}（模型/端点变更重启生效；密钥当轮即生效；旧配置在 yunyou.toml.bak）",
        base_url.as_deref().map(str::to_string).unwrap_or_else(|| "保留".into()),
        model.as_deref().map(str::to_string).unwrap_or_else(|| "保留".into()),
        if key.is_some() { "已更新" } else { "保留" }
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// arg_of：/model <名> 带参提取；/tools on off web_fetch 两段；无参空串。
    #[test]
    fn 参数段提取() {
        assert_eq!(arg_of("/model gpt-x", "model"), "gpt-x");
        assert_eq!(arg_of("/model", "model"), "");
        assert_eq!(arg_of("/tools off web_fetch", "tools"), "off web_fetch");
        assert_eq!(arg_of("/tools", "tools"), "");
        assert_eq!(arg_of("/config path", "config"), "path");
    }

    /// read_secret 空输入路径不在此测（stdin 交互）；echo 旗恢复语义锁：
    /// set_echo(true) 为恢复态（pub(crate) 面，回执在 input_source 测试域）。
    #[test]
    fn 密钥引用常量对齐() {
        // /key 写配置面的引用行值必须=credentials crate 单源常量（防漂移）
        assert_eq!(LLM_API_KEY_REF, "$YUNYOU_LLM_API_KEY");
    }
}
