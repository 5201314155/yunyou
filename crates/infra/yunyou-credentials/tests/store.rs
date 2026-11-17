//! 宿主托管存储集成测试（从 store.rs 单元测试区迁出，第四条行数拆分纪律）。
//! 行为口径与迁出前一致：往返持久化/环境兜底/空值铁律/600 权限/损坏 fail-loud。

use yunyou_credentials::{CredentialService, CredentialInfo, HostCredentialStore};

mod store_tests {
    use super::*;
    use std::path::PathBuf;

    /// 独立临时目录（测试间互不串扰；用完清理）。
    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunyou-cred-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("临时目录应可创建");
        dir
    }

    /// set → resolve 往返 + describe 事实面 + 重新 open 持久化读回。
    #[tokio::test]
    async fn set_resolve往返与持久化() {
        let path = temp_home("roundtrip").join("credentials.toml");
        let store = HostCredentialStore::open(path.clone()).expect("空表打开");
        store.set("$YUNYOU_TEST_KEY", "sk-test-123").await.expect("set 应成功");
        let handle = store.resolve("$YUNYOU_TEST_KEY").await.expect("resolve 应成功").expect("应有值");
        handle.with_value(|value| assert_eq!(value, "sk-test-123"));
        let info = store.describe("$YUNYOU_TEST_KEY").await.expect("describe 应成功");
        assert_eq!(info, CredentialInfo { configured: true, source: Some("store".into()) });
        // 持久化读回（新实例=模拟重启）
        let reopened = HostCredentialStore::open(path).expect("重启后应可打开");
        let again = reopened.resolve("$YUNYOU_TEST_KEY").await.expect("resolve 应成功").expect("持久化值应可读回");
        again.with_value(|value| assert_eq!(value, "sk-test-123"));
        std::fs::remove_dir_all(temp_home("roundtrip")).ok();
    }

    /// 环境兜底与存储优先：存储有值用存储；存储无值落环境；环境空串视为无值。
    #[tokio::test]
    async fn 环境兜底与存储优先() {
        let path = temp_home("env").join("credentials.toml");
        let store = HostCredentialStore::open(path).expect("打开");
        std::env::set_var("YUNYOU_TEST_ENV_ONLY", "env-value");
        std::env::set_var("YUNYOU_TEST_EMPTY_ENV", "");
        std::env::set_var("YUNYOU_TEST_BOTH", "env-value");
        store.set("$YUNYOU_TEST_BOTH", "store-value").await.expect("set 应成功");
        let only = store.resolve("$YUNYOU_TEST_ENV_ONLY").await.expect("resolve 应成功").expect("环境值应兜底");
        only.with_value(|value| assert_eq!(value, "env-value"));
        let both = store.resolve("$YUNYOU_TEST_BOTH").await.expect("resolve 应成功").expect("存储应优先");
        both.with_value(|value| assert_eq!(value, "store-value"));
        let info = store.describe("$YUNYOU_TEST_ENV_ONLY").await.expect("describe 应成功");
        assert_eq!(info.source.as_deref(), Some("env"), "环境层供给应报 source=env");
        assert!(
            store.resolve("$YUNYOU_TEST_EMPTY_ENV").await.expect("resolve 应成功").is_none(),
            "环境空串应视为未配置（空值=absent 处处一致）"
        );
        std::env::remove_var("YUNYOU_TEST_ENV_ONLY");
        std::env::remove_var("YUNYOU_TEST_EMPTY_ENV");
        std::env::remove_var("YUNYOU_TEST_BOTH");
        std::fs::remove_dir_all(temp_home("env")).ok();
    }

    /// 未配置引用：resolve=None、describe configured=false（官方 describe 语义）。
    #[tokio::test]
    async fn 未配置引用读作absent() {
        let path = temp_home("absent").join("credentials.toml");
        let store = HostCredentialStore::open(path).expect("打开");
        assert!(store.resolve("$YUNYOU_TEST_MISSING").await.expect("resolve 应成功").is_none());
        let info = store.describe("$YUNYOU_TEST_MISSING").await.expect("describe 应成功");
        assert_eq!(info, CredentialInfo { configured: false, source: None });
        std::fs::remove_dir_all(temp_home("absent")).ok();
    }

    /// 空值铁律：set("") 拒绝；存储文件里的空串读作未配置（不冒充已配置密钥）。
    #[tokio::test]
    async fn 空值拒绝与空串absent() {
        let path = temp_home("empty").join("credentials.toml");
        let store = HostCredentialStore::open(path).expect("打开");
        let err = store.set("$YUNYOU_TEST_KEY", "").await.unwrap_err();
        assert!(format!("{err}").contains("拒绝空值"), "set 空值应点名拒绝：{err}");
        // 存储文件手放空串（绕过 set 的拒绝口），验证读取侧同样按未配置处理
        std::fs::write(
            temp_home("empty").join("credentials.toml"),
            "[references]\nYUNYOU_TEST_KEY = \"\"\n",
        )
        .expect("应可写测试文件");
        let reopened = HostCredentialStore::open(temp_home("empty").join("credentials.toml")).expect("打开");
        assert!(reopened.resolve("$YUNYOU_TEST_KEY").await.expect("resolve 应成功").is_none());
        let info = reopened.describe("$YUNYOU_TEST_KEY").await.expect("describe 应成功");
        assert!(!info.configured, "空串存储值必须报未配置（官方铁律）");
        std::fs::remove_dir_all(temp_home("empty")).ok();
    }

    /// 非法引用 fail-loud：resolve/set/describe 全拦（fail-loud 对齐任务书）。
    #[tokio::test]
    async fn 非法引用三面拦截() {
        let path = temp_home("badref").join("credentials.toml");
        let store = HostCredentialStore::open(path).expect("打开");
        for reference in ["sk-plain", "$bad-name", "$9BAD"] {
            assert!(store.resolve(reference).await.is_err(), "resolve({reference}) 应 Err");
            assert!(store.set(reference, "v").await.is_err(), "set({reference}) 应 Err");
            assert!(store.describe(reference).await.is_err(), "describe({reference}) 应 Err");
        }
        std::fs::remove_dir_all(temp_home("badref")).ok();
    }

    /// unset 语义：删除后 absent；缺失引用 unset = no-op（对齐官方）。
    #[tokio::test]
    async fn unset删除与缺失no_op() {
        let path = temp_home("unset").join("credentials.toml");
        let store = HostCredentialStore::open(path).expect("打开");
        store.set("$YUNYOU_TEST_KEY", "v1").await.expect("set 应成功");
        store.unset("$YUNYOU_TEST_KEY").await.expect("unset 应成功");
        assert!(store.resolve("$YUNYOU_TEST_KEY").await.expect("resolve 应成功").is_none());
        store.unset("$YUNYOU_TEST_KEY").await.expect("缺失 unset 应 no-op");
        std::fs::remove_dir_all(temp_home("unset")).ok();
    }

    /// 落盘权限：600（owner 独占，unix；任务书红线落盘侧防线）。
    #[cfg(unix)]
    #[tokio::test]
    async fn 落盘文件600权限() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_home("perm").join("credentials.toml");
        let store = HostCredentialStore::open(path.clone()).expect("打开");
        store.set("$YUNYOU_TEST_KEY", "v").await.expect("set 应成功");
        let mode = std::fs::metadata(&path).expect("文件应存在").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "凭据文件必须 600 权限");
        std::fs::remove_dir_all(temp_home("perm")).ok();
    }

    /// 损坏文件 fail-loud：解析失败必须报错（严禁静默重置=替用户删密钥）。
    #[tokio::test]
    async fn 损坏存储文件报错() {
        let path = temp_home("corrupt").join("credentials.toml");
        std::fs::write(&path, "!!!! 不是 toml !!!=").expect("应可写");
        let result = HostCredentialStore::open(path);
        assert!(result.is_err(), "损坏文件必须 fail-loud");
        std::fs::remove_dir_all(temp_home("corrupt")).ok();
    }

    /// rename 前崩溃残留 .tmp 不卡死写路径（审二轮 P2 回归：create_new 撞名
    /// AlreadyExists 曾使一切 set/unset 永久失败；persist 现前置清理残留）。
    #[tokio::test]
    async fn 残留tmp不卡死写路径() {
        let path = temp_home("stale").join("credentials.toml");
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, "rename 前崩溃的残留垃圾").expect("应可写残留文件");
        let store = HostCredentialStore::open(path).expect("打开");
        store
            .set("$YUNYOU_TEST_KEY", "v")
            .await
            .expect("残留 .tmp 后 set 必须成功（撞名卡死回归）");
        let text =
            std::fs::read_to_string(temp_home("stale").join("credentials.toml")).expect("应可读");
        assert!(text.contains("YUNYOU_TEST_KEY"), "落盘内容应正确：{text}");
        std::fs::remove_dir_all(temp_home("stale")).ok();
    }
}
