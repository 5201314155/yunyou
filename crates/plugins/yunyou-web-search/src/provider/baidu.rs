//! Baidu 后端（www.baidu.com/s；表序第二——零配置备选）：解析
//! `div.result.c-container` 块（`h3 > a` 标题链接+正文段）。
//!
//! # 反爬如实注记（2026-11-19 实测）
//! 百度对非浏览器客户端返回「百度安全验证」页（本机 curl 实证 302→验证页），
//! 带浏览器 UA 也不保证过——本 provider 按已知结构实现+离线夹具锁，活测依赖
//! 客户端指纹（候浏览器仿真批）；主路径 Bing CN（表序第一）不受此限。块结构
//! 变更=零命中 fail-loud（错误含原始响应片段可诊断）。

use super::{clamp_snippet, SearchError, SearchHit, SearchProvider};
use yunyou_util::BoxFut;

pub struct BaiduProvider;

const ENDPOINT: &str = "https://www.baidu.com/s";

impl SearchProvider for BaiduProvider {
    fn id(&self) -> &'static str {
        "baidu"
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
                .query(&[("wd", query), ("rn", &limit.to_string())])
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
                )
                .send()
                .await
                .map_err(|err| SearchError {
                    provider: "baidu",
                    message: format!("请求失败：{err}"),
                    rate_limited: err.status().is_some_and(|s| s.as_u16() == 429),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(SearchError {
                    provider: "baidu",
                    message: format!("HTTP {status}；原始响应片段：{}", body.chars().take(200).collect::<String>()),
                    rate_limited: status.as_u16() == 429,
                });
            }
            let html = resp
                .text()
                .await
                .map_err(|err| SearchError { provider: "baidu", message: format!("读响应失败：{err}"), rate_limited: false })?;
            if html.contains("百度安全验证") {
                return Err(SearchError {
                    provider: "baidu",
                    message: "百度反爬验证页（非浏览器客户端指纹被拦，fail-loud）——主路径 Bing CN 可用；候浏览器仿真批".into(),
                    rate_limited: false,
                });
            }
            parse_baidu_html(&html, limit).ok_or_else(|| SearchError {
                provider: "baidu",
                message: format!(
                    "结果页解析零命中（HTML 结构可能已变更，fail-loud）；原始响应片段：{}",
                    html.chars().take(200).collect::<String>()
                ),
                rate_limited: false,
            })
        })
    }
}

/// 解析 Baidu 结果页：`result c-container` 块内 `h3 > a`（标题+链接；百度链
/// 常为 baidu.com/link?url= 跳转链，原样保留=可跳转）+块内首个正文 div 文本。
pub(crate) fn parse_baidu_html(html: &str, limit: usize) -> Option<Vec<SearchHit>> {
    let block_marker = "c-container";
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
        let link = block.find("<h3").and_then(|h3| {
            let seg = &block[h3..];
            let a = seg.find("<a")?;
            let after_a = &seg[a..];
            let href_rel = after_a.find("href=\"")?;
            let rest = &after_a[href_rel + "href=\"".len()..];
            let url = rest[..rest.find('"').unwrap_or(rest.len())].to_string();
            let gt = after_a.find('>')?;
            let tail = &after_a[gt + 1..];
            let end = tail.find("</a>")?;
            let title = strip_tags(&tail[..end]);
            Some((url, title))
        });
        // 摘要：锚定已知正文标记（c-abstract/content-right/c-span-last；百度
        // 结构多变，标记缺失=无摘要不伪造）
        let snippet = block.find("<h3").and_then(|h3| {
            for marker in ["c-abstract", "content-right", "c-span-last"] {
                if let Some(pos) = block[h3..].find(marker) {
                    let seg = &block[h3 + pos..];
                    let gt = seg.find('>')?;
                    let tail = &seg[gt + 1..];
                    let end = tail.find("</div>").or_else(|| tail.find("</span>"))?;
                    let text = strip_tags(&tail[..end]);
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
            None
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
    out.trim().to_string()
}

/// 离线结构夹具（代表性形态；反爬活测受限已注记——此测锁解析器本身）。
#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r##"<div class="result c-container" id="1">
<h3 class="t"><a href="http://www.baidu.com/link?url=abc123" target="_blank">Rust 程序设计语言</a></h3>
<div class="c-abstract">生产环境中的 Rust 全世界已有数百家公司在使用。</div>
</div>
<div class="result c-container" id="2">
<h3 class="t"><a href="http://www.baidu.com/link?url=def456">Rust 教程</a></h3>
<div class="c-span-last">Rust 是一门赋予每个人构建可靠且高效软件能力的语言。</div>
</div>"##;

    /// 快照：标题/跳转链/正文段提取。
    #[test]
    fn baidu解析快照() {
        let hits = parse_baidu_html(FIXTURE, 8).expect("夹具应解析出结果");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].url, "http://www.baidu.com/link?url=abc123");
        assert_eq!(hits[0].title, "Rust 程序设计语言");
        assert!(hits[0].snippet.contains("生产环境中的 Rust"));
        assert_eq!(hits[1].url, "http://www.baidu.com/link?url=def456");
    }

    /// 结构变更 fail-loud。
    #[test]
    fn 结构变更零命中() {
        assert!(parse_baidu_html("<html>验证页</html>", 8).is_none());
    }
}
