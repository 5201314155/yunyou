//! Bing CN 后端（cn.bing.com；表序默认——国内直连零配置开箱即搜，2026-11-19
//! 业主令「国内你还要我翻墙？」默认序重排：lead 实测 cn.bing 200/DDG 000）。
//! 解析：`li.b_algo` 结果块内 `<h2><a href=标题链接>` + `b_caption` 段 `<p>` 摘要。
//! HTML 结构变更=零命中 fail-loud（错误含原始响应片段可诊断），真页快照夹具锁。

use super::{clamp_snippet, SearchError, SearchHit, SearchProvider};
use yunyou_util::BoxFut;

pub struct BingProvider;

const ENDPOINT: &str = "https://cn.bing.com/search";

impl SearchProvider for BingProvider {
    fn id(&self) -> &'static str {
        "bing"
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
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
                )
                .send()
                .await
                .map_err(|err| SearchError {
                    provider: "bing",
                    message: format!("请求失败：{err}"),
                    rate_limited: err.status().is_some_and(|s| s.as_u16() == 429),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(SearchError {
                    provider: "bing",
                    message: format!("HTTP {status}；原始响应片段：{}", snippet_of(&body, 200)),
                    rate_limited: status.as_u16() == 429,
                });
            }
            let html = resp
                .text()
                .await
                .map_err(|err| SearchError { provider: "bing", message: format!("读响应失败：{err}"), rate_limited: false })?;
            parse_bing_html(&html, limit).ok_or_else(|| SearchError {
                provider: "bing",
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

/// 解析 Bing 结果页（轻量扫描 `li.b_algo` 块：h2 锚点 url+标题、b_caption 段
/// 摘要；无 HTML 库防依赖膨胀）。
pub(crate) fn parse_bing_html(html: &str, limit: usize) -> Option<Vec<SearchHit>> {
    let block_marker = "<li class=\"b_algo\"";
    let mut hits = Vec::new();
    let mut cursor = 0usize;
    while hits.len() < limit {
        let Some(start) = html[cursor..].find(block_marker) else { break };
        let block_start = cursor + start;
        let block_end = html[block_start + block_marker.len()..]
            .find(block_marker)
            .map(|p| block_start + block_marker.len() + p)
            .unwrap_or(html.len());
        let block = &html[block_start..block_end];
        // 标题+url：块内第一个 <h2> 里的 <a href="…">text</a>
        let link = block.find("<h2").and_then(|h2| {
            let seg = &block[h2..];
            let href_rel = seg.find("href=\"")?;
            let rest = &seg[href_rel + "href=\"".len()..];
            let url = rest[..rest.find('"').unwrap_or(rest.len())].to_string();
            let gt = seg[seg.find("<a").unwrap_or(0)..].find('>')?;
            let tail = &seg[seg.find("<a").unwrap_or(0) + gt + 1..];
            let end = tail.find("</a>")?;
            let title = strip_tags(&tail[..end]);
            Some((url, title))
        });
        // 摘要：b_caption 之后第一个 <p…>text</p>
        let snippet = block.find("b_caption").and_then(|cap| {
            let seg = &block[cap..];
            let p = seg.find("<p")?;
            let gt = seg[p..].find('>')?;
            let tail = &seg[p + gt + 1..];
            let end = tail.find("</p>")?;
            Some(strip_tags(&tail[..end]))
        });
        if let Some((url, title)) = link {
            hits.push(SearchHit {
                url,
                title,
                snippet: clamp_snippet(&snippet.unwrap_or_default()),
                page_age: None,
            });
        }
        cursor = block_start + block_marker.len();
    }
    (!hits.is_empty()).then_some(hits)
}

/// 剥标签+常见实体解码（&ensp;/&#0183; 等排版噪声归一为空格/·）。
fn strip_tags(s: &str) -> String {
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
    out.replace("&ensp;", " ")
        .replace("&nbsp;", " ")
        .replace("&#0183;", "·")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// 真页快照测试（夹具=2026-11-19 cn.bing.com/search?q=rust+landlock 实抓节选，
/// 结构变更时此测红=fail-loud 可诊断）。
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r##"<ol id="b_results"><li class="b_algo" data-id iid=SERP.5314>
<h2><a target="_blank" target="_blank" href="https://rust-lang.org/zh-CN/" h="ID=SERP,5117.2"><strong>Rust</strong> 程序设计语言</a></h2>
<div class="b_caption"><p class="b_lineclamp2">23 小时之前&ensp;&#0183;&ensp;生产环境中的 Rust 全世界已有数百家公司在生产环境中使用 Rust，以达到快速、跨平台、低资源占用的目的。 很多著 …</p></div>
</li><li class="b_algo" data-id iid=SERP.5315>
<h2><a target="_blank" href="https://www.runoob.com/rust/rust-tutorial.html" h="ID=SERP,5132.1">Rust 教程 | 菜鸟教程</a></h2>
<div class="b_caption"><p class="b_lineclamp3">Rust 是一门赋予每个人构建可靠且高效软件能力的语言。</p></div>
</li></ol>"##;

    /// 快照：真页结构解析（url/标题剥标签/摘要实体归一）。
    #[test]
    fn bing解析快照() {
        let hits = parse_bing_html(FIXTURE, 8).expect("夹具应解析出结果");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "https://rust-lang.org/zh-CN/");
        assert_eq!(hits[0].title, "Rust 程序设计语言", "<strong> 标签剥除");
        assert!(hits[0].snippet.contains("生产环境中的 Rust"), "摘要提取");
        assert!(hits[0].snippet.contains("·"), "实体归一 &#0183;→·");
        assert!(!hits[0].snippet.contains("&ensp;"));
        assert_eq!(hits[1].url, "https://www.runoob.com/rust/rust-tutorial.html");
    }

    /// 结构变更 fail-loud：无 b_algo 块=零命中 None。
    #[test]
    fn 结构变更零命中() {
        assert!(parse_bing_html("<html>captcha</html>", 8).is_none());
    }

    /// limit 截断。
    #[test]
    fn limit截断() {
        let hits = parse_bing_html(FIXTURE, 1).expect("应命中");
        assert_eq!(hits.len(), 1);
    }
}
