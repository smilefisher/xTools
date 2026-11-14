//! 索引服务客户端：连本机命名管道，把查询转发给 `xtools-indexd`。
//!
//! 服务在就绪时主程序完全不建本地索引（省 100MB+ 内存），
//! 连不上就自动回退到进程内索引，功能不受影响。

use xtools_indexer::query::Hit;

pub const PIPE_NAME: &str = "xtools-index-v1";
const CONNECT_TIMEOUT_MS: u32 = 250;

/// 探活：返回服务端索引条数。
pub fn ping() -> Option<usize> {
    let client = xtools_pipe::Client::connect(PIPE_NAME, CONNECT_TIMEOUT_MS).ok()?;
    let response = client.request(r#"{"op":"ping"}"#).ok()?;
    let value: serde_json::Value = serde_json::from_str(&response).ok()?;
    if value.get("ok")?.as_bool()? {
        value.get("entries")?.as_u64().map(|entries| entries as usize)
    } else {
        None
    }
}

/// 远程查询；服务不可用或协议出错都返回 None，由调用方回退。
pub fn search(text: &str, limit: usize) -> Option<Vec<Hit>> {
    let payload = serde_json::json!({
        "op": "search",
        "query": text,
        "limit": limit,
    })
    .to_string();

    let client = xtools_pipe::Client::connect(PIPE_NAME, CONNECT_TIMEOUT_MS).ok()?;
    let response = client.request(&payload).ok()?;
    let value: serde_json::Value = serde_json::from_str(&response).ok()?;
    if !value.get("ok")?.as_bool()? {
        return None;
    }
    serde_json::from_value(value.get("hits")?.clone()).ok()
}
