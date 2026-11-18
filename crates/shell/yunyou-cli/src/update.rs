//! `yunyou update` 自更新（池 task-18，业主令「主路径=下载编译好的新版本」）。
//!
//! 三段纪律：①下载校验（主源=github release-bin raw，与 install.sh v3 同源单一
//! 事实源；版本比对=资产即真源——解包跑 `--version` 对比当前，零远端元数据约定）；
//! ②原子替换（current_exe canonicalize 解 symlink 到真身 → `.new` 写+自检 →
//! rename 原子覆盖；rename 免「半新半旧」窗口）；③回滚（`.bak` 保留+人话指路；
//! `--from-source`=cargo install 重拉重编回退路径，非主路径）。
//!
//! 下载/替换函数分离（测试注入面）：网络路径真机档，本地路径纯函数化单测。

use std::path::{Path, PathBuf};
use std::process::Command;

/// release-bin 分支 raw 基址（与 install.sh v3 同源——单一事实源，禁双源漂移）。
pub const RELEASE_BIN_BASE: &str = "https://raw.githubusercontent.com/5201314155/yunyou/release-bin";
pub const SOURCE_REPO: &str = "https://github.com/5201314155/yunyou.git";

/// 当前运行架构对应的资产名（与 install.sh v3 分支同口径）。
pub fn asset_name() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "yunyou-arm64.gz",
        _ => "yunyou.gz",
    }
}

/// 下载远端资产到临时目录并解包，返回解包后二进制路径（失败=人话 Err）。
///
/// curl 命令形态（install.sh 同款依赖：环境必有 curl，缺=人话指路）。
pub fn download_to(dir: &Path) -> Result<PathBuf, String> {
    let url = format!("{}/{}", RELEASE_BIN_BASE, asset_name());
    let gz = dir.join("yunyou.gz");
    let output = Command::new("curl")
        .args(["-fsSL", &url, "-o"])
        .arg(&gz)
        .output()
        .map_err(|err| format!("curl 启动失败（请确认已安装 curl）：{err}"))?;
    if !output.status.success() {
        return Err(format!(
            "预编译包下载失败（{url}）：{}——预编译包未发布时请到 releases 页检查，或 --from-source 源码升级",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let unpacked = Command::new("gunzip")
        .arg("-f")
        .arg(&gz)
        .status()
        .map_err(|err| format!("gunzip 启动失败：{err}"))?;
    if !unpacked.success() {
        return Err("解包失败（gz 损坏？重试或 --from-source）".into());
    }
    Ok(dir.join("yunyou"))
}

/// 跑二进制的 --version 取版本串（失败=Err 人话）。
pub fn read_version(binary: &Path) -> Result<String, String> {
    let output = Command::new(binary)
        .arg("--version")
        .output()
        .map_err(|err| format!("执行 {} 失败：{err}", binary.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} --version 退出非零（资产损坏？）",
            binary.display()
        ));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Err("--version 输出为空（资产损坏？）".into());
    }
    Ok(text)
}

/// 版本行提取 semver 段（--version 输出形如「yunyou 0.1.0 …」或含 v 前缀）。
pub fn extract_semver(version_line: &str) -> Option<(u64, u64, u64)> {
    for token in version_line.split_whitespace() {
        let t = token.trim_start_matches(['v', 'V']);
        let core = t.split(['-', '+']).next().unwrap_or(t);
        let parts: Vec<&str> = core.split('.').collect();
        if parts.len() == 3 {
            if let (Ok(a), Ok(b), Ok(c)) = (
                parts[0].parse::<u64>(),
                parts[1].parse::<u64>(),
                parts[2].parse::<u64>(),
            ) {
                return Some((a, b, c));
            }
        }
    }
    None
}

/// 版本比较：远端 > 当前 才升级（相等/更低=已是最新）。
pub fn should_upgrade(remote: &str, current: &str) -> bool {
    match (extract_semver(remote), extract_semver(current)) {
        (Some(r), Some(c)) => r > c,
        // 解析失败=保守不升（人话在调用方提示 --check 核对）
        _ => false,
    }
}

/// 解析运行位真身（symlink 形态解到 target/release 真身；链接本身不动）。
pub fn resolve_real_binary() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("取自身路径失败：{err}"))?;
    std::fs::canonicalize(&exe).map_err(|err| format!("解析真身失败：{err}"))
}

/// 原子替换（本地纯函数段，tmp 可测）：新件自检→备份→rename 覆盖。
///
/// 返回人话结算句（含 .bak 位置与还原指路）。
pub fn replace_binary(new_binary: &Path, real: &Path) -> Result<String, String> {
    // ① 新件自检（替换前跑 --version——损坏资产绝不落位）
    let new_version =
        read_version(new_binary).map_err(|err| format!("新件自检失败（未替换）：{err}"))?;
    // ② 备份当前（.bak；上一次 .bak 被覆盖=回滚面永远指向「上上个运行版」）
    let bak = real.with_extension("bak");
    if real.exists() {
        std::fs::copy(real, &bak).map_err(|err| format!("备份 .bak 失败（未替换）：{err}"))?;
    }
    // ③ 原子写：新件拷到同目录 .new（同文件系统 rename 才原子）→ rename 覆盖
    let staging = real.with_extension("new");
    std::fs::copy(new_binary, &staging).map_err(|err| format!("新件落盘失败：{err}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755))
            .map_err(|err| format!("新件授权失败：{err}"))?;
    }
    std::fs::rename(&staging, real).map_err(|err| format!("原子替换失败：{err}"))?;
    Ok(format!(
        "✓ 已更新到 {new_version}（原版本备份在 {}——启动异常可执行 cp {bak} {real} 还原）",
        bak.display(),
        bak = bak.display(),
        real = real.display(),
    ))
}

/// `yunyou update` 主流程（--check 只查不升；--from-source 走重编回退路径）。
pub fn run_update(check_only: bool, from_source: bool) -> Result<String, String> {
    if from_source {
        // 回退路径（非主路径，任务书原文）：git 仓库重拉重编
        let output = Command::new("cargo")
            .args(["install", "--git", SOURCE_REPO, "yunyou-cli", "--force"])
            .status()
            .map_err(|err| format!("cargo 启动失败（源码升级需 Rust 工具链）：{err}"))?;
        return if output.success() {
            Ok("✓ 源码升级完成（cargo install 重编）。运行 yunyou --version 核对。".into())
        } else {
            Err("源码升级失败（cargo install 非零退出）——检查网络与 Rust 工具链".into())
        };
    }
    let current = crate::cli::version_line();
    let tmp = std::env::temp_dir().join(format!("yunyou-update-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&tmp);
    let _guard = RemoveDir(&tmp);
    let downloaded = download_to(&tmp)?;
    let remote = read_version(&downloaded)?;
    if !should_upgrade(&remote, &current) {
        return Ok(format!(
            "已是最新（当前 {current}，远端 {remote}）。--check 可随时复查。"
        ));
    }
    if check_only {
        return Ok(format!(
            "有新版本：{remote}（当前 {current}）。执行 yunyou update 升级。"
        ));
    }
    let real = resolve_real_binary()?;
    replace_binary(&downloaded, &real)
}

/// 临时目录守卫（用完即清；临时产物不滞留纪律）。
struct RemoveDir<'a>(&'a Path);
impl Drop for RemoveDir<'_> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)] // 测试面豁免（K-14 观察期口径）

    use super::*;

    /// 造一个「可跑 --version」的假二进制（shell 脚本，echo 固定行）。
    fn fake_binary(dir: &Path, version_line: &str) -> PathBuf {
        let p = dir.join(format!("fake-{}", version_line.replace(['.', ' '], "-")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&p, format!("#!/bin/sh\necho \"{version_line}\"\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    #[test]
    fn semver提取与比较() {
        assert_eq!(extract_semver("yunyou 0.1.0"), Some((0, 1, 0)));
        assert_eq!(extract_semver("yunyou v1.2.3 (build 5)"), Some((1, 2, 3)));
        assert_eq!(extract_semver("1.10.0-rc1"), Some((1, 10, 0)));
        assert_eq!(extract_semver("garbage"), None);
        assert!(should_upgrade("yunyou 0.2.0", "yunyou 0.1.0"));
        assert!(!should_upgrade("yunyou 0.1.0", "yunyou 0.1.0"), "相等=不升");
        assert!(!should_upgrade("yunyou 0.1.0", "yunyou 0.2.0"), "远端更低=不升");
        assert!(!should_upgrade("garbage", "yunyou 0.1.0"), "解析失败=保守不升");
    }

    #[test]
    fn 原子替换_自检备份与结算句() {
        let dir = std::env::temp_dir().join(format!("yunyou-upd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("yunyou");
        std::fs::write(&real, "old").unwrap();
        let new_binary = fake_binary(&dir, "yunyou 9.9.9");
        let msg = replace_binary(&new_binary, &real).expect("替换应成功");
        assert!(msg.contains("9.9.9") && msg.contains(".bak"), "结算句：{msg}");
        // 新件已落位（内容=假二进制内容）+备份在
        assert!(std::fs::read_to_string(&real).unwrap().contains("9.9.9"));
        assert!(real.with_extension("bak").exists(), "备份应在");
        // .new 暂存件已消费（rename 走完不留残件）
        assert!(!real.with_extension("new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 自检失败不替换() {
        let dir = std::env::temp_dir().join(format!("yunyou-upd2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("yunyou");
        std::fs::write(&real, "old-content").unwrap();
        // 损坏资产：非可执行文本（--version 必失败）
        let broken = dir.join("broken");
        std::fs::write(&broken, "not a binary").unwrap();
        let err = replace_binary(&broken, &real).expect_err("损坏资产应拒");
        assert!(err.contains("自检失败"), "实际：{err}");
        // 原件分毫未动（fail-loud 不半替换）
        assert_eq!(std::fs::read_to_string(&real).unwrap(), "old-content");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve真身_普通文件形态() {
        let dir = std::env::temp_dir().join(format!("yunyou-upd3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("yunyou");
        std::fs::write(&real, "x").unwrap();
        // 直接以普通文件路径调用（canonicalize 形态）——resolve 函数本身测 env 依赖，
        // 此处验证 replace 对 canonical 路径工作
        let new_binary = fake_binary(&dir, "yunyou 2.0.0");
        replace_binary(&new_binary, &real).expect("替换应成功");
        assert!(std::fs::read_to_string(&real).unwrap().contains("2.0.0"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 网络真机档（默认 ignore：跑 release 资产真实下载，CI 不依赖外网）。
    #[test]
    #[ignore]
    fn 网络档_真实下载与版本核对() {
        let tmp = std::env::temp_dir().join(format!("yunyou-upd-net-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let binary = download_to(&tmp).expect("下载应成功");
        let version = read_version(&binary).expect("自检应过");
        println!("远端版本：{version}");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
