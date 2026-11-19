//! Brave Search 后端（表序第三；免费档 1q/s，key 在位才可用——结果质量最佳，
//! 有 key 用户的升级项）。

use super::{clamp_snippet, SearchError, SearchHit, SearchProvider};
use yunyou_util::BoxFut;

pub struct BraveProvider {
    pub api_key: Option<String>,
}

const ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";

impl SearchProvider for BraveProvider {
    fn id(&self) -> &'static str {
        "brave"
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
                    provider: "brave",
                    message: "缺少 API key（BRAVE_API_KEY 或 [plugins.yunyou-web-search].brave_api_key）".into(),
                    rate_limited: false,
                });
            };
            let resp = client
                .get(ENDPOINT)
                .query(&[("q", query), ("count", &limit.to_string())])
                .header("X-Subscription-Token", key)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|err| SearchError {
                    provider: "brave",
                    message: format!("请求失败：{err}"),
                    rate_limited: err.status().is_some_and(|s| s.as_u16() == 429),
                })?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(SearchError {
                    provider: "brave",
                    message: format!("HTTP {status}；原始响应片段：{}", body.chars().take(200).collect::<String>()),
                    rate_limited: status.as_u16() == 429,
                });
            }
            let parsed: serde_json::Value = resp
                .json()
                .await
                .map_err(|err| SearchError { provider: "brave", message: format!("JSON 解析失败：{err}"), rate_limited: false })?;
            let items = parsed["web"]["results"]
                .as_array()
                .ok_or_else(|| SearchError {
                    provider: "brave",
                    message: format!(
                        "响应缺 web.results（结构变更 fail-loud）；原始响应片段：{}",
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
                    snippet: clamp_snippet(r["description"].as_str().unwrap_or_default()),
                    page_age: r["age"].as_str().map(str::to_string),
                })
                .collect())
        })
    }
}
