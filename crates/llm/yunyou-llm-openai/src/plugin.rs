//! Provider 插件：装载即注册 `dyn Llm` 服务（对标「服务即组件」，方案 §1.9/§2.5）。

use super::llm_error;
use crate::stream::OpenAiStream;
use tokio_util::sync::CancellationToken;
use yunyou::{ConfigValue, Context, FiberOutput, Plugin, PluginLevel, PluginManifest, Result, ServiceId, projection_of};
use yunyou_llm::{GenerateRequest, Llm, LlmFailure, LlmStream};
use yunyou_util::BoxFut;
use std::sync::Arc;
use std::time::Duration;

/// Provider 配置（schemars 派生 → 内核装载前校验，方案 §2.2）。
// deny_unknown_fields（挑刺报告 20261117 F-4）：与 config 镜像段（yunyou-config
// LlmConfig）防呆双轨对齐，拼错键报错不静默吞；本结构无 flatten，加 deny 无互斥风险。
#[derive(Debug, Clone, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpenAiConfig {
    /// 端点根地址（如 `https://api.deepseek.com`；不含 `/chat/completions`）。
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// API 密钥（由宿主装配层注入，绝不入代码/git）。
    /// `role=secret` 标注（schemars extend）：redact 结构性脱敏与 settings 投影的
    /// 「不回显/不预填」对本字段自动生效——与 yunyou.toml 配置面 LlmConfig 同标，
    /// 装配层注入明文真值的字段必须在插件 schema 上闭合同一条防线（防投影消费回显）。
    #[serde(default)]
    #[schemars(extend("role" = "secret"))]
    pub api_key: String,
    /// 默认模型名（请求未指定时使用）。
    #[serde(default = "default_model")]
    pub model: String,
    /// 请求超时（秒；0 = 不限时）。默认 300。
    /// 语义（D-19-1 起 / task-13 扩）：约束两段，各段独立计时（不做跨段合并）——
    /// ①**连接+收头**段（首字节前的总时长）；②非 2xx 错误体读取段（错误体
    /// 挂住不再无限等）。最坏两段串行 2×timeout_secs。读流段不受它约束（推理
    /// 模型 thinking 就是流内字节，时长不可预算），改由
    /// [`Self::stream_idle_timeout_ms`] 的空闲看门狗管。
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    /// 流式读取空闲超时（毫秒；0 = 不设看门狗）。默认 30_000：
    /// chunk 间无任何字节超过该时长即判流死（D-19-1：总预算对流式响应是
    /// 错误超时语义——推理时长不可预算，业主实测 82.3s 思考被总预算中途
    /// 杀死整轮失败；空闲语义容忍任意长的思考，只抓真死链）。
    #[serde(default = "default_stream_idle_timeout_ms")]
    pub stream_idle_timeout_ms: u64,
}

fn default_base_url() -> String {
    "https://api.deepseek.com".into()
}

fn default_model() -> String {
    "deepseek-chat".into()
}

fn default_timeout_secs() -> u64 {
    300
}

fn default_stream_idle_timeout_ms() -> u64 {
    30_000
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        serde_json::from_value(serde_json::Value::Object(serde_json::Map::new()))
            .expect("全默认值构造必然成功")
    }
}

/// OpenAI 兼容 Provider 插件。
///
/// # 依赖
/// 无（Provider 只依赖契约 crate 与内核，方案 §2.6 规则 3）。
/// # 注册
/// setup 内 `ctx.provide(ServiceId::of::<dyn Llm>(), ...)`。
#[derive(Default)]
pub struct OpenAiPlugin;

/// 端点连接参数（client + 配置，Arc 共享进流式 future）。
struct LlmInner {
    client: reqwest::Client,
    config: OpenAiConfig,
}

/// OpenAI 兼容的 [`Llm`] 实现（廉价克隆：内部 Arc）。
#[derive(Clone)]
pub struct OpenAiLlm {
    inner: Arc<LlmInner>,
}

impl Llm for OpenAiLlm {
    fn stream(&self, req: GenerateRequest) -> BoxFut<'static, Result<Box<dyn LlmStream>>> {
        let inner = self.inner.clone();
        Box::pin(async move { stream_inner(&inner, req, CancellationToken::new()).await })
    }

    /// R2：SSE 读循环 select! 取消令牌（见 [`crate::stream::OpenAiStream`]）。
    fn stream_cancellable(
        &self,
        req: GenerateRequest,
        cancel: CancellationToken,
    ) -> BoxFut<'static, Result<Box<dyn LlmStream>>> {
        let inner = self.inner.clone();
        Box::pin(async move { stream_inner(&inner, req, cancel).await })
    }
}

/// 发起流式调用：POST {base_url}/chat/completions (stream=true)。
///
/// # 失败语义
/// HTTP 非 2xx → `Err`（含端点 message，见 wire::parse_error_body）；
/// 2xx 后的 SSE 层错误由流内 `next()` 逐步返回。
async fn stream_inner(
    inner: &LlmInner,
    req: GenerateRequest,
    cancel: CancellationToken,
) -> Result<Box<dyn LlmStream>> {
    // 请求体构造与快照测试共用同一纯函数（杜绝「测试的序列化 ≠ 发出的序列化」）
        let body = super::request::build_request_body(&inner.config.model, &req);
        let url = format!("{}/chat/completions", inner.config.base_url.trim_end_matches('/'));
        let mut request = inner
            .client
            .post(&url)
            .json(&body);
        if !inner.config.api_key.is_empty() {
            request = request.bearer_auth(&inner.config.api_key);
        }
        // D-19-1：send 段（连接+收头，首字节前）的预算由 timeout_secs 配置、
        // 此处 tokio timeout 应用层实施——client 不再设 reqwest 总预算
        //（builder.timeout 会连 body 流一起包进 TotalTimeoutBody，推理模型的
        // thinking 是流内字节，总预算中途到点即杀掉整轮健康流——本缺陷本体）。
        let send_budget = if inner.config.timeout_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(inner.config.timeout_secs))
        };
        let send_fut = request.send();
        // R2 取消织入（task-31 根因②）：send 段（连接+收头，首字节前）原本只受
        // timeout 预算约束、对 cancel 零感知——对话回合中 Ctrl-C 后回合要挂到
        // 中转响应头到达（实测中转排队 18-35s，最坏满 timeout 预算 300s）才被
        // 读循环的 cancel select 解围。此处与读循环同构织入：取消即中止连接
        // 尝试，错误形态同构（LlmFailure::cancelled，还原路径同一）。
        // 超时治理全景（task-13 起）四段独立预算，互不合并：①connect_timeout
        // 15s（TCP 握手，client 层）；②send 段 tokio timeout=timeout_secs
        //（连接+收头）；③非 2xx 错误体读取段=全量 timeout_secs（下方错误体分支，
        // 最坏两段串行 2×timeout_secs——不做跨段总预算合并，属更大改动不本缺陷面）；
        // ④流内空闲看门狗 stream_idle_timeout_ms（读流段，D-19-1）。
        // Timeout 预算分支原样保留在 send 内层（D-19-1 分类铁律不动）。
        let send_result = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                return Err(LlmFailure::cancelled().into_cordis());
            }
            result = async {
                match send_budget {
                    Some(budget) => match tokio::time::timeout(budget, send_fut).await {
                        Ok(inner) => inner.map_err(|err| {
                            if err.is_timeout() {
                                LlmFailure::timeout(format!("请求 {url} 超时: {err}"))
                            } else {
                                LlmFailure::connection(format!("请求 {url} 失败: {err}"))
                            }
                            .into_cordis()
                        }),
                        Err(elapsed) => {
                            // 超时不与网络错误混类（任务18 §4 铁律②）：连接/收头预算
                            // 耗尽=本地超时，默认不重试（§3.1 安全默认律：不确定结果
                            // 不自动重试）；仅真连接失败归 Connection。
                            Err(LlmFailure::timeout(format!(
                                "请求 {url} 连接/收头超时（{elapsed:?} 预算内未收到响应头）"
                            ))
                            .into_cordis())
                        }
                    },
                    None => send_fut.await.map_err(|err| {
                        if err.is_timeout() {
                            LlmFailure::timeout(format!("请求 {url} 超时: {err}"))
                        } else {
                            LlmFailure::connection(format!("请求 {url} 失败: {err}"))
                        }
                        .into_cordis()
                    }),
                }
            } => result,
        };
        let response = send_result?;
        let status = response.status();
        if !status.is_success() {
            // 错误体读取治理（task-13，发现人=云垒挑刺 P1-1；分型裁决=云枢预审）：
            // 裸 await 缺口=send 段预算只护到响应头，端点返回错误状态后挂住 body
            // 即整轮无限挂且 Ctrl-C 不可解（超时治理第四段缺口，见上方全景注释）。
            // 织入形态与 send 段同款（先例 task-31 第二刀 87fe9a9）：①全量
            // timeout_secs 预算（各段独立计时，不追剩余量——错误体属小体积诊断
            // 信息，简化且与 send 段可分型对测）；②取消令牌贯穿（biased select，
            // 取消优先于读体——Ctrl-C 即断不等 body，三处 cancel 分支同构）；
            // ③分型跟事实走（云枢裁决）：HTTP 状态在 text() 之前已到手且为真，
            // body 读取超时/IO 失败**不改分型**——仍归 from_http_status(status)
            // 原分型（挂 body 的 429 与可读 body 的 429 同分型同重试语义，Timeout
            // 重分类=把真 429/500 藏进默认不重试档=语义降级+与可读 body 形态行为
            // 分裂），仅 message 注记「错误体读取超时/失败」供诊断；body 读取中
            // 被取消→Cancelled（铁律①取消优先于一切，先于分型——与 send 段
            // 「彼时状态未知故归 Timeout」前提不同）。
            let read_outcome = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    return Err(LlmFailure::cancelled().into_cordis());
                }
                // 三态：None=预算耗尽（body 事实不完整）；Some(Ok)=读满；
                // Some(Err)=读取 IO 失败（连接重置等）。reqwest 错误不跨臂混型。
                read = async {
                    let body_fut = response.text();
                    match send_budget {
                        Some(budget) => tokio::time::timeout(budget, body_fut).await.ok(),
                        None => Some(body_fut.await),
                    }
                } => read,
            };
            let mut notice = String::new();
            let body_text = match read_outcome {
                Some(Ok(text)) => text,
                Some(Err(_)) => {
                    notice = "（错误体读取失败：body 事实不完整，仅以状态码分型）".to_string();
                    String::new()
                }
                None => {
                    notice = format!(
                        "（错误体读取超时：{}s 预算内未收完，仅以状态码分型）",
                        inner.config.timeout_secs
                    );
                    String::new()
                }
            };
            let detail = super::wire::parse_error_body(&body_text).message().to_string();
            let detail = if notice.is_empty() { detail } else { format!("{detail}{notice}") };
            return Err(
                LlmFailure::from_http_status(status.as_u16(), format!("端点返回 {status}: {detail}"))
                    .into_cordis(),
            );
        }
    // 空闲看门狗预算（D-19-1）：config 毫秒→Duration；0=不设看门狗（不限时直连）
    let idle = if inner.config.stream_idle_timeout_ms == 0 {
        None
    } else {
        Some(Duration::from_millis(inner.config.stream_idle_timeout_ms))
    };
    Ok(Box::new(OpenAiStream::new(response, cancel, idle)))
}

impl Plugin for OpenAiPlugin {
    fn name(&self) -> &str {
        "yunyou-llm-openai"
    }

    /// 声明面（方案 §2.2）：对齐 llm.v1 插槽规格、provide `dyn Llm`、无依赖。
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            id: "yunyou-llm-openai".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            level: PluginLevel::Bundled,
            contracts: vec!["llm.v1".into()],
            provides: vec![projection_of::<dyn Llm>()],
            inject: vec![],
            ui: vec![],
            auto_enable: true,
        }
    }

    fn config_schema(&self) -> Option<ConfigValue> {
        Some(serde_json::to_value(schemars::schema_for!(OpenAiConfig)).expect("schema 可序列化"))
    }

    fn setup(self: Arc<Self>, ctx: Context, config: ConfigValue) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let cfg: OpenAiConfig = serde_json::from_value(config)
                .map_err(|err| llm_error("config", format!("配置反序列化失败: {err}")))?;
            // D-19-1：client 层只设连接限时（连接段固定 15s——TCP 握手时长可
            // 预算），**不设 reqwest 总预算**：builder.timeout 会连 body 流一起
            // 包进 TotalTimeoutBody，推理模型的 thinking 是流内字节、时长不可
            // 预算——总预算中途到点即杀掉整轮健康流（业主实测 82.3s 案例）。
            // 读流段由 stream.rs 空闲看门狗管（chunk 间无字节判死，idle 可配）。
            let client = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .build()
                .map_err(|err| llm_error("http", format!("HTTP 客户端构建失败: {err}")))?;
            let llm: Arc<dyn Llm> = Arc::new(OpenAiLlm { inner: Arc::new(LlmInner { client, config: cfg }) });
            ctx.provide(ServiceId::of::<dyn Llm>(), llm)?;
            Ok(FiberOutput)
        })
    }
}

#[cfg(test)]
#[path = "plugin_tests.rs"]
mod tests;

#[cfg(test)]
mod config_deny_tests {
    use super::*;

    /// F-4（挑刺报告 20261117）：组件侧 Config 拼错键要报错（与 config 镜像
    /// 段 deny 双轨对齐），不静默吞。
    #[test]
    fn 拼错键报错不静默吞() {
        let err = serde_json::from_value::<OpenAiConfig>(serde_json::json!({
            "base_url": "http://x", "model": "m", "api_keyy": "oops"
        }))
        .expect_err("未知键 api_keyy 必报");
        assert!(err.to_string().contains("unknown field"), "{err}");
    }
}

/// 非 2xx 错误体读取治理测试（task-13）：独立 mod 防 plugin_tests.rs 爆 400
/// 硬顶（同 #[path] 外挂先例），mock 形态与既有离线 mock 同款。
#[cfg(test)]
#[path = "error_body_tests.rs"]
mod error_body_tests;
