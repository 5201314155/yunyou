//! 审计日志（网关落地方案 §3.3）：结构化记录落 `<yunyou-home>/logs/
//! gateway-audit.jsonl`——**认证失败必留痕、审计不可被业务端点关闭**（方案
//! 安全纪律 7）。jsonl 介质=与 IPC 帧同律（可调试+与 schema 纪律同源）。
//!
//! 事件词汇（§3.3 冻结集）：pair_issued/pair_consumed/pair_rejected/
//! lock_engaged/token_issued/token_revoked/auth_failure/scope_changed/
//! device_limit_rejected。

use std::io::Write;
use std::sync::Mutex;

/// 审计事件词汇（§3.3 冻结集；新事件=方案修订）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEvent {
    /// 配对令牌签发。
    PairIssued,
    /// 配对消费成功。
    PairConsumed,
    /// 配对被拒（过期/已消费/未锁定外原因）。
    PairRejected,
    /// 失败锁定生效。
    LockEngaged,
    /// 会话令牌签发。
    TokenIssued,
    /// 令牌吊销。
    TokenRevoked,
    /// 认证失败（必留痕）。
    AuthFailure,
    /// scope 变更（换发新令牌）。
    ScopeChanged,
    /// 设备数上限拒绝。
    DeviceLimitRejected,
}

impl AuditEvent {
    /// 词汇串（与 §3.3 表一一对应）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PairIssued => "pair_issued",
            Self::PairConsumed => "pair_consumed",
            Self::PairRejected => "pair_rejected",
            Self::LockEngaged => "lock_engaged",
            Self::TokenIssued => "token_issued",
            Self::TokenRevoked => "token_revoked",
            Self::AuthFailure => "auth_failure",
            Self::ScopeChanged => "scope_changed",
            Self::DeviceLimitRejected => "device_limit_rejected",
        }
    }
}

/// 审计条目（结构化字段；jsonl 一行一记录）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRecord<'a> {
    /// 事件（§3.3 词汇）。
    pub event: AuditEvent,
    /// unix 毫秒（调用方注入时钟——渲染/审计零时钟纪律同源）。
    pub at_ms: u64,
    /// 源 IP（公网模式取可信反代链，§3.2；内网/IPC 场景可为 "local"）。
    pub source_ip: &'a str,
    /// 设备 id（可得时；配对前=None）。
    pub device_id: Option<&'a str>,
    /// 结果原因码（人话短语）。
    pub reason: &'a str,
}

/// 追加一条审计（jsonl 一行；写入失败=Err 上抛——**审计不可静默失败**，
/// 调用方（认证路径）失败时必须拒绝业务继续，方案 §3.3 必留痕语义）。
///
/// 结构化序列化（云垒 P2-7 拆雷，task-30②）：原 format! 直拼 source_ip/reason
/// 进 JSON 无转义——携带 `"` 或换行的外部输入（source_ip 未来按 §3.2 取自反代
/// 链）即可断行/伪造审计记录；serde_json 字段值自动转义（引号/换行/反斜杠
/// 原样入 JSON 语义不逃逸），device_id 用 Option 语义序列化（None→null）。
pub fn record<W: Write>(sink: &mut W, record: &AuditRecord<'_>) -> std::io::Result<()> {
    let line = serde_json::json!({
        "at_ms": record.at_ms,
        "event": record.event.as_str(),
        "source_ip": record.source_ip,
        "device_id": record.device_id,
        "reason": record.reason,
    });
    sink.write_all(line.to_string().as_bytes())?;
    sink.write_all(b"\n")?;
    sink.flush()
}

/// 审计句柄（F-3 接线：GatewayState 持 Arc 共享；内部互斥串行化追加——
/// 审计不可静默失败，[`Self::log`] 失败上抛由调用方拒绝业务继续）。
pub struct AuditLog {
    /// 内存档（测试/内嵌用）：缓冲可经 [`Self::snapshot`] 回读断言。
    memory: Option<std::sync::Arc<Mutex<Vec<u8>>>>,
    writer: Mutex<Box<dyn Write + Send>>,
}

impl AuditLog {
    /// 文件档（生产：`<yunyou-home>/logs/gateway-audit.jsonl` 追加写；
    /// 父目录调用方保证存在）。
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { memory: None, writer: Mutex::new(Box::new(file)) })
    }

    /// 内存档（测试断言用）。
    pub fn in_memory() -> Self {
        let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        Self { memory: Some(std::sync::Arc::clone(&buf)), writer: Mutex::new(Box::new(Vec::new())) }
    }

    /// 追加一条（时间/IP/设备/原因由调用方注入）。
    pub fn log(
        &self,
        event: AuditEvent,
        at_ms: u64,
        source_ip: &str,
        device_id: Option<&str>,
        reason: &str,
    ) -> std::io::Result<()> {
        if let Some(buf) = &self.memory {
            let mut sink = buf.lock().expect("审计内存缓冲锁");
            return record(
                &mut *sink,
                &AuditRecord { event, at_ms, source_ip, device_id, reason },
            );
        }
        let mut sink = self.writer.lock().expect("审计写句柄锁");
        record(&mut *sink, &AuditRecord { event, at_ms, source_ip, device_id, reason })
    }

    /// 内存档回读（测试断言；文件档=None）。
    pub fn snapshot(&self) -> Option<Vec<u8>> {
        self.memory.as_ref().map(|buf| buf.lock().expect("审计内存缓冲锁").clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// jsonl 形态与必留痕语义：一行一记录、字段齐、写入失败上抛不吞。
    #[test]
    fn 审计jsonl形态() {
        let mut sink = Vec::new();
        record(
            &mut sink,
            &AuditRecord {
                event: AuditEvent::AuthFailure,
                at_ms: 1_234,
                source_ip: "192.168.1.7",
                device_id: None,
                reason: "配对令牌已过期",
            },
        )
        .unwrap();
        let text = String::from_utf8(sink.clone()).unwrap();
        assert!(text.starts_with('{') && text.ends_with("}\n"), "{text:?}");
        assert!(text.contains("\"event\":\"auth_failure\""));
        assert!(text.contains("\"device_id\":null"));
        assert!(text.contains("\"reason\":\"配对令牌已过期\""));
        // 两笔=两行（jsonl 一行一记录）
        record(
            &mut sink,
            &AuditRecord {
                event: AuditEvent::PairIssued,
                at_ms: 2_000,
                source_ip: "local",
                device_id: Some("d1"),
                reason: "扫码配对签发",
            },
        )
        .unwrap();
        assert_eq!(String::from_utf8(sink).unwrap().lines().count(), 2);
    }

    /// 注入载荷不逃逸（task-30②，云垒 P2-7 拆雷验收）：source_ip/reason 携带
    /// `"`、换行、反斜杠——serde_json 字段值自动转义，jsonl 仍一行一记录且
    /// parse 回读字段原样（修前 format! 直拼=断行/伪造审计记录）。
    #[test]
    fn 注入载荷不逃逸() {
        let mut sink = Vec::new();
        record(
            &mut sink,
            &AuditRecord {
                event: AuditEvent::AuthFailure,
                at_ms: 42,
                source_ip: "1.2.3.4\"}\n{\"event\":\"forged\",\"reason\":\"fake",
                device_id: Some("d\"evil"),
                reason: "含\"引号与\\反斜杠及\n换行",
            },
        )
        .unwrap();
        let text = String::from_utf8(sink.clone()).unwrap();
        assert_eq!(
            text.lines().count(),
            1,
            "注入载荷不得断行成第二条记录（一行一记录铁律）：{text:?}"
        );
        // parse 回读：字段值原样（转义只是序列化形态，语义无损）
        let line = text.trim_end();
        let parsed: serde_json::Value = serde_json::from_str(line).expect("审计行应始终合法 JSON");
        assert_eq!(parsed["event"], "auth_failure");
        assert_eq!(
            parsed["source_ip"],
            "1.2.3.4\"}\n{\"event\":\"forged\",\"reason\":\"fake",
            "注入载荷原样入字段值（语义无损）"
        );
        assert_eq!(parsed["reason"], "含\"引号与\\反斜杠及\n换行");
        assert_eq!(parsed["device_id"], "d\"evil");
    }
}
