//! yunyou-config 配置模型测试（task-33⑤b 预防拆：主体贴 400 硬顶，tests
//! mod 外挂 #[path] 保持同 crate 私有可达；外迁零语义改动）。

use super::*;

    /// 默认值应与台账环境事实一致（本机中转端点 + 测试模型；key 恒为空）。
    /// 同时锁定 system_prompt.persona 的缺省语义（task-10：云悠品牌预设）。
    #[test]
    fn 默认值对齐环境事实() {
        let cfg = DshConfig::default();
        assert_eq!(cfg.llm.base_url, "http://127.0.0.1:8317/v1");
        assert_eq!(cfg.llm.model, "z-ai/glm-5.3-flash");
        assert_eq!(cfg.llm.api_key, "", "key 绝不出现在代码默认值里");
        assert_eq!(cfg.llm.timeout_secs, 300, "D2：与 OpenAiConfig 双 default 对齐（旧 120 与「默认 300 不变兼容」注释矛盾）");
        assert_eq!(cfg.llm.stream_idle_timeout_ms, 30_000, "D2：流内空闲看门狗默认 30s，透传链双侧一致");
        assert_eq!(cfg.tools.root_dir, ".");
        assert_eq!(cfg.agent_loop.tool_timeout_secs, 300);
        assert_eq!(cfg.agent_loop.persona, None);
        // persona 缺省语义（Default 手写实现，审查契约补充 2-a 的护栏）：
        assert_eq!(
            cfg.system_prompt.persona.as_deref(),
            Some(default_persona()),
            "无 system_prompt 段的配置应落云悠品牌预设（原语义为无人设，task-10 变更）"
        );
        assert!(default_persona().contains("云悠"), "品牌预设应含云悠口径");
    }

    /// 注入式 env 覆盖：存在则覆盖、缺省则保留默认。
    #[test]
    fn env_覆盖只动存在键() {
        let mut cfg = DshConfig::default();
        let source = |key: &str| match key {
            ENV_LLM_MODEL => Some("test/model-x".to_string()),
            _ => None,
        };
        apply_env_overrides(&mut cfg, &source);
        assert_eq!(cfg.llm.model, "test/model-x");
        assert_eq!(cfg.llm.base_url, "http://127.0.0.1:8317/v1", "未声明的键不动");
        assert_eq!(cfg.llm.api_key, "", "缺省 key 不应被凭空填充");
    }

    /// S2 迁移兼容（验收口径：存量五段配置不破坏）：无 [plugins] 段的旧
    /// yunyou.toml 零改动反序列化成功，plugins 落空表（全部走 auto_enable），
    /// 且序列化 round-trip 不引入新键（[plugins] 缺省不输出）。
    #[test]
    fn 存量五段配置零改动可用() {
        let legacy = "\
[llm]\n\
base_url = \"http://127.0.0.1:8317/v1\"\n\
\n\
[session]\n\
\n\
[system_prompt]\n\
\n\
[tools]\n\
\n\
[agent_loop]\n";
        let cfg: DshConfig = toml::from_str(legacy).expect("存量配置应零改动可用");
        assert!(cfg.plugins.entries.is_empty(), "无 [plugins] 段应落空表");
        // round-trip 语义等价：重序列化（可能带空 [plugins] 表头，合法）后再反序列化
        // 应与原配置相等——存量语义零漂移。
        let round_trip = toml::to_string(&cfg).expect("序列化不应失败");
        let reloaded: DshConfig = toml::from_str(&round_trip).expect("round-trip 应可再反序列化");
        assert_eq!(cfg, reloaded, "round-trip 语义等价（存量语义零漂移）");
    }

    /// [plugins] 已知段反序列化（方案 §2.4 第 2 步形态）：enable 开关 + config 段值。
    #[test]
    fn plugins_段_enable与config反序列化() {
        let text = "\
[plugins.yunyou-tools]\n\
enable = false\n\
\n\
[plugins.yunyou-tools.config]\n\
root_dir = \"/tmp/workdir\"\n";
        let cfg: DshConfig = toml::from_str(text).expect("[plugins] 已知段应可反序列化");
        let entry = cfg.plugins.entries.get("yunyou-tools").expect("插件键应入表");
        assert_eq!(entry.enable, Some(false), "enable 开关应落值");
        assert_eq!(entry.config["root_dir"], "/tmp/workdir", "config 段值应透传");
    }

    /// 裁决 D2：deny_unknown_fields 保留——[plugins] 是已知段不是未知段；
    /// 顶层未知键与条目内未知键都仍报错（拼写防呆不放松）。
    #[test]
    fn deny_unknown_fields_保留_已知段内外都拦未知键() {
        // 顶层未知键（[plugins] 之外）照旧拒绝
        let top_unknown = "[llm]\n\n[unknown_section]\n";
        assert!(
            toml::from_str::<DshConfig>(top_unknown).is_err(),
            "顶层未知段必须仍被拒绝（D2：deny 保留）"
        );
        // 条目内未知键拒绝（enable/config 之外）
        let entry_unknown = "[plugins.yunyou-tools]\nenable = false\nwhatever = 1\n";
        assert!(
            toml::from_str::<DshConfig>(entry_unknown).is_err(),
            "插件条目内未知键必须被拒绝（拼写防呆）"
        );
    }

    /// task-34⑤：[llm] 嵌套段 deny_unknown_fields——拼错键不再静默忽略，
    /// serde 报错自动点名未知键并列合法键（「报错列合法键」验收锚点）。
    #[test]
    fn llm_段未知键拒绝并列合法键() {
        let misspelled = "[llm]\ntimeout_seconds = 0\n";
        let err = toml::from_str::<DshConfig>(misspelled)
            .expect_err("[llm] 未知键必须拒绝（静默忽略=用户以为配了超时实际落默认）");
        let text = err.to_string();
        assert!(text.contains("timeout_seconds"), "报错应点名未知键：{text}");
        assert!(text.contains("timeout_secs"), "报错应列合法键：{text}");
        // 合法键零冲击：五个合法键原样可用
        let legal = "[llm]\nbase_url = \"http://x/v1\"\napi_key = \"k\"\nmodel = \"m\"\ntimeout_secs = 30\nstream_idle_timeout_ms = 5000\n";
        let cfg = toml::from_str::<DshConfig>(legal).expect("合法 [llm] 键集应全通过");
        assert_eq!(cfg.llm.timeout_secs, 30);
        assert_eq!(cfg.llm.stream_idle_timeout_ms, 5000);
    }

    /// task-18（挑刺 P1-2 残留收口）：deny_unknown_fields 补齐其余段——
    /// llm 段之外的段拼错键同样不再静默落默认（timeout_seconds 教训同型，
    /// 每段各一条拼错用例；workspace 段系卡面六段外巡检发现同型，一并收口）。
    #[test]
    fn 各段拼错键全拒不再静默() {
        for (tag, text, bad_key) in [
            ("session", "[session]\npathh = \"/tmp/s.jsonl\"\n", "pathh"),
            ("system_prompt", "[system_prompt]\npersona_text = \"x\"\n", "persona_text"),
            ("tools", "[tools]\nroot_diir = \"/w\"\n", "root_diir"),
            ("agent_loop", "[agent_loop]\ntool_timeout = 10\n", "tool_timeout"),
            ("retry", "[retry]\ninitial_delay = 500\n", "initial_delay"),
            ("credentials", "[credentials]\nstore_file = \"c.toml\"\n", "store_file"),
            ("workspace", "[workspace]\nrooot = \"/w\"\n", "rooot"),
        ] {
            let err = toml::from_str::<DshConfig>(text)
                .expect_err(&format!("[{tag}] 段拼错键 {bad_key} 必须报错（静默=用户以为配了实际落默认）"));
            let shown = err.to_string();
            assert!(
                shown.contains(bad_key),
                "[{tag}] 报错应点名拼错键 {bad_key}：{shown}"
            );
        }
    }

    /// task-24：[sandbox] 段 deny_unknown_fields（七段同型收口——K-16 sweep
    /// 机械归位：悬空 test 落回 mod tests 内，域=云垒批结构残缺清偿）。
    #[test]
    fn sandbox_段拼错键拒绝() {
        let misspelled = "[sandbox]\nenabled = true\nbad_key = 1\n";
        let err = toml::from_str::<DshConfig>(misspelled)
            .expect_err("[sandbox] 未知键必须拒绝（静默忽略=假旋钮温床）");
        assert!(err.to_string().contains("bad_key"), "{err}");
    }
