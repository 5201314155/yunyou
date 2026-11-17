//! 宿主托管凭据存储：`<yunyou-home>/credentials.toml`（600 权限）＋进程环境兜底。
//!
//! 解析顺序对齐官方 provider 语义（packages/credentials index.ts 模块注释）：
//! 托管存储优先，ambient（进程环境变量）兜底；**空值 = 未配置，处处一致**
//! （resolve 跳过、describe 报 false——空串永不冒充已配置密钥）。
//!
//! 落盘形态（toml crate 同源序列化，600 权限原子写：临时文件 + rename）：
//! ```toml
//! [references]
//! YUNYOU_LLM_API_KEY = "..."
//! ```

use crate::reference::{reference_name, validate_reference};
use crate::{CredentialInfo, CredentialService, SecretHandle};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use yunyou::{CordisError, ErrorKind, Result};
use yunyou_util::BoxFut;

/// 存储文件结构（`[references]` 表）。
#[derive(Serialize, Deserialize, Default)]
struct StoreFile {
    #[serde(default)]
    references: HashMap<String, String>,
}

/// 宿主托管存储：进程内一份内存表 + 低频落盘（[`CredentialService`] 的默认实现）。
///
/// 并发模型：`entries` 用 `std::sync::Mutex`（临界区纯内存，resolve 无 await）；
/// `set`/`unset` 的落盘在锁内做——凭据写入是低频管理面操作（迁移/管理命令），
/// 锁内串行化避免双写交错；高频路径（resolve）锁内只做 HashMap 读。
pub struct HostCredentialStore {
    path: PathBuf,
    entries: Mutex<HashMap<String, String>>,
}

impl HostCredentialStore {
    /// 打开（或创建）托管存储：文件不存在 = 空表；存在但损坏 = Err（fail-loud，
    /// 损坏的凭据文件严禁静默重置——那等于替用户删密钥）。
    pub fn open(path: PathBuf) -> Result<Self> {
        let entries = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let file: StoreFile = toml::from_str(&text).map_err(|err| {
                    CordisError::new(
                        ErrorKind::Internal,
                        // F-2：脱敏摘要（message+行号），不透传含明文 key 的源码行
                        format!("凭据存储 {} 解析失败：{}", path.display(), crate::toml_error_summary(&text, &err)),
                    )
                })?;
                file.references
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(err) => {
                return Err(CordisError::new(
                    ErrorKind::Internal,
                    format!("凭据存储 {} 读取失败：{err}", path.display()),
                ))
            }
        };
        Ok(Self { path, entries: Mutex::new(entries) })
    }

    /// 锁内落盘（临时文件 600 → rename 原子替换；见类型级并发注记）。
    fn persist(&self, entries: &HashMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(persist_error(&self.path))?;
        }
        let text = toml::to_string(&StoreFile { references: entries.clone() })
            .map_err(|err| CordisError::new(ErrorKind::Internal, format!("凭据存储序列化失败：{err}")))?;
        let tmp = self.path.with_extension("toml.tmp");
        // 原子写前置清理：rename 前崩溃残留的 .tmp 会让下方 create_new 永久
        // AlreadyExists——卡死一切写路径（审二轮 P2）。忽略 NotFound；双写竞态
        // 不在一期威胁模型（单进程单实例，写侧仅启动期迁移）。
        let _ = std::fs::remove_file(&tmp);
        write_secret_file(&tmp, &text).map_err(persist_error(&self.path))?;
        std::fs::rename(&tmp, &self.path).map_err(persist_error(&self.path))?;
        Ok(())
    }

    /// 锁内三层查找：存储非空值 → 环境非空值 → 无。返回 (值, 供给层)。
    fn lookup(&self, name: &str) -> Option<(String, &'static str)> {
        let entries = self.entries.lock().expect("凭据表锁");
        if let Some(value) = entries.get(name) {
            // 官方铁律：空存储值 = 未配置，继续落环境层，空串永不冒充密钥
            if !value.is_empty() {
                return Some((value.clone(), "store"));
            }
        }
        drop(entries);
        match std::env::var(name) {
            Ok(value) if !value.is_empty() => Some((value, "env")),
            _ => None,
        }
    }

    /// 同步写入口（迁移器 [`crate::migrate`] 用）：校验+插入+落盘，与 trait `set`
    /// 共享同一逻辑（凭据写入是纯同步路径，async 包装只为 trait 契约形状）。
    pub(crate) fn set_sync(&self, reference: &str, value: &str) -> Result<()> {
        validate_reference(reference).map_err(|msg| CordisError::new(ErrorKind::ConfigValidation, msg))?;
        if value.is_empty() {
            // 官方铁律：empty values are rejected, use unset
            return Err(CordisError::new(
                ErrorKind::ConfigValidation,
                "凭据 set 拒绝空值（空值 = 未配置；清除请用 unset）",
            ));
        }
        let name = reference_name(reference).expect("validate 通过后必有名字段");
        let mut entries = self.entries.lock().expect("凭据表锁");
        entries.insert(name.to_string(), value.to_string());
        self.persist(&entries)
    }
}

/// 落盘 I/O 错误的统一包装（含路径，便于定位）。
fn persist_error(path: &std::path::Path) -> impl Fn(std::io::Error) -> CordisError + '_ {
    move |err| {
        CordisError::new(
            ErrorKind::Internal,
            format!("凭据存储 {} 落盘失败：{err}", path.display()),
        )
    }
}

/// 密钥文件专用写入：unix 下 600 权限（owner 独占；任务书红线「key 零入 git 零入
/// 日志」的落盘侧防线——对齐台账 3B yunyou.toml 600 先例）。
fn write_secret_file(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
        file.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(path, text)
}

impl CredentialService for HostCredentialStore {
    // 凭据读写是纯同步路径（锁 + 小文件 IO，无 await 点）；trait 的 BoxFut<'static>
    // 形状为服务面统一惯例（session/llm 同款），故实现为「同步求值 → owned 结果
    // 装 future」——future 体不借用 self/入参，'static 成立。
    fn resolve(&self, reference: &str) -> BoxFut<'static, Result<Option<SecretHandle>>> {
        let result = self.resolve_sync(reference);
        Box::pin(async move { result })
    }

    fn describe(&self, reference: &str) -> BoxFut<'static, Result<CredentialInfo>> {
        let result = self.describe_sync(reference);
        Box::pin(async move { result })
    }

    fn set(&self, reference: &str, value: &str) -> BoxFut<'static, Result<()>> {
        let result = self.set_sync(reference, value);
        Box::pin(async move { result })
    }

    fn unset(&self, reference: &str) -> BoxFut<'static, Result<()>> {
        let result = self.unset_sync(reference);
        Box::pin(async move { result })
    }
}

impl HostCredentialStore {
    /// 同步解析（[`CredentialService::resolve`] 的实体；校验+三层查找）。
    fn resolve_sync(&self, reference: &str) -> Result<Option<SecretHandle>> {
        validate_reference(reference).map_err(|msg| CordisError::new(ErrorKind::ConfigValidation, msg))?;
        let name = reference_name(reference).expect("validate 通过后必有名字段");
        Ok(self.lookup(name).map(|(value, _)| SecretHandle::new(value)))
    }

    /// 同步描述（[`CredentialService::describe`] 的实体；只报事实面永不含值）。
    fn describe_sync(&self, reference: &str) -> Result<CredentialInfo> {
        validate_reference(reference).map_err(|msg| CordisError::new(ErrorKind::ConfigValidation, msg))?;
        let name = reference_name(reference).expect("validate 通过后必有名字段");
        Ok(match self.lookup(name) {
            Some((_, source)) => CredentialInfo { configured: true, source: Some(source.to_string()) },
            None => CredentialInfo { configured: false, source: None },
        })
    }

    /// 同步移除（[`CredentialService::unset`] 的实体；缺失 = no-op 不落盘）。
    fn unset_sync(&self, reference: &str) -> Result<()> {
        validate_reference(reference).map_err(|msg| CordisError::new(ErrorKind::ConfigValidation, msg))?;
        let name = reference_name(reference).expect("validate 通过后必有名字段");
        let mut entries = self.entries.lock().expect("凭据表锁");
        if entries.remove(name).is_some() {
            self.persist(&entries)?;
        }
        Ok(())
    }
}

