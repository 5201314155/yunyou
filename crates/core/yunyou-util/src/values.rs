//! JSON 值的深比较工具（对标 cosmokit `types.ts:127` 的 `deepEqual`，严格形态）。
//!
//! 语义：结构递归相等；数值跨整型/浮点按 JS 数值域比较（`1` 与 `1.0` 相等）；
//! 对象键集合不同即不等（与键序无关）；`Null` 仅与 `Null` 相等。
//! serde_json 的 `Value` 不可能成环，无需 TS 版的祖先防环集。

use serde_json::Value;

/// 深比较两个 JSON 值是否相等。
pub fn deep_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Number(x), Value::Number(y)) => number_equal(x, y),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(i, j)| deep_equal(i, j))
        }
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| deep_equal(v, w)))
        }
        _ => false,
    }
}

/// 数值比较：先精确相等（同变体），再跨变体对齐 JS 数值域（`1 === 1.0`）；
/// 双方都是整型变体时不回退 f64——避免超出 2^53 的大整数被有损抹平。
fn number_equal(x: &serde_json::Number, y: &serde_json::Number) -> bool {
    if x == y {
        return true;
    }
    let x_int = x.is_i64() || x.is_u64();
    let y_int = y.is_i64() || y.is_u64();
    if x_int && y_int {
        return false;
    }
    match (x.as_f64(), y.as_f64()) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::deep_equal;
    use serde_json::{json, Value};

    #[test]
    fn 嵌套结构与键序无关() {
        let a = json!({"b": [1, {"c": 2}], "a": "x"});
        let b = json!({"a": "x", "b": [1, {"c": 2}]});
        assert!(deep_equal(&a, &b));
    }

    #[test]
    fn 数值跨变体对齐_js_语义() {
        assert!(deep_equal(&json!(1), &json!(1.0)));
        assert!(!deep_equal(&json!(1), &json!(1.5)));
        // 超精度整数不做有损比较
        let big = json!(9_007_199_254_740_993u64);
        assert!(deep_equal(&big, &big));
        assert!(!deep_equal(&big, &json!(9_007_199_254_740_992u64)));
    }

    #[test]
    fn 结构差异与空值判定() {
        assert!(!deep_equal(&json!([1, 2]), &json!([1, 2, 3])));
        assert!(!deep_equal(&json!({"a": 1}), &json!({"a": 1, "b": 2})));
        assert!(deep_equal(&Value::Null, &Value::Null));
        assert!(!deep_equal(&Value::Null, &json!(0)));
        assert!(!deep_equal(&json!([1]), &json!("1")));
    }
}
