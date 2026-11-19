//! DuckDuckGo HTML 后端（零配置真免费，表序默认）：解析 html.duckduckgo.com
//! 结果页——`result__a` 锚=标题+跳转链（/l/?uddg= 还原），`result__snippet`=
//! 摘要。HTML 结构变更即坏：解析 fail-loud（错误含原始响应片段，设计稿验收 3
//! 可诊断），解析器独立文件+快照测试兜底。

use super::{clamp_snippet, resolve_ddg_link, SearchError, SearchHit, SearchProvider};
use yunyou_util::BoxFut;

pub struct DuckDuckGoProvider;

const ENDPOINT: &str = "https://html.duckduckgo.com/html/";

impl SearchProvider for DuckDuckGoProvider {
    fn id(&self) -> &'static str {
        "duckduckgo"
    }

    fn needs_key(&self) -> bool {
        false
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a str,
        limit: usize,
    ) -> BoxFut<'a, Result<Vec<SearchHit>, SearchError>> {
        Box::pin(async move {
            let resp = client
                .get(ENDPOINT)
                .query(&[("q", query)])
                .header("User-Agent", "Mozilla/5.0 (compatible; yunyou-web-search)")
                .send()
                .await
                .map_err(|err| SearchError {
                    provider: "duckduckgo",
                    message: format!("请求失败：{err}"),
                    rate_limited: err.status().is_some_and(|s| s.as_u16() == 429),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(SearchError {
                    provider: "duckduckgo",
                    message: format!("HTTP {status}；原始响应片段：{}", snippet_of(&body, 200)),
                    rate_limited: status.as_u16() == 429,
                });
            }
            let html = resp
                .text()
                .await
                .map_err(|err| SearchError { provider: "duckduckgo", message: format!("读响应失败：{err}"), rate_limited: false })?;
            parse_ddg_html(&html, limit).ok_or_else(|| SearchError {
                provider: "duckduckgo",
                message: format!(
                    "结果页解析零命中（HTML 结构可能已变更，fail-loud）；原始响应片段：{}",
                    snippet_of(&html, 200)
                ),
                rate_limited: false,
            })
        })
    }
}

/// 原始响应片段（诊断面；截 200 字符）。
pub(crate) fn snippet_of(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// 解析 DDG HTML 结果页（无 HTML 库轻量扫描：锚定 class 标记逐块提取——
/// 防依赖膨胀；结构变更=零命中 fail-loud）。
pub(crate) fn parse_ddg_html(html: &str, limit: usize) -> Option<Vec<SearchHit>> {
    let mut hits = Vec::new();
    // 结果块锚：<a class="result__a" href="…">标题</a>；摘要锚：result__snippet
    let marker = "result__a";
    let mut cursor = 0usize;
    while hits.len() < limit {
        let Some(anchor) = html[cursor..].find(marker) else { break };
        let after = cursor + anchor;
        // href="…" 提取（marker 后第一个 href=）
        let href = html[after..].find("href=\"").map(|p| {
            let rest = &html[after + p + "href=\"".len()..];
            let end = rest.find('"').unwrap_or(rest.len());
            rest[..end].to_string()
        });
        // 标题=锚点标签文本（> 到 </a>）
        let title = html[after..]
            .find('>')
            .and_then(|gt| {
                let rest = &html[after + gt + 1..];
                let end = rest.find("</a>")?;
                Some(rest[..end].to_string())
            })
            .map(strip_tags);
        // 同结果块的摘要：锚点之后的第一个 result__snippet（至下一个 result__a
        // 之前；DDG 结果块内先锚后摘要）
        let next_anchor = html[after + marker.len()..].find(marker).map(|p| after + marker.len() + p).unwrap_or(html.len());
        let snippet = html[after..next_anchor]
            .find("result__snippet")
            .and_then(|sp| {
                let seg = &html[after + sp..];
                let gt = seg.find('>')?;
                let rest = &seg[gt + 1..];
                let end = rest.find("</a>")?;
                Some(strip_tags(rest[..end].to_string()))
            });
        if let (Some(href), Some(title)) = (href, title) {
            let url = resolve_ddg_link(&href);
            hits.push(SearchHit {
                url,
                title,
                snippet: clamp_snippet(&snippet.unwrap_or_default()),
                page_age: None,
            });
        }
        cursor = after + marker.len();
    }
    (!hits.is_empty()).then_some(hits)
}

/// 剥标签（摘要/标题内嵌 <b>/<span> 等；轻量去 <> 段）。
fn strip_tags(s: String) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// 快照测试：固定 HTML 夹具解析（离线；结构变更时此测红=fail-loud 可诊断）。
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r##"<div class="result">
<a rel="nofollow" class="result__a" href="/l/?uddg=https%3A%2F%2Fdocs.rs%2Flinux-raw-sys&amp;rut=abc">linux-raw-sys - Rust</a>
<a class="result__snippet" href="#">Definitions for Linux kernel ABIs <b>crate</b> docs.</a>
</div>
<div class="result">
<a rel="nofollow" class="result__a" href="https://direct.example.com/rust">Direct link</a>
<a class="result__snippet" href="#">second snippet</a>
</div>"##;

    /// 快照：跳转链还原+标题/摘要提取+直链透传。
    #[test]
    fn ddg解析快照() {
        let hits = parse_ddg_html(FIXTURE, 8).expect("夹具应解析出结果");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://docs.rs/linux-raw-sys");
        assert_eq!(hits[0].title, "linux-raw-sys - Rust");
        assert!(hits[0].snippet.contains("Definitions for Linux kernel ABIs crate docs."));
        assert_eq!(hits[1].url, "https://direct.example.com/rust");
    }

    /// 结构变更 fail-loud：无结果锚=零命中 None（错误层附响应片段）。
    #[test]
    fn 结构变更零命中() {
        assert!(parse_ddg_html("<html>captcha page</html>", 8).is_none());
    }

    /// limit 截断。
    #[test]
    fn limit截断() {
        let hits = parse_ddg_html(FIXTURE, 1).expect("应命中");
        assert_eq!(hits.len(), 1);
    }
}
