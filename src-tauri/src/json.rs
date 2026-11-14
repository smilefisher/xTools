//! JSON 工具：格式化 / 压缩 / 键排序 / 校验 / JSONPath 查询。
//!
//! 全部在 Rust 侧完成，前端只负责显示与编辑；大文件先用体积闸门挡住，
//! tape 懒解析（>20MB 不建整树）留到后续版本。

use serde::Serialize;
use serde_json::{Map, Value};

/// 超过这个体积直接拒绝，避免把 UI 拖死
const MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct JsonError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

impl JsonError {
    fn from_serde(err: &serde_json::Error) -> Self {
        Self {
            message: err.to_string(),
            line: err.line(),
            column: err.column(),
        }
    }

    fn plain(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            line: 0,
            column: 0,
        }
    }
}

fn parse(text: &str) -> Result<Value, JsonError> {
    if text.len() > MAX_BYTES {
        return Err(JsonError::plain(format!(
            "内容超过 {} MB，暂不支持整树解析",
            MAX_BYTES / 1024 / 1024
        )));
    }
    serde_json::from_str(text).map_err(|err| JsonError::from_serde(&err))
}

#[derive(Debug, Serialize)]
pub struct JsonResult {
    pub ok: bool,
    pub output: String,
    pub error: Option<JsonError>,
    pub elapsed_ms: f64,
    pub input_bytes: usize,
    pub output_bytes: usize,
}

fn ok_result(output: String, elapsed_ms: f64, input_bytes: usize) -> JsonResult {
    let output_bytes = output.len();
    JsonResult {
        ok: true,
        output,
        error: None,
        elapsed_ms,
        input_bytes,
        output_bytes,
    }
}

fn err_result(error: JsonError, elapsed_ms: f64, input_bytes: usize) -> JsonResult {
    JsonResult {
        ok: false,
        output: String::new(),
        error: Some(error),
        elapsed_ms,
        input_bytes,
        output_bytes: 0,
    }
}

/// 格式化；`indent` 传 2 / 4 表示空格数，传 "tab" 表示制表符。
pub fn format(text: &str, indent: &str, sort_keys: bool) -> JsonResult {
    let started = std::time::Instant::now();
    let input_bytes = text.len();
    let elapsed = || started.elapsed().as_secs_f64() * 1000.0;

    let value = match parse(text) {
        Ok(value) => value,
        Err(err) => return err_result(err, elapsed(), input_bytes),
    };

    let unit = match indent.trim() {
        "tab" | "\\t" => "\t".to_string(),
        other => {
            let width = other.parse::<usize>().unwrap_or(2).clamp(1, 8);
            " ".repeat(width)
        }
    };

    let mut out = String::with_capacity(input_bytes + input_bytes / 2);
    write_value(&value, &mut out, &unit, 0, sort_keys);
    ok_result(out, elapsed(), input_bytes)
}

pub fn minify(text: &str) -> JsonResult {
    let started = std::time::Instant::now();
    let input_bytes = text.len();
    let elapsed = || started.elapsed().as_secs_f64() * 1000.0;

    match parse(text) {
        Ok(value) => {
            let out = serde_json::to_string(&value).unwrap_or_default();
            ok_result(out, elapsed(), input_bytes)
        }
        Err(err) => err_result(err, elapsed(), input_bytes),
    }
}

pub fn validate(text: &str) -> JsonResult {
    let started = std::time::Instant::now();
    let input_bytes = text.len();
    let elapsed = || started.elapsed().as_secs_f64() * 1000.0;

    match parse(text) {
        Ok(value) => {
            let summary = format!(
                "合法 JSON · 根类型 {} · 体积 {}",
                type_name(&value),
                human_size(input_bytes)
            );
            ok_result(summary, elapsed(), input_bytes)
        }
        Err(err) => err_result(err, elapsed(), input_bytes),
    }
}

/// JSONPath 子集：`$.a.b[0]`、`$.list[*].name`、`$..key`（递归找键）。
pub fn query(text: &str, path: &str) -> JsonResult {
    let started = std::time::Instant::now();
    let input_bytes = text.len();
    let elapsed = || started.elapsed().as_secs_f64() * 1000.0;

    let value = match parse(text) {
        Ok(value) => value,
        Err(err) => return err_result(err, elapsed(), input_bytes),
    };

    let tokens = match tokenize_path(path) {
        Ok(tokens) => tokens,
        Err(err) => return err_result(err, elapsed(), input_bytes),
    };

    let mut current = vec![value];
    let mut recursive_key: Option<String> = None;

    for token in tokens {
        match token {
            Token::Key(key) => {
                current = current
                    .into_iter()
                    .filter_map(|value| match value {
                        Value::Object(map) => map.get(&key).cloned(),
                        _ => None,
                    })
                    .collect();
            }
            Token::Index(index) => {
                current = current
                    .into_iter()
                    .filter_map(|value| match value {
                        Value::Array(items) => items.get(index as usize).cloned(),
                        _ => None,
                    })
                    .collect();
            }
            Token::Wildcard => {
                current = current
                    .into_iter()
                    .flat_map(|value| match value {
                        Value::Array(items) => items,
                        _ => vec![],
                    })
                    .collect();
            }
            Token::Recursive(key) => {
                recursive_key = Some(key);
            }
        }
    }

    if let Some(key) = recursive_key {
        let mut found = Vec::new();
        collect_recursive(&current, &key, &mut found);
        current = found;
    }

    let output = match current.len() {
        0 => "未命中".to_string(),
        1 => serde_json::to_string_pretty(&current[0]).unwrap_or_default(),
        _ => serde_json::to_string_pretty(&Value::Array(current)).unwrap_or_default(),
    };
    ok_result(output, elapsed(), input_bytes)
}

fn collect_recursive(values: &[Value], key: &str, out: &mut Vec<Value>) {
    for value in values {
        match value {
            Value::Object(map) => {
                if let Some(found) = map.get(key) {
                    out.push(found.clone());
                }
                for child in map.values() {
                    collect_recursive(std::slice::from_ref(child), key, out);
                }
            }
            Value::Array(items) => collect_recursive(items, key, out),
            _ => {}
        }
    }
}

enum Token {
    Key(String),
    Index(i64),
    Wildcard,
    Recursive(String),
}

fn tokenize_path(path: &str) -> Result<Vec<Token>, JsonError> {
    let mut chars = path.trim().chars().peekable();
    if chars.peek() == Some(&'$') {
        chars.next();
    }

    let mut tokens = Vec::new();
    let mut buffer = String::new();

    let flush_key = |buffer: &mut String, tokens: &mut Vec<Token>| {
        if !buffer.is_empty() {
            tokens.push(Token::Key(std::mem::take(buffer)));
        }
    };

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                flush_key(&mut buffer, &mut tokens);
                if chars.peek() == Some(&'.') {
                    chars.next();
                    let mut key = String::new();
                    while let Some(&next) = chars.peek() {
                        if next == '.' || next == '[' {
                            break;
                        }
                        key.push(next);
                        chars.next();
                    }
                    if key.is_empty() {
                        return Err(JsonError::plain("$.. 后面要跟键名"));
                    }
                    tokens.push(Token::Recursive(key));
                }
            }
            '[' => {
                flush_key(&mut buffer, &mut tokens);
                let mut inner = String::new();
                for next in chars.by_ref() {
                    if next == ']' {
                        break;
                    }
                    inner.push(next);
                }
                let inner = inner.trim();
                if inner == "*" {
                    tokens.push(Token::Wildcard);
                } else if let Ok(index) = inner.parse::<i64>() {
                    tokens.push(Token::Index(if index < 0 { 0 } else { index }));
                } else {
                    let key = inner.trim_matches(|c| c == '\'' || c == '"').to_string();
                    if key.is_empty() {
                        return Err(JsonError::plain("[] 里要写下标或键名"));
                    }
                    tokens.push(Token::Key(key));
                }
            }
            other => buffer.push(other),
        }
    }
    flush_key(&mut buffer, &mut tokens);
    Ok(tokens)
}

fn write_value(value: &Value, out: &mut String, indent: &str, level: usize, sort_keys: bool) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => out.push_str(&number.to_string()),
        Value::String(text) => out.push_str(&serde_json::to_string(text).unwrap_or_default()),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                push_indent(out, indent, level + 1);
                write_value(item, out, indent, level + 1, sort_keys);
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent, level);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{\n");
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            if sort_keys {
                entries.sort_by(|a, b| a.0.cmp(b.0));
            }
            for (index, (key, item)) in entries.iter().enumerate() {
                push_indent(out, indent, level + 1);
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push_str(": ");
                write_value(item, out, indent, level + 1, sort_keys);
                if index + 1 < entries.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            push_indent(out, indent, level);
            out.push('}');
        }
    }
}

fn push_indent(out: &mut String, indent: &str, level: usize) {
    for _ in 0..level {
        out.push_str(indent);
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn human_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1048576.0)
    }
}

/// 让 Map 保持插入顺序的辅助（serde_json 开了 preserve_order 后即为此行为）。
#[allow(dead_code)]
fn ordered_map() -> Map<String, Value> {
    Map::new()
}

/// 树视图节点（扁平数组，前端按 parent/depth 渲染折叠）。
#[derive(Debug, Serialize)]
pub struct TreeNode {
    pub id: usize,
    pub parent: i64,
    pub depth: usize,
    pub key: String,
    pub kind: &'static str,
    pub preview: String,
    pub path: String,
    pub has_children: bool,
}

#[derive(Debug, Serialize)]
pub struct JsonTree {
    pub ok: bool,
    pub nodes: Vec<TreeNode>,
    pub total: usize,
    pub truncated: bool,
    pub error: Option<JsonError>,
    pub elapsed_ms: f64,
}

/// 把 JSON 压成扁平节点列表；节点数超上限就截断（前端后续可懒展开）。
pub fn tree(text: &str, limit: usize) -> JsonTree {
    let started = std::time::Instant::now();
    let value = match parse(text) {
        Ok(value) => value,
        Err(err) => {
            return JsonTree {
                ok: false,
                nodes: Vec::new(),
                total: 0,
                truncated: false,
                error: Some(err),
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
            };
        }
    };

    let limit = limit.clamp(16, 20_000);
    let mut nodes = Vec::new();
    let mut total = 0usize;
    walk_tree(&value, "$", 0, -1, &mut nodes, &mut total, limit);

    JsonTree {
        ok: true,
        truncated: total > nodes.len(),
        total,
        nodes,
        error: None,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
    }
}

fn walk_tree(
    value: &Value,
    path: &str,
    depth: usize,
    parent: i64,
    out: &mut Vec<TreeNode>,
    total: &mut usize,
    limit: usize,
) {
    *total += 1;
    if out.len() >= limit {
        return;
    }

    let id = out.len();
    let has_children = match value {
        Value::Object(map) => !map.is_empty(),
        Value::Array(items) => !items.is_empty(),
        _ => false,
    };

    out.push(TreeNode {
        id,
        parent,
        depth,
        key: display_key(path),
        kind: type_name(value),
        preview: preview_of(value),
        path: path.to_string(),
        has_children,
    });

    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{path}.{key}");
                walk_tree(child, &child_path, depth + 1, id as i64, out, total, limit);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let child_path = format!("{path}[{index}]");
                walk_tree(child, &child_path, depth + 1, id as i64, out, total, limit);
            }
        }
        _ => {}
    }
}

/// 从路径里截出这一段的名字：`$.a.b[2]` → `[2]`。
fn display_key(path: &str) -> String {
    if path == "$" {
        return "$".into();
    }
    if let Some(index) = path.rfind('[') {
        return path[index..].to_string();
    }
    match path.rfind('.') {
        Some(index) => path[index + 1..].to_string(),
        None => path.to_string(),
    }
}

fn preview_of(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => {
            let clipped: String = text.chars().take(80).collect();
            if text.chars().count() > 80 {
                format!("\"{clipped}…\"")
            } else {
                format!("\"{clipped}\"")
            }
        }
        Value::Array(items) => format!("[{} 项]", items.len()),
        Value::Object(map) => format!("{{{} 个键}}", map.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_flattens_with_paths() {
        let result = tree(r#"{"a":{"b":[1,2]},"c":true}"#, 100);
        assert!(result.ok);
        assert!(!result.truncated);
        // 根 + a + b + 2 个元素 + c
        assert_eq!(result.nodes.len(), 6);
        let paths: Vec<&str> = result.nodes.iter().map(|n| n.path.as_str()).collect();
        assert!(paths.contains(&"$.a.b[0]"));
        assert!(paths.contains(&"$.c"));
        assert_eq!(result.nodes[1].depth, 1);
        assert!(result.nodes[1].has_children);
    }

    #[test]
    fn tree_truncates() {
        let big = format!(
            "[{}]",
            (0..500).map(|i| i.to_string()).collect::<Vec<_>>().join(",")
        );
        let result = tree(&big, 32);
        assert!(result.ok);
        assert!(result.truncated);
        assert!(result.nodes.len() <= 32);
    }

    #[test]
    fn tree_reports_error() {
        let result = tree("{oops", 10);
        assert!(!result.ok);
        assert!(result.error.is_some());
    }

    #[test]
    fn format_preserves_order() {
        let input = r#"{"b":1,"a":[1,2]}"#;
        let result = format(input, "2", false);
        assert!(result.ok);
        assert!(result.output.starts_with("{\n  \"b\": 1,"));
    }

    #[test]
    fn format_sorts_keys() {
        let result = format(r#"{"b":1,"a":2}"#, "2", true);
        assert!(result.ok);
        let a = result.output.find("\"a\"").unwrap();
        let b = result.output.find("\"b\"").unwrap();
        assert!(a < b);
    }

    #[test]
    fn minify_compact() {
        let result = minify("{\n \"a\": [1, 2]\n}");
        assert!(result.ok);
        assert_eq!(result.output, r#"{"a":[1,2]}"#);
    }

    #[test]
    fn reports_error_position() {
        let result = format("{\n  \"a\": ,\n}", "2", false);
        assert!(!result.ok);
        let err = result.error.unwrap();
        assert_eq!(err.line, 2);
        assert!(err.column > 0);
    }

    #[test]
    fn query_paths() {
        let doc = r#"{"list":[{"name":"a"},{"name":"b"}],"meta":{"name":"m"}}"#;
        let result = query(doc, "$.list[*].name");
        assert!(result.ok);
        assert!(result.output.contains("\"a\"") && result.output.contains("\"b\""));

        let single = query(doc, "$.meta.name");
        assert_eq!(single.output, "\"m\"");

        let recursive = query(doc, "$..name");
        assert!(recursive.output.contains("\"m\""));
    }

    #[test]
    fn rejects_non_object_index() {
        let result = query(r#"{"a":[1]}"#, "$.a.b");
        assert!(result.ok);
        assert_eq!(result.output, "未命中");
    }
}
