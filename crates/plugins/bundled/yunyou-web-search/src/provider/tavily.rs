//! Tavily 后端（表序第四；免费档 1k 次/月，key 在位才可用——LLM 摘要型结果，
//! 适合做 v2 的「深搜」档）。

use super::{clamp_snippet, SearchError, SearchHit, SearchProvider};
use yunyou_util::BoxFut;

pub struct TavilyProvider {
    pub api_key: Option<String>,
}

const ENDPOINT: &str = "https://api.tavily.com/search";

impl SearchProvider for TavilyProvider {
    fn id(&self) -> &'static str {
        "tavily"
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn available(&self) -> bool {
        self.api_key.as_deref().is_some_and(|k| !k.trim().is_empty())
    }

    fn search<'a>(
        &'a self,
        client: &'a reqwest::Client,
        query: &'a str,
        limit: usize,
    ) -> BoxFut<'a, Result<Vec<SearchHit>, SearchError>> {
        let key = self.api_key.clone();
        Box::pin(async move {
            let Some(key) = key.filter(|k| !k.trim().is_empty()) else {
                return Err(SearchError {
                    provider: "tavily",
                    message: "缺少 API key（TAVILY_API_KEY 或 [plugins.yunyou-web-search].tavily_api_key）".into(),
                    rate_limited: false,
                });
            };
            let resp = client
                .post(ENDPOINT)
                .json(&serde_json::json!({
                    "api_key": key,
                    "query": query,
                    "max_results": limit,
                }))
                .send()
                .await
                .map_err(|err| SearchError {
                    provider: "tavily",
                    message: format!("请求失败：{err}"),
                    rate_limited: err.status().is_some_and(|s| s.as_u16() == 429),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(SearchError {
                    provider: "tavily",
                    message: format!("HTTP {status}；原始响应片段：{}", body.chars().take(200).collect::<String>()),
                    rate_limited: status.as_u16() == 429,
                });
            }
            let parsed: serde_json::Value = resp
                .json()
                .await
                .map_err(|err| SearchError { provider: "tavily", message: format!("JSON 解析失败：{err}"), rate_limited: false })?;
            let items = parsed["results"]
                .as_array()
                .ok_or_else(|| SearchError {
                    provider: "tavily",
                    message: format!(
                        "响应缺 results（结构变更 fail-loud）；原始响应片段：{}",
                        serde_json::to_string(&parsed).unwrap_or_default().chars().take(200).collect::<String>()
                    ),
                    rate_limited: false,
                })?;
            Ok(items
                .iter()
                .take(limit)
                .map(|r| SearchHit {
                    url: r["url"].as_str().unwrap_or_default().to_string(),
                    title: r["title"].as_str().unwrap_or_default().to_string(),
                    snippet: clamp_snippet(r["content"].as_str().unwrap_or_default()),
                    page_age: r["published_date"].as_str().map(str::to_string),
                })
                .collect())
        })
    }
}
