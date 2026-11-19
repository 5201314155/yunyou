//! SearchProvider trait + 四后端归一化（设计稿 §二）：每 provider 把自家响应
//! 折成 SearchHit（dsh「normalized search result」同职责）；dedupe by url 在
//! 插件层统一做（design §二：收拢一层防四实现各写一遍）。

pub mod baidu;
pub mod bing;
pub mod brave;
pub mod duckduckgo;
pub mod searxng;
pub mod tavily;

/// 单条归一化结果（url/title/snippet/page_age；snippet 已在 provider 内截断）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub url: String,
    pub title: String,
    pub snippet: String,
    pub page_age: Option<String>,
}

/// 检索错误（结构化：不吞错不假结果；诊断面=原始响应片段，设计稿验收 3）。
#[derive(Debug, Clone)]
pub struct SearchError {
    /// 后端 id（错误归因）。
    pub provider: &'static str,
    /// 人话原因（含原始响应片段时标注）。
    pub message: String,
    /// 是否限速类（可退避重试）。
    pub rate_limited: bool,
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.provider, self.message)
    }
}

/// 搜索后端 trait（设计稿 §二接口草案；async fn 进 trait 用 BoxFut）。
pub trait SearchProvider: Send + Sync {
    fn id(&self) -> &'static str;
    /// 免 key 后端返回 false（零配置可换）。
    fn needs_key(&self) -> bool;
    /// key 是否在位（needs_key=false 恒 true；key 系看 config/env 注入值）。
    fn available(&self) -> bool {
        true
    }
    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a str,
        limit: usize,
    ) -> yunyou_util::BoxFut<'a, Result<Vec<SearchHit>, SearchError>>;
}

/// snippet 截断（字符边界安全，纪律 §四.3）。
pub(crate) fn clamp_snippet(s: &str) -> String {
    if s.chars().count() <= crate::SNIPPET_MAX_CHARS {
        return s.trim().to_string();
    }
    let cut: String = s.chars().take(crate::SNIPPET_MAX_CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// url 解码（DDG /l/?uddg= 跳转链还原；百分号解码+透传其余字节）。
pub(crate) fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 1 && i + 2 < bytes.len() + 1 {
            let hex = bytes.get(i + 1..i + 3);
            if let Some(h) = hex {
                if let Ok(v) = u8::from_str_radix(std::str::from_utf8(h).unwrap_or(""), 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 从 DDG 跳转链提取真 url：`/l/?uddg=<encoded>&rut=…` → decoded；已是绝对
/// url 原样返回。
pub(crate) fn resolve_ddg_link(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let rest = &href[pos + "uddg=".len()..];
        let end = rest.find('&').unwrap_or(rest.len());
        let decoded = url_decode(&rest[..end]);
        if decoded.starts_with("http") {
            return decoded;
        }
    }
    href.to_string()
}

/// 按 provider id 构造后端实例（表序=免费优先序；key 从 config/env 注入）。
pub(crate) fn build_providers(cfg: &crate::plugin::WebSearchConfig) -> Vec<Box<dyn SearchProvider>> {
    let key = |env: &str| -> Option<String> {
        cfg.brave_api_key
            .clone()
            .or_else(|| std::env::var(env).ok())
    };
    // 各后端构造（表序）；key 系按 brave→tavily 各自的 key 槽取
    let brave_key = cfg
        .brave_api_key
        .clone()
        .or_else(|| std::env::var("BRAVE_API_KEY").ok());
    let tavily_key = cfg
        .tavily_api_key
        .clone()
        .or_else(|| std::env::var("TAVILY_API_KEY").ok());
    let _ = key; // key 闭包已被上两行取代（留痕：env 名单=brave/tavily）
    // 表序=国内优先（2026-11-19 业主令重排：cn.bing/baidu 直连 200，DDG/searx.be
    // 国内直连 000——海外用户/翻墙场景走显式 provider 配置降位序）
    vec![
        Box::new(bing::BingProvider),
        Box::new(baidu::BaiduProvider),
        Box::new(searxng::SearxngProvider { base_url: cfg.searxng_base_url.clone() }),
        Box::new(duckduckgo::DuckDuckGoProvider),
        Box::new(brave::BraveProvider { api_key: brave_key }),
        Box::new(tavily::TavilyProvider { api_key: tavily_key }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// url 解码：百分号+加号空格+原样透传。
    #[test]
    fn url解码() {
        assert_eq!(url_decode("a%20b+c"), "a b c");
        assert_eq!(url_decode("https://x/y%3Fz"), "https://x/y?z");
        assert_eq!(url_decode("plain"), "plain");
    }

    /// DDG 跳转链还原：uddg= 提取解码；非跳转链原样。
    #[test]
    fn ddg跳转链还原() {
        assert_eq!(
            resolve_ddg_link("/l/?uddg=https%3A%2F%2Fdocs.rs&rut=abc"),
            "https://docs.rs"
        );
        assert_eq!(resolve_ddg_link("https://direct.example.com"), "https://direct.example.com");
    }

    /// snippet 截断：300 字符封顶+字符边界安全（多字节不裂）。
    #[test]
    fn snippet截断() {
        let long = "中".repeat(400);
        let clamped = clamp_snippet(&long);
        assert_eq!(clamped.chars().count(), 301, "300 字+省略号");
        assert!(clamp_snippet("  short  ") == "short");
    }
}
