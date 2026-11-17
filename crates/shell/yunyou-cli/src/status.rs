//! 插件状态表（方案 §2.4 第 5 步：启动输出 id/version/enabled/state——
//! 「面板可开关」的 CLI 观测面一期形态；装载器 L1 起增量 `source` 来源列
//! 与 `/plugins` 命令（K3，plugins_cmd.rs）同源消费本数据面）。
//!
//! 渲染为纯函数（输入状态行、输出文本），装配层负责打印——测试直接断言文本，
//! 不依赖 stdout 劫持。诊断输出与启动横幅同理：观测面走 stdout，异常诊断走
//! stderr（对齐 run.rs「观测点提示走 stderr，不进 mock 帧序」的既有口径）。

/// 插件状态行（状态表数据单元，方案 §2.4 第 5 步四列＋装载器 L1 来源列；随
/// [`crate::assemble::Assembled::plugin_status`] 对外可读——面板/网关二期的同源数据面）。
pub struct PluginStatus {
    /// 插件 id（= manifest.id，bundled 用 crate 名）。
    pub id: String,
    /// 插件自身版本（semver，取自 manifest）。
    pub version: String,
    /// 启用态：true = 已注册装配；false = 被 [plugins] 段禁用（跳过并记日志）。
    pub enabled: bool,
    /// 状态名：fiber 稳定态（[`yunyou::FiberState::as_str`]）、"disabled"
    ///（未注册即不进内核，无 fiber 态）、或 "unsupported"（discovered 未命中
    /// bundled 实现，需 L2 子进程运行时——当前版本不可装载）。
    pub state: &'static str,
    /// 发现来源（装载器 L1，K3）：`"bundled"`＝编译期清单；
    /// `"discovered"`＝plugins/ 目录 manifest 文件发现（命中 bundled 实现
    /// 合并装载，或未命中待 L2）。
    pub source: &'static str,
}

/// Pending 诊断文本（方案 §5.3 验收 1）：依赖被禁用插件的消费者保持 Pending，
/// 输出「依赖未就绪」而非崩溃；重新启用后 epoch 机制自动接线（ACTIVE）。
pub(crate) fn pending_diagnosis(id: &str, missing: &[String]) -> String {
    format!(
        "插件 {id} 依赖未就绪（保持 Pending，不阻塞启动）：缺失服务 {missing:?}；\
         在 [plugins] 段启用对应提供者插件并重启即可恢复（内核 epoch 机制自动接线）"
    )
}

/// 渲染插件状态表：列对齐文本（id/version/enabled/state/source 值域全 ASCII，
/// 宽度可靠）。行序 = 内置注册表序（[plugins] 段 map 序不透出）；
/// discovered 未命中行排尾部（装配层追加序）。
pub(crate) fn render_status_table(rows: &[PluginStatus]) -> String {
    let width = rows.iter().map(|r| r.id.len()).max().unwrap_or(8).max(8);
    let mut out = String::from("插件状态表：\n");
    out.push_str(&format!(
        "  {:<width$}  version   enabled   state        source\n",
        "id",
        width = width
    ));
    for r in rows {
        let enabled = if r.enabled { "enabled" } else { "disabled" };
        out.push_str(&format!(
            "  {:<width$}  {:<9}  {:<9}  {:<12}  {}\n",
            r.id,
            r.version,
            enabled,
            r.state,
            r.source,
            width = width
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 状态表渲染：五列齐全（L1 增量来源列）、行序保持传入序、措辞正确。
    #[test]
    fn 状态表渲染五列与行序() {
        let rows = vec![
            PluginStatus {
                id: "yunyou-llm-openai".into(),
                version: "0.1.0".into(),
                enabled: true,
                state: "active",
                source: "bundled",
            },
            PluginStatus {
                id: "yunyou-tools".into(),
                version: "0.2.0".into(),
                enabled: false,
                state: "disabled",
                source: "discovered",
            },
            PluginStatus {
                id: "yunyou-agent-loop".into(),
                version: "0.1.0".into(),
                enabled: true,
                state: "pending",
                source: "bundled",
            },
        ];
        let text = render_status_table(&rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "标题+表头+3 行：{text:?}");
        assert!(text.contains("插件状态表"), "标题应为中文观测面：{text}");
        // 行序 = 传入序（注册表序），且五列值都可见（K3 来源列：bundled/discovered）
        let first = lines[2];
        assert!(first.contains("yunyou-llm-openai") && first.contains("0.1.0")
            && first.contains("enabled") && first.contains("active")
            && first.contains("bundled"), "首行：{first:?}");
        assert!(lines[3].contains("disabled") && lines[3].contains("discovered"),
            "禁用行 state=disabled+来源 discovered：{:?}", lines[3]);
        assert!(lines[4].contains("pending"), "Pending 行可观测：{:?}", lines[4]);
    }

    /// Pending 诊断措辞（§5.3 验收 1 口径）：「依赖未就绪」+ 恢复路径提示。
    #[test]
    fn pending诊断措辞含恢复路径() {
        let text = pending_diagnosis(
            "yunyou-agent-loop",
            &["dyn RegistryService".to_string()],
        );
        assert!(text.contains("依赖未就绪"), "§5.3 口径措辞：{text}");
        assert!(text.contains("yunyou-agent-loop"), "应点名插件：{text}");
        assert!(text.contains("Pending"), "应说明保持 Pending：{text}");
        assert!(text.contains("[plugins]"), "应给出恢复路径：{text}");
    }
}
