//! MessageHub（契约 message.v1）T1 实现批测试矩阵（融合终版 §5.3 十二例）+观测面补验。
//!
//! 身份路径全部走 Context 身份入口（方案甲：私有 task_local 运载）；直调 trait
//! 对象的用例专测「无运载即拒」的防伪造语义（④⑤⑪）。观测通知为锁外 spawn，
//! 存在性断言一律轮询等待（wait_for），不做时序硬假设。

use std::sync::{Arc, Mutex};
use std::time::Duration;
use yunyou::manifest::validate_manifest_static;
use yunyou::{
    Address, ConfigValue, Context, DeliveredMessage, DeliveryStatus, ErrorKind, FiberHandle,
    FiberOutput, HistoryEntry, HistoryQuery, Listener, ListenerOutcome, MessageHandler, MessageHub,
    MessageKind, OutgoingMessage, Plugin, PluginLevel, PluginManifest, Result, SentReceipt,
    ServiceId,
};
use yunyou_util::BoxFut;

type Slot<T> = Arc<Mutex<Option<T>>>;

fn slot<T>() -> Slot<T> {
    Arc::new(Mutex::new(None))
}

fn take<T>(slot: &Slot<T>) -> T {
    slot.lock().unwrap().take().expect("setup 结果未落盘")
}

fn msg(body: &str) -> OutgoingMessage {
    OutgoingMessage { kind: MessageKind::Text, body: body.to_string() }
}

fn member(name: &str) -> Address {
    Address::Member(name.to_string())
}

/// 闭包插件：setup 内在成员 fiber 的 ctx 上执行一步（身份操作的真实调用形态）。
/// step 存 Mutex<Option<F>>：FnOnce 无法从 Arc<Self> 移出，装载时 take 一次。
struct StepPlugin<F> {
    name: String,
    step: Mutex<Option<F>>,
}

impl<F> Plugin for StepPlugin<F>
where
    F: FnOnce(&Context) -> Result<()> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn setup(
        self: Arc<Self>,
        ctx: Context,
        _config: ConfigValue,
    ) -> BoxFut<'static, Result<FiberOutput>> {
        Box::pin(async move {
            let step = self.step.lock().unwrap().take().expect("step 只执行一次");
            step(&ctx)?;
            Ok(FiberOutput)
        })
    }
}

/// 在独立插件 fiber（成员 fiber）内执行一步并取回其返回值；同时返回 fiber 句柄，
/// 供降级/续投/订阅摘除类用例处置（fiber 死亡=dispose）。断言一律在测试体内做，
/// step 内只产出数据——防跨任务 panic 被 tokio 吞掉造成假绿。
async fn run_member_step<T, F>(ctx: &Context, fiber_name: &str, step: F) -> (FiberHandle, Result<T>)
where
    T: Send + 'static,
    F: FnOnce(&Context) -> Result<T> + Send + Sync + 'static,
{
    let out: Slot<Result<T>> = slot();
    let out_for_step = out.clone();
    let handle = ctx.plugin(
        StepPlugin {
            name: fiber_name.to_string(),
            step: Mutex::new(Some(move |member_ctx: &Context| {
                let result = step(member_ctx);
                *out_for_step.lock().unwrap() = Some(result);
                Ok(())
            })),
        },
        serde_json::json!({}),
    );
    handle.await_ready().await.expect("插件 fiber 应装载成功");
    let result = take(&out);
    (handle, result)
}

/// ctx.get 取用范式（规格.md 消费方式）：内核 root 已装配，直接可得。
fn hub_of(ctx: &Context) -> Arc<dyn MessageHub> {
    ctx.get::<dyn MessageHub>(ServiceId::of::<dyn MessageHub>())
        .expect("内核 root 已装配 MessageHub")
}

/// 轮询等待（锁外 spawn 的观测通知；5ms×400≈2s 上限）。
async fn wait_for(cond: impl Fn() -> bool) {
    for _ in 0..400 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("等待条件超时（约 2s）");
}

/// ①重名 open_mailbox fail-loud（任何态：Pending 与已认领 Online 均拒）。
#[tokio::test]
async fn 例一_重名开箱任何态均拒() {
    let ctx = Context::new();
    ctx.open_mailbox("alice").expect("首开应成功");
    assert_eq!(ctx.open_mailbox("alice").unwrap_err().kind(), &ErrorKind::HubDuplicateMember);
    let (_, joined) =
        run_member_step(&ctx, "alice-fiber", |c| c.register_member("alice").map(|_| ())).await;
    joined.expect("Pending 认领应成功");
    assert_eq!(ctx.open_mailbox("alice").unwrap_err().kind(), &ErrorKind::HubDuplicateMember);
}

/// ②空白名 fail-loud（空名=永不可达地址，注册时堵；register 同律）。
#[tokio::test]
async fn 例二_空白名即拒() {
    let ctx = Context::new();
    for blank in ["", "   ", "\t\n"] {
        assert_eq!(
            ctx.open_mailbox(blank).unwrap_err().kind(),
            &ErrorKind::HubInvalidName,
            "open_mailbox({blank:?})"
        );
        assert_eq!(
            ctx.register_member(blank).unwrap_err().kind(),
            &ErrorKind::HubInvalidName,
            "register({blank:?})"
        );
    }
}

/// ②b 名字白名单（task-18，云垒实测票）：成员名流向会话文件路径
///（teammate-{name}.jsonl），穿越类按字符集根除——路径分量/空白/大写/超长
/// 全拒（HubInvalidName），合法 kebab-case 全过。open_mailbox 与 register
/// 双入口同律（validate_member_name 单一源）。
#[tokio::test]
async fn 例二b_名字白名单穿越类全拒() {
    let ctx = Context::new();
    // 穿越探针三形态+同族变体（白名单下 / 与 . 整体落不进合法字符集）
    for bad in [
        "../evil",        // 相对穿越
        "/etc/evil",      // 绝对路径
        "..",             // 裸父目录
        "a/b",            // 子路径形态
        "teammate-../x",  // 前缀伪装
        "x/../../y",      // 多级回溯
        "..%2f",          // URL 编码穿越（% 不在白名单=词法层拒；云垒复核建议锁死防「白名单已够」幻觉）
        "%2e%2e/",        // URL 编码父目录（同上）
        "a.b",            // 点号（文件扩展面）
        "all",            // 保留广播地址（task-30 打回-轻①：注册即拦防 to=all 误投）
        "Bad-Name",       // 大写（kebab-case 惯例外）
        "含中文",          // 非 ASCII
        "has space",      // 空白字符
        &format!("a{}", "x".repeat(64)), // 65 字节超限
    ] {
        assert_eq!(
            ctx.open_mailbox(bad).unwrap_err().kind(),
            &ErrorKind::HubInvalidName,
            "open_mailbox({bad:?}) 应拒"
        );
        assert_eq!(
            ctx.register_member(bad).unwrap_err().kind(),
            &ErrorKind::HubInvalidName,
            "register({bad:?}) 应拒"
        );
    }
    // 合法名字全过（kebab-case 惯例字符集内：小写/数字/中划线/下划线，恰 64）
    for good in ["alice", "code-reviewer", "team_lead2", "a1", "yun-lei-a27", &"x".repeat(64)] {
        ctx.open_mailbox(good)
            .unwrap_or_else(|_| panic!("合法名 {good:?} 应通过"));
    }
}

/// ③register 已 Online 名 fail-loud（他人绑定=路由欺骗；同 fiber 已绑他名同拒）。
#[tokio::test]
async fn 例三_重复绑定即拒() {
    let ctx = Context::new();
    let (_, first) =
        run_member_step(&ctx, "fiber-a", |c| c.register_member("alice").map(|_| ())).await;
    first.expect("首绑应成功");
    let (_, second) =
        run_member_step(&ctx, "fiber-b", |c| c.register_member("alice").map(|_| ())).await;
    assert_eq!(second.unwrap_err().kind(), &ErrorKind::HubDuplicateMember);
    // 一 fiber 一成员名（语义加严，云枢 retro 项③）
    let (_, third) = run_member_step(&ctx, "fiber-c", |c| {
        c.register_member("bob")?;
        c.register_member("carol").map(|_| ())
    })
    .await;
    assert_eq!(third.unwrap_err().kind(), &ErrorKind::HubDuplicateMember);
}

/// ④无 fiber 注册/订阅报 EffectInactive（直调 trait 对象=无运载）。
#[tokio::test]
async fn 例四_无fiber注册订阅即拒() {
    let ctx = Context::new();
    let hub = hub_of(&ctx);
    assert_eq!(hub.register("solo").unwrap_err().kind(), &ErrorKind::EffectInactive);
    assert_eq!(hub.open_mailbox("solo").unwrap_err().kind(), &ErrorKind::EffectInactive);
    let handler: MessageHandler =
        Arc::new(|_msg: &DeliveredMessage| Box::pin(async { Ok(()) }));
    assert_eq!(hub.subscribe(handler).unwrap_err().kind(), &ErrorKind::EffectInactive);
}

/// ⑤send 未绑定成员 fiber=HubUnknownMember（防伪造拦截：无身份者发不出）。
#[tokio::test]
async fn 例五_无身份发送即拒() {
    let ctx = Context::new();
    let err = ctx.hub_send(member("lead"), msg("hi")).unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::HubUnknownMember);
    let hub = hub_of(&ctx);
    let err = hub.send(Address::All, msg("hi")).unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::HubUnknownMember);
}

/// ⑥send 未在册名=HubUnknownMember（typo 不静默吞）。
#[tokio::test]
async fn 例六_投未在册名即拒() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 自发上线");
    let err = ctx.hub_send(member("ghost"), msg("在吗")).unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::HubUnknownMember);
}

/// ⑦spawn 窗口缓冲→注册→首次 drain 取净（离线补投）。
#[tokio::test]
async fn 例七_补投全链() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    ctx.open_mailbox("alice").expect("预注册 spawn 窗口信箱");
    let receipt = ctx.hub_send(member("alice"), msg("开工吧")).expect("缓冲受理");
    assert_eq!((receipt.recipients, receipt.buffered), (1, 1));
    let (_, drained) = run_member_step(&ctx, "alice-fiber", |c| {
        c.register_member("alice")?;
        c.drain_inbox()
    })
    .await;
    let drained = drained.expect("补投链应成功");
    assert_eq!(drained.len(), 1);
    assert_eq!(drained[0].from, "lead");
    assert_eq!(drained[0].body, "开工吧");
}

/// ⑧fiber 死亡降级 Pending→重注册→残留消息续投（信箱与未取净消息保留）。
#[tokio::test]
async fn 例八_降级续投() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    let (bob_fiber, first) =
        run_member_step(&ctx, "bob-fiber", |c| c.register_member("bob").map(|_| ())).await;
    first.expect("bob 上线应成功");
    for body in ["一", "二"] {
        let receipt = ctx.hub_send(member("bob"), msg(body)).expect("在线受理");
        assert_eq!(receipt.buffered, 0);
    }
    bob_fiber.dispose().await.expect("处置 bob fiber");
    let bob =
        hub_of(&ctx).members().into_iter().find(|m| m.name == "bob").expect("bob 应在册");
    assert!(!bob.online);
    assert!(bob.fiber.is_none());
    let receipt = ctx.hub_send(member("bob"), msg("三")).expect("死亡后缓冲受理");
    assert_eq!((receipt.recipients, receipt.buffered), (1, 1));
    let (_, drained) = run_member_step(&ctx, "bob-fiber-2", |c| {
        c.register_member("bob")?;
        c.drain_inbox()
    })
    .await;
    let drained = drained.expect("重注册应成功");
    let bodies: Vec<&str> = drained.iter().map(|m| m.body.as_str()).collect();
    assert_eq!(bodies, ["一", "二", "三"]);
}

/// ⑨Pending 名 send 回执计 buffered+留痕 outcomes=Buffered（真账）。
#[tokio::test]
async fn 例九_缓冲回执与留痕真账() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    ctx.open_mailbox("carol").expect("预注册");
    let receipt = ctx.hub_send(member("carol"), msg("在吗")).expect("缓冲受理");
    assert_eq!((receipt.recipients, receipt.buffered), (1, 1));
    let history = hub_of(&ctx).history(HistoryQuery { limit: 10 }).expect("查询");
    let entry = history.last().expect("留痕应有此条");
    assert_eq!(entry.from, "lead");
    assert_eq!(entry.to, "carol");
    assert_eq!(entry.outcomes, vec![("carol".to_string(), DeliveryStatus::Buffered)]);
}

/// ⑩广播 All 不含 from+零成员 fail-loud（task-30① lead 预裁翻转原「空集
/// 照样 Ok」口径：零成员广播=「发出去了没人收到」的假成功，改人话 Err）。
#[tokio::test]
async fn 例十_广播与空集() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    let err = ctx
        .hub_send(Address::All, msg("大家好"))
        .expect_err("零成员广播应人话拒绝（task-30① 翻转）");
    assert!(err.message().contains("无在册成员"), "人话点名零收件人: {err}");
    let (dave, joined) =
        run_member_step(&ctx, "dave-fiber", |c| c.register_member("dave").map(|_| ())).await;
    joined.expect("dave 上线");
    dave.dispose().await.expect("处置（让 dave 可在新 fiber 内 drain）");
    let receipt = ctx.hub_send(Address::All, msg("集合")).expect("广播");
    assert_eq!(receipt.recipients, 1);
    assert!(ctx.drain_inbox().expect("lead drain").is_empty(), "广播不含 from 自己");
    let (_, drained) = run_member_step(&ctx, "dave-fiber-2", |c| {
        c.register_member("dave")?;
        c.drain_inbox()
    })
    .await;
    let drained = drained.expect("dave 应收到广播");
    assert_eq!(drained.len(), 1);
    assert_eq!((drained[0].from.as_str(), drained[0].body.as_str()), ("lead", "集合"));
}

/// ⑪drain 非成员 fiber=HubUnknownMember（越权读拦截）。
#[tokio::test]
async fn 例十一_越权读取即拒() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    let (_, unauthorized) = run_member_step(&ctx, "stranger-fiber", |c| c.drain_inbox()).await;
    assert_eq!(unauthorized.unwrap_err().kind(), &ErrorKind::HubUnknownMember);
    assert_eq!(hub_of(&ctx).drain().unwrap_err().kind(), &ErrorKind::HubUnknownMember);
}

/// ⑫留痕环形 1000 丢最旧+manifest contracts=["message.v1"] 过静态校验。
#[tokio::test]
async fn 例十二_环形上限与契约登记() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    for i in 0..1001 {
        let receipt = ctx.hub_send(member("lead"), msg(&format!("m{i}"))).expect("自发自收");
        assert_eq!(receipt.recipients, 1);
    }
    let history = hub_of(&ctx).history(HistoryQuery { limit: 1000 }).expect("查询");
    assert_eq!(history.len(), 1000);
    assert_eq!(history[0].log_id, 2, "最旧一条（log 1）应被丢");
    assert_eq!(history.last().unwrap().log_id, 1001);
    let manifest = PluginManifest {
        id: "yunyou-team".into(),
        version: "0.1.0".into(),
        level: PluginLevel::Bundled,
        contracts: vec!["message.v1".into()],
        provides: vec![],
        inject: vec![],
        ui: vec![],
        auto_enable: true,
    };
    assert!(validate_manifest_static(&manifest).is_empty());
}

/// 补验：冻结数据形态 serde 往返（规格.md 兼容纪律=二期 IPC schema 种子的最低承诺）。
#[test]
fn 例十三_冻结形态serde往返() {
    let receipt = SentReceipt { log_id: 7, recipients: 2, buffered: 1 };
    let json = serde_json::to_string(&receipt).unwrap();
    assert_eq!(json, r#"{"log_id":7,"recipients":2,"buffered":1}"#);
    assert_eq!(serde_json::from_str::<SentReceipt>(&json).unwrap(), receipt);
    let member = Address::Member("lead".into());
    assert_eq!(
        serde_json::from_str::<Address>(&serde_json::to_string(&member).unwrap()).unwrap(),
        member
    );
    assert_eq!(serde_json::from_str::<Address>("\"all\"").unwrap(), Address::All);
    assert_eq!(serde_json::from_str::<MessageKind>("\"report\"").unwrap(), MessageKind::Report);
    assert_eq!(
        serde_json::from_str::<DeliveryStatus>("\"buffered\"").unwrap(),
        DeliveryStatus::Buffered
    );
    let pending = yunyou::MemberInfo { name: "p".into(), online: false, fiber: None };
    let json = serde_json::to_string(&pending).unwrap();
    assert!(!json.contains("fiber"), "None 应被 skip_serializing_if 省略: {json}");
    assert_eq!(serde_json::from_str::<yunyou::MemberInfo>(&json).unwrap(), pending);
    let entry = HistoryEntry {
        log_id: 1,
        at_ms: 42,
        from: "a".into(),
        to: "All".into(),
        kind: MessageKind::System,
        body: "b".into(),
        outcomes: vec![("b".into(), DeliveryStatus::Delivered)],
    };
    let json = serde_json::to_string(&entry).unwrap();
    assert_eq!(serde_json::from_str::<HistoryEntry>(&json).unwrap(), entry);
    // 兼容纪律：outcomes 缺失 → serde(default) 空账
    let degraded = r#"{"log_id":1,"at_ms":0,"from":"a","to":"All","kind":"text","body":""}"#;
    assert!(serde_json::from_str::<HistoryEntry>(degraded).unwrap().outcomes.is_empty());
}

/// 补验：观测键 message.v1/watch 六位序 payload（冻结面 #9）+缓冲状态位。
#[tokio::test]
async fn 例十四_watch观测照发() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    ctx.open_mailbox("carol").expect("预注册 Pending 名");
    let seen: Arc<Mutex<Vec<Vec<serde_json::Value>>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_for_listener = seen.clone();
    let listener: Listener = Arc::new(move |args: &[serde_json::Value]| {
        let seen = seen_for_listener.clone();
        let args = args.to_vec();
        Box::pin(async move {
            seen.lock().unwrap().push(args);
            Ok(ListenerOutcome::Continue)
        })
    });
    ctx.on(yunyou::hub::WATCH_KEY, listener).expect("注册观测监听");
    ctx.hub_send(member("carol"), msg("在吗")).expect("缓冲受理");
    wait_for(|| !seen.lock().unwrap().is_empty()).await;
    let payload = seen.lock().unwrap()[0].clone();
    assert_eq!(payload.len(), 6, "六位序冻结：[log_id, from, to, kind, body, 状态]");
    assert_eq!(payload[1], serde_json::json!("lead"));
    assert_eq!(payload[2], serde_json::json!("carol"));
    assert_eq!(payload[3], serde_json::json!("text"));
    assert_eq!(payload[5], serde_json::json!("buffered"), "含缓冲目标即标 buffered");
}

/// 补验：subscribe 观测镜像（推，尽力而为）——成员 fiber 订阅后收自己入站镜像；
/// handler 失败不上抛的兜底由 demote/降级机制保证（观测随 fiber 终结摘除）。
#[tokio::test]
async fn 例十五_订阅入站镜像() {
    let ctx = Context::new();
    ctx.register_member("lead").expect("lead 上线");
    let seen: Arc<Mutex<Vec<DeliveredMessage>>> = Arc::new(Mutex::new(Vec::new()));
    let seen_for_sub = seen.clone();
    let (dave_fiber, joined) = run_member_step(&ctx, "dave-fiber", move |c| {
        c.register_member("dave")?;
        let handler: MessageHandler = Arc::new(move |message: &DeliveredMessage| {
            let seen = seen_for_sub.clone();
            let message = message.clone();
            Box::pin(async move {
                seen.lock().unwrap().push(message);
                Ok(())
            })
        });
        c.subscribe_messages(handler).map(|_| ())
    })
    .await;
    joined.expect("订阅链应成功");
    ctx.hub_send(member("dave"), msg("镜像我")).expect("投递");
    wait_for(|| !seen.lock().unwrap().is_empty()).await;
    let mirror = seen.lock().unwrap()[0].clone();
    assert_eq!((mirror.from.as_str(), mirror.body.as_str()), ("lead", "镜像我"));
    dave_fiber.dispose().await.expect("处置（降级自动摘除观测）");
}
