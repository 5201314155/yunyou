//! web 壳层错误：地址解析 / 网络 IO 两类，Display 全中文可直读（对齐
//! yunyou-cli AppError 的错误风格；run.rs 接线侧统一转 AppError 上抛）。

use std::fmt;

#[derive(Debug)]
pub enum WebError {
    /// 监听地址形态不合法（--addr 需 host:port）。
    Addr(String),
    /// 网络 IO 失败（bind / serve / 信号）。
    Io(std::io::Error),
}

impl fmt::Display for WebError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Addr(msg) => write!(f, "监听地址错误：{msg}"),
            Self::Io(err) => write!(f, "web 服务 IO 错误：{err}"),
        }
    }
}

impl std::error::Error for WebError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Addr(_) => None,
            Self::Io(err) => Some(err),
        }
    }
}

impl From<std::io::Error> for WebError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Display 人话：地址类带原文、IO 类带底层错误链。
    #[test]
    fn 错误文案中文可直读() {
        let addr = WebError::Addr("缺端口".into());
        assert_eq!(addr.to_string(), "监听地址错误：缺端口");
        let io = WebError::Io(std::io::Error::other("拒绝连接"));
        assert!(io.to_string().contains("拒绝连接"), "IO 文案应带底层错误：{}", io);
        assert!(std::error::Error::source(&io).is_some(), "IO 应有错误源");
        assert!(std::error::Error::source(&addr).is_none(), "地址类无错误源");
    }
}
