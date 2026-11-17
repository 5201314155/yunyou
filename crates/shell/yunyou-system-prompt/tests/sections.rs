//! 契约③ section 机制验收测试（S3 落点；四件＝段位表排序 / 同名替换语义 / 插值正反例 /
//! 注销后 compose 不含该段，另补 complete 互斥校验与 persona 断头路根治两件）。

use yunyou_system_prompt::{
    DefaultSystemPrompt, Section, SectionRegistry, SystemPromptError, SystemPrompt,
    IDENTITY_TEXT, ORDER_HARNESS_IDENTITY, SECTION_HARNESS_IDENTITY, SECTION_PERSONA_PREFIX,
};
use yunyou_llm::ToolSchema;
use std::sync::Arc;

/// 测试用工具集。
fn tools() -> Vec<ToolSchema> {
    vec![
        ToolSchema {
            name: "bash".into(),
            description: "执行命令".into(),
            parameters: serde_json::json!({}),
        },
        ToolSchema {
            name: "read".into(),
            description: "读文件".into(),
            parameters: serde_json::json!({}),
        },
    ]
}

/// 段文本在拼装结果中的位置（用于断言先后顺序）。
fn at(text: &str, needle: &str) -> usize {
    text.find(needle).unwrap_or_else(|| panic!("拼装结果缺少「{needle}」: {text}"))
}

// ---------- 测试四件之一：段位表排序 ----------

/// 段按 order 升序拼接；段序相等时按段名字典序（对标官方 :58-61）。
#[test]
fn 段位表排序_升序且相等段序按段名() {
    let registry = Arc::new(SectionRegistry::default());
    // 身份段自建（段位最低）：with_registry 尊重调用方注册表，不替宿主补默认段。
    let _identity = registry
        .register_section(Section::new(
            SECTION_HARNESS_IDENTITY,
            ORDER_HARNESS_IDENTITY,
            IDENTITY_TEXT,
        ))
        .expect("注册成功");
    // 500 档同名序：z 段与 a 段同段序，字典序 a 先出；-5 档早于 500；身份段 -1000 最前。
    let _late = registry
        .register_section(Section::new("z:策略", 500.0, "策略段"))
        .expect("注册成功");
    let _same_order = registry
        .register_section(Section::new("a:规范", 500.0, "规范段"))
        .expect("注册成功");
    let _early = registry
        .register_section(Section::new("m:最早", -5.0, "最早段"))
        .expect("注册成功");
    let prompt = DefaultSystemPrompt::with_registry(Arc::clone(&registry));
    let out = prompt.compose("", &[]);

    let p_identity = at(&out, "编码助手");
    let p_early = at(&out, "最早段");
    let p_a = at(&out, "规范段");
    let p_z = at(&out, "策略段");
    assert!(p_identity < p_early, "身份段（-1000）应最前");
    assert!(p_early < p_a, "最早段（-5）应早于 500 档");
    assert!(p_a < p_z, "同段序 500 应按段名字典序（a 先于 z）");
}

// ---------- 测试四件之二：同名替换语义 ----------

/// 同名注册＝替换（shadow 栈）：后注册者生效，卸下后先前那层重新生效
/// （对标官方人设槽「同名正是替换而非重复」，:189-200）。
#[tokio::test]
async fn 同名注册_替换且注销后还原前层() {
    let registry = Arc::new(SectionRegistry::default());
    let lower = registry
        .register_section(Section::new("my:slot", 50.0, "前层文本"))
        .expect("注册成功");
    let upper = registry
        .register_section(Section::new("my:slot", 50.0, "后层文本"))
        .expect("同名替换成功");
    let prompt = DefaultSystemPrompt::with_registry(Arc::clone(&registry));

    let out = prompt.compose("", &[]);
    assert!(out.contains("后层文本"), "后注册者应生效: {out}");
    assert!(!out.contains("前层文本"), "前层应被顶替而非并存: {out}");

    upper.dispose().await;
    let out = prompt.compose("", &[]);
    assert!(out.contains("前层文本"), "卸下后层后前层应重新生效: {out}");
    assert!(!out.contains("后层文本"), "后层应已注销: {out}");

    lower.dispose().await;
    let out = prompt.compose("", &[]);
    assert!(!out.contains("前层文本") && !out.contains("后层文本"), "两层都卸完后应无痕: {out}");
}

// ---------- 测试四件之三：变量插值正反例 ----------

/// 正例：合法 `{{var}}` 被替换；反例：未注册变量＝空串（不炸），畸形引用＝抛错。
#[tokio::test]
async fn 变量插值_正例替换_未注册空串_畸形抛错() {
    let registry = Arc::new(SectionRegistry::default());
    registry.register_variable("cwd", "/work/yunyou").expect("变量登记");
    let disposer = registry
        .register_section(Section::new("my:env", 50.0, "目录是 {{cwd}}，缺的是 {{missing}}。"))
        .expect("注册成功");
    let prompt = DefaultSystemPrompt::with_registry(Arc::clone(&registry));

    let out = prompt.compose("", &[]);
    assert!(out.contains("目录是 /work/yunyou"), "已注册变量应替换: {out}");
    assert!(out.contains("缺的是 。"), "未注册变量应落空串而非失败: {out}");

    // 畸形引用：`{{}}` 空名 → 注册期抛错（对标官方 :368-370）。
    let bad_empty = registry.register_section(Section::new("my:bad", 60.0, "看 {{}} 这里"));
    assert!(
        matches!(bad_empty, Err(SystemPromptError::MalformedVariable { .. })),
        "空名引用应抛 MalformedVariable，实得: {bad_empty:?}"
    );
    // 畸形引用：大写/空格名 → 抛错。
    let bad_case = registry.register_section(Section::new("my:bad2", 60.0, "看 {{Cwd}} 这里"));
    assert!(
        matches!(bad_case, Err(SystemPromptError::MalformedVariable { .. })),
        "大写变量名应抛 MalformedVariable，实得: {bad_case:?}"
    );
    // 畸形引用不得落表：前面注册失败的两段（连同其畸形引用原文）不能出现在结果里。
    assert!(
        !out.contains("{{}}") && !out.contains("{{Cwd}}"),
        "畸形段未落表，畸形引用不应出现: {out}"
    );

    // 孤立 `{{`（后文无 `}}`）是字面散文，原样保留（对标官方 :357-363）。
    let prose = registry
        .register_section(Section::new("my:prose", 70.0, "这里有个 {{ 孤立的开括号"))
        .expect("字面散文应放行");
    let out = prompt.compose("", &[]);
    assert!(out.contains("{{ 孤立的开括号"), "孤立开括号应原样保留: {out}");

    prose.dispose().await;
    disposer.dispose().await;
}

// ---------- 测试四件之四：注销后 compose 不含该段 ----------

/// 卸载逆序清理：disposer 注销后该段从拼装结果消失，其余段不受影响。
#[tokio::test]
async fn 注销后_compose不含该段() {
    let registry = Arc::new(SectionRegistry::default());
    let keep = registry
        .register_section(Section::new("my:keep", 50.0, "常驻段"))
        .expect("注册成功");
    let drop_me = registry
        .register_section(Section::new("my:drop", 60.0, "临时段"))
        .expect("注册成功");
    let prompt = DefaultSystemPrompt::with_registry(Arc::clone(&registry));

    assert!(prompt.compose("", &[]).contains("临时段"));
    drop_me.dispose().await;
    let out = prompt.compose("", &[]);
    assert!(!out.contains("临时段"), "注销后不应含该段: {out}");
    assert!(out.contains("常驻段"), "其余段不受影响: {out}");

    keep.dispose().await;
    assert!(!prompt.compose("", &[]).contains("常驻段"));
}

// ---------- 补充件：complete 互斥校验 ----------

/// 单个互斥段生效＝它就是完整提示词（其余段让位）；两个同时生效＝装配失败。
#[tokio::test]
async fn complete互斥_单段独占_多段报错() {
    let registry = Arc::new(SectionRegistry::default());
    // 先自建宿主默认身份段（with_registry 不替宿主补默认段，见 lib.rs with_registry），
    // 「宿主默认段让位」断言才有真值可言（防空真）。
    registry
        .register_section(Section::new(
            SECTION_HARNESS_IDENTITY,
            ORDER_HARNESS_IDENTITY,
            IDENTITY_TEXT,
        ))
        .expect("注册成功");
    let sole = registry
        .register_section(Section::new("my:sole", 500.0, "唯一完整提示").complete())
        .expect("注册成功");
    let ordinary = registry
        .register_section(Section::new("my:ordinary", 600.0, "普通段"))
        .expect("注册成功");
    let prompt = DefaultSystemPrompt::with_registry(Arc::clone(&registry));

    let out = prompt.compose("", &[]);
    assert!(out.contains("唯一完整提示"), "互斥段文本应在: {out}");
    assert!(!out.contains("普通段"), "其余段应让位: {out}");
    assert!(!out.contains("编码助手"), "宿主默认段也应让位: {out}");

    // 第二个 complete 段注册成功（注册期无法预知全局），装配期报错。
    let rival = registry
        .register_section(Section::new("my:rival", 700.0, "第二个完整提示").complete())
        .expect("注册本身合法");
    let err = prompt.try_compose("", &[]);
    assert!(
        matches!(err, Err(SystemPromptError::CompleteConflict { .. })),
        "两个 complete 段应报 CompleteConflict，实得: {err:?}"
    );
    // 降级路径（compose）不炸：忽略互斥、按段位表照常拼装，仅记 error 日志。
    let lossy = prompt.compose("", &[]);
    assert!(lossy.contains("普通段"), "降级装配应恢复有序拼装: {lossy}");

    rival.dispose().await;
    ordinary.dispose().await;
    sole.dispose().await;
}

// ---------- 补充件：persona 断头路根治 ----------

/// 人设槽：运行期 persona 同名 shadow 替换默认段（空＝保留默认），组件侧不再断头。
#[tokio::test]
async fn persona槽_运行期替换默认段() {
    let prompt = DefaultSystemPrompt::with_persona("默认人设".into()).expect("人设合法");
    let out = prompt.compose("临时人设", &[]);
    assert!(out.contains("临时人设"), "运行期人设应替换默认段: {out}");
    assert!(!out.contains("默认人设"), "默认段应被顶替而非并存: {out}");

    let out = prompt.compose("", &[]);
    assert!(out.contains("默认人设"), "空人设应保留默认段: {out}");
    assert!(!out.contains("临时人设"), "临时人设不落表，不应残留: {out}");
}

/// 纯函数语义：相同输入相同输出（存量契约不破）。
#[test]
fn 组装可复现_且工具按传入序注入() {
    let prompt = DefaultSystemPrompt::default();
    let a = prompt.compose("你是测试人设", &tools());
    let b = prompt.compose("你是测试人设", &tools());
    assert_eq!(a, b);
    assert!(a.contains("你是测试人设"));
    assert!(a.contains("- bash: 执行命令"));
    assert!(a.contains("- read: 读文件"));
    assert!(at(&a, "- bash: 执行命令") < at(&a, "- read: 读文件"), "工具按调用方传入序");
}

/// 无工具时明确说明（模型不会臆造工具）。
#[test]
fn 无工具说明() {
    let out = DefaultSystemPrompt::default().compose("", &[]);
    assert!(out.contains("没有可用工具"));
}

/// 段序非有限（NaN/无穷）注册即抛错（对标官方 :481-483）。
#[test]
fn 段序非有限_注册抛错() {
    let registry = Arc::new(SectionRegistry::default());
    let err = registry.register_section(Section::new("my:nan", f64::NAN, "文本"));
    assert!(
        matches!(err, Err(SystemPromptError::NonFiniteOrder { .. })),
        "NaN 段序应抛 NonFiniteOrder，实得: {err:?}"
    );
    let inf = registry.register_section(Section::new("my:inf", f64::INFINITY, "文本"));
    assert!(
        matches!(inf, Err(SystemPromptError::NonFiniteOrder { .. })),
        "无穷段序应抛 NonFiniteOrder，实得: {inf:?}"
    );
    let empty = registry.register_section(Section::new("  ", 10.0, "文本"));
    assert!(
        matches!(empty, Err(SystemPromptError::EmptySectionName)),
        "空段名应抛 EmptySectionName，实得: {empty:?}"
    );
}

/// 人设槽名导出：外部组件可据此注册自己的替换段（契约③ 的槽位键公开面）。
#[test]
fn 人设槽名导出可用() {
    assert_eq!(SECTION_PERSONA_PREFIX, "deployment:persona-prefix");
}

// ---------- 契约③ 消费者路径：trait 注册接口 + 变量登记 ----------

/// 插件只拿得到 `dyn SystemPrompt`：经 trait 注册段（注册即 effect）+ 经注册表句柄
/// 登记变量，均可在拼装结果中观察到；disposer 注销后干净退场。
#[tokio::test]
async fn trait注册接口_注册即effect_变量登记生效() {
    let concrete = DefaultSystemPrompt::default();
    concrete
        .sections()
        .register_variable("branch", "main")
        .expect("变量登记");
    let prompt: Arc<dyn SystemPrompt> = Arc::new(concrete);
    let disposer = prompt
        .register_section(Section::new("plug:onboarding", 50.0, "当前分支 {{branch}}"))
        .expect("注册成功");

    let out = prompt.compose("", &[]);
    assert!(out.contains("当前分支 main"), "trait 注册的段应生效并插值: {out}");

    disposer.dispose().await;
    let out = prompt.compose("", &[]);
    assert!(!out.contains("当前分支"), "注销后应无痕: {out}");
}