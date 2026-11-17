//! 网关启动配置（方案 §5 两维：网络维度×配对维度正交；§5.2 公网 TLS 强制
//! fail-loud）。
//!
//! 两维正交（§4.3）：配对方式（扫码/验证码）与网络维度独立选择；差异仅
//! endpoint 形态与 TLS——鉴权路径四格完全同构（§6）。

use std::path::PathBuf;

/// 网络维度（方案 §5）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkMode {
    /// 内网（§5.1）：局域网明文 ws://IP:7900，依托内网边界；配对=扫码/验证码。
    Lan { /// 监听地址（如 192.168.x.x:7900；0.0.0.0 需显式确认内网边界）。
        addr: String },
    /// 公网（§5.2）：wss:// 域名 + **TLS 强制 fail-loud**——证书/私钥任一缺失
    /// 或不可读=拒绝启动（不提供明文公网选项，防白嫖/滥用面）。
    Public {
        addr: String,
        cert_path: PathBuf,
        key_path: PathBuf,
    },
}

impl NetworkMode {
    /// 启动前校验（§5.2 fail-loud）：公网档证书/私钥文件必须存在可读。
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Lan { .. } => Ok(()),
            Self::Public { cert_path, key_path, .. } => {
                for (label, path) in [("证书", cert_path), ("私钥", key_path)] {
                    if !path.is_file() {
                        return Err(format!(
                            "公网模式 TLS {label}不可读（{}）：公网明文禁止启动（方案 §5.2 fail-loud）",
                            path.display()
                        ));
                    }
                }
                Ok(())
            }
        }
    }

    /// endpoint 形态（QR 载荷与文档用）。
    pub fn endpoint_prefix(&self) -> &'static str {
        match self {
            Self::Lan { .. } => "ws://",
            Self::Public { .. } => "wss://",
        }
    }
}

/// 配对方式（方案 §4 两维之二；与 pairing::PairingMethod 对应，配置层词汇）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairingMode {
    /// 扫码（QR 载荷渲染，§4.1）。
    Qr,
    /// 验证码（设备码流程，§4.2）。
    DeviceCode,
}

/// 限速阈值（方案 §3.2 三档表；yunyou.toml [gateway] 段可覆盖，缺省=方案定案值）。
#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitConfig {
    /// 业务端点（设备/面板）：次/分/令牌（§3.2 定案 60）。
    pub business_per_min: f64,
    /// WS 握手：次/分/IP（§3.2 定案 10）。
    pub ws_handshake_per_min: f64,
    /// healthz 探活：次/分/IP（原方案定案 6 对 UI 轮询过紧→放宽 30；
    /// 接线批注记：App 常态探活应走带令牌业务端点，healthz 仅启动/巡检用）。
    pub healthz_per_min: f64,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self { business_per_min: 60.0, ws_handshake_per_min: 10.0, healthz_per_min: 30.0 }
    }
}

/// IP 级失败锁定（方案 §3.2 双维之「源 IP 维」；G-2 接线参数，缺省=会话维同值）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpLockConfig {
    /// 滑窗内连续失败 N 次（缺省 5，与会话维 FAIL_LIMIT 对齐）。
    pub fail_limit: u32,
    /// 锁定时长（毫秒；缺省 15 分钟，与会话维 LOCK_MS 对齐）。
    pub lock_ms: u64,
    /// 失败计数滑窗（毫秒；窗满计数清零重计，缺省 15 分钟）。
    pub window_ms: u64,
}

impl Default for IpLockConfig {
    fn default() -> Self {
        Self {
            fail_limit: 5,
            lock_ms: 15 * 60 * 1000,
            window_ms: 15 * 60 * 1000,
        }
    }
}

/// 网关全量配置（yunyou.toml [gateway] 段候选形态；全字段有默认）。
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// 网络维度。
    pub network: NetworkMode,
    /// 启用的配对方式（两维正交：可单开/双开；空=网关不开放配对仅服务已配对设备）。
    pub pairing_modes: Vec<PairingMode>,
    /// gateway.db 路径（相对 yunyou-home）。
    pub db_path: PathBuf,
    /// 活跃设备数上限（§3.1 默认 10）。
    pub device_limit: u32,
    /// 会话令牌滑动过期天数（G-3：缺省 30 天未活跃即失效，重新配对换发）。
    pub session_ttl_days: u32,
    /// 可信反代开关（缺省 false）：false=源 IP 取 socket 对端地址（X-Forwarded-For
    /// 一律不采信——防伪造头绕限速/IP 锁）；true=公网反代部署，采信反代链首跳。
    pub trust_proxy: bool,
    /// 限速阈值（§3.2 三档；G-1 接线消费）。
    pub rate: RateLimitConfig,
    /// IP 级失败锁定（§3.2 源 IP 维；G-2 接线消费）。
    pub ip_lock: IpLockConfig,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            network: NetworkMode::Lan { addr: "127.0.0.1:7900".into() },
            pairing_modes: vec![PairingMode::Qr, PairingMode::DeviceCode],
            db_path: PathBuf::from("gateway.db"),
            device_limit: 10,
            session_ttl_days: 30,
            trust_proxy: false,
            rate: RateLimitConfig::default(),
            ip_lock: IpLockConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §5.2 fail-loud 锁：公网档证书/私钥缺失=拒绝启动（明文公网禁止）。
    #[test]
    fn 公网档证书缺失拒绝启动() {
        let mode = NetworkMode::Public {
            addr: "0.0.0.0:7900".into(),
            cert_path: PathBuf::from("/nonexistent/cert.pem"),
            key_path: PathBuf::from("/nonexistent/key.pem"),
        };
        let err = mode.validate().unwrap_err();
        assert!(err.contains("fail-loud"), "{err}");
        // 内网档恒过
        assert!(NetworkMode::Lan { addr: "127.0.0.1:7900".into() }.validate().is_ok());
    }

    /// 真实临时证书文件存在=公网校验通过（fail-loud 只拦缺失不验内容——
    /// 内容有效性由 TLS 握手期暴露，阶段 B 网络层职责）。
    #[test]
    fn 公网档证书在位校验通过() {
        let dir = std::env::temp_dir().join(format!("gw-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cert = dir.join("cert.pem");
        let key = dir.join("key.pem");
        std::fs::write(&cert, b"-----BEGIN CERTIFICATE-----").unwrap();
        std::fs::write(&key, b"-----BEGIN PRIVATE KEY-----").unwrap();
        let mode = NetworkMode::Public { addr: "0.0.0.0:7900".into(), cert_path: cert, key_path: key };
        assert!(mode.validate().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
