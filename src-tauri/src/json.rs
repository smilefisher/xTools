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

    // 校验走流式解析（IgnoredAny），不建整棵 Value 树：
    // 100MB 的文档也能校验，内存只占解析缓冲区。
    let mut stream = serde_json::Deserializer::from_str(text).into_iter::<serde::de::IgnoredAny>();
    let mut documents = 0usize;
    let mut failure = None;

    for item in stream.by_ref() {
        match item {
            Ok(_) => documents += 1,
            Err(err) => {
                failure = Some(JsonError::from_serde(&err));
                break;
            }
        }
    }

    if let Some(err) = failure {
        return err_result(err, elapsed(), input_bytes);
    }

    if documents == 0 {
        return err_result(
            JsonError::plain("没有解析到任何 JSON 内容"),
            elapsed(),
            input_bytes,
        );
    }

    let summary = format!(
        "合法 JSON · {} 个顶层文档 · 体积 {} · 流式校验",
        documents,
        human_size(input_bytes)
    );
    ok_result(summary, elapsed(), input_bytes)
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

/// 大文件大纲：不建整树，只扫出顶层结构（键名 + 类型 + 字节区间）。
#[derive(Debug, Serialize)]
pub struct OutlineNode {
    pub depth: u8,
    pub key: String,
    pub kind: &'static str,
    pub start: usize,
    pub end: usize,
    pub bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonOutline {
    pub ok: bool,
    pub nodes: Vec<OutlineNode>,
    pub truncated: bool,
    pub root_kind: &'static str,
    pub elapsed_ms: f64,
    pub error: Option<JsonError>,
}

/// 结构扫描器：只走一遍字节，跳过深层内容，内存 O(节点数)。
///
/// 用于 >20MB 的文档——整树解析会吃掉几百 MB，这里只取「有哪些键、各自多大」。
pub fn outline(text: &str, max_depth: u8, limit: usize) -> JsonOutline {
    let started = std::time::Instant::now();
    let bytes = text.as_bytes();
    let limit = limit.clamp(8, 50_000);
    let max_depth = max_depth.clamp(1, 4);

    let mut scanner = Scanner {
        bytes,
        pos: 0,
        nodes: Vec::new(),
        limit,
        max_depth,
        truncated: false,
        error: None,
    };

    scanner.skip_ws();
    let root_pos = scanner.pos;
    let root_kind = match scanner.peek() {
        Some(b'{') => "object",
        Some(b'[') => "array",
        Some(b'"') => "string",
        Some(_) => "scalar",
        None => {
            return JsonOutline {
                ok: false,
                nodes: Vec::new(),
                truncated: false,
                root_kind: "unknown",
                elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
                error: Some(JsonError::plain("内容为空")),
            }
        }
    };

    scanner.scan_value(0, String::from("$"));
    scanner.skip_ws();
    let end = scanner.pos;

    if scanner.nodes.is_empty() && scanner.error.is_none() {
        scanner.nodes.push(OutlineNode {
            depth: 0,
            key: "$".into(),
            kind: root_kind,
            start: root_pos,
            end,
            bytes: end.saturating_sub(root_pos),
        });
    }

    JsonOutline {
        ok: scanner.error.is_none(),
        nodes: scanner.nodes,
        truncated: scanner.truncated,
        root_kind,
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        error: scanner.error,
    }
}

struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
    nodes: Vec<OutlineNode>,
    limit: usize,
    max_depth: u8,
    truncated: bool,
    error: Option<JsonError>,
}

impl<'a> Scanner<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) {
        self.pos += 1;
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.bump();
            } else {
                break;
            }
        }
    }

    fn fail(&mut self, message: &str) {
        if self.error.is_none() {
            self.error = Some(JsonError::plain(message));
        }
    }

    fn scan_string(&mut self) -> Option<String> {
        // 进入时 pos 指向开引号
        if self.peek() != Some(b'"') {
            return None;
        }
        self.bump();
        let start = self.pos;
        while let Some(b) = self.peek() {
            match b {
                b'\\' => {
                    self.bump();
                    self.bump();
                }
                b'"' => {
                    let raw = &self.bytes[start..self.pos];
                    self.bump();
                    return Some(String::from_utf8_lossy(raw).into_owned());
                }
                _ => self.bump(),
            }
        }
        self.fail("字符串没有闭合");
        None
    }

    fn scan_scalar(&mut self) {
        while let Some(b) = self.peek() {
            if b == b',' || b == b'}' || b == b']' || b == b' ' || b == b'\n' || b == b'\r' || b == b'\t'
            {
                break;
            }
            self.bump();
        }
    }

    /// 扫描一个值，depth 为该值所在层级；返回其结束位置。
    fn scan_value(&mut self, depth: u8, key: String) {
        self.skip_ws();
        let start = self.pos;

        match self.peek() {
            Some(b'{') => {
                self.bump();
                // 容器在「开括号」时就登记，闭括号时补结束位置——
                // 否则大数组会把上限耗在子元素上，父节点反而记不下来
                let idx = self.begin(depth, key, "object", start);
                self.scan_object(depth, idx);
            }
            Some(b'[') => {
                self.bump();
                let idx = self.begin(depth, key, "array", start);
                self.scan_array(depth, idx);
            }
            Some(b'"') => {
                self.scan_string();
                self.record(depth, key, "string", start, self.pos);
            }
            Some(b't') | Some(b'f') | Some(b'n') => {
                self.scan_scalar();
                if self.pos == start {
                    self.fail("字面量异常");
                    return;
                }
                let kind = if self.bytes[start] == b'n' { "null" } else { "boolean" };
                self.record(depth, key, kind, start, self.pos);
            }
            Some(b'}') | Some(b']') | Some(b',') | Some(b':') => {
                self.fail("值的位置出现了结构字符");
            }
            Some(_) => {
                self.scan_scalar();
                if self.pos == start {
                    self.fail("数值解析失败");
                    return;
                }
                self.record(depth, key, "number", start, self.pos);
            }
            None => self.fail("内容意外结束"),
        }
    }

    fn begin(
        &mut self,
        depth: u8,
        key: String,
        kind: &'static str,
        start: usize,
    ) -> Option<usize> {
        if key.is_empty() || depth > self.max_depth {
            return None;
        }
        if self.nodes.len() >= self.limit {
            self.truncated = true;
            return None;
        }
        let index = self.nodes.len();
        self.nodes.push(OutlineNode {
            depth,
            key,
            kind,
            start,
            end: start,
            bytes: 0,
        });
        Some(index)
    }

    fn finish(&mut self, index: Option<usize>, end: usize) {
        if let Some(index) = index {
            let node = &mut self.nodes[index];
            node.end = end;
            node.bytes = end.saturating_sub(node.start);
        }
    }

    fn record(&mut self, depth: u8, key: String, kind: &'static str, start: usize, end: usize) {
        // 空 key 表示「不登记」（大数组只展开前若干个元素，避免刷屏）
        if key.is_empty() || depth > self.max_depth {
            return;
        }
        if self.nodes.len() >= self.limit {
            self.truncated = true;
            return;
        }
        self.nodes.push(OutlineNode {
            depth,
            key,
            kind,
            start,
            end,
            bytes: end.saturating_sub(start),
        });
    }

    fn scan_object(&mut self, depth: u8, node: Option<usize>) {
        // 已消费 '{'
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'}') => {
                    self.bump();
                    self.finish(node, self.pos);
                    return;
                }
                Some(b',') => {
                    self.bump();
                    continue;
                }
                Some(b'"') => {
                    let Some(child_key) = self.scan_string() else { return };
                    self.skip_ws();
                    if self.peek() != Some(b':') {
                        self.fail("对象缺少冒号");
                        return;
                    }
                    self.bump();
                    self.scan_value(depth + 1, child_key);
                }
                Some(_) => {
                    self.fail("对象里出现意外字符");
                    return;
                }
                None => {
                    self.fail("对象没有闭合");
                    return;
                }
            }
        }
    }

    fn scan_array(&mut self, depth: u8, node: Option<usize>) {
        let mut index = 0usize;
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b']') => {
                    self.bump();
                    self.finish(node, self.pos);
                    return;
                }
                Some(b',') => {
                    self.bump();
                    continue;
                }
                Some(_) => {
                    let before = self.pos;
                    // 数组只展开前 20 个元素，剩下的扫过去但不登记
                    let key = if index < 20 {
                        format!("[{index}]")
                    } else {
                        String::new()
                    };
                    self.scan_value(depth + 1, key);
                    if self.pos == before {
                        self.fail("数组元素解析无进展");
                        return;
                    }
                    index += 1;
                }
                None => {
                    self.fail("数组没有闭合");
                    return;
                }
            }
        }
    }
}

/// 取一段字节区间并格式化——大文件里点开某个节点时才解析这一小块。
pub fn slice(text: &str, start: usize, end: usize, indent: &str) -> JsonResult {
    let started = std::time::Instant::now();
    if start >= end || end > text.len() {
        return err_result(JsonError::plain("区间无效"), 0.0, 0);
    }
    let piece = &text[start..end];
    let mut result = format(piece, indent, false);
    result.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_lists_top_level_keys() {
        let doc = r#"{"a":1,"b":{"c":[1,2,3]},"d":"文本"}"#;
        let result = outline(doc, 2, 100);
        assert!(result.ok, "{:?}", result.error);
        assert_eq!(result.root_kind, "object");

        let keys: Vec<&str> = result.nodes.iter().map(|n| n.key.as_str()).collect();
        assert!(keys.contains(&"a"));
        assert!(keys.contains(&"b"));
        assert!(keys.contains(&"d"));
        // 嵌套层也能拿到 c
        assert!(keys.contains(&"c"));

        // 区间必须能切回合法 JSON
        let b = result.nodes.iter().find(|n| n.key == "b").unwrap();
        let sliced = slice(doc, b.start, b.end, "2");
        assert!(sliced.ok);
    }

    #[test]
    fn outline_handles_escaped_strings_and_numbers() {
        let doc = r#"{"k\"q":"a\"b\\c","n":-1.5e10,"t":true,"z":null}"#;
        let result = outline(doc, 1, 100);
        assert!(result.ok, "{:?}", result.error);
        let count = result.nodes.len();
        assert!(count >= 4, "节点太少：{count}");
    }

    #[test]
    fn outline_reports_broken_document() {
        let result = outline(r#"{"a":[1,2}"#, 2, 100);
        assert!(!result.ok);
        assert!(result.error.is_some());
    }

    #[test]
    fn outline_scales_to_large_documents() {
        // 约 8MB、20 万条数组元素：整树解析要几百 MB，大纲扫描应当毫秒级
        let mut doc = String::with_capacity(9 << 20);
        doc.push_str("{\"items\":[");
        for i in 0..200_000 {
            if i > 0 {
                doc.push(',');
            }
            doc.push_str(&format!("{{\"id\":{i},\"name\":\"item-{i}\"}}"));
        }
        doc.push_str("],\"meta\":{\"count\":200000}}");

        let started = std::time::Instant::now();
        let result = outline(&doc, 1, 100);
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;

        assert!(result.ok, "{:?}", result.error);
        assert!(result.nodes.iter().any(|n| n.key == "items"));
        assert!(result.nodes.iter().any(|n| n.key == "meta"));
        assert!(elapsed < 2000.0, "扫描太慢：{elapsed} ms");
    }

    #[test]
    fn validate_handles_large_documents() {
        // 约 6MB：整树解析会很慢，流式校验应当很快且成功
        let mut doc = String::with_capacity(6 << 20);
        doc.push('[');
        for i in 0..200_000 {
            if i > 0 {
                doc.push(',');
            }
            doc.push_str(&format!("{{\"i\":{i},\"s\":\"value-{i}\"}}"));
        }
        doc.push(']');

        let result = validate(&doc);
        assert!(result.ok, "校验失败：{:?}", result.error);
        assert!(result.output.contains("1 个顶层文档"));
        // 6MB 流式校验应在 1 秒内
        assert!(result.elapsed_ms < 1000.0, "耗时异常：{}", result.elapsed_ms);
    }

    #[test]
    fn validate_rejects_broken_stream() {
        let result = validate("{\"a\":1} {\"b\":");
        assert!(!result.ok);
        assert!(result.error.is_some());
    }

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
