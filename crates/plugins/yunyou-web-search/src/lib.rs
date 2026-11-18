//! web_search 单工具搜索插件（设计稿 20261118 已批）：模型只见一个搜索工具，
//! 后端差异全部藏在 provider 层——单工具换后端，不学 dsh 一 provider 一包。
//!
//! # provider 序（国内优先，2026-11-19 业主令「国内你还要我翻墙？」重排）
//! bing（cn.bing.com HTML，零配置默认，国内直连）→ baidu（HTML 备选零配置，
//! 反爬注记见 provider/baidu.rs）→ searxng（可配 `searxng_base_url`，自建/海外
//! 实例）→ duckduckgo（海外用户/翻墙场景降位）→ brave（免费 key）→ tavily
//! （免费 key，LLM 摘要型）。国内开箱即搜；海外场景 settings 显式指定即换。
//! settings `web_search.provider` 显式指定优先；未指定按表序取「可用者」（
//! needs_key=false 或 key 在位）；key 过渡期走 config/env（凭据缝⑨开闸后迁移）。
//!
//! # 纪律（设计稿 §三/§四）
//! citations 三纪律照抄 dsh citationSnippets（provider.ts:138-152）：首见 URL 的
//! cited_text 赢、空项跳过、map 去重——摘要与来源 URL 绑定输出，无 url 的
//! snippet 不单独出现；结果 ≤8 条、snippet ≤300 字符（防上下文爆）；单 provider
//! 超时 10s 换下一候选，限速退避重试 ≤2 次后放弃，全部失败返回结构化错误
//! （不吞错不假结果）。本插件不走 ctx.llm（搜索=HTTP+解析，LLM 改写属上游）。

mod plugin;
mod provider;
mod render;
mod tool;

pub use plugin::{WebSearchConfig, WebSearchPlugin};
pub use tool::{search, SearchOutcome};
pub use provider::{SearchError, SearchHit, SearchProvider};
pub use tool::WebSearchTool;

/// 检索参数上限纪律（设计稿 §四.3）。
pub(crate) const MAX_RESULTS: usize = 8;
pub(crate) const SNIPPET_MAX_CHARS: usize = 300;
/// 单 provider 超时（超时即换下一候选）。
pub(crate) const PROVIDER_TIMEOUT_SECS: u64 = 10;
/// 限速退避重试上限（设计稿验收 5：≤2 次后放弃）。
pub(crate) const RETRY_MAX: usize = 2;

/// 四后端 id（表序=免费优先序；显式配置按 id 指定）。
pub const PROVIDER_ORDER: &[&str] = &["duckduckgo", "searxng", "brave", "tavily"];

/// citations 三纪律（dsh citationSnippets 照抄）：首见 URL 的摘要赢、空项跳过、
/// 按 URL 去重——渲染层输入为归一化结果，输出与结果项一一绑定。
pub(crate) fn citations_from(hits: &[SearchHit]) -> Vec<(String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for hit in hits {
        if hit.url.trim().is_empty() || hit.snippet.trim().is_empty() {
            continue; // 空项跳过：无 url/无摘要不单独出现
        }
        if seen.insert(hit.url.clone()) {
            out.push((hit.url.clone(), hit.snippet.clone())); // 首见赢
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(url: &str, snippet: &str) -> SearchHit {
        SearchHit {
            url: url.into(),
            title: "t".into(),
            snippet: snippet.into(),
            page_age: None,
        }
    }

    /// citations 三纪律（设计稿验收 4）：首见赢/空项跳过/去重。
    #[test]
    fn citations三纪律() {
        let hits = vec![
            hit("https://a", "第一个"),
            hit("https://b", ""),
            hit("", "无 url 不单独出现"),
            hit("https://a", "重复 url 首见赢"),
            hit("https://c", "第三条"),
        ];
        let c = citations_from(&hits);
        assert_eq!(
            c,
            vec![
                ("https://a".to_string(), "第一个".to_string()),
                ("https://c".to_string(), "第三条".to_string()),
            ]
        );
    }
}
