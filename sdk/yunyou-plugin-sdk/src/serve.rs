//! 协议服务循环（serve 的实现面；帧序镜像宿主 ProxyPlugin，fail-loud 不降级）。
//!
//! 同步 std::io 形态（无 tokio——SDK 零异步依赖，主线程阻塞即可）：宿主侧
//! ProxyPlugin 的读泵/写泵按行对发，插件侧逐行读→逐行回，时序天然吻合。

use std::io::{BufRead, Write};
use std::process::ExitCode;

use yunyou_plugin_proto::{
    decode_frame, encode_frame, Hello, HelloErr, HelloOk, PluginProtoFrame, ToolInvoke,
    ToolResult, ToolResultErr, ToolResultOk, ToolsList, ToolsListOk, ToolOutcome, PROTOCOL_VERSION,
};

use crate::Plugin;

/// 协议服务主循环：读 stdin 一行→解帧→分派→回帧；EOF/shutdown 退出。
pub(crate) fn serve(plugin: Plugin) -> ExitCode {
    // 启动诊断（stderr——宿主 drain 透传主板日志；stdout 是协议专用通道）
    eprintln!(
        "[yunyou-plugin-sdk:{}] v{} 就绪（category={:?}，工具数={}）",
        plugin.id(),
        plugin.version(),
        plugin.category_ref(),
        plugin.tools().len()
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut reader = stdin.lock();
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // 宿主退出（stdin EOF）：正常终止
                return ExitCode::SUCCESS;
            }
            Ok(_) => {}
            Err(err) => {
                eprintln!("[yunyou-plugin-sdk] stdin 读取失败（退出）: {err}");
                return ExitCode::FAILURE;
            }
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue; // 空行容错（宿主握手期可能补发换行）
        }
        let frame = match decode_frame(trimmed) {
            Ok(frame) => frame,
            Err(err) => {
                // 坏帧：stderr 报告+继续（宿主读泵对坏行同款容错；不退出——
                // 单行损坏不应杀插件）
                eprintln!("[yunyou-plugin-sdk:{}] 坏帧跳过: {err}", plugin.id());
                continue;
            }
        };
        match frame {
            // ① 握手：版本比对，不符=hello_err+exit(1)（fail-loud，不静默降级）
            PluginProtoFrame::Hello(Hello { protocol_version, capabilities: _ }) => {
                if protocol_version == PROTOCOL_VERSION {
                    let reply = PluginProtoFrame::HelloOk(HelloOk {
                        protocol_version: PROTOCOL_VERSION,
                        plugin_name: plugin.id().to_string(),
                    });
                    if let Err(err) = write_frame(&mut stdout, &reply) {
                        eprintln!("[yunyou-plugin-sdk:{}] hello_ok 写出失败: {err}", plugin.id());
                        return ExitCode::FAILURE;
                    }
                } else {
                    let reason = format!(
                        "协议版本不兼容：宿主={protocol_version}，插件 SDK 支持={PROTOCOL_VERSION}（升级任一侧后重试）"
                    );
                    let _ = write_frame(
                        &mut stdout,
                        &PluginProtoFrame::HelloErr(HelloErr { reason }),
                    );
                    return ExitCode::FAILURE;
                }
            }
            // ② 工具清单
            PluginProtoFrame::ToolsList(ToolsList) => {
                let reply = PluginProtoFrame::ToolsListOk(ToolsListOk {
                    tools: plugin.descriptors(),
                });
                if let Err(err) = write_frame(&mut stdout, &reply) {
                    eprintln!("[yunyou-plugin-sdk:{}] tools_list_ok 写出失败: {err}", plugin.id());
                    return ExitCode::FAILURE;
                }
            }
            // ③ 工具调用：白名单校验（未上报名拒绝）→解析 args（协议面为 JSON
            // 文本）→执行→回 tool_result
            PluginProtoFrame::ToolInvoke(ToolInvoke { invoke_id, name, args }) => {
                let outcome = match plugin.find_tool(&name) {
                    None => ToolOutcome::Err(ToolResultErr {
                        code: "tool/unknown-tool".into(),
                        message: format!("工具 \"{name}\" 未在本插件 tools_list 上报（白名单拒绝）"),
                        hint: None,
                    }),
                    Some(tool) => match serde_json::from_str::<serde_json::Value>(&args) {
                        Err(err) => ToolOutcome::Err(ToolResultErr {
                            code: "tool/invalid-args".into(),
                            message: format!("参数不是合法 JSON：{err}"),
                            hint: Some("对照工具 parameters 的 JSON Schema 修正参数".into()),
                        }),
                        Ok(parsed) => match tool.execute(parsed) {
                            Ok(value) => match serde_json::to_string(&value) {
                                Ok(text) => ToolOutcome::Ok(ToolResultOk { value: text }),
                                Err(err) => ToolOutcome::Err(ToolResultErr {
                                    code: "tool/exec".into(),
                                    message: format!("返回值序列化失败：{err}"),
                                    hint: None,
                                }),
                            },
                            Err(failure) => ToolOutcome::Err(ToolResultErr {
                                code: failure.code,
                                message: failure.message,
                                hint: failure.hint,
                            }),
                        },
                    },
                };
                let reply = PluginProtoFrame::ToolResult(ToolResult { invoke_id, outcome });
                if let Err(err) = write_frame(&mut stdout, &reply) {
                    eprintln!("[yunyou-plugin-sdk:{}] tool_result 写出失败: {err}", plugin.id());
                    return ExitCode::FAILURE;
                }
            }
            // ④ 卸载通知：立即退出（宿主 killpg 兜底，不悬挂）
            PluginProtoFrame::Shutdown(_) => return ExitCode::SUCCESS,
            // 其余帧=宿主→插件方向之外的回声（协议误用）：stderr 报告不退出
            other => {
                eprintln!("[yunyou-plugin-sdk:{}] 非预期帧忽略: {other:?}", plugin.id());
            }
        }
    }
}

/// 写一帧（单行 JSON+换行）。
fn write_frame<W: Write>(out: &mut W, frame: &PluginProtoFrame) -> std::io::Result<()> {
    let line = encode_frame(frame).map_err(std::io::Error::other)?;
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()
}
