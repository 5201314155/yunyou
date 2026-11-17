//! 写后必验原语（task-70 P0）：write_all→sync_all→stat 对账三步。
//!
//! 背景：业主真机 CJK 文件名 write 报「成功写入 15520 字节」实未落盘——
//! 写调用 Ok≠落盘事实。本原语使静默失败在工具层结构性不可能：数据落介质
//! （fsync）+盘上字节数对账（不等=报错），回显字节数=stat 到的字节数。
use std::path::Path;

/// 写入内容并验证落盘：返回 stat 到的字节数（对账失败=Err，禁静默 ok）。
pub(crate) async fn write_verified(path: &Path, content: &[u8]) -> std::io::Result<u64> {
    use tokio::io::AsyncWriteExt;
    let mut f = tokio::fs::File::create(path).await?;
    f.write_all(content).await?;
    f.sync_all().await?;
    let meta = f.metadata().await?;
    let on_disk = meta.len();
    if on_disk != content.len() as u64 {
        return Err(std::io::Error::other(format!(
            "落盘对账失败：写入 {} 字节、盘上 {} 字节——静默成功已被拦截，先 read 核对再重试",
            content.len(),
            on_disk
        )));
    }
    Ok(on_disk)
}
