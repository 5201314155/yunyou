//! 自愈/静态档检测器（task-48，方案 lead 批；规范 §4.14）。
//!
//! 职责：滑动窗口统计**原位重绘字节量**，超阈判定降级（YUNYOU_STATIC 语义），
//! 回合界重探与连续健康自动恢复。纯逻辑零 I/O 零时钟——时间由调用方注入
//! （毫秒 u64），单测全确定性（防误报①③同源纪律）。
//!
//! **口径（单测锁死，防后人放宽）**：只计 `\r\e[K` 活动行**改写**字节
//! （[`SelfHeal::note_rewrite`]）；冻结落定（freeze_line）与纯追加内容
//! **不计入**（[`SelfHeal::note_appended`] 仅为口径对照存在，不移动窗口）。
//! 理由：高延迟链路的带宽杀手是「同屏反复重写」，一次性落定内容是用户
//! 必需信息，计入会把健康流式误判成病理。
//!
//! 【接线位（候 task-51 错峰）】本模块暂未被 Renderer 持有——接线点=
//! render/mod.rs interactive 判定细化（候云刺 task-51 六缝入库后叠刀）；
//! 届时 rewrite 调用路径喂 [`SelfHeal::note_rewrite`]、回合界喂
//! [`SelfHeal::new_turn`]、降级态替换 interactive 判定。

/// 滑动窗口宽（毫秒；规范 §4.14「5s 窗口」）。
pub const WINDOW_MS: u64 = 5_000;
/// 默认阈值：窗口内原位重绘字节量上限（256KB/5s ≈ 2G 边缘网络持续写终端）。
pub const DEFAULT_WINDOW_BYTES: usize = 256 * 1024;
/// 降级后连续健康回合数达标即恢复交互态（防单次抖动永久降级）。
pub const RECOVER_AFTER_TURNS: u32 = 3;
/// 降级告知（一次性；规范 §4.14 人话）。
pub const DEGRADE_NOTICE: &str =
    "检测到高延迟环境，已切换省流量模式（YUNYOU_STATIC）——内容仍完整输出";

/// 单条重绘记录（时间，字节）。
#[derive(Debug, Clone, Copy)]
struct Sample {
    at_ms: u64,
    bytes: usize,
}

/// 墙钟生产外观（task-48 接线）：内部 epoch（构造时刻）起算毫秒——生产路径
/// 用 [`WallClock`] 喂真实时间；单测继续用 `*_at` 注入 API 保持零时钟。
#[derive(Debug)]
pub struct WallClock {
    epoch: std::time::Instant,
}

impl Default for WallClock {
    fn default() -> Self {
        Self::new()
    }
}

impl WallClock {
    pub fn new() -> Self {
        Self { epoch: std::time::Instant::now() }
    }
    pub fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

/// 自愈检测器：滑动窗字节量 + 降级态机。
///
/// 时间全由调用方注入（毫秒单调即可，不要求真实时钟）——单测/快照/接线层
/// 各自可控，本结构零时钟零随机。
#[derive(Debug)]
pub struct SelfHeal {
    window_bytes: usize,
    /// 滑动窗样本（按 at_ms 递增；VecDeque=evict pop_front O(1)——task-20②
    /// 计数器化：全窗 iter().sum() O(W) 每 delta 一遍收口为增量维护）。
    samples: std::collections::VecDeque<Sample>,
    /// 窗口内字节量增量计数（task-20②）：滑入加/滑出减，判定 O(1) 读——
    /// 不变量=window_sum == samples.iter().map(bytes).sum()（测试锁）。
    window_sum: usize,
    /// 墙钟外观（task-48 接线）：生产路径 `note_rewrite_wall` 用。
    clock: WallClock,
    /// 降级态：true=暂态静默（YUNYOU_STATIC 语义）。
    degraded: bool,
    /// 降级告知是否已发（一次性）。
    noticed: bool,
    /// 连续健康回合计数（恢复判定）。
    healthy_turns: u32,
    /// 显式 env 强制（YUNYOU_STATIC=1）：永不自动恢复。
    forced: bool,
}

impl Default for SelfHeal {
    fn default() -> Self {
        Self::new(false, DEFAULT_WINDOW_BYTES)
    }
}

impl SelfHeal {
    /// 构造：`forced`=YUNYOU_STATIC=1（env 由接线层读取传入，本模块不碰 env
    /// ——零环境依赖纪律）；`threshold_bytes`=窗口阈值（默认
    /// [`DEFAULT_WINDOW_BYTES`]，接线层日后可自配置透传）。
    pub fn new(forced: bool, threshold_bytes: usize) -> Self {
        Self {
            window_bytes: threshold_bytes,
            samples: std::collections::VecDeque::new(),
            window_sum: 0,
            clock: WallClock::new(),
            degraded: forced, // 显式档：出厂即静态，永不自动恢复
            noticed: forced,  // 显式档无「降级告知」事件（用户自己选的）
            healthy_turns: 0,
            forced,
        }
    }

    /// 原位重绘发生（口径内）：记入滑动窗。`bytes`=该次重写的完整字节量
    /// （含 `\r\e[K` 前缀与重写行本体）。
    pub fn note_rewrite(&mut self, bytes: usize, now_ms: u64) {
        // 零样本不入窗（task-20②）：0 字节重绘=无重绘发生——旧形态 0 样本
        // 照 push 使 W 膨胀（高频流式下 evict+sum 全窗 O(W) 扫 0 值垃圾）。
        if bytes == 0 {
            return;
        }
        self.samples.push_back(Sample { at_ms: now_ms, bytes });
        self.window_sum += bytes;
        self.evict(now_ms);
    }

    /// 冻结落定/纯追加发生（**口径外**）：不移动窗口——仅为调用方语义显式化
    /// 与单测对照保留；实现零操作（锁死口径防放宽的另一半）。
    pub fn note_appended(&mut self, _bytes: usize, _now_ms: u64) {}

    /// 墙钟生产外观（task-48 接线）：记账+滑窗判定+触发判定一体——返回
    /// `Some(告知)`=本次刚降级（接线层发一次性人话）；已降级返回 None。
    pub fn note_rewrite_wall(&mut self, bytes: usize) -> Option<&'static str> {
        let now = self.clock.now_ms();
        // 零样本不入窗（task-20②）：追加/冻结路径的 0 记账不再 push（W 膨胀
        // 根因）——evict 照跑保持 5s 窗时效（陈旧样本出窗），判定结果与旧
        // 逻辑等价（0 字节样本对 sum 贡献为零，检查密度降但正确性不变）。
        if bytes > 0 {
            self.samples.push_back(Sample { at_ms: now, bytes });
            self.window_sum += bytes;
        }
        self.evict(now);
        let total = self.window_sum;
        if total > self.window_bytes {
            self.healthy_turns = 0;
            if !self.degraded && !self.forced {
                self.degraded = true;
                self.noticed = false;
            }
        }
        self.take_notice()
    }

    /// 回合界墙钟外观：清窗重探+连续健康恢复（接线层在回合完结处调用）。
    /// 返回 true=本回合恢复交互态。
    pub fn new_turn_wall(&mut self) -> bool {
        self.new_turn(self.clock.now_ms())
    }

    /// 滑窗判定（每次重绘后调用）：窗口内字节量超阈 → 一次性降级。
    /// 返回 true=本次调用**刚**触发降级（接线层据此发一次性告知）；已降级态
    /// 再超阈返回 false，但**健康回合计数清零**（降级回合内又泛滥=该回合不
    /// 健康防抖动，恢复时钟重走）。
    pub fn poll_degrade(&mut self, now_ms: u64) -> bool {
        self.evict(now_ms);
        let total = self.window_sum;
        if total > self.window_bytes {
            self.healthy_turns = 0;
            if !self.degraded && !self.forced {
                self.degraded = true;
                self.noticed = false; // 待接线层发告知
                return true;
            }
        }
        false
    }

    /// 降级告知消费（一次性）：接线层取走文案后置位，不重复发。
    pub fn take_notice(&mut self) -> Option<&'static str> {
        if self.degraded && !self.noticed {
            self.noticed = true;
            return Some(DEGRADE_NOTICE);
        }
        None
    }

    /// 回合界（REPL 每轮对话开始/结束各算一界）：清窗重探 + 连续健康恢复。
    /// 返回 true=本轮回合恢复交互态（接线层据此还原 interactive）。
    pub fn new_turn(&mut self, now_ms: u64) -> bool {
        self.evict(now_ms);
        self.samples.clear();
        self.window_sum = 0;
        if self.forced {
            return false; // 显式档永不自动恢复
        }
        if self.degraded {
            // 上一回合若全程未再超阈（窗口已被回合界清空=无新增样本），
            // 记一次健康回合；连续达标即恢复。
            self.healthy_turns += 1;
            if self.healthy_turns >= RECOVER_AFTER_TURNS {
                self.degraded = false;
                self.healthy_turns = 0;
                return true;
            }
        }
        false
    }

    /// 当前是否降级态（接线层 interactive 判定的 OR 项）。
    pub fn is_degraded(&self) -> bool {
        self.degraded
    }

    /// 滑窗过期样本清除（5s 窗口语义；task-20② 计数器化）：样本按 at_ms
    /// 递增入队——过期段恒在前缀，pop_front 逐个扣减 window_sum（O(过期数)
    /// 摊销，替代 retain 全窗扫+重建）。
    fn evict(&mut self, now_ms: u64) {
        while let Some(front) = self.samples.front() {
            if now_ms.saturating_sub(front.at_ms) <= WINDOW_MS {
                break;
            }
            self.window_sum -= front.bytes;
            self.samples.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 口径锁·正例：窗口内重绘字节超阈 → 一次性降级告知。
    #[test]
    fn 超阈_一次性降级() {
        let mut h = SelfHeal::new(false, 1024);
        // 1024B/s × 2s = 2048B > 1024 阈
        h.note_rewrite(600, 0);
        h.note_rewrite(600, 1_000);
        assert!(h.poll_degrade(1_500), "超阈应触发降级");
        assert!(h.is_degraded());
        assert_eq!(h.take_notice(), Some(DEGRADE_NOTICE), "告知一次性");
        assert_eq!(h.take_notice(), None, "二次取告知应 None（不重复刷屏）");
    }

    /// 口径锁·冻结/追加不计入（防放宽主锁）：同样的字节量走 note_appended
    /// 永不触发——健康流式的冻结落定内容本就不进口径。
    #[test]
    fn 冻结与追加_不计入口径() {
        let mut h = SelfHeal::new(false, 1024);
        for t in 0..10 {
            h.note_appended(600, t * 400); // 10×600B 全走追加口径
        }
        assert!(!h.poll_degrade(4_000), "追加口径不得触发降级");
        // 对照组：同量走重绘口径即触发（口径分流的判别力）
        let mut h2 = SelfHeal::new(false, 1024);
        for t in 0..10 {
            h2.note_rewrite(600, t * 400);
        }
        assert!(h2.poll_degrade(4_000), "同量重绘口径应触发（判别力）");
    }

    /// 滑窗过期：5s 前的重绘字节不参与判定（窗口语义）。
    #[test]
    fn 滑窗过期_不累积() {
        let mut h = SelfHeal::new(false, 1024);
        h.note_rewrite(2_000, 0); // 单次即超阈，但立刻过期
        assert!(!h.poll_degrade(5_500), "过期样本不得参与判定");
        h.note_rewrite(10, 5_600); // 窗口内仅 10B
        assert!(!h.poll_degrade(5_700), "窗内少量不触发");
    }

    /// 回合界重探：降级后新回合清窗；连续 3 健康回合自动恢复；期间重绘超阈
    /// 则健康计数清零重来（防抖动）。
    #[test]
    fn 回合界_三健康回合恢复_超阈清零() {
        let mut h = SelfHeal::new(false, 1024);
        h.note_rewrite(2_000, 0);
        assert!(h.poll_degrade(100), "首触发");
        // 回合 1：无重绘（回合界已清样本）→ 健康 1
        assert!(!h.new_turn(6_000), "健康 1/3 不恢复");
        // 回合 2 中途再超阈 → 已降级不重复告知，但健康计数清零
        h.note_rewrite(2_000, 7_000);
        assert!(!h.poll_degrade(7_100), "已降级态不重复触发");
        assert!(!h.new_turn(12_000), "超阈后健康计数应清零（1/3）");
        assert!(!h.new_turn(18_000), "健康 2/3");
        assert!(h.new_turn(24_000), "连续 3 健康回合应恢复交互态");
        assert!(!h.is_degraded());
    }

    /// 显式档（YUNYOU_STATIC=1）：出厂即静态、永不自动恢复、无降级告知事件。
    #[test]
    fn 显式档_永不自动恢复() {
        let mut h = SelfHeal::new(true, 1024);
        assert!(h.is_degraded(), "显式档出厂即静态");
        assert_eq!(h.take_notice(), None, "用户自选无告知事件");
        for t in 0..10 {
            h.new_turn(t * 6_000); // 任意多健康回合
        }
        assert!(h.is_degraded(), "显式档永不自动恢复");
    }

    /// 阈值注入：接线层日后自配置透传的形状预验（本批用默认常量）。
    #[test]
    fn 阈值可注入() {
        let mut h = SelfHeal::new(false, 10);
        h.note_rewrite(11, 0);
        assert!(h.poll_degrade(100), "小阈值下 11B 即超");
    }
}

/// task-20② 防回潮与一致性锁（追加于 tests mod 尾）。
#[cfg(test)]
mod counter_tests {
    use super::*;

    /// 窗内样本数访问器（测试断言用；生产零开销）。
    #[cfg(test)]
    impl SelfHeal {
        fn window_len(&self) -> usize {
            self.samples.len()
        }
        fn window_sum_snapshot(&self) -> usize {
            self.window_sum
        }
    }

    /// 防回潮锁（卡面验收「零样本不入窗」）：0 字节记账不得入窗——高频流式
    /// 下 0 值垃圾样本曾使 W 膨胀 2-3 倍（挑刺 P2-2 实测形态）。
    #[test]
    fn 零字节记账不入窗() {
        let mut h = SelfHeal::new(false, 1024);
        h.note_rewrite_wall(0); // 追加路径 0 记账
        h.note_rewrite_wall(0);
        assert_eq!(h.window_len(), 0, "0 字节样本不得入窗（W 膨胀根因）");
        assert_eq!(h.window_sum_snapshot(), 0);
        // 非零样本正常入窗
        h.note_rewrite_wall(100);
        assert_eq!(h.window_len(), 1);
        assert_eq!(h.window_sum_snapshot(), 100);
    }

    /// 增量计数一致性锁：window_sum ≡ 逐样本和（不变量——滑入加/滑出减/
    /// clear 清零三路径后仍成立）。
    #[test]
    fn 增量计数与逐样本和一致() {
        let mut h = SelfHeal::new(false, 100_000);
        for t in 0..50u64 {
            h.note_rewrite(37 + t as usize, t * 100); // 注入版：确定性时间
        }
        let iter_sum: usize = h.samples.iter().map(|s| s.bytes).sum();
        assert_eq!(h.window_sum_snapshot(), iter_sum, "滑入后一致性");
        h.evict(50 * 100 + WINDOW_MS + 1); // 全部过期（WINDOW_MS 本即 u64，冗余 cast 清——lead 两行机械修②）
        assert_eq!(h.window_sum_snapshot(), 0, "全过期后 sum=0");
        assert_eq!(h.window_len(), 0, "全过期后窗空");
    }

    /// 行为等价抽样：同场景（重绘超阈→降级→0 记账流→回合界恢复）新旧逻辑
    /// 判定一致——既有 selfheal tests 七条为等价主证据，本条补 0 记账流穿插。
    #[test]
    fn 零记账穿插_判定等价() {
        let mut h = SelfHeal::new(false, 1024);
        h.note_rewrite_wall(600); // 非 0：入窗
        h.note_rewrite_wall(0); // 0：不入窗不改变 sum
        h.note_rewrite_wall(600); // 非 0：累计 1200>1024 → 触发
        assert!(h.is_degraded(), "0 记账穿插不得改变触发判定");
        assert_eq!(h.window_sum_snapshot(), 1200);
        // 0 记账流期间窗时效保持（evict 照跑）——5s 后旧样本过期
        h.note_rewrite_wall(0);
        // （时钟由 wall 内部推进，此处依赖 note 内 now 单调——过期验证由
        // 「滑窗过期_不累积」既有测试承担，本条聚焦 0 穿插。）
    }

    /// CPU 对比（打印呈现，不断言——云尺 wall time 纪律）：计数器化前每
    /// delta O(W)（retain 全窗+sum 全窗），化后 O(过期数) 摊销+O(1) 判定。
    #[test]
    fn cpu_对比打印() {
        let mut h = SelfHeal::new(false, 256 * 1024);
        let started = std::time::Instant::now();
        for _ in 0..2000 {
            h.note_rewrite_wall(120 + (started.elapsed().subsec_millis() as usize % 50));
        }
        let elapsed = started.elapsed();
        println!(
            "2000 次 note（W≈2000 窗）计数器化后耗时 {elapsed:?}——旧形态同场景为 \
             2000×O(W)≈2×10^6 次迭代（retain+sum 全窗各一遍），推演降幅 ≈W 倍"
        );
    }
}
