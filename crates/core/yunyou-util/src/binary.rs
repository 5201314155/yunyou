//! LEB128 无符号变长整数编解码。
//!
//! 方案 §2.9 为 yunyou-util 预留的「二进制编码」位：一期内核不直接使用，
//! 为阶段 3 子进程插件协议（JSON-RPC over stdio 的帧长度前缀，方案 §2.7.1）
//! 与未来 dsh-procmgr 提供统一的编解码实现，避免各写一份。

/// 把 `value` 以 LEB128 变长编码追加到 `buf`（每字节低 7 位承载数据，最高位为续传标志）。
pub fn encode_u64(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            buf.push(byte);
            return;
        }
        buf.push(byte | 0x80);
    }
}

/// 从 `buf` 的 `offset` 处解码一个 LEB128 变长整数（**规范形态严格模式**）。
///
/// 返回 `(值, 消耗字节数)`。以下情形一律返回 `None`（协议面前缀不容歧义形态，
/// 同一长度只允许一种字节序列）：
/// - 缓冲区过早结束；
/// - 超过 10 字节（u64 上限）；
/// - 第 10 字节高位参与数据（超出 u64 位宽，防静默截断）——只接受 `0x00/0x01`；
/// - 非规范编码（冗余的续传前导：末字节数据位为 0 但前一数据字节本可终止序列）。
pub fn decode_u64(buf: &[u8], offset: usize) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    let mut consumed = 0usize;
    loop {
        let byte = *buf.get(offset + consumed)?;
        consumed += 1;
        if shift >= 63 {
            // 第 10 字节：只剩最高 1 位可用，高位参与即截断损耗，拒绝
            if byte & 0x7f > 1 {
                return None;
            }
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            // 规范性检查：末字节数据位为 0 且不是首字节 → 前序字节携带了本可
            // 省略的续传标记（如 [0x80,0x00] 编码 0、[0x81,0x00] 编码 1），拒绝
            if byte & 0x7f == 0 && consumed > 1 {
                return None;
            }
            return Some((value, consumed));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_u64, encode_u64};

    #[test]
    fn 往返编解码() {
        let cases = [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX];
        for value in cases {
            let mut buf = Vec::new();
            encode_u64(&mut buf, value);
            let (decoded, used) = decode_u64(&buf, 0).expect("应可解码");
            assert_eq!(decoded, value, "值 {value} 往返不一致");
            assert_eq!(used, buf.len(), "应恰好消耗全部字节");
        }
    }

    #[test]
    fn 边界与容错() {
        // 0 与 127 都是单字节
        let mut buf = Vec::new();
        encode_u64(&mut buf, 0);
        assert_eq!(buf, vec![0x00]);
        buf.clear();
        encode_u64(&mut buf, 127);
        assert_eq!(buf, vec![0x7f]);
        // 截断的缓冲区返回 None
        assert_eq!(decode_u64(&[0x80], 0), None);
        // 超长序列（>10 字节）拒绝
        assert_eq!(decode_u64(&[0xff; 12], 0), None);
        // 带偏移的解码
        let mut with_prefix = vec![0xaa, 0xbb];
        encode_u64(&mut with_prefix, 300);
        let (v, used) = decode_u64(&with_prefix, 2).expect("偏移后应可解码");
        assert_eq!((v, used), (300, 2));
    }

    #[test]
    fn 规范性拒绝() {
        // 非规范编码拒绝：冗余前导 0x80（两字节编码 0）
        assert_eq!(decode_u64(&[0x80, 0x00], 0), None);
        // 非规范编码拒绝：末字节为 0 的非零值（[0x81,0x00] 应为 [0x01]）
        assert_eq!(decode_u64(&[0x81, 0x00], 0), None);
        // 链式冗余前导同样拒绝（末字节为 0：[0x80,0x80,0x00] 应为 [0x01]）
        assert_eq!(decode_u64(&[0x80, 0x80, 0x00], 0), None);
        // 对照：[0x80,0x80,0x01] 是 16384 的规范编码，正常解码
        assert_eq!(decode_u64(&[0x80, 0x80, 0x01], 0), Some((16384, 3)));
        // 第 10 字节高位参与数据：拒绝（防 u64 静默截断），只收 0x00/0x01
        let ten_bytes_high: Vec<u8> = [0xff_u8; 9].into_iter().chain([0x7f]).collect();
        assert_eq!(decode_u64(&ten_bytes_high, 0), None);
        // 第 10 字节 = 0x01 合法：u64::MAX 的完整 10 字节编码
        let mut buf = Vec::new();
        encode_u64(&mut buf, u64::MAX);
        assert_eq!(buf.len(), 10);
        assert_eq!(decode_u64(&buf, 0), Some((u64::MAX, 10)));
        // 规范形态正常解码不受影响
        assert_eq!(decode_u64(&[0x7f], 0), Some((127, 1)));
        assert_eq!(decode_u64(&[0xff, 0x01], 0), Some((255, 2)));
    }
}
