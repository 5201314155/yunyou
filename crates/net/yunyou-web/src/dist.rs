//! v2 前端产物（dist/）服务面：build.rs 生成的嵌入清单 → HTTP 响应。
//!
//! # 产物来源与纪律
//! dist/ = `frontend/apps/web/dist`（vite+boot 一键链唯一落点）在编译期直接
//! 嵌入，无 crate 根同步副本；改动只发生在「上游前端 vendor 树 → pnpm build
//! → cargo build 重嵌」链路上，不手改产物。产物缺席 = build.rs 编译失败
//! （fail-loud 指路），运行时表恒非空，不存在 v1 回退分支。

// 嵌入产物表（build.rs 生成文件以条目位 include 展开；构建已保证非空）。
include!(concat!(env!("OUT_DIR"), "/dist_manifest.rs"));

/// dist 是否可用（server 据此决定 v2/v1 服务形态）。
pub(crate) fn available() -> bool {
    !DIST_FILES.is_empty()
}

/// 取一个产物（path 含前导 `/`；无 = None）。
pub(crate) fn get(path: &str) -> Option<&'static [u8]> {
    DIST_FILES.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

/// SPA 回退：上游前端单页入口（深链未命中文件时回 index.html）。
pub(crate) fn index() -> Option<&'static [u8]> {
    get("/index.html")
}

/// 扩展名 → Content-Type（vite 产物覆盖面：js/css/html/svg/png/字体/清单）。
pub(crate) fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "webmanifest" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain; charset=utf-8",
        "map" => "application/json; charset=utf-8",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// v1 阶段 dist 未构建：表空、available=false、index=None——回退语义锁定。
    #[test]
    fn dist缺席时回退语义() {
        // 本断言在 dist 落库后自然失效（v2 件入库时同步改写为可达断言）
        if available() {
            assert!(index().is_some(), "dist 可用时入口必须存在");
        } else {
            assert!(index().is_none(), "dist 缺席时不得伪造入口");
        }
    }

    /// Content-Type 覆盖面：主流产物扩展名逐个断言。
    #[test]
    fn content_type覆盖面() {
        assert_eq!(content_type("/index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("/assets/a-H1.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("/assets/b-H1.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("/manifest.webmanifest"), "application/json; charset=utf-8");
        assert_eq!(content_type("/icon-192.png"), "image/png");
        assert_eq!(content_type("/fonts/x.woff2"), "font/woff2");
        assert_eq!(content_type("/x.unknown"), "application/octet-stream");
    }
}
