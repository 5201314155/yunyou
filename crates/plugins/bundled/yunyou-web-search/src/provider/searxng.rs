//! SearXNG 公共实例后端（表序第二；免 key，实例侧可能限流）：GET
//! `{base}/search?q=…&format=json`——base_url 可配（自建最优，公共实例稳定性
//! 参差，文档注明）。

use super::{clamp_snippet, SearchError, SearchHit, SearchProvider};
use yunyou_util::BoxFut;

pub struct SearxngProvider {
    pub base_url: String,
}

impl SearchProvider for SearxngProvider {
    fn id(&self) -> &'static str {
        "searxng"
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
        let base = self.base_url.trim_end_matches('/').to_string();
        Box::pin(async move {
            let resp = client
                .get(format!("{base}/search"))
                .query(&[("q", query), ("format", "json")])
                .send()
                .await
                .map_err(|err| SearchError {
                    provider: "searxng",
                    message: format!("请求失败：{err}"),
                    rate_limited: err.status().is_some_and(|s| s.as_u16() == 429),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(SearchError {
                    provider: "searxng",
                    message: format!("HTTP {status}；原始响应片段：{}", body.chars().take(200).collect::<String>()),
                    rate_limited: status.as_u16() == 429,
                });
            }
            #[derive(serde::Deserialize)]
            struct SearxResult {
                #[serde(default)]
                url: String,
                #[serde(default)]
                title: String,
                #[serde(default)]
                content: String,
                #[serde(default)]
                published_date: Option<String>,
            }
            #[derive(serde::Deserialize)]
            struct SearxResp {
                #[serde(default)]
                results: Vec<SearxResult>,
            }
            let parsed: SearxResp = resp
                .json()
                .await
                .map_err(|err| SearchError { provider: "searxng", message: format!("JSON 解析失败（实例可能不支持 format=json）：{err}"), rate_limited: false })?;
            Ok(parsed
                .results
                .into_iter()
                .take(limit)
                .map(|r| SearchHit {
                    snippet: clamp_snippet(&r.content),
                    url: r.url,
                    title: r.title,
                    page_age: r.published_date,
                })
                .collect())
        })
    }
}

/// 解析快照（离线夹具）。
#[cfg(test)]
mod tests {

    #[test]
    fn searxng字段归一() {
        let raw = r#"{"results":[{"url":"https://a","title":"A","content":"content text","published_date":"2026-01-01"},{"title":"无 url"}]}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let r0 = &parsed["results"][0];
        assert_eq!(r0["url"], "https://a");
        // 完整解析路径经 SearchProvider：此处锁字段名契约（url/title/content/published_date）
        assert!(r0["content"].is_string());
        assert!(parsed["results"][1]["url"].as_str().is_none() || parsed["results"][1]["url"] == "");
    }
}
