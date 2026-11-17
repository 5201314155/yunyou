//! assemble 单元测试（task-60：hello unmatched 示范件入状态表 + teardown_fail
//! 既有回归；#[path] 外挂保持 assemble.rs 主文件 ≤400——家规第四条）。
use super::*;

    use yunyou_config::default_persona;

    /// P1-3 装配断言：Default 装配下 agent_loop.persona 应桥接为云悠品牌预设
    /// （system_prompt 段缺省值经桥接落入 agent_loop 段，人设断头路闭合）。
    #[test]
    fn persona_桥接_default装配落到品牌预设() {
        let config = DshConfig::default();
        let raw = serde_json::to_value(&config).expect("Default 序列化不应失败");
        let bridged = bridge_persona(raw, &config);
        assert_eq!(
            bridged["agent_loop"]["persona"],
            Value::String(default_persona().to_string()),
            "Default 装配：agent_loop.persona 应为品牌预设值"
        );
        assert!(default_persona().contains("云悠"), "品牌预设应含云悠口径");
    }

    /// 用户显式配置的 agent_loop.persona 优先，桥接不覆盖用户值。
    #[test]
    fn persona_桥接_显式配置尊重用户值() {
        let mut config = DshConfig::default();
        config.agent_loop.persona = Some("自定义 worker 人设".into());
        let raw = serde_json::to_value(&config).expect("序列化不应失败");
        let bridged = bridge_persona(raw, &config);
        assert_eq!(
            bridged["agent_loop"]["persona"],
            Value::String("自定义 worker 人设".into()),
            "显式 agent_loop.persona 不应被桥接覆盖"
        );
    }

    /// agent_loop.persona 显式空串视为缺省：继承 system_prompt.persona。
    #[test]
    fn persona_桥接_空串视为缺省继承系统段() {
        let mut config = DshConfig::default();
        config.agent_loop.persona = Some(String::new());
        config.system_prompt.persona = Some("toml 里写的系统段人设".into());
        let raw = serde_json::to_value(&config).expect("序列化不应失败");
        let bridged = bridge_persona(raw, &config);
        assert_eq!(
            bridged["agent_loop"]["persona"],
            Value::String("toml 里写的系统段人设".into()),
            "agent_loop.persona 空串应继承 system_prompt.persona"
        );
    }

    /// system_prompt.persona 为 None 时桥接为空串（无人设，compose 空人设分支）。
    #[test]
    fn persona_桥接_系统段缺人设时落空串() {
        let mut config = DshConfig::default();
        config.system_prompt.persona = None;
        let raw = serde_json::to_value(&config).expect("序列化不应失败");
        let bridged = bridge_persona(raw, &config);
        assert_eq!(
            bridged["agent_loop"]["persona"],
            Value::String(String::new()),
            "系统段无人设时桥接为空串（不臆造）"
        );
    }

    /// 真装配回归：表驱动装配五件套全 Active，agent_loop 句柄在位
    ///（Default 配置不联网；形态对齐 tests/smoke.rs）。
    #[tokio::test(flavor = "multi_thread")]
    async fn default_装配_表驱动五件套全就绪() {
        let home = std::env::temp_dir().join(format!("yunyou-cli-assemble-s2-{}", std::process::id()));
        let assembled = assemble(&DshConfig::default(), &home, &home.join("yunyou.toml"))
            .await
            .expect("Default 五件套装配应成功（不联网）");
        let agent_loop = assembled.agent_loop.as_ref().expect("agentLoop 未禁用应在位");
        assert!(agent_loop.is_active(), "agentLoop 应 Active");
        assembled.root.dispose().await.expect("应优雅关闭");
        agent_loop.wait_disposed().await;
        std::fs::remove_dir_all(&home).ok();
    }

    /// teardown_fail 收口语义：上抛装配错误且整树可卸载（fail-loud 不留残尸）。
    #[tokio::test(flavor = "multi_thread")]
    async fn teardown_fail_卸树并上抛() {
        let root = Context::new();
        let err = teardown_fail(&root, AppError::Assemble("组件 failing 装载失败：模拟".into())).await;
        assert!(err.to_string().contains("装载失败"), "{err}");
        root.dispose().await.expect("收口后应仍可整树卸载（幂等安全）");
    }

    /// task-60①②：hello unmatched 示范件（<yunyou-home>/plugins/hello/manifest.toml，
    /// id 未命中 bundled 注册表）→ 发现层 unmatched 收集 → plugin_status 行
    /// state=unsupported/source=discovered → /plugins 状态表可见。③口径：
    /// 路径=<yunyou-home>/plugins/（非仓库相对路径——WorkspaceRoot 单源同理）。
    /// 35a678e 账实差更正后的行为锁：示范件在位即有行（此前全历史零存在=无行根因）。
    #[tokio::test]
    async fn hello仪式件unmatched入状态表() {
        let home = std::env::temp_dir().join(format!("yunyou-assemble-t60-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("plugins/hello")).expect("示范件目录");
        std::fs::write(
            home.join("plugins/hello/manifest.toml"),
            "id = \"hello\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\n",
        )
        .expect("示范件写入");
        let config = yunyou_config::DshConfig::default();
        let config_path = home.join("yunyou.toml");
        let assembled = assemble(&config, &home, &config_path)
            .await
            .expect("装配应成功（unmatched 不拒装）");
        let row = assembled
            .plugin_status
            .iter()
            .find(|r| r.id == "hello")
            .expect("unmatched 示范件应入状态表（task-60①②）");
        assert_eq!(row.state, "unsupported");
        assert_eq!(row.source, "discovered");
        assert!(!row.enabled, "unmatched 无实现不装载");
        assembled.root.dispose().await.expect("测试收尾卸载");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 池 task-6 文件式人设：persona.md 命中即 shadow toml 字符串人设（写进既有
    /// shadow 槽，bridge_persona 对非空槽不覆盖）；空文件回落 toml。
    #[test]
    fn persona_md_shadow_toml_空文件回落() -> Result<(), Box<dyn std::error::Error>> {
        let root = std::env::temp_dir().join(format!("yunyou-cli-persona-md-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".yunyou"))?;
        let mut config = DshConfig::default();
        config.system_prompt.persona = Some("toml 人设".into());
        let raw = || serde_json::to_value(&config).expect("序列化不应失败");
        // ① md 存在 → shadow toml（apply 后槽非空，bridge 不再覆盖）
        std::fs::write(root.join(".yunyou/persona.md"), "# 身份\nmd 人设")?;
        let applied = crate::persona_md::apply_to_raw(raw(), &root, &root.join("home"));
        assert_eq!(applied["agent_loop"]["persona"], Value::String("# 身份\nmd 人设".into()));
        let bridged = bridge_persona(applied, &config);
        assert_eq!(bridged["agent_loop"]["persona"], Value::String("# 身份\nmd 人设".into()));
        // ② md 空文件 → 回落 toml（bridge 照常填 system_prompt.persona）
        std::fs::write(root.join(".yunyou/persona.md"), "  \n")?;
        let applied = crate::persona_md::apply_to_raw(raw(), &root, &root.join("home"));
        let bridged = bridge_persona(applied, &config);
        assert_eq!(bridged["agent_loop"]["persona"], Value::String("toml 人设".into()));
        let _ = std::fs::remove_dir_all(&root);
        Ok(())
    }
