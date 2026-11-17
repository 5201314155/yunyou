//! 命令注册缝验收测试（任务29 C1，设计 §九验收矩阵）：
//! 注册/list 顺序稳定、同名 Err fail-loud、disposer 除名、别名不遮蔽精确名。

use std::sync::Arc;
use yunyou::ErrorKind;
use yunyou_commands::{
    CommandDefinition, CommandHandler, CommandInvocation, CommandKind, CommandOutput,
    CommandRegistry, CommandsService,
};

/// 空转 handler（注册表测试不执行 handler，只验目录行为）。
fn dummy_handler() -> CommandHandler {
    Arc::new(|_inv: CommandInvocation| {
        Box::pin(async { Ok(CommandOutput::success(String::new())) })
    })
}

/// 造一个动作类命令定义。
fn def(name: &str, aliases: &[&str], description: &str) -> CommandDefinition {
    CommandDefinition {
        name: name.to_string(),
        aliases: aliases.iter().map(|s| s.to_string()).collect(),
        description: description.to_string(),
        kind: CommandKind::Action,
        accepts_args: false,
        handler: dummy_handler(),
    }
}

/// 注册并返回 Disposer（漏登记 disposer 即违规，本测试持有以自查）。
fn register(
    registry: &dyn CommandsService,
    name: &str,
    aliases: &[&str],
    description: &str,
) -> yunyou::Disposer {
    registry.register(def(name, aliases, description)).expect("注册应成功")
}

/// 注册成功且 list 按名字典序稳定输出（设计 §二：顺序稳定可复现）。
#[test]
fn 注册与list字典序稳定() {
    let registry = CommandRegistry::new();
    let _d1 = register(&registry, "quit", &["退出"], "退出");
    let _d2 = register(&registry, "help", &["帮助"], "看命令");
    let _d3 = register(&registry, "new", &["新建"], "新会话");
    let names: Vec<String> = registry.list().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, vec!["help", "new", "quit"], "list 应按 name 字典序");
    // 重复 list 结果一致（可复现）
    let again: Vec<String> = registry.list().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, again);
}

/// 同名重复注册 → Err(ServiceDuplicate) 点名命令名，且原条目不被覆盖（fail-loud）。
#[test]
fn 同名注册报错且不覆盖() {
    let registry = CommandRegistry::new();
    let _d = register(&registry, "help", &["帮助"], "第一条");
    let err = registry
        .register(def("help", &["另一个"], "第二条"))
        .expect_err("同名注册必须报错");
    assert_eq!(err.kind(), &ErrorKind::ServiceDuplicate, "错误类别应为 ServiceDuplicate");
    assert!(err.message().contains("help"), "错误信息应点名命令名：{}", err.message());
    // 原条目完好：别名与描述都还是第一条的
    let found = registry.find("help").expect("原条目应在");
    assert_eq!(found.description, "第一条", "静默覆盖被拒绝，原定义保留");
    assert_eq!(found.aliases, vec!["帮助".to_string()]);
}

/// disposer 除名：dispose 后目录不可见，同名可重新注册；
/// 注册表实例析构后残留 disposer 执行为 no-op（弱引用兜底，不悬挂不复活）。
#[tokio::test]
async fn disposer除名与析构兜底() {
    let registry = CommandRegistry::new();
    let disposer = register(&registry, "quit", &["退出"], "退出");
    assert!(registry.find("quit").is_some(), "注册后应可见");
    disposer.dispose().await;
    assert!(registry.find("quit").is_none(), "除名后 find 不可见");
    assert!(registry.list().is_empty(), "除名后 list 为空");
    // 除名后同名可重新注册（不残留僵尸条目）
    let _d2 = register(&registry, "quit", &["退出"], "重新注册");
    assert_eq!(registry.find("quit").expect("重注册应可见").description, "重新注册");
    // 注册表实例析构后，残留 disposer 执行 no-op（内部弱引用升级失败）
    let orphan = {
        let short_lived = CommandRegistry::new();
        register(&short_lived, "gone", &[], "短命注册表")
    };
    orphan.dispose().await; // 不炸即通过
}

/// 注册即 effect 的登记形态：disposer 登记为 fiber effect 后，fiber 卸载自动除名
/// （与 ctx.effect 同语义；本测试是「注册即 effect」纪律的最小闭环）。
#[tokio::test(flavor = "multi_thread")]
async fn 注册即effect随fiber卸载除名() {
    let root = yunyou::Context::new();
    let fiber = root.extend(yunyou::ContextMeta { name: Some("命令注册方".into()) });
    let registry = CommandRegistry::new();
    // 在 fiber 上登记注册（模拟插件 setup：register → disposer 登记为 effect）
    let registry_for_setup = registry.clone();
    fiber
        .effect("注册 /quit", move || {
            let disposer = registry_for_setup.register(def("quit", &["退出"], "退出"))?;
            Ok(Some(disposer))
        })
        .expect("effect 登记应成功");
    assert!(registry.find("quit").is_some(), "effect 登记后立即可见");
    // fiber 卸载 → effect 逆序清理 → 命令从目录消失
    fiber.dispose().await.expect("fiber 应优雅卸载");
    assert!(registry.find("quit").is_none(), "fiber 卸载后命令应自动除名");
    assert!(registry.list().is_empty());
    root.dispose().await.expect("root 应优雅关闭");
}

/// 别名解析：别名命中；且精确名优先——别名不遮蔽任何精确名（设计 §三）。
#[test]
fn 别名解析与精确名优先() {
    let registry = CommandRegistry::new();
    // 命令 A：精确名 x，别名 y；命令 B：精确名 y —— find("y") 必须命中 B
    let _da = register(&registry, "x", &["y"], "别名带y的动作类");
    let _db = register(&registry, "y", &[], "精确名叫y");
    let found = registry.find("y").expect("y 应命中");
    assert_eq!(found.name, "y", "精确名优先：别名不遮蔽精确名");
    assert_eq!(found.description, "精确名叫y");
    // 别名解析正常路径
    let by_alias = registry.find("帮助");
    assert!(by_alias.is_none(), "未注册别名前应 miss");
    drop(_da);
    let registry2 = CommandRegistry::new();
    let _d1 = register(&registry2, "help", &["帮助"], "看命令");
    let found = registry2.find("帮助").expect("别名应命中");
    assert_eq!(found.name, "help");
    assert_eq!(found.kind, CommandKind::Action);
}

/// 目录投影剥离 handler：CommandDescriptor 不含 handler，含全量展示字段。
#[test]
fn 目录投影字段齐全() {
    let registry = CommandRegistry::new();
    let _d = register(&registry, "help", &["帮助", "h"], "看命令");
    let all = registry.list();
    assert_eq!(all.len(), 1);
    let c = &all[0];
    assert_eq!(c.name, "help");
    assert_eq!(c.aliases, vec!["帮助".to_string(), "h".to_string()]);
    assert_eq!(c.description, "看命令");
    assert_eq!(c.kind, CommandKind::Action);
}

/// 未知 token / 空 token 解析为空（解析入口的兜底行为）。
#[test]
fn 未知与空token解析为空() {
    let registry = CommandRegistry::new();
    let _d = register(&registry, "help", &["帮助"], "看命令");
    assert!(registry.find("nope").is_none());
    assert!(registry.find("").is_none(), "空 token 不应命中任何命令");
    // 前缀不算命中（find 是精确/别名解析，前缀过滤是面板职责）
    assert!(registry.find("hel").is_none());
}

/// 分发入口 handler_of：与 find 同解析规则（精确名优先→别名），
/// 未知命令返回 None（未知名不进 handler 不落日志）。
#[test]
fn 分发入口与find同规则() {
    let registry = CommandRegistry::new();
    let _da = register(&registry, "help", &["帮助"], "看命令");
    let _db = register(&registry, "y", &[], "精确名叫y");
    let _dc = register(&registry, "x", &["y"], "别名带y");
    assert!(registry.handler_of("y").is_some(), "精确名 y 可分发（不遮蔽）");
    assert!(registry.handler_of("帮助").is_some(), "别名可分发");
    assert!(registry.handler_of("nope").is_none(), "未知名不可分发");
    assert!(registry.handler_of("").is_none());
}

/// 别名×别名冲突检测（task-17②）：同名 fail-loud 纪律补齐别名面——修复前
/// 仅查主名，别名撞别名按 find 字典序遍历**静默**胜出（实测技能 alias「退出」
/// 可劫持壳命令 /quit 的别名「退出」）。别名×主名两方向不拦：设计解析规则
/// 「别名不遮蔽任何精确名」（精确名优先）定义共存语义（「分发入口与find同
/// 规则」测试即依赖 x 别名 y 与精确名 y 共存），不属遮蔽缝；disposer 除名
/// 后别名占用随条目释放可重注册。
#[tokio::test]
async fn 别名冲突拦截与点名与释放() {
    let registry = CommandRegistry::new();
    let d = register(&registry, "quit", &["退出"], "退出");
    // 新别名 × 既有别名 → Err 点名（唯一静默遮蔽缝，fail-loud 补齐）
    let err = registry
        .register(def("bar", &["退出"], "别名撞别名"))
        .expect_err("别名撞既有别名应 Err");
    assert_eq!(err.kind(), &ErrorKind::ServiceDuplicate, "错误类别应为 ServiceDuplicate");
    assert!(err.to_string().contains("退出"), "点名冲突源: {err}");
    // 既有目录不受冲突注册影响（fail-loud 非半落状态）
    assert!(registry.find("bar").is_none(), "冲突命令不应入表");
    // 别名 × 既有主名：共存合法（精确名优先设计），find 仍归精确名
    let _dfoo = register(&registry, "foo", &["quit"], "别名撞主名共存");
    assert_eq!(
        registry.find("quit").map(|c| c.name).as_deref(),
        Some("quit"),
        "精确名优先：别名撞主名时精确名仍赢"
    );
    // disposer 除名释放占用（await 式，对齐既有「disposer除名」测试形态）：
    // /quit 除名后同名与别名可重注册（占用随条目删除）
    d.dispose().await;
    let _d2 = register(&registry, "quit", &["退出"], "重注册应成功");
    assert!(registry.find("quit").is_some());
}
