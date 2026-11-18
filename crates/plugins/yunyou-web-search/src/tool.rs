//! web_search 工具（设计稿 §一：对外唯一工具面）：provider 选择/降级/重试
//! 收口在此——工具面零后端知识，切换后端只改 settings 一行（验收 2）。
//! 检索核心 [`search`] 独立成函数：SDK web-search 外部插件（分发链）复用
//! 同一实现，bundled 与外部不两套逻辑。

use crate::provider::{build_providers, SearchProvider};
use crate::render::render_results;
use crate::{citations_from, MAX_RESULTS, PROVIDER_TIMEOUT_SECS, RETRY_MAX};
use yunyou_tools::contract::{ToolDefinition, ToolExec, ToolOutcome};
use yunyou_util::BoxFut;
use std::sync::Arc;

/// 检索产物（SDK web-search 外部插件复用面；citations 已按三纪律收口）。
pub struct SearchOutcome {
    pub provider: String,
    pub hits: Vec<crate::provider::SearchHit>,
    pub citations: Vec<(String, String)>,
}

/// 候选链：显式指定者打头（key 缺失=available false 跳过留痕），其余按表序
/// 补位——单 provider 超时/失败即换下一候选，全部失败返回结构化错误。
pub(crate) fn candidate_chain(
    cfg: &crate::plugin::WebSearchConfig,
) -> Vec<Box<dyn SearchProvider>> {
    let mut providers = build_providers(cfg);
    if let Some(want) = cfg.provider.as_deref() {
        if let Some(pos) = providers.iter().position(|p| p.id() == want) {
            let picked = providers.remove(pos);
            providers.insert(0, picked);
        }
    }
    providers
}

/// 执行链核心（SDK 复用入口）：按候选序尝试；限速退避重试 ≤RETRY_MAX 次
/// （500ms 倍增）；单 provider 超时 10s 换候选；Err=逐候选尝试留痕串
/// （不吞错不假结果，设计稿 §四.4）。
pub async fn search(
    cfg: &crate::plugin::WebSearchConfig,
    query: &str,
) -> Result<SearchOutcome, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(PROVIDER_TIMEOUT_SECS))
        .build()
        .map_err(|err| format!("HTTP 客户端构建失败：{err}"))?;
    let chain = candidate_chain(cfg);
    let mut attempts: Vec<String> = Vec::new();
    for provider in &chain {
        if !provider.available() {
            attempts.push(format!("[{}] 跳过（key 缺失）", provider.id()));
            continue;
        }
        let mut backoff = std::time::Duration::from_millis(500);
        for attempt in 0..=RETRY_MAX {
            let result = provider
                .search(&client, query, cfg.max_results.min(MAX_RESULTS))
                .await;
            match result {
                Ok(hits) => {
                    let citations = citations_from(&hits);
                    return Ok(SearchOutcome {
                        provider: provider.id().to_string(),
                        hits,
                        citations,
                    });
                }
                Err(err) => {
                    attempts.push(format!("[{}#{attempt}] {err}", provider.id()));
                    if err.rate_limited && attempt < RETRY_MAX {
                        tokio::time::sleep(backoff).await;
                        backoff *= 2;
                        continue;
                    }
                    break; // 换下一候选
                }
            }
        }
    }
    Err(attempts.join("\n"))
}

/// 工具执行链（ToolOutcome 收口）：核心链之上叠渲染与模型面形态。
async fn run_search(cfg: &crate::plugin::WebSearchConfig, query: &str) -> ToolOutcome {
    let outcome = match search(cfg, query).await {
        Ok(outcome) => outcome,
        Err(attempts) => {
            return ToolOutcome::err_h(
                "tool/web-search",
                format!("全部搜索后端失败（{query}）：\n{attempts}"),
                "检查网络连通性与 provider 配置（[plugins.yunyou-web-search].provider）；Brave/Tavily 需 key（BRAVE_API_KEY/TAVILY_API_KEY）",
            );
        }
    };
    let rendered = render_results(&outcome.hits);
    let citations_json: Vec<serde_json::Value> = outcome
        .citations
        .into_iter()
        .map(|(url, cited_text)| serde_json::json!({"url": url, "cited_text": cited_text}))
        .collect();
    ToolOutcome::ok(serde_json::json!({
        "provider": outcome.provider,
        "query": query,
        "results": rendered,
        "count": outcome.hits.len(),
        "citations": citations_json,
    }))
}

/// web_search 工具定义（模型面）。
pub struct WebSearchTool {
    pub config: Arc<crate::plugin::WebSearchConfig>,
}

impl ToolDefinition for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "联网搜索：返回带来源引用（url+摘要绑定）的网页结果。用于查文档/找资料/核实事实；每条结果带来源 URL，引用时注明出处。"
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "搜索关键词（尽量具体，可用引号短语）"}
            },
            "required": ["query"]
        })
    }

    fn execute(&self, args: serde_json::Value, _exec: ToolExec) -> BoxFut<'static, ToolOutcome> {
        let config = self.config.clone();
        Box::pin(async move {
            let Some(query) = args.get("query").and_then(|v| v.as_str()) else {
                return ToolOutcome::err("tool/args", "缺少 query 参数");
            };
            run_search(&config, query).await
        })
    }

    fn output_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "provider": {"type": "string", "description": "实际使用的后端"},
                "query": {"type": "string", "description": "搜索词"},
                "results": {"type": "string", "description": "渲染后的结果（标题+来源+摘要绑定）"},
                "count": {"type": "integer", "description": "结果条数（≤8）"},
                "citations": {"type": "array", "items": {"type": "object"}, "description": "url→摘要引用映射（三纪律收口）"}
            },
            "required": ["provider", "query", "results", "count", "citations"]
        }))
    }

    fn presentation_meta(&self) -> Option<yunyou_tools::contract::PresentationMeta> {
        Some(yunyou_tools::contract::PresentationMeta::new(
            "联网搜索",
            "搜索网页并带来源引用",
            "search",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::candidate_chain;
    use crate::plugin::WebSearchConfig;
    use super::run_search;
    use yunyou_tools::contract::ToolOutcome;

    /// 候选链：显式 provider 打头；未指定按表序（免费优先）。
    #[test]
    fn 候选链序() {
        let mut cfg = WebSearchConfig::default();
        let chain = candidate_chain(&cfg);
        assert_eq!(chain[0].id(), "duckduckgo", "零配置默认=DDG");
        cfg.provider = Some("tavily".into());
        cfg.tavily_api_key = Some("k".into());
        let chain = candidate_chain(&cfg);
        assert_eq!(chain[0].id(), "tavily", "显式指定打头");
        assert!(chain[0].available());
    }

    /// key 缺失的显式指定=available false（跳过并留痕）。
    #[test]
    fn key缺失跳过留痕() {
        let cfg = WebSearchConfig::default();
        let chain = candidate_chain(&cfg);
        let brave = chain.iter().find(|p| p.id() == "brave").unwrap();
        assert!(!brave.available());
    }

    /// run_search 全失败=结构化错误含逐候选尝试记录（不吞错不假结果）。
    #[tokio::test(flavor = "multi_thread")]
    async fn 全失败结构化错误() {
        let mut cfg = WebSearchConfig::default();
        cfg.searxng_base_url = "http://127.0.0.1:1".into(); // 必然连接失败
        let out = run_search(&cfg, "test query").await;
        match out {
            ToolOutcome::Err(err) => {
                assert!(err.message.contains("全部搜索后端失败"));
                assert!(err.message.contains("duckduckgo"), "逐候选尝试留痕：{}", err.message);
            }
            other => panic!("全失败应 Err：{other:?}"),
        }
    }

    /// 验收①（活测，#[ignore]：需网络；交卷时显式跑一次取证据）——
    /// 零配置 DDG 真搜带 citations。
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "需网络；验收①显式跑"]
    async fn 零配置真搜带citations() {
        let cfg = WebSearchConfig::default();
        let out = run_search(&cfg, "rust landlock").await;
        match out {
            ToolOutcome::Ok(v) => {
                assert_eq!(v["provider"], "duckduckgo");
                assert!(v["count"].as_u64().unwrap_or(0) > 0, "应出结果：{v:?}");
                assert!(v["citations"].as_array().is_some_and(|c| !c.is_empty()), "应带 citations");
                assert!(v["results"].as_str().unwrap().contains("来源："), "url+摘要绑定渲染");
                println!("验收①原始输出：{}", serde_json::to_string_pretty(&v).unwrap());
            }
            other => panic!("真搜应成功：{other:?}"),
        }
    }
}
