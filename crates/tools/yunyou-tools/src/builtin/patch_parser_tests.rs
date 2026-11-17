//! patch_parser 单元测试（task-22；文法三 hunk/宽容层/错误形态）。
use super::*;

#[test]
fn 三级hunk解析() {
    let patch = "*** Begin Patch\n*** Add File: new.txt\n+第一行\n+第二行\n*** Delete File: old.txt\n*** Update File: a.rs\n@@ fn main\n context 不变\n-旧实现\n+新实现\n*** End Patch";
    let hunks = parse_patch(patch).expect("应解析成功");
    assert_eq!(hunks.len(), 3);
    match &hunks[0] {
        Hunk::Add { path, lines } => {
            assert_eq!(path, "new.txt");
            assert_eq!(lines, &vec!["第一行".to_string(), "第二行".to_string()]);
        }
        other => panic!("应为 Add: {other:?}"),
    }
    assert!(matches!(&hunks[1], Hunk::Delete { path } if path == "old.txt"));
    match &hunks[2] {
        Hunk::Update { path, move_to, chunks } => {
            assert_eq!(path, "a.rs");
            assert!(move_to.is_none());
            assert_eq!(chunks.len(), 1);
            let c = &chunks[0];
            assert_eq!(c.change_context.as_deref(), Some("fn main"));
            assert_eq!(c.old_lines, vec!["context 不变".to_string(), "旧实现".to_string()]);
            assert_eq!(c.new_lines, vec!["context 不变".to_string(), "新实现".to_string()]);
            assert_eq!(c.context_pairs, vec![(0, 0)], "上下文行两侧索引对");
        }
        other => panic!("应为 Update: {other:?}"),
    }
}

#[test]
fn heredoc壳剥离与marker容错() {
    let wrapped = "<<'EOF'\n*** Begin Patch\n*** Add File: x\n+内容\n*** End Patch\nEOF";
    let hunks = parse_patch(wrapped).expect("heredoc 壳应剥离");
    assert!(matches!(&hunks[0], Hunk::Add { path, .. } if path == "x"));
    // marker 前后空白宽容
    let padded = "  *** Begin Patch  \n*** Add File: y\n+ok\n*** End Patch ";
    assert!(parse_patch(padded).is_ok());
}

#[test]
fn 错误形态带行号与示范() {
    // 首行非 Begin
    let e = parse_patch("随便什么").unwrap_err();
    assert_eq!(e.line, 1);
    assert!(e.message.contains("*** Begin Patch"), "{e:?}");
    // 非法 hunk 头（枚举三种合法形式）
    let e = parse_patch("*** Begin Patch\n*** 乱写\n*** End Patch").unwrap_err();
    assert!(e.message.contains("Add File") && e.message.contains("Update File"), "{e:?}");
    // 更新块行前缀非法（三前缀语义说明）
    let e = parse_patch("*** Begin Patch\n*** Update File: a\n@@\n无前缀行\n*** End Patch").unwrap_err();
    assert!(e.message.contains("' '"), "{e:?}");
    // 缺收尾
    let e = parse_patch("*** Begin Patch\n*** Add File: a\n+x").unwrap_err();
    assert!(e.message.contains("End Patch"), "{e:?}");
}

#[test]
fn move_to与end_of_file哨兵解析() {
    let patch = "*** Begin Patch\n*** Update File: a.rs\n*** Move to: b.rs\n@@\n-旧\n+新\n*** End of File\n*** End Patch";
    let hunks = parse_patch(patch).expect("应解析");
    match &hunks[0] {
        Hunk::Update { path, move_to, chunks } => {
            assert_eq!(path, "a.rs");
            assert_eq!(move_to.as_deref(), Some("b.rs"));
            assert!(chunks[0].is_end_of_file);
        }
        other => panic!("应为 Update: {other:?}"),
    }
}
