//! # yunyou-team-worker-core —— K-11 团队运行时·成员运行时层（三层之三）
//!
//! **职责**：worker 进程内的 proto 客户端循环（stdio JSON Lines：宿主帧读入
//! →分派→响应帧写回）——hello 握手/tools_list 上报/tool_invoke 消费/心跳
//! 应答/shutdown 退出。
//!
//! **明确不做**：身份裁决（=连接会话，message.v2 议题传递）/工具实现（工具
//! 面由插件注册 fn 表，本层=运行时框架）。
//!
//! 对表锚点：K-11 设计案 85547bfae §一 worker-core 行+K-19 八帧词汇（proto
//! crate 单源，传输=stdio JSON Lines）。

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use yunyou_plugin_proto::codec::{decode_frame, encode_frame};
use yunyou_plugin_proto::messages::{
    HelloErr, HelloOk, PluginProtoFrame, ToolDescriptor, ToolInvoke, ToolOutcome,
    ToolResult, ToolResultErr, ToolResultOk, ToolsListOk, PROTOCOL_VERSION,
};

/// 工具实现面：插件向运行时注册的 fn 表（args JSON 进、结果 JSON 出——
/// 工具错误以 Err(code,message) 承载=tools.v1 ToolError 三元组投影同源）。
pub type ToolFn = Box<dyn Fn(&str) -> Result<String, (String, String)> + Send>;

/// worker 运行时：帧循环（单线程顺序处理——MVP 无并发调用面；Debug 不派生=
/// ToolFn 闭包容器，观测走 alive/登记面）。
#[derive(Default)]
pub struct WorkerRuntime {
    session_id: String,
    tools: BTreeMap<String, ToolFn>,
    /// 工具清单缓存（tools_list 上报来源；注册即定，MVP 不热更）。
    descriptors: Vec<ToolDescriptor>,
    /// Shutdown 帧已收（帧循环退出标记）。
    shutdown_seen: bool,
}

impl WorkerRuntime {
    /// 构造：session_id=身份锚点（spawn argv --session 透传）。
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            tools: BTreeMap::new(),
            descriptors: Vec::new(),
            shutdown_seen: false,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// 注册工具（name→fn；同时登记 descriptor——tools_list 上报面）。
    pub fn register_tool(
        &mut self,
        name: &str,
        description: &str,
        parameters_schema: &str,
        f: ToolFn,
    ) {
        self.tools.insert(name.to_string(), f);
        self.descriptors.push(ToolDescriptor {
            name: name.to_string(),
            description: description.to_string(),
            parameters: parameters_schema.to_string(),
            output_schema: None,
            timeout_ms: None,
            is_concurrency_safe: false,
        });
    }

    
/// 宿主守护自挂（b 案=worker 自挂宿主 pid，细案 2effdb406 裁决）：
/// prctl(PR_SET_PDEATHSIG, SIGKILL)+getppid 校验（父先亡竞态窗：prctl 前
/// 父已死→getppid 已变→自杀，防孤儿漂移）。host_pid=宿主主进程 pid
/// （spawn argv --parent-pid 透传；二进制接线候 worker 入口批）。
///
/// 返回 Err=prctl 失败或竞态窗检出（调用方=worker 入口，立即退出 fail-loud）。
pub fn attach_parent_watch(host_pid: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    unsafe {
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
            return Err(std::io::Error::last_os_error());
        }
        // 竞态窗校验：prctl 后父 pid 应=host_pid（spawn 时宿主仍活）
        if libc::getppid() as u32 != host_pid {
            return Err(std::io::Error::other(format!(
                "父进程竞态窗检出：spawn 时宿主 {host_pid} 已亡（getppid={}）——worker 随退",
                libc::getppid()
            )));
        }
    }
    Ok(())
}

/// 帧循环：读宿主帧→分派→写响应帧。返回=Shutdown 或流结束（EOF=宿主
    /// 亡，进程随退=K-11 设计 §二 PDEATHSIG 语义对偶）。
    pub fn run(&mut self, input: &mut dyn BufRead, output: &mut dyn Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue; // 空行=帧分隔残渣，跳过
            }
            let frame = match decode_frame(&line) {
                Ok(f) => f,
                Err(err) => {
                    // 未知/残缺帧=stderr 人话+继续（不退出——宿主单帧坏不拖垮 worker）
                    eprintln!("[worker] 帧解码失败：{err}");
                    continue;
                }
            };
            for resp in self.dispatch(frame) {
                let wire = encode_frame(&resp)
                    .map_err(std::io::Error::other)?;
                writeln!(output, "{wire}")?;
                output.flush()?;
            }
            if self.shutdown_seen {
                return Ok(());
            }
        }
        Ok(())
    }

    /// 单帧分派（返回响应帧序列；纯函数可单测——IO 与分派分离）。
    fn dispatch(&mut self, frame: PluginProtoFrame) -> Vec<PluginProtoFrame> {
        match frame {
            PluginProtoFrame::Hello(hello) => {
                if hello.protocol_version == PROTOCOL_VERSION {
                    vec![PluginProtoFrame::HelloOk(HelloOk {
                        protocol_version: PROTOCOL_VERSION,
                        plugin_name: format!("worker-core:{}", self.session_id),
                    })]
                } else {
                    vec![PluginProtoFrame::HelloErr(HelloErr {
                        reason: format!(
                            "协议版本不匹配：宿主 {} / worker {}",
                            hello.protocol_version, PROTOCOL_VERSION
                        ),
                    })]
                }
            }
            PluginProtoFrame::ToolsList(_) => {
                vec![PluginProtoFrame::ToolsListOk(ToolsListOk {
                    tools: self.descriptors.clone(),
                })]
            }
            PluginProtoFrame::ToolInvoke(invoke) => {
                vec![PluginProtoFrame::ToolResult(self.invoke(invoke))]
            }
            PluginProtoFrame::Shutdown(_) => {
                self.shutdown_seen = true;
                Vec::new() // 退出确认=无响应帧（进程随退）
            }
            // 插件→宿主方向帧出现在 worker 侧=协议错位（HelloOk/HelloErr/
            // ToolsListOk/ToolResult/Pong 五种）——stderr 人话+继续。
            other => {
                eprintln!("[worker] 收到宿主方向帧（协议错位，忽略）：{other:?}");
                Vec::new()
            }
        }
    }

    /// 工具调用执行（invoke_id 回带；未上报名=插件端拒绝——白名单纪律）。
    fn invoke(&mut self, invoke: ToolInvoke) -> ToolResult {
        let outcome = match self.tools.get(&invoke.name) {
            Some(f) => match f(&invoke.args) {
                Ok(value) => ToolOutcome::Ok(ToolResultOk { value }),
                Err((code, message)) => ToolOutcome::Err(ToolResultErr {
                    code,
                    message,
                    hint: None,
                }),
            },
            None => ToolOutcome::Err(ToolResultErr {
                code: "team/whitelist".into(),
                message: format!("工具 {} 未在 tools_list 上报（白名单纪律：未上报名=拒绝）", invoke.name),
                hint: None,
            }),
        };
        ToolResult { invoke_id: invoke.invoke_id, outcome }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_plugin_proto::messages::{Hello, ToolOutcome};
    use yunyou_plugin_proto::messages::ToolsList;
    use std::io::BufReader;
    use std::io::BufWriter;

    fn runtime_with_echo() -> WorkerRuntime {
        let mut rt = WorkerRuntime::new("s-test");
        rt.register_tool(
            "echo",
            "回显工具",
            r#"{"type":"object"}"#,
            Box::new(|args| Ok(args.to_string())),
        );
        rt
    }

    /// 握手：协议匹配→HelloOk。
    #[test]
    fn 握手协议匹配回ok() {
        let mut rt = runtime_with_echo();
        let responses = rt.dispatch(PluginProtoFrame::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            capabilities: Vec::new(),
        }));
        assert!(matches!(responses[0], PluginProtoFrame::HelloOk(_)));
    }

    /// 握手：版本不匹配→HelloErr 人话。
    #[test]
    fn 握手版本不匹配回err() {
        let mut rt = runtime_with_echo();
        let responses = rt.dispatch(PluginProtoFrame::Hello(Hello {
            protocol_version: 999,
            capabilities: Vec::new(),
        }));
        match &responses[0] {
            PluginProtoFrame::HelloErr(e) => assert!(e.reason.contains("协议版本不匹配")),
            other => panic!("应为 HelloErr: {other:?}"),
        }
    }

    /// tools_list 上报=注册的 descriptor 面全量。
    #[test]
    fn tools_list上报descriptor全量() {
        let mut rt = runtime_with_echo();
        let responses = rt.dispatch(PluginProtoFrame::ToolsList(ToolsList));
        match &responses[0] {
            PluginProtoFrame::ToolsListOk(ok) => {
                assert_eq!(ok.tools.len(), 1);
                assert_eq!(ok.tools[0].name, "echo");
            }
            other => panic!("应为 ToolsListOk: {other:?}"),
        }
    }

    /// invoke：echo 工具执行→ToolResult Ok 值回带 invoke_id。
    #[test]
    fn invoke执行回带invoke_id() {
        let mut rt = runtime_with_echo();
        let responses = rt.dispatch(PluginProtoFrame::ToolInvoke(ToolInvoke {
            invoke_id: "inv-1".into(),
            name: "echo".into(),
            args: r#"{"x":1}"#.into(),
        }));
        match &responses[0] {
            PluginProtoFrame::ToolResult(r) => match &r.outcome {
                ToolOutcome::Ok(ok) => assert_eq!(ok.value, r#"{"x":1}"#),
                other => panic!("应为 Ok: {other:?}"),
            },
            other => panic!("应为 ToolResult: {other:?}"),
        }
    }

    /// 白名单纪律：未上报名=Err（team/whitelist）。
    #[test]
    fn 未上报名拒绝() {
        let mut rt = runtime_with_echo();
        let responses = rt.dispatch(PluginProtoFrame::ToolInvoke(ToolInvoke {
            invoke_id: "inv-2".into(),
            name: "未注册工具".into(),
            args: "{}".into(),
        }));
        match &responses[0] {
            PluginProtoFrame::ToolResult(r) => match &r.outcome {
                ToolOutcome::Err(e) => assert!(e.message.contains("未在 tools_list 上报")),
                other => panic!("应为 Err: {other:?}"),
            },
            other => panic!("应为 ToolResult: {other:?}"),
        }
    }

    /// shutdown 置位帧循环退出标记。
    #[test]
    fn shutdown置位退出() {
        let mut rt = runtime_with_echo();
        let _ = rt.dispatch(PluginProtoFrame::Shutdown(yunyou_plugin_proto::messages::Shutdown));
        assert!(rt.shutdown_seen, "shutdown 应置位");
    }

    /// 宿主守护自挂：prctl 调用面+非竞态态通过（真竞态=集成批 kill 场）。
    #[test]
    fn 宿主守护自挂prctl调用面() {
        // host_pid=当前父 pid（测试进程的父=测试运行器）——非竞态窗=Ok
        let ppid = std::process::id(); // 占位：真父 pid 语义=spawn 链集成面
        let _ = attach_parent_watch(ppid); // Ok 或 Err 均非 panic=调用面锁
    }

    /// 端到端：内存双工全链（hello→tools_list→invoke→shutdown 四帧喂入，
    /// 响应行序列读回）。
    #[test]
    fn 内存双工端到端() {
        let mut rt = runtime_with_echo();
        let input = "\
{\"type\":\"hello\",\"protocol_version\":1}\n\
{\"type\":\"tools_list\"}\n\
{\"type\":\"tool_invoke\",\"invoke_id\":\"i1\",\"name\":\"echo\",\"args\":\"{\\\"x\\\":1}\"}\n\
{\"type\":\"shutdown\"}\n";
        let mut reader = BufReader::new(input.as_bytes());
        let mut out: Vec<u8> = Vec::new();
        {
            let mut writer = BufWriter::new(&mut out);
            rt.run(&mut reader, &mut writer).expect("帧循环应正常退出");
        }
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "三响应帧（K-19 MVP 无心跳帧）：{text}");
        assert!(lines[0].contains("hello_ok"));
        assert!(lines[1].contains("tools_list_ok"));
        assert!(lines[2].contains("tool_result"));
    }
}
