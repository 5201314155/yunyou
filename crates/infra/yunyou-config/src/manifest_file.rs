//! manifest.toml 落盘投影与发现层（装载器 L1，K1；设计《装载器最小方案》§3）。
//!
//! # 定位
//! 「安装即自动注册」的发现层文件面：`<yunyou-home>/plugins/<目录>/manifest.toml`
//! 是内核 [`PluginManifest`] 的**落盘投影**——字段集为其子集（设计 §6 原文），
//! serde 字段名与语义同源（零新格式）；解析后转 [`PluginManifest`] 复用内核
//! [`validate_manifest_static`] 同口径校验（单源，防两份校验漂移）。
//!
//! # 与代码 manifest 的关系（D1 代码真源不破）
//! 落盘文件是**声明投影**：命中 bundled 实现的条目由装配层（K2）取其
//! auto_enable 覆盖代码缺省（`[plugins.<id>]` 配置段仍最高优先），其余字段
//! （contracts/provides/inject/ui）以 bundled 代码为准——文件声明漂移由装配层
//! 警告可见，不改变装载行为。
//!
//! # schemars 双形态（预留兑现口径）
//! 「schemars 生成落盘」的预留本期以 serde 形态同源复用兑现（字段名/语义与
//! PluginManifest 一致＝零新格式）；ManifestFile 整体 JsonSchema derive 因
//! 内核 UiEntry 尚无 derive 暂缺（补 derive 后本类型一行加上，格式不受影响）。
//!
//! # L2 接缝（K4 冻结接口，设计 §4）
//! [`DiscoveredPlugin`] 统一产出（命中与否都产出）；L2 届时把装配层「bundled
//! 匹配」分支旁的「未命中」分支换成子进程 Runner，本层接口不变。
//!
//! # 目录扫描信任边界补记（nit 挂账认领，lead 2026-11-14 18:1x）
//! `read_dir` 对符号链接目录会**跟链读**：plugins/ 下的 symlink 目录同样被
//! 扫描。无害性＝L1 只读声明文件、零执行（设计 §6 信任边界原文），symlink
//! 最多改变「哪些 manifest.toml 被读到」，不扩大执行面；读到的内容仍过同
//! 口径静态校验，坏内容照常 fail-loud，故无需专门的链接过滤。

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yunyou::manifest::{validate_manifest_static, PluginLevel, PluginManifest};

/// manifest 文件固定名（目录约定：每插件一目录，目录内一个 manifest.toml）。
pub const MANIFEST_FILE_NAME: &str = "manifest.toml";

/// 落盘 manifest 投影（PluginManifest 的文件形态；字段集为其子集）。
// deny_unknown_fields（挑刺报告 20261117 F-1）：拼错的 manifest 键要报错，不静默
// 吞（与 yunyou.toml 配置面防呆口径对齐）；字段全为必填或有 default，无 flatten。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    /// 插件全局唯一标识（必填；命中判定键）。
    pub id: String,
    /// 插件自身版本（必填；semver）。
    pub version: String,
    /// 插件两级；缺省 bundled（与内核 minimal 缺省一致）。
    #[serde(default = "default_level")]
    pub level: PluginLevel,
    /// 对齐的插槽规格版本（必填；内核静态校验②非空 + name.vN 格式）。
    pub contracts: Vec<String>,
    /// provide 服务键投影；缺省空。
    #[serde(default)]
    pub provides: Vec<String>,
    /// inject 冗余声明；缺省空。
    #[serde(default)]
    pub inject: Vec<String>,
    /// UI 注册缝贡献；缺省空。
    #[serde(default)]
    pub ui: Vec<yunyou::manifest_ui::UiEntry>,
    /// 是否默认启用；缺省 true。
    #[serde(default = "default_true")]
    pub auto_enable: bool,
    // ── market v2 字段（task-10/K-19 刀2；加法演进 serde default 律——v1 文件
    // 零感知，v2 文件经本结构发现层不拒；**权威校验在 cli market_manifest
    // parse_and_validate 五类违例**，此处只承载发现层不拒装的文件形状）──
    /// 落盘格式版本；v2 文件=Some(2)，v1 文件=None。
    #[serde(default)]
    pub format: Option<u32>,
    /// 分类目录约定（install 自动归类落 plugins/<category>/<id>/）。
    #[serde(default)]
    pub category: Option<String>,
    /// [runtime] 段（v2 必填；发现层只承载形状，缺段校验在 market 权威校验）。
    #[serde(default)]
    pub runtime: Option<RuntimeFileSection>,
    /// [permissions] 段（v2 可选；缺省全最小）。
    #[serde(default)]
    pub permissions: Option<PermissionsFileSection>,
}

/// market v2 [runtime] 段的文件形状（字段与 cli market_manifest::RuntimeSection
/// 同源；发现层零校验只承载）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RuntimeFileSection {
    /// 装载形态：唯一合法值 "subprocess"（权威校验在 cli 侧）。
    #[serde(default)]
    pub r#type: String,
    /// 插件入口 argv（相对插件目录）。
    #[serde(default)]
    pub entry: Vec<String>,
    /// 单次工具调用超时毫秒（缺省 30000）。
    #[serde(default)]
    pub request_timeout_ms: Option<u64>,
}

/// market v2 [permissions] 段的文件形状（同上，零校验只承载）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct PermissionsFileSection {
    /// Landlock 写白名单附加根（$SELF=插件目录）。
    #[serde(default)]
    pub writable: Vec<String>,
    /// 凭据名声明（MVP 非空=拒装）。
    #[serde(default)]
    pub credentials: Vec<String>,
    /// 网络放行开关（MVP 恒拒 true）。
    #[serde(default)]
    pub network: bool,
}

fn default_level() -> PluginLevel {
    PluginLevel::Bundled
}

fn default_true() -> bool {
    true
}

impl ManifestFile {
    /// 转内核 manifest（补齐子集缺省）——静态校验与装配层消费的统一形态。
    pub fn to_manifest(&self) -> PluginManifest {
        PluginManifest {
            id: self.id.clone(),
            version: self.version.clone(),
            level: self.level,
            contracts: self.contracts.clone(),
            provides: self.provides.clone(),
            inject: self.inject.clone(),
            ui: self.ui.clone(),
            auto_enable: self.auto_enable,
        }
    }
}

/// 发现层统一产出（K4 L2 接缝；命中与否都产出，设计 §4）。
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// manifest.toml 路径（L2 子进程 Runner 的接缝字段）。
    pub manifest_path: PathBuf,
    /// 插件 id（以文件内 `id` 字段为准；目录名仅组织建议不强制相等）。
    pub id: String,
    /// 解析后的落盘投影。
    pub file: ManifestFile,
}

/// 解析单个 manifest.toml 文本：toml 解析 + 内核静态校验同口径（违例逐条报）。
pub fn parse_manifest_toml(path: &Path, text: &str) -> Result<ManifestFile, Vec<String>> {
    let file: ManifestFile = toml::from_str(text)
        .map_err(|err| vec![format!("{}: TOML 解析失败：{err}", path.display())])?;
    let issues = validate_manifest_static(&file.to_manifest());
    if issues.is_empty() {
        Ok(file)
    } else {
        Err(issues
            .into_iter()
            .map(|issue| format!("{}: {issue}", path.display()))
            .collect())
    }
}

/// 发现层：扫描 `plugins/` 下每子目录的 manifest.toml（目录约定，设计 §3）。
///
/// - 目录不存在/不可读 → 空表（未装任何插件是常态，不报错）；
/// - 子目录无 manifest.toml → 跳过（目录里放非插件内容不报）；
/// - 任一文件解析失败/校验违例 → `Err`（**全部**文件的错误聚合逐条报，
///   fail-loud 不静默——对齐 yunyou.toml 校验「多问题一次报齐」风格）；
/// - 同 id 多目录重复声明 → 报错（「每插件一目录」防呆：重复声明即配置漂移）。
pub fn discover_plugins(plugins_dir: &Path) -> Result<Vec<DiscoveredPlugin>, Vec<String>> {
    let mut discovered = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let entries = match std::fs::read_dir(plugins_dir) {
        Ok(entries) => entries,
        Err(_) => return Ok(discovered), // 目录不存在＝无插件，常态空表
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue; // 顶层散文件（README/.DS_Store 类）不报
        }
        let manifest_path = path.join(MANIFEST_FILE_NAME);
        if !manifest_path.is_file() {
            // 无 manifest 的目录：可能是分类父目录（task-10 install 归类落
            // plugins/<category>/<id>/）——下钻一层继续扫；非分类目录（如
            // README 目录）自然空扫无害。深度恒 2 层（分类约定一层，防深递归）。
            scan_level(&path, &mut discovered, &mut errors);
            continue;
        }
        parse_into(&manifest_path, &mut discovered, &mut errors);
    }
    // 同 id 重复声明：一插件一目录的防呆（两目录同 id = 配置漂移，fail-loud）
    let mut seen: BTreeMap<String, PathBuf> = BTreeMap::new();
    for d in &discovered {
        if let Some(first) = seen.insert(d.id.clone(), d.manifest_path.clone()) {
            errors.push(format!(
                "插件 id {:?} 重复声明：{} 与 {}——每插件一目录，请删除多余声明",
                d.id,
                first.display(),
                d.manifest_path.display()
            ));
        }
    }
    if errors.is_empty() {
        Ok(discovered)
    } else {
        Err(errors)
    }
}

/// 单层扫描：读目录内每个 `<子目录>/manifest.toml`（分类子目录用）。
fn scan_level(
    dir: &Path,
    discovered: &mut Vec<DiscoveredPlugin>,
    errors: &mut Vec<String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let manifest_path = path.join(MANIFEST_FILE_NAME);
        if !manifest_path.is_file() {
            continue;
        }
        parse_into(&manifest_path, discovered, errors);
    }
}

/// 读+解析一个 manifest.toml（成功入 discovered，违例聚合进 errors）。
fn parse_into(
    manifest_path: &Path,
    discovered: &mut Vec<DiscoveredPlugin>,
    errors: &mut Vec<String>,
) {
    match std::fs::read_to_string(manifest_path) {
        Ok(text) => match parse_manifest_toml(manifest_path, &text) {
            Ok(file) => discovered.push(DiscoveredPlugin {
                manifest_path: manifest_path.to_path_buf(),
                id: file.id.clone(),
                file,
            }),
            Err(mut issues) => errors.append(&mut issues),
        },
        Err(err) => errors.push(format!("{}: 读取失败：{err}", manifest_path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时 plugins 目录（pid 隔离，用完即清）。
    fn temp_plugins(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunyou-cfg-plugins-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 投影解析：全字段同源；最小两字段落缺省（子集语义：level/auto_enable/
    /// provides/inject/ui 缺省与内核 minimal 对齐）。
    #[test]
    fn 投影解析_全字段与最小字段() {
        let full = parse_manifest_toml(
            Path::new("/p/manifest.toml"),
            "id = \"yunyou-tools\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\nauto_enable = false\n",
        )
        .expect("全字段应解析");
        assert_eq!(full.id, "yunyou-tools");
        assert!(!full.auto_enable, "显式 false 生效");
        let minimal = parse_manifest_toml(
            Path::new("/p/manifest.toml"),
            "id = \"hello\"\nversion = \"0.2.0\"\ncontracts = [\"tools.v1\"]\n",
        )
        .expect("最小字段应解析（其余缺省）");
        assert_eq!(minimal.level, PluginLevel::Bundled, "level 缺省 bundled");
        assert!(minimal.auto_enable, "auto_enable 缺省 true");
        assert!(minimal.provides.is_empty() && minimal.inject.is_empty() && minimal.ui.is_empty());
        // 转 PluginManifest：字段同源（零新格式的投影语义）
        let manifest = minimal.to_manifest();
        assert_eq!(manifest.id, "hello");
        assert_eq!(manifest.contracts, vec!["tools.v1"]);
    }

    /// 必填缺失与校验违例逐条带路径报（fail-loud 不静默）。
    #[test]
    fn 缺必填与校验违例逐条报() {
        let errs = parse_manifest_toml(Path::new("/p/m.toml"), "version = \"0.1.0\"\ncontracts = []\n")
            .expect_err("缺 id 必报");
        assert!(errs[0].contains("/p/m.toml") && errs[0].contains("TOML 解析失败"), "{errs:?}");
        let errs = parse_manifest_toml(
            Path::new("/p/m.toml"),
            "id = \"x\"\nversion = \"0.1.0\"\ncontracts = [\"bad\"]\n",
        )
        .expect_err("contracts 格式非法必报（内核同口径）");
        assert!(errs[0].contains("格式非法"), "{errs:?}");
        let errs = parse_manifest_toml(Path::new("/p/m.toml"), "id = \"x\"\nversion = \"0.1.0\"\ncontracts = []\n")
            .expect_err("contracts 空必报（内核同口径②）");
        assert!(errs[0].contains("不得为空"), "{errs:?}");
    }

    /// F-1（挑刺报告 20261117）：拼错的 manifest 键要报错，不静默吞。
    #[test]
    fn 拼错键报错不静默吞() {
        let errs = parse_manifest_toml(
            Path::new("/p/m.toml"),
            "id = \"x\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\nauto_enble = false\n",
        )
        .expect_err("未知键 auto_enble 必报");
        assert!(
            errs[0].contains("unknown field") || errs[0].contains("未知"),
            "应报未知键：{errs:?}"
        );
    }

    /// 发现层：多插件产出、目录缺失空表、无 manifest 目录跳过、坏文件聚合报、
    /// 重复 id 报。
    #[test]
    fn 发现层_产出_空表_跳过_聚合报() {
        // 多插件 + 无 manifest 目录 + 散文件全忽略
        let dir = temp_plugins("ok");
        for (name, text) in [
            ("yunyou-tools", "id = \"yunyou-tools\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\n"),
            ("hello", "id = \"yunyou-session\"\nversion = \"0.1.0\"\ncontracts = [\"session.v1\"]\n"),
        ] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("manifest.toml"), text).unwrap();
        }
        std::fs::create_dir_all(dir.join("empty-dir")).unwrap();
        std::fs::write(dir.join("README.md"), "not a plugin").unwrap();
        let found = discover_plugins(&dir).expect("合法布局应发现成功");
        assert_eq!(found.len(), 2, "空目录与散文件应跳过: {found:?}");
        assert!(found.iter().any(|d| d.id == "yunyou-tools" && d.manifest_path.ends_with("yunyou-tools/manifest.toml")));
        // 目录不存在：常态空表
        assert!(discover_plugins(&dir.join("no-such")).expect("缺失目录应空表").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        // 坏文件：错误聚合逐条报（两个坏文件一次报齐）
        let dir = temp_plugins("bad");
        for name in ["a", "b"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("manifest.toml"), "version = \"0.1.0\"\n").unwrap();
        }
        let errs = discover_plugins(&dir).expect_err("坏文件应 fail-loud");
        assert_eq!(errs.len(), 2, "两坏文件错误应聚合一次报齐: {errs:?}");
        assert!(errs.iter().all(|e| e.contains("TOML 解析失败")), "{errs:?}");
        let _ = std::fs::remove_dir_all(&dir);
        // 重复 id：fail-loud
        let dir = temp_plugins("dup");
        for name in ["one", "two"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join("manifest.toml"), "id = \"same\"\nversion = \"0.1.0\"\ncontracts = [\"tools.v1\"]\n").unwrap();
        }
        let errs = discover_plugins(&dir).expect_err("重复 id 应报");
        assert!(errs[0].contains("重复声明"), "{errs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
