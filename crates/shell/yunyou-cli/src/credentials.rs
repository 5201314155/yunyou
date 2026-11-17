//! 凭据接线（S5 收口合入，lead 裁决②＝装配层解析；精确规格＝匠二 2026-11-14）：
//!
//! ①**启动迁移**：存量明文 key 入托管存储＋文本级定点改写（匠二 migrate 设计）；
//!   迁移与解析面**同批原子合入**（匠二硬要求：迁移上线而解析面缺席＝下次启动
//!   401 断裂窗口）。
//! ②**llm 段引用解析**：`llm.api_key` 为 `$NAME` 引用时经宿主持有的
//!   [`HostCredentialStore`] 解析为真值再透传（对标官方 loader $ 插值语义——
//!   宿主装配时解引用，provider 零感知，llm-openai 不为静态配置引用引入对
//!   credentials 的 inject 耦合；一期只解析 llm.api_key 一个槽，通用 $ 插值引擎
//!   二期随 loader 需求再议，勿做万能插值）。宿主消费自己的基础设施不经服务总线
//!   （服务面是给插件/面板的）——装配层即 llm-openai lib.rs:14 既有口径的
//!   「装配层写入配置」，也是 SecretHandle 设计里的「宿主侧消费点」。
//!
//! 防泄漏语义留痕（方案 §7 风险 7）：解析把值从托管存储放进**内存段值**（插件
//! config 入口，不落盘/不打印/不进会话/不进日志）——`SecretHandle::with_value`
//! 闭包内仅构造内存值，属「宿主侧消费点流向请求头」的必经路径；序列化面泄漏由
//! schema `role=secret` 标注（redact/settings 投影）拦截。

use crate::AppError;
use yunyou_credentials::{is_reference, migrate_plaintext_key, reference_name, CredentialService, HostCredentialStore};
use serde_json::Value;
use std::io::Write as _;
use std::path::Path;

/// 打开宿主持有的托管存储（迁移与解析共用一份实例；路径取 credentials 段
/// 补值后的 store_path——用户可自定义，非写死 home）。文件不存在＝空表。
pub(crate) fn open_store(store_path: &Path) -> Result<HostCredentialStore, AppError> {
    HostCredentialStore::open(store_path.to_path_buf())
        .map_err(|err| AppError::Assemble(format!("凭据存储 {} 打开失败：{err}", store_path.display())))
}

/// 启动迁移（装配序列内、credentials 插件装载前调用——store 为 open 时快照
/// 内存表，插件实例必须读到迁移后的文件）。无配置文件（回退默认态）＝无操作；
/// 明文 key → 入托管存储＋配置文件定点改写写回（600 权限，tmp+rename 原子写）；
/// 日志只记迁移字段名，**绝不记值**（「key 零入日志」）。
pub(crate) fn run_startup_migration(
    store: &HostCredentialStore,
    config_path: &Path,
) -> Result<(), AppError> {
    let config_text = match std::fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(AppError::Io(err)),
    };
    let report = migrate_plaintext_key(store, &config_text)
        .map_err(|err| AppError::Assemble(format!("存量明文 key 迁移失败：{err}")))?;
    if let Some(rewritten) = &report.config_text {
        write_config_atomic_600(config_path, rewritten)?;
        eprintln!(
            "[启动] 已把配置中的明文 key 迁入凭据存储 {}（配置文件已改写为凭据引用，迁移字段：{:?}）",
            config_path.display(),
            report.migrated
        );
    }
    Ok(())
}

/// llm 段引用解析（匠二精确规格 3）：`api_key` 为引用 → 经宿主 store 解析替换，
/// 未配置 → fail-loud 报人话+两条修复路径（不静默空串——静默＝请求 401 难排查）；
/// 非引用（明文/env 覆盖值）原样直通（迁移期兼容语义）。
pub(crate) async fn resolve_llm_api_key(
    cfg: &mut Value,
    api_key: &str,
    store: &HostCredentialStore,
) -> Result<(), AppError> {
    if !is_reference(api_key) {
        return Ok(());
    }
    match store.resolve(api_key).await {
        Ok(Some(handle)) => {
            let value = handle.with_value(|v| v.to_string());
            cfg["api_key"] = Value::String(value);
            Ok(())
        }
        Ok(None) => {
            // 人话指路（task-34①）：点名引用名 + 给两条可执行修复路径，
            // 不再用「宿主托管存储/进程环境」等内部词汇。
            let name = reference_name(api_key).unwrap_or(api_key);
            Err(AppError::Assemble(format!(
                "未找到 API key：凭据引用 {api_key} 在凭据存储与环境变量中均无值。\
                 请设置环境变量后重新运行（如 export {name}=\"sk-…\"），\
                 或在配置文件直接写明文 key（下次启动会自动迁入凭据存储）"
            )))
        }
        Err(err) => Err(AppError::Assemble(format!(
            "API key 引用 {api_key} 读取失败：{err}"
        ))),
    }
}

/// 配置文件原子写回（tmp + rename；对齐 store.rs `write_secret_file` 同款：
/// unix 下 OpenOptions mode(0o600) **创建即锁权限**——评审②：`File::create`
/// 以 0644 起步、先写明文后 chmod 存在崩溃残留明文窗口，创建时设权消除之）。
fn write_config_atomic_600(path: &Path, text: &str) -> Result<(), AppError> {
    let tmp = path.with_extension("toml.mig-tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(AppError::Io)?;
        file.write_all(text.as_bytes()).map_err(AppError::Io)?;
        file.sync_all().map_err(AppError::Io)?;
    }
    #[cfg(not(unix))]
    {
        let mut file = std::fs::File::create(&tmp).map_err(AppError::Io)?;
        file.write_all(text.as_bytes()).map_err(AppError::Io)?;
        file.sync_all().map_err(AppError::Io)?;
    }
    std::fs::rename(&tmp, path).map_err(AppError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yunyou_credentials::reference_name;

    /// 解析面（裁决②单槽）：$REF → store 真值替换；未配置 → fail-loud 明确报；
    /// 非引用（明文）直通零动作。用真实 HostCredentialStore 全链（SecretHandle
    /// 构造口为 crate 内私有，mock 不可行——真实存储更贴运行态）。
    #[tokio::test]
    async fn 解析面_引用替换_未配置报错_明文直通() {
        let dir = std::env::temp_dir().join(format!("yunyou-cli-cred-resolve-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("应能建临时目录");
        let store = open_store(&dir.join("credentials.toml")).expect("应能开存储");
        // 未配置引用 → fail-loud（不静默空串）
        let mut cfg = serde_json::json!({ "api_key": "$YUNYOU_LLM_API_KEY" });
        let err = resolve_llm_api_key(&mut cfg, "$YUNYOU_LLM_API_KEY", &store)
            .await
            .expect_err("未配置引用应报错");
        assert!(
            err.to_string().contains("未找到 API key")
                && err.to_string().contains("export YUNYOU_LLM_API_KEY"),
            "报错应人话点名引用并给修复路径：{err}"
        );
        // set 入托管 → resolve 替换为真值
        store
            .set("$YUNYOU_LLM_API_KEY", "sk-test-真值")
            .await
            .expect("写入托管存储应成功");
        let mut cfg = serde_json::json!({ "api_key": "$YUNYOU_LLM_API_KEY" });
        resolve_llm_api_key(&mut cfg, "$YUNYOU_LLM_API_KEY", &store)
            .await
            .expect("已配置引用应解析成功");
        assert_eq!(cfg["api_key"], "sk-test-真值", "段值应替换为托管存储真值");
        // 明文直通零动作（迁移期兼容）
        let mut cfg = serde_json::json!({ "api_key": "sk-plaintext" });
        resolve_llm_api_key(&mut cfg, "sk-plaintext", &store).await.expect("明文应直通");
        assert_eq!(cfg["api_key"], "sk-plaintext", "明文段值不应被改动");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 启动迁移：明文 key 入托管＋文本定点改写（注释字节级保留）＋600 权限写回；
    /// 幂等重跑无操作；无配置文件无操作。
    #[tokio::test]
    async fn 迁移_明文入托管_文本改写_幂等() {
        let dir = std::env::temp_dir().join(format!("yunyou-cli-cred-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("应能建临时目录");
        let config_path = dir.join("yunyou.toml");
        std::fs::write(
            &config_path,
            "# 用户注释必须保留\n[llm]\nbase_url = \"http://127.0.0.1:1\"\napi_key = \"sk-明文-abc\"\n",
        )
        .expect("应能写配置");
        let store = open_store(&dir.join("credentials.toml")).expect("应能开存储");
        run_startup_migration(&store, &config_path).expect("迁移应成功");
        // 配置文件：api_key 行改为引用，注释与其他行字节级保留
        let rewritten = std::fs::read_to_string(&config_path).expect("应能读回");
        assert!(rewritten.contains("api_key = \"$YUNYOU_LLM_API_KEY\""), "应改写为引用：{rewritten}");
        assert!(rewritten.contains("# 用户注释必须保留"), "注释应字节级保留：{rewritten}");
        assert!(rewritten.contains("base_url = \"http://127.0.0.1:1\""), "其他行应保留：{rewritten}");
        // 托管存储：真值入库（经 resolve 验证，不读文件明文比对）
        let resolved = store
            .resolve("$YUNYOU_LLM_API_KEY")
            .await
            .expect("resolve 不应报错")
            .expect("迁移后应已配置");
        assert_eq!(resolved.with_value(|v| v.to_string()), "sk-明文-abc", "托管值应为原明文");
        // 幂等：重跑无操作（文件不再变化）
        let before = rewritten.clone();
        run_startup_migration(&store, &config_path).expect("幂等重跑应成功");
        let after = std::fs::read_to_string(&config_path).expect("应能读回");
        assert_eq!(before, after, "幂等重跑不应改写文件");
        // 无配置文件：无操作
        run_startup_migration(&store, &dir.join("不存在.toml")).expect("无文件应无操作");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 引用语法守门：解析只认 `$NAME` 形态（reference 模块同构），明文与空值不误触。
    #[test]
    fn 引用判定_形态边界() {
        assert!(is_reference("$YUNYOU_LLM_API_KEY"));
        assert_eq!(reference_name("$YUNYOU_LLM_API_KEY"), Some("YUNYOU_LLM_API_KEY"));
        assert!(!is_reference("sk-plaintext"), "明文不是引用");
        assert!(!is_reference(""), "空值不是引用（空值=未配置，官方铁律）");
        assert!(!is_reference("$"), "裸 $ 不是引用");
    }
}
