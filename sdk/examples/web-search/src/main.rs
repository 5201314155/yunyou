//! web_search 外部插件（分发链 release-bin 产物；设计稿 web-search-provider
//! 插件设计 20261118+2026-11-19 国内优先重排）：单工具六后端（Bing CN 零配置
//! 默认国内直连/Baidu 备选/SearXNG/DDG 降位/Brave/Tavily）。
//!
//! 检索实现复用 crates/plugins/yunyou-web-search 的 `search` 核心函数——
//! bundled 内置与外部分发不两套逻辑（citations 三纪律/≤8 条/10s 超时换后端
//! 全部单源）。工具执行是同步签名（SDK `tool()`），HTTP 经一次性 tokio
//! runtime block_on（搜索调用低频，代价可接受；注释留痕）。

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use yunyou_plugin_sdk::{tool, Plugin, ToolFailure};

/// web_search 参数。
#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// 搜索关键词（尽量具体，可用引号短语）
    query: String,
}

fn main() -> std::process::ExitCode {
    let config = yunyou_web_search::WebSearchConfig::default();
    Plugin::new("web-search", "0.1.0")
        .category("tools")
        .description("联网搜索：返回带来源引用（url+摘要绑定）的网页结果")
        .tool(tool("web_search", "联网搜索：返回带来源引用（url+摘要绑定）的网页结果。引用时注明出处。", move |args: SearchArgs| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| ToolFailure::with_hint("tool/web-search", format!("runtime 构建失败：{err}"), "重试一次；持续失败请报障"))?;
            let outcome = rt.block_on(yunyou_web_search::search(&config, &args.query));
            match outcome {
                Ok(result) => {
                    let rendered = crate_render(&result.hits);
                    Ok(json!({
                        "provider": result.provider,
                        "query": args.query,
                        "results": rendered,
                        "count": result.hits.len(),
                        "citations": result.citations.into_iter()
                            .map(|(url, cited_text)| json!({"url": url, "cited_text": cited_text}))
                            .collect::<Vec<_>>(),
                    }))
                }
                Err(attempts) => Err(ToolFailure::with_hint(
                    "tool/web-search",
                    format!("全部搜索后端失败（{}）：\n{attempts}", args.query),
                    "检查网络连通性；Brave/Tavily 需 key（BRAVE_API_KEY/TAVILY_API_KEY）",
                )),
            }
        }))
        .serve()
}

/// 结果渲染（与 bundled render.rs 同形态：标题+来源+摘要绑定行）。
fn crate_render(hits: &[yunyou_web_search::SearchHit]) -> String {
    if hits.is_empty() {
        return "（无结果）".into();
    }
    let mut out = String::new();
    for (i, hit) in hits.iter().enumerate() {
        out.push_str(&format!(
            "[{}] {}\n    来源：{}\n    摘要：{}\n",
            i + 1,
            hit.title,
            hit.url,
            hit.snippet
        ));
    }
    out
}
