//! 结果渲染（citations 纪律收口层）：摘要与来源 URL 绑定输出——无 url 的
//! snippet 不单独出现（三纪律已在 lib.rs citations_from 收拢，此处只做呈现
//! 形态：每条=标题+url+snippet 绑定行）。

use crate::provider::SearchHit;

/// 渲染为模型可读文本（每条带 [n] 序号；url 与 snippet 同行绑定）。
pub(crate) fn render_results(hits: &[SearchHit]) -> String {
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

/// 快照：绑定形态（无 url 项被 citations_from 过滤前——本层输入已归一，
/// 空 url 项测试锁死不单独出现）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 渲染绑定形态() {
        let hits = vec![SearchHit {
            url: "https://a".into(),
            title: "标题A".into(),
            snippet: "摘要A".into(),
            page_age: None,
        }];
        let text = render_results(&hits);
        assert!(text.contains("[1] 标题A"));
        assert!(text.contains("来源：https://a"));
        assert!(text.contains("摘要：摘要A"));
        assert_eq!(render_results(&[]), "（无结果）");
    }
}
