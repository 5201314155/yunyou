//! patch_match 单元测试（task-22；四级递降/游标/EOF 哨兵/行尾三态/拆段）。
use super::*;

fn src(text: &str) -> SourceFile {
    SourceFile::parse(text)
}

#[test]
fn 四级递降_精确到标点归一() {
    let f = src("fn a() {}\nfn b() {}\n");
    let pat = vec!["fn b() {}".to_string()];
    assert_eq!(seek_sequence(&f.lines, &pat, 0, false), Some(1));
    // 第 2 级：尾空白
    let f2 = src("fn b() {}   \n");
    assert_eq!(seek_sequence(&f2.lines, &pat, 0, false), Some(0), "trim_end 容忍");
    // 第 3 级：缩进漂移
    let f3 = src("    fn b() {}\n");
    assert_eq!(seek_sequence(&f3.lines, &pat, 0, false), Some(0), "双边 trim 容忍");
    // 第 4 级：Unicode dash 归一
    let f4 = src("fn b() {} — 注释\n");
    let pat4 = vec!["fn b() {} - 注释".to_string()];
    assert_eq!(seek_sequence(&f4.lines, &pat4, 0, false), Some(0), "EN DASH 归一");
    // 不存在 → None
    let f5 = src("完全不同\n");
    assert_eq!(seek_sequence(&f5.lines, &pat, 0, false), None);
}

#[test]
fn eof哨兵_尾对齐优先() {
    let f = src("head\nmid\ntail body\n");
    let pat = vec!["tail body".to_string()];
    assert_eq!(seek_sequence(&f.lines, &pat, 0, true), Some(2), "eof 声明=尾对齐优先");
}

#[test]
fn 空pattern即no_op() {
    let f = src("a\n");
    assert_eq!(seek_sequence(&f.lines, &[], 0, false), Some(0));
}

#[test]
fn 行尾三态保真_crlf不翻转() {
    let f = src("keep1\r\nchange me\r\nkeep2\r\n");
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: vec!["change me".into()],
        new_lines: vec!["changed".into()],
        context_pairs: Vec::new(),
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("应匹配");
    let out = apply_replacements(&f, &reps);
    assert_eq!(out, "keep1\r\nchanged\r\nkeep2\r\n", "CRLF 文件修改后行尾不得翻转");
}

#[test]
fn 上下文拆段_混合行尾只动变化行() {
    let f = src("a\nb\r\nc\nd\n");
    // chunk：a(上下文) b→B(替换) c(上下文) d→D(替换)
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: vec!["a".into(), "b".into(), "c".into(), "d".into()],
        new_lines: vec!["a".into(), "B".into(), "c".into(), "D".into()],
        context_pairs: vec![(0, 0), (2, 2)],
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("应匹配");
    assert_eq!(reps.len(), 2, "上下文行拆段：两段独立替换");
    assert_eq!(reps[0], Replacement { start: 1, end: 2, new_lines: vec!["B".into()] });
    let out = apply_replacements(&f, &reps);
    assert_eq!(out, "a\nB\r\nc\nD\n", "未动行保留原行尾（b 的 CRLF 已随段删除、B 用 preferred）");
}

#[test]
fn eof哨兵重试_尾空串形态() {
    let f = src("head\nold text\n");
    // 真实 diff 的 old 尾常带空串（终结换行的产物）——直搜失败去尾重搜
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: vec!["old text".into(), String::new()],
        new_lines: vec!["new text".into(), String::new()],
        context_pairs: Vec::new(),
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("去尾空串重搜应命中");
    let out = apply_replacements(&f, &reps);
    assert_eq!(out, "head\nnew text\n");
}

#[test]
fn 纯新增chunk_锚定文件尾() {
    let f = src("existing\n");
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: Vec::new(),
        new_lines: vec!["appended".into()],
        context_pairs: Vec::new(),
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("纯新增应记尾追加");
    let out = apply_replacements(&f, &reps);
    assert_eq!(out, "existing\nappended\n");
}

#[test]
fn 非对称增删_单趟行序推进不丢行() {
    // F-1 回归锁：old 2 行换 new 4 行（上下文夹非对称替换）——旧实现用 old
    // 侧偏移切 new 侧会静默丢行/越界
    let f = src("ctx1\nold1\nold2\nctx2\ntail\n");
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: vec!["ctx1".into(), "old1".into(), "old2".into(), "ctx2".into()],
        new_lines: vec!["ctx1".into(), "n1".into(), "n2".into(), "n3".into(), "n4".into(), "ctx2".into()],
        context_pairs: vec![(0, 0), (3, 5)],
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("应匹配");
    assert_eq!(reps.len(), 1);
    assert_eq!(reps[0].start, 1);
    assert_eq!(reps[0].end, 3);
    assert_eq!(reps[0].new_lines, vec!["n1", "n2", "n3", "n4"]);
    assert_eq!(apply_replacements(&f, &reps), "ctx1\nn1\nn2\nn3\nn4\nctx2\ntail\n");
}

#[test]
fn 非对称删多补一_中间段() {
    let f = src("a\nx\ny\nz\nb\n");
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: vec!["a".into(), "x".into(), "y".into(), "z".into(), "b".into()],
        new_lines: vec!["a".into(), "mid".into(), "b".into()],
        context_pairs: vec![(0, 0), (4, 2)],
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("应匹配");
    assert_eq!(reps.len(), 1);
    assert_eq!(reps[0].start, 1);
    assert_eq!(reps[0].end, 4);
    assert_eq!(reps[0].new_lines, vec!["mid".to_string()]);
    assert_eq!(apply_replacements(&f, &reps), "a\nmid\nb\n");
}

#[test]
fn 上下文后纯插入_不再静默丢行() {
    // F-1 伴生症：两个上下文行之间只有 '+' 行——旧实现 runs 为空、插入整段丢失
    let f = src("a\nb\n");
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: vec!["a".into(), "b".into()],
        new_lines: vec!["a".into(), "plus".into(), "b".into()],
        context_pairs: vec![(0, 0), (1, 2)],
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("应匹配");
    assert_eq!(reps.len(), 1, "纯插入段必须产零宽替换");
    assert_eq!(reps[0].start, reps[0].end);
    assert_eq!(reps[0].new_lines, vec!["plus".to_string()]);
    assert_eq!(apply_replacements(&f, &reps), "a\nplus\nb\n");
}

#[test]
fn 纯删除段_上下文夹全删() {
    let f = src("a\ndead1\ndead2\nb\n");
    let chunk = UpdateChunk {
        change_context: None,
        old_lines: vec!["a".into(), "dead1".into(), "dead2".into(), "b".into()],
        new_lines: vec!["a".into(), "b".into()],
        context_pairs: vec![(0, 0), (3, 1)],
        is_end_of_file: false,
    };
    let reps = compute_replacements(&f, &[chunk]).expect("应匹配");
    assert_eq!(reps.len(), 1);
    assert_eq!(reps[0].start, 1);
    assert_eq!(reps[0].end, 3);
    assert!(reps[0].new_lines.is_empty(), "纯删除=空 new_lines");
    assert_eq!(apply_replacements(&f, &reps), "a\nb\n");
}

#[test]
fn 游标单调推进_同文件多chunk不回退() {
    // 同一行内容出现两次：游标保证第二 chunk 从第一 chunk 之后匹配
    let f = src("dup\nx\ndup\ny\n");
    let c1 = UpdateChunk {
        change_context: None,
        old_lines: vec!["dup".into()],
        new_lines: vec!["D1".into()],
        context_pairs: Vec::new(),
        is_end_of_file: false,
    };
    let c2 = c1.clone();
    let reps = compute_replacements(&f, &[c1, c2]).expect("应匹配");
    assert_eq!(reps.len(), 2);
    assert_eq!(reps[0].start, 0, "第一 chunk 吃首个 dup");
    assert_eq!(reps[1].start, 2, "游标推进：第二 chunk 吃后续 dup 不回退");
}
