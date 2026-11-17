//! 工具域共享 CWD 服务：bash 与文件类工具的统一「当前目录」视图（P0 缺陷一）。
//!
//! # 规则（task-50 rev3+task-52 全族单义改版）
//! - 相对路径语义：fs 六件（read/write/edit/glob/grep）的相对路径一律**锚定
//!   工作区根**（root），不随 bash cd 漂移——相对语义可预测+结果回显绝对路径，
//!   模型拼错当场可见（业主定调「相对路径绑 WorkspaceRoot 不绑 cwd」）；
//!   bash 子进程 spawn 的 current_dir 仍取本服务当前值（起始=根），pwd 捕获
//!   保留作留痕诊断与 spawn 基准，fs 工具不再消费它做解析锚；
//! - 安全硬约束：解析结果经 canonicalize 唯一化+starts_with(根) 运行时闸
//!   （symlink/.. 越界词法盲视的运行时补强）；bash 捕获值越界/不可解析
//!   一律 fail-loud 回 root 并在留痕区记录，不信任调用方输入；
//! - 绝对路径行为不变：候选为绝对路径时原样参与唯一化与根校验。
//!
//! # 落位取舍（自证）
//! 本服务是工具域内部共享状态：由 [`crate::builtin::builtin_tools`] 构造后以
//! Arc 分发六个内置工具（同一实例＝视图统一的机制本身），不经内核 Context
//! 结构体、不进 ctx.provide——无跨 crate 消费者时先提服务注入是过度设计；
//! 未来其他域要读 cwd 时再升级为服务注入（规格冻结加法演进）。
//!
//! # bash pwd 捕获的防伪造成本论证（自证；云镜 P2-A2 措辞降调）
//! - 捕获指令由宿主在门禁扫描**之后**拼接到命令尾部：模型面只见自己给的 cmd，
//!   wrapper 文本与捕获文件路径均不在模型面；捕获文件由宿主预创建 0600
//!   （bash `>` 对已存在文件是截断写不提权，见 bash.rs `CaptureFile::new`）；
//! - **不主张「路径不可预知」**：同机其他进程可枚举 /tmp/yunyou-cwd-* 通配
//!   （一行即可定向覆盖捕获文件）——防伪立足点不在路径隐蔽性，在**收益侧**：
//!   捕获值 canonicalize 后过 root 校验、越界即 clamp 回 root，伪造 pwd 的
//!   最大收益只能是「root 内的错误子目录」（定向覆盖收益自伤级），不构成
//!   越界写（文件工具 path_guard 恒守 root）；
//! - 用户命令前台写捕获文件会被末尾 `pwd > 捕获文件` 覆盖（宿主追加的行在
//!   用户命令之后执行）；
//! - 残余窗口＝后台子进程延迟覆盖捕获文件：需模型显式写针对性伪造代码且
//!   收益如上自伤级，防伪到此为止（升级为内核进程树审计，成本与收益不成
//!   比例）。

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use yunyou::CordisError;

/// 留痕区容量（最近 N 条 cwd 事件，诊断与测试断言用）。
const TRAIL_CAP: usize = 32;

/// 共享 CWD 服务（六内置工具共享同一实例；root 恒定，cwd 恒在 root 内）。
/// task-71①：工作区根三段链解析（装配期一次性调用，会话稳定不重检——52
/// 反漂移语义保持）。①[workspace] root 显式配置（相对=相对进程 cwd）→
/// ②git rev-parse --show-toplevel（git 自解析符号链接，兜底 canonicalize）
/// →③进程 cwd 兜底（非 git 目录场景=现行为）。配置非法不 fail-loud：stderr
/// 警告+链继续（显式配置写错静默吞掉=违背意图，警告留痕交给用户修）。
pub fn resolve_workspace_root(config_root: Option<&str>, process_cwd: &Path) -> PathBuf {
    if let Some(cfg) = config_root {
        let p = Path::new(cfg);
        let cand = if p.is_absolute() { p.to_path_buf() } else { process_cwd.join(p) };
        match cand.canonicalize() {
            Ok(c) => return c,
            Err(err) => eprintln!(
                "[workspace] root 配置无法解析（{cfg}：{err}），回退自动检测链——请修正配置"
            ),
        }
    }
    if let Ok(out) = std::process::Command::new("git")
        .arg("-C")
        .arg(process_cwd)
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(&s);
                return p.canonicalize().unwrap_or(p);
            }
        }
    }
    process_cwd.to_path_buf()
}

pub struct CwdService {
    root: PathBuf,
    current: RwLock<PathBuf>,
    trail: RwLock<VecDeque<String>>,
}

impl CwdService {
    /// 构造：root 规范化为绝对路径（canonicalize 失败＝目录尚不存在，退化为
    /// 进程 cwd 拼接的绝对形式，path_guard 按分量前缀判定不受影响）。
    pub fn new(root: &Path) -> Self {
        let absolute = std::fs::canonicalize(root).unwrap_or_else(|_| absolutize(root));
        Self {
            current: RwLock::new(absolute.clone()),
            root: absolute,
            trail: RwLock::new(VecDeque::new()),
        }
    }

    /// 路径约束锚点（恒定）。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 当前目录快照（bash 子进程 spawn 的 current_dir 与留痕诊断用）。
    pub fn cwd(&self) -> PathBuf {
        self.current.read().unwrap().clone()
    }

    /// 工作区根（task-50 rev3+task-52 全族单义：fs 工具相对路径的唯一锚点；
    /// 根=三段链解析（resolve_workspace_root：[workspace] root→git 根→启动目录
    /// 兜底，task-71①）——装配期定死，两者关系写进工具 description，模型拼路径
    /// 有据可依）。
    pub fn workspace_root(&self) -> &Path {
        &self.root
    }

    /// 工具路径解析·工作区根锚定（task-50 rev3，业主定调「相对路径绑
    /// WorkspaceRoot 不绑 cwd」；替代旧 resolve 的「随 bash cd 漂移」语义——
    /// 相对语义可预测+结果回显绝对路径，模型拼错当场可见）。
    ///
    /// 唯一化三步（防穿越的运行时闸，强于词法检查——symlink 越界在词法上
    /// 不可见，用例 d 实证）：①候选=根 join 相对（绝对路径原样候选）；
    /// ②canonicalize 唯一化——本体存在则化本体，不存在（写入新文件的常态）
    /// 则化父目录再拼文件名（坑①：新文件本体不可 canonicalize）；③
    /// starts_with(规范化根) 校验，越界 fail-loud。
    pub fn resolve_workspace(&self, path: &str) -> Result<PathBuf, CordisError> {
        let candidate = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.root.join(path)
        };
        let canonical_root =
            std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
        let canonical = if candidate.exists() {
            std::fs::canonicalize(&candidate).unwrap_or(candidate)
        } else {
            match candidate.parent().map(std::fs::canonicalize) {
                Some(Ok(dir)) => dir.join(candidate.file_name().unwrap_or_default()),
                _ => candidate,
            }
        };
        if !canonical.starts_with(&canonical_root) {
            return Err(CordisError::new(
                yunyou::ErrorKind::Internal,
                format!(
                    "路径越界（运行时校验）：{path} 解析为 {}，位于工作区根 {} 之外",
                    canonical.display(),
                    canonical_root.display()
                ),
            ));
        }
        Ok(canonical)
    }

    /// bash 执行后写入捕获的 pwd：合法→更新 cwd；越界/不可解析→fail-loud 回
    /// root 并留痕。返回是否接受。
    pub fn set_captured(&self, raw_pwd: &str) -> bool {
        let trimmed = raw_pwd.trim();
        let verified = std::fs::canonicalize(trimmed).ok().filter(|dir| dir.starts_with(&self.root));
        let mut current = self.current.write().unwrap();
        match verified {
            Some(dir) => {
                let shown = dir.display().to_string();
                *current = dir;
                self.push_trail(format!("cwd 更新为 {shown}"));
                true
            }
            None => {
                *current = self.root.clone();
                self.push_trail(format!("cwd 捕获值越界或不可解析，已回退根目录（捕获原文 {trimmed}）"));
                false
            }
        }
    }

    /// bash 未产出捕获（命令提前退出/捕获缺失）：cwd 维持不变，仅留痕。
    pub fn note_uncaptured(&self) {
        self.push_trail("bash 未产出 cwd 捕获，cwd 维持不变".into());
    }

    /// 诊断留痕（最近事件，按序；测试断言与排障用）。
    pub fn trail(&self) -> Vec<String> {
        self.trail.read().unwrap().iter().cloned().collect()
    }

    fn push_trail(&self, line: String) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut trail = self.trail.write().unwrap();
        if trail.len() >= TRAIL_CAP {
            trail.pop_front();
        }
        trail.push_back(format!("[{secs}] {line}"));
    }
}

/// 相对路径 → 绝对（基于进程 cwd；不折叠 `..`——canonicalize 失败场景的保守形态）。
fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yunyou-cwd-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        dir
    }

    /// 相对路径锚定工作区根（task-50 rev3）：cwd 变化不改变拼接基准。
    #[test]
    fn 相对路径锚定工作区根() {
        let root = temp_root("resolve");
        let service = CwdService::new(&root);
        assert_eq!(service.resolve_workspace("a.txt").unwrap(), root.join("a.txt"));
        assert!(service.set_captured(root.join("sub").to_str().unwrap()));
        assert_eq!(service.cwd(), root.join("sub"));
        assert_eq!(
            service.resolve_workspace("b.txt").unwrap(),
            root.join("b.txt"),
            "rev3：相对锚根，不随 cwd 漂移"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 绝对路径行为不变：root 内合法、越界拒绝（cwd 在哪都不影响绝对路径判定）。
    #[test]
    fn 绝对路径行为不变() {
        let root = temp_root("abs");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let service = CwdService::new(&root);
        assert!(service.set_captured(root.join("sub").to_str().unwrap()));
        assert!(service
            .resolve_workspace(root.join("sub/c.txt").to_str().unwrap())
            .is_ok());
        assert!(service.resolve_workspace("/etc/passwd").is_err());
        // cwd 已在 sub，仍不得借相对 .. 逃出 root（canonicalize 运行时闸）
        assert!(service.resolve_workspace("../../../etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 越界捕获 fail-loud 回 root 并留痕；未捕获留痕不更新。
    #[test]
    fn 越界捕获回退根目录留痕() {
        let root = temp_root("clamp");
        let service = CwdService::new(&root);
        assert!(!service.set_captured("/etc"));
        assert_eq!(service.cwd(), root);
        assert!(service.trail().last().unwrap().contains("回退根目录"));
        service.note_uncaptured();
        assert_eq!(service.cwd(), root);
        assert!(service.trail().last().unwrap().contains("维持不变"));
        // 合法捕获正常更新
        assert!(service.set_captured(root.join("sub").to_str().unwrap()));
        assert_eq!(service.cwd(), root.join("sub"));
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod t71_tests {
    //! task-71①：根绑定三段链解析器四态单测（temp git init 实测）。
    use super::*;

    #[test]
    fn 非git目录兜底启动目录() {
        let dir = std::env::temp_dir().join(format!("t71a-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = resolve_workspace_root(None, &dir);
        assert_eq!(r, dir, "非 git=启动目录兜底");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn git仓内解析到仓库根() {
        let dir = std::env::temp_dir().join(format!("t71b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("proj/sub")).unwrap();
        let ok = std::process::Command::new("git")
            .arg("-C").arg(dir.join("proj"))
            .args(["init", "-q"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return; // 环境无 git=跳过（CI 兜底不假红）
        }
        let r = resolve_workspace_root(None, &dir.join("proj/sub"));
        assert_eq!(r, dir.join("proj"), "子目录解析到仓库根");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 显式配置覆盖git检测() {
        let dir = std::env::temp_dir().join(format!("t71c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("alt")).unwrap();
        let r = resolve_workspace_root(Some(dir.join("alt").to_str().unwrap()), &dir);
        assert_eq!(r, dir.join("alt"), "显式配置优先");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 非法配置警告回退自动链() {
        let dir = std::env::temp_dir().join(format!("t71d-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let r = resolve_workspace_root(Some("/nonexistent-t71/xyz"), &dir);
        assert_eq!(r, dir, "非法配置=回退兜底（警告走 stderr）");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
