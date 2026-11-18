//! 插件级授权（task-16 修正令 2026-11-19，业主点破「授权也可以写支持联网的
//! 插件，你这不是写死了吗」——lead 修正令：凭据/网络门从恒拒改**用户授权制**，
//! 手机权限模型：审批粒度从调用级扩展到安装级）。
//!
//! # 语义（修正令五点）
//! ①manifest [permissions] network/credentials 不再恒拒装配；②走 Ask 缝：
//! 新审批对象 PluginPermission{plugin_id, requested, purpose}，用户 y 授权→
//! 授权记录持久化（plugin 级，类似手机「记住我的选择」）→放行；n/忽略→人话拒装；
//! ③授权持久化=~/.yunyou/plugins-authorizations.json（600），卸载时提示清理；
//! ④L0 高危拦截不受影响（命令级与插件级两层并存）；⑤工具自判层照旧。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yunyou_agent_loop::{ApprovalDecision, ApprovalSubject, ApprovalUiRegistry};

/// 单插件授权记录（仅记录**已授**项；未记录=未授权——缺席即拒，不存否认态）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PluginGrant {
    /// 网络面已授。
    #[serde(default)]
    pub network: bool,
    /// 已授凭据名清单（逐名授权，不整包放行）。
    #[serde(default)]
    pub credentials: Vec<String>,
}

/// 授权持久化文件形态（~/.yunyou/plugins-authorizations.json）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthorizationsFile {
    /// 插件 id → 授权记录。
    #[serde(default)]
    pub plugins: BTreeMap<String, PluginGrant>,
}

impl AuthorizationsFile {
    /// 缺省路径（home 投影；取不到=None=仅内存判定不可持久化）。
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|home| {
            PathBuf::from(home).join(".yunyou").join("plugins-authorizations.json")
        })
    }

    /// 加载（缺文件=空表；坏 JSON=Err 上抛——授权面 fail-loud，静默空表=授权
    /// 记录意外丢失后重问用户尚可，但此处选 Err 让调用方人话报，防「以为授过」）。
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("读取 {} 失败：{e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("解析 {} 失败：{e}", path.display()))
    }

    /// 保存（600 权限+临时文件原子 rename，对齐 credentials 托管存储惯例）。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, &text).map_err(|e| format!("写 {} 失败：{e}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, path).map_err(|e| format!("落位 {} 失败：{e}", path.display()))
    }

    /// 卸载清理（修正令③：卸载插件时提示清理——删除授权记录并落盘；
    /// 返回是否存在过记录）。
    pub fn remove_plugin(&mut self, plugin_id: &str) -> bool {
        self.plugins.remove(plugin_id).is_some()
    }
}

/// 授权申请项（manifest [permissions] 解析后的规范化形态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantRequest {
    /// 网络面。
    Network,
    /// 某具名凭据。
    Credential(String),
}

/// 判定已授权集：requested 逐项对照持久化记录；全部已授=Ok（放行）；
/// 缺项清单=Err 带缺项（调用方走 Ask 缝或拒装）。
pub fn granted(
    file: &AuthorizationsFile,
    plugin_id: &str,
    requested: &[GrantRequest],
) -> Result<(), Vec<String>> {
    let grant = file.plugins.get(plugin_id).cloned().unwrap_or_default();
    let missing: Vec<String> = requested
        .iter()
        .filter(|req| match req {
            GrantRequest::Network => !grant.network,
            GrantRequest::Credential(name) => !grant.credentials.contains(name),
        })
        .map(|req| match req {
            GrantRequest::Network => "network".to_string(),
            GrantRequest::Credential(name) => format!("credentials:{name}"),
        })
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(missing)
    }
}

/// 授权决策全流程（装载侧入口；修正令②）：已授→Ok；缺项→Ask 缝呈现
/// PluginPermission 对象——用户授→记录持久化→Ok；拒/忽略/呈现故障→人话拒装。
///
/// # 返回
/// Ok(())=放行；Err(人话)=拒装（含用户拒绝与 fail-closed）。
///
/// # 形态（P0 协查教训兑现）
/// **async**——setup 本是 BoxFut 上下文，直接 await 呈现器（带 5 分钟上限
/// 超时），严禁同步 block_on（runtime 内 block_on=挂死形态）。
pub async fn authorize_plugin(
    presenters: &ApprovalUiRegistry,
    file: &mut AuthorizationsFile,
    store_path: Option<&Path>,
    plugin_id: &str,
    requested: &[GrantRequest],
    purpose: &str,
) -> Result<(), String> {
    if granted(file, plugin_id, requested).is_ok() {
        return Ok(());
    }
    let requested_labels: Vec<String> = requested
        .iter()
        .map(|req| match req {
            GrantRequest::Network => "network".to_string(),
            GrantRequest::Credential(name) => format!("credentials:{name}"),
        })
        .collect();
    let subject = ApprovalSubject::PluginPermission { plugin_id, requested: &requested_labels, purpose };
    let kind = yunyou_agent_loop::ApprovalSubjectKind::PluginPermission;
    let Some(presenter) = presenters.presenter_for(kind) else {
        return Err("插件权限审批呈现器未注册（无头场景 fail-closed）：拒装。可用 `yunyou plugin authorize` 显式授权后重装。".into());
    };
    let decision = match tokio::time::timeout(
        std::time::Duration::from_secs(300),
        presenter.present(subject),
    )
    .await
    {
        Ok(Ok(d)) => d,
        Ok(Err(e)) => {
            return Err(format!("审批呈现故障：{e}（fail-closed 拒装）"));
        }
        Err(_) => {
            return Err("授权等待超时（5 分钟），按拒绝处理（fail-closed）".into());
        }
    };
    match decision {
        ApprovalDecision::Approved => {
            let grant = file.plugins.entry(plugin_id.to_string()).or_default();
            for req in requested {
                match req {
                    GrantRequest::Network => grant.network = true,
                    GrantRequest::Credential(name) => {
                        if !grant.credentials.contains(name) {
                            grant.credentials.push(name.clone());
                        }
                    }
                }
            }
            if let Some(path) = store_path {
                file.save(path)?;
            }
            Ok(())
        }
        ApprovalDecision::Denied { message } => {
            Err(format!("插件「{plugin_id}」权限申请被拒绝：{message}（未授项={requested_labels:?}）"))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // 测试断言惯例（同包既有测试同律）
mod tests {
    use super::*;
    use std::sync::Arc;
    use yunyou_agent_loop::ApprovalPresenter;
    use yunyou::Result;
    use yunyou_util::BoxFut;

    struct YesPresenter;
    impl ApprovalPresenter for YesPresenter {
        fn subject(&self) -> yunyou_agent_loop::ApprovalSubjectKind {
            yunyou_agent_loop::ApprovalSubjectKind::PluginPermission
        }
        fn present(&self, _s: ApprovalSubject<'_>) -> BoxFut<'_, Result<ApprovalDecision>> {
            Box::pin(async move { Ok(ApprovalDecision::Approved) })
        }
    }

    struct NoPresenter;
    impl ApprovalPresenter for NoPresenter {
        fn subject(&self) -> yunyou_agent_loop::ApprovalSubjectKind {
            yunyou_agent_loop::ApprovalSubjectKind::PluginPermission
        }
        fn present(&self, _s: ApprovalSubject<'_>) -> BoxFut<'_, Result<ApprovalDecision>> {
            Box::pin(async move {
                Ok(ApprovalDecision::Denied { message: "用户拒绝".into() })
            })
        }
    }

    fn reqs() -> Vec<GrantRequest> {
        vec![GrantRequest::Network, GrantRequest::Credential("openai_key".into())]
    }

    /// 授权全流程：无呈现器=fail-closed 拒装；拒绝=人话拒装；授权=记录落盘
    /// （600 权限）+二次装载免问（granted 判定）。
    #[tokio::test]
    async fn 授权全流程三态() {
        let dir = std::env::temp_dir().join(format!("perm-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("plugins-authorizations.json");
        // 无呈现器：fail-closed 拒装（装配环境有 runtime 但 present 会走
        // Handle::try_current——测试在 tokio 内，present 链路可达）
        let mut file = AuthorizationsFile::default();
        let no_ui = yunyou_agent_loop::ApprovalUiRegistry::default();
        let err = authorize_plugin(&no_ui, &mut file, Some(&store), "p1", &reqs(), "用途").await;
        assert!(err.is_err(), "无呈现器应拒装");
        // 拒绝呈现器：人话拒装且不落盘
        let ui = yunyou_agent_loop::ApprovalUiRegistry::default();
        ui.register(Arc::new(NoPresenter)).unwrap();
        let err = authorize_plugin(&ui, &mut file, Some(&store), "p1", &reqs(), "用途").await.unwrap_err();
        assert!(err.contains("被拒绝"), "{err}");
        assert!(!store.exists(), "拒绝不得落盘");
        // 授权呈现器：Ok+落盘
        let ui2 = yunyou_agent_loop::ApprovalUiRegistry::default();
        ui2.register(Arc::new(YesPresenter)).unwrap();
        authorize_plugin(&ui2, &mut file, Some(&store), "p1", &reqs(), "用途").await.unwrap();
        assert!(store.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&store).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "授权文件应 600");
        }
        // 二次装载：granted 判定免问（新文件读回）
        let file2 = AuthorizationsFile::load(&store).unwrap();
        assert!(granted(&file2, "p1", &reqs()).is_ok());
        assert!(granted(&file2, "p2", &reqs()).is_err(), "他插件未授仍拒");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 卸载清理：remove_plugin 返回存在性+落盘后记录消失。
    #[tokio::test]
    async fn 卸载清理() {
        let dir = std::env::temp_dir().join(format!("perm-auth2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("auth.json");
        let mut file = AuthorizationsFile::default();
        let ui = yunyou_agent_loop::ApprovalUiRegistry::default();
        ui.register(Arc::new(YesPresenter)).unwrap();
        authorize_plugin(&ui, &mut file, Some(&store), "p1", &reqs(), "用途").await.unwrap();
        assert!(file.remove_plugin("p1"));
        assert!(!file.remove_plugin("p1"));
        file.save(&store).unwrap();
        assert!(AuthorizationsFile::load(&store).unwrap().plugins.get("p1").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
