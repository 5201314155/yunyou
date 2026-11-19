//! L0 高危免疫清单（task-16，设计稿 §三；代码内置**不可配置**——bypass
//! 模式也拦，对齐 claude 旁路免疫安全检查 permissions.ts:1243-1252）。
//!
//! # 判定面（v1：bash 意图文本级扫描）
//! 高危形态按 bash 命令文本扫描（子串/词级形态清单+逐条测试锁）。v1 不依赖
//! bash_guard lex（tools 域内部件未导出）；heredoc/转义逃逸形态的根除候 S4
//! 工作区边界批与 bash_guard 扫描器族合流（bash_guard 已有 heredoc/参数
//! 切分扫描器，导出面候 tools 域协调——诚实注记，此处为文本级止损层）。

/// 高危形态清单（单源；命中即 Deny，任何规则/模式不可豁免）。
/// 注意：匹配在小写化后的整段命令文本上做，形态均含词边界敏感写法防误杀。
pub(crate) const DENYLIST: &[&str] = &[
    // 根/家目录毁灭变体（rm -rf / 及 ~、/* 变体；含 --no-preserve-root）
    "rm -rf /", "rm -fr /", "rm -rf /*", "rm -fr /*", "--no-preserve-root",
    // 盘符/块设备毁灭
    "mkfs.", "dd if=/dev/zero", "dd of=/dev/sd", "dd of=/dev/nvme", "> /dev/sda", "> /dev/nvme",
    // fork 炸弹形态
    ":(){ :|:& };:",
    // 整盘权限放开
    "chmod -r 777 /", "chmod 777 /",
    // 敏感凭据文件写（凭据纪律：key 永不落盘到可写路径之外）
    "~/.ssh/authorized_keys", "~/.dsh/.credentials.yaml",
    // git 钩子植入（仓库级代码执行面）
    ".git/hooks/",
];

/// 判定命令文本是否命中高危清单（大小写不敏感）。
pub(crate) fn is_denylisted(command: &str) -> bool {
    let lowered = command.to_ascii_lowercase();
    DENYLIST.iter().any(|pattern| lowered.contains(pattern))
}

/// 工具调用级判定入口：非 bash 工具恒不命中（清单语义=命令毁灭面）；
/// bash/其他 shell 形工具扫 arguments 全文（含嵌套 JSON 串内的命令文本）。
pub(crate) fn check(tool: &str, arguments: &str) -> Option<&'static str> {
    let shell_like = matches!(tool, "bash" | "shell" | "Bash");
    if !shell_like {
        return None;
    }
    is_denylisted(arguments).then_some(
        "命中高危拦截清单（旁路免疫层）：该命令形态属不可配置拒绝面，任何模式/规则不可豁免。如确需执行请联系用户手工操作。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 命中矩阵：毁灭/凭据/钩子/炸弹各形。
    #[test]
    fn 高危命中() {
        for command in [
            "rm -rf /",
            "sudo rm -fr /*",
            "rm --no-preserve-root -rf /",
            "mkfs.ext4 /dev/sda1",
            "dd if=/dev/zero of=/dev/sda",
            "echo x > /dev/sda",
            ":(){ :|:& };:",
            "chmod -R 777 /",
            "echo key > ~/.ssh/authorized_keys",
            "cat x > ~/.dsh/.credentials.yaml",
            "cp evil.sh .git/hooks/pre-commit",
        ] {
            assert!(is_denylisted(command), "应命中：{command}");
        }
        assert!(check("bash", "{\"command\":\"rm -rf /\"}").is_some());
    }

    /// 免误杀矩阵：正常命令不命中（含形近词）。
    #[test]
    fn 正常命令免误杀() {
        for command in [
            "rm -rf ./build",        // 项目内清理（工作区边界归 L5，不在 L0）
            "rm -rf target",
            "mkdocs build",
            "dd if=a of=b bs=1M",    // 非 /dev 块设备
            "chmod 755 deploy.sh",
            "cargo build --release",
            "git push origin main",
            "cat ~/.ssh/id_rsa.pub", // 读公钥不禁（写 authorized_keys 才拦）
        ] {
            assert!(!is_denylisted(command), "误杀：{command}");
        }
        // 非 bash 工具恒不命中
        assert!(check("web_fetch", "rm -rf /").is_none());
    }
}
