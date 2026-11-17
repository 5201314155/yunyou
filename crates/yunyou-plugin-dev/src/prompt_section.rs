//! creation_mode 提示段五条（task-78/K-2-③ 设计面定稿，终稿权=云清）。
//!
//! 随创造模式启用注入系统提示段常驻（register_section 契约③缝，
//! task-19 先例同款）；禁用时不注入。文本=设计面候选，云清终审定稿。

/// 提示段五条（设计面候选文本；终稿权=云清，D1-D5 管线先例）。
pub const PROMPT_SECTIONS: [&str; 5] = [
    // 1. manifest 先行
    "开发插件从 manifest.toml 开始——先写声明面（id/version/contracts），\
     再落代码实现面；声明与实现不一致时以代码为准（D1 真源纪律），\
     但先声明是设计纪律。",
    // 2. fail-loud 循环
    "插件装载失败=人话+逐条修复指引，绝不静默降级；\
     开发期遇到装载错误按指引修复后重启验证，形成「写→装→错→修」循环。",
    // 3. 红线三条
    "红线三条：①绝不向用户手机号发送验证码或任何短信；\
     ②插件不得绕过宿主凭据体系自取密钥；\
     ③插件不得未经授权访问宿主文件系统边界之外路径。",
    // 4. 交付清单+装后验证义务
    "交付=三件套（tgz 安装包/安装教程.md/介绍.md）落 workspace dist；\
     推送后必须真机验证插件可用（本机同构 CLI 验证加载与工具注册），\
     不许只推不验。",
    // 5. 授权话术
    "装载、启用、交付、推送四动作=用户执行，代理只准备物料与给出精确命令\
     ——不代用户按按钮、不代用户确认授权。",
];

/// 段名（register_section ③缝参数一）。
pub const SECTION_NAME: &str = "creation_mode";

/// 常驻标志（③缝参数三：随会话生命周期常驻）。
pub const SECTION_PERSISTENT: bool = true;

/// 拼接完整提示段（五条编号连排）。
pub fn render_section() -> String {
    let mut out = String::from("【插件开发守则（创造模式）】\n");
    for (idx, section) in PROMPT_SECTIONS.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", idx + 1, section));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 五条齐备且编号连排() {
        assert_eq!(PROMPT_SECTIONS.len(), 5);
        let rendered = render_section();
        for keyword in ["manifest.toml", "fail-loud", "红线三条", "三件套", "授权"] {
            assert!(rendered.contains(keyword), "提示段缺关键词：{keyword}");
        }
        assert!(rendered.contains("1. "), "编号连排起始");
        assert!(rendered.contains("5. "), "编号连排收尾");
    }

    #[test]
    fn 段名与常驻标志() {
        assert_eq!(SECTION_NAME, "creation_mode");
        assert!(SECTION_PERSISTENT);
    }
}
