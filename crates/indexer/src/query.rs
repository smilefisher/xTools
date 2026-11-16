//! 查询引擎：空格 AND 分词、`ext:` 过滤、并行扫描 + 有界 top-k 堆。
//!
//! 为什么不用倒排索引：3-gram 倒排对 200 万文件要 ~200MB 内存，而这里用
//! memchr 加速的线性扫描 + rayon 并行，实测几百万条也能压到 10~30ms。

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{Entry, Index};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Hit {
    pub idx: u32,
    pub score: u32,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    #[serde(default)]
    pub mtime: u64,
}

#[derive(Clone, Debug)]
enum Term {
    Text(Vec<u8>),
    Ext(Vec<u8>),
    Path(Vec<u8>),
    SizeMin(u64),
    SizeMax(u64),
}

#[derive(Clone, Debug, Default)]
pub struct Query {
    terms: Vec<Term>,
    raw: String,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let mut terms = Vec::new();
        for token in input.split_whitespace() {
            let lower = ascii_lower(token);
            if lower.is_empty() {
                continue;
            }
            if lower.starts_with(b"ext:") {
                let ext = lower[4..].to_vec();
                if !ext.is_empty() {
                    terms.push(Term::Ext(ext));
                }
            } else if lower.starts_with(b"path:") {
                let part = lower[5..].to_vec();
                if !part.is_empty() {
                    terms.push(Term::Path(part));
                }
            } else if lower.starts_with(b"size:") {
                if let Some((min, max)) = parse_size(&lower[5..]) {
                    if let Some(min) = min {
                        terms.push(Term::SizeMin(min));
                    }
                    if let Some(max) = max {
                        terms.push(Term::SizeMax(max));
                    }
                }
            } else {
                terms.push(Term::Text(lower));
            }
        }
        Self {
            terms,
            raw: input.to_string(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }
}

#[derive(PartialEq, Eq)]
struct Cand {
    score: u32,
    idx: u32,
}

impl Ord for Cand {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score.cmp(&other.score).then(self.idx.cmp(&other.idx))
    }
}

impl PartialOrd for Cand {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 并行扫描 + 有界 top-k，避免「命中几十万条」时的分配与全排序开销。
pub fn search(index: &Index, query: &Query, limit: usize) -> Vec<Hit> {
    if query.is_empty() || index.is_empty() {
        return Vec::new();
    }

    let keep = limit.max(1) * 4;
    let names = &index.names;
    let terms = &query.terms;

    let heap = index
        .entries
        .par_iter()
        .enumerate()
        .fold(
            || BinaryHeap::with_capacity(keep + 1),
            |mut heap, (i, entry)| {
                // 墓碑条目（增量删除）不参与匹配
                if entry.name_len == 0 {
                    return heap;
                }
                let name = &names
                    [entry.name_off as usize..entry.name_off as usize + entry.name_len as usize];
                if let Some(score) = score_entry(name, terms, index, i as u32, entry) {
                    heap.push(Reverse(Cand {
                        score,
                        idx: i as u32,
                    }));
                    if heap.len() > keep {
                        heap.pop();
                    }
                }
                heap
            },
        )
        .reduce(
            || BinaryHeap::new(),
            |mut acc, part| {
                for item in part {
                    acc.push(item);
                    if acc.len() > keep {
                        acc.pop();
                    }
                }
                acc
            },
        );

    let mut top: Vec<Cand> = heap.into_iter().map(|Reverse(c)| c).collect();
    top.sort_unstable_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| index.name(a.idx).len().cmp(&index.name(b.idx).len()))
            .then_with(|| index.name(a.idx).cmp(index.name(b.idx)))
    });
    top.truncate(limit);

    top.into_iter()
        .map(|cand| {
            let idx = cand.idx;
            let entry = &index.entries[idx as usize];
            Hit {
                idx,
                score: cand.score,
                path: index.path_of(idx),
                name: index.name(idx).to_string(),
                is_dir: entry.is_dir,
                size: entry.size,
                mtime: entry.mtime,
            }
        })
        .collect()
}

/// `>10m` / `<1k` / `10m-20m` / `10m`（视为 ≥）
fn parse_size(raw: &[u8]) -> Option<(Option<u64>, Option<u64>)> {
    let text = std::str::from_utf8(raw).ok()?.trim();
    if text.is_empty() {
        return None;
    }

    let parse_one = |value: &str| -> Option<u64> {
        let value = value.trim();
        let (number, multiplier) = match value.chars().last()? {
            'k' | 'K' => (&value[..value.len() - 1], 1024u64),
            'm' | 'M' => (&value[..value.len() - 1], 1024 * 1024),
            'g' | 'G' => (&value[..value.len() - 1], 1024 * 1024 * 1024),
            _ => (value, 1),
        };
        number.parse::<u64>().ok().map(|n| n * multiplier)
    };

    if let Some(rest) = text.strip_prefix('>').or_else(|| text.strip_prefix(">=")) {
        return parse_one(rest).map(|min| (Some(min), None));
    }
    if let Some(rest) = text.strip_prefix('<').or_else(|| text.strip_prefix("<=")) {
        return parse_one(rest).map(|max| (None, Some(max)));
    }
    if let Some((low, high)) = text.split_once('-') {
        return Some((parse_one(low), parse_one(high)));
    }
    parse_one(text).map(|min| (Some(min), None))
}

fn score_entry(name: &[u8], terms: &[Term], index: &Index, idx: u32, entry: &Entry) -> Option<u32> {
    let mut score = 0u32;
    for term in terms {
        match term {
            Term::Text(needle) => score += score_text(name, needle)?,
            Term::Ext(want) => {
                if !has_extension(name, want) {
                    return None;
                }
                score += 100;
            }
            Term::Path(want) => {
                if !path_matches(index, idx, want) {
                    return None;
                }
                score += 50;
            }
            Term::SizeMin(min) => {
                if entry.is_dir || entry.size < *min {
                    return None;
                }
            }
            Term::SizeMax(max) => {
                if entry.is_dir || entry.size > *max {
                    return None;
                }
            }
        }
    }
    Some(score)
}

/// 沿父链找目录名是否包含关键词（不拼完整路径，省一次分配）。
fn path_matches(index: &Index, idx: u32, needle: &[u8]) -> bool {
    let mut cursor = index.entries[idx as usize].parent;
    let mut depth = 0;
    while cursor != crate::NO_PARENT && depth < 64 {
        let entry = &index.entries[cursor as usize];
        let name = &index.names
            [entry.name_off as usize..entry.name_off as usize + entry.name_len as usize];
        if find_ci(name, needle).is_some() {
            return true;
        }
        cursor = entry.parent;
        depth += 1;
    }
    false
}

/// 单条名字打分；不匹配返回 None。
fn score_text(name: &[u8], needle: &[u8]) -> Option<u32> {
    if needle.is_empty() {
        return Some(0);
    }
    if name.len() == needle.len() {
        return eq_ci(name, needle).then_some(1_000);
    }
    if name.len() > needle.len() && eq_ci(&name[..needle.len()], needle) {
        return Some(900 - (name.len() - needle.len()).min(60) as u32);
    }
    let pos = find_ci(name, needle)?;
    let boundary = pos > 0 && matches!(name[pos - 1], b' ' | b'-' | b'_' | b'.' | b'(' | b'[');
    Some(if boundary {
        850
    } else {
        800u32.saturating_sub(pos.min(60) as u32)
    })
}

fn has_extension(name: &[u8], ext: &[u8]) -> bool {
    let Some(dot) = name.iter().rposition(|b| *b == b'.') else {
        return false;
    };
    let actual = &name[dot + 1..];
    actual.len() == ext.len() && eq_ci(actual, ext)
}

fn ascii_lower(input: &str) -> Vec<u8> {
    input.bytes().map(|b| b.to_ascii_lowercase()).collect()
}

#[inline]
fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_ascii_lowercase() == *y)
}

fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let lower = needle[0];
    let upper = lower.to_ascii_uppercase();
    let last = hay.len() - needle.len();
    let mut start = 0usize;

    while start <= last {
        // 用 memchr2 一次跳一大段，避免逐字节比较
        let offset = if lower == upper {
            memchr::memchr(lower, &hay[start..=last])?
        } else {
            memchr::memchr2(lower, upper, &hay[start..=last])?
        };
        let pos = start + offset;
        if eq_ci(&hay[pos..pos + needle.len()], needle) {
            return Some(pos);
        }
        start = pos + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_and_path_filters() {
        let root = std::env::temp_dir().join("xtools-query-filter-test");
        let sub = root.join("src");
        let _ = std::fs::create_dir_all(&sub);
        let _ = std::fs::write(sub.join("big.bin"), vec![0u8; 4096]);
        let _ = std::fs::write(root.join("small.txt"), b"hi");

        let (index, _) = crate::build_walk(&[root.clone()]);

        let by_size = Query::parse("size:>1k");
        let hits = search(&index, &by_size, 10);
        assert!(hits.iter().any(|hit| hit.name == "big.bin"), "size 过滤没命中");
        assert!(!hits.iter().any(|hit| hit.name == "small.txt"));

        let by_path = Query::parse("path:src");
        let hits = search(&index, &by_path, 10);
        assert!(!hits.is_empty(), "path 过滤没命中");

        let combined = Query::parse("ext:bin size:>1k path:src");
        let hits = search(&index, &combined, 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "big.bin");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn parse_size_variants() {
        assert_eq!(parse_size(b">10m"), Some((Some(10 * 1024 * 1024), None)));
        assert_eq!(parse_size(b"<1k"), Some((None, Some(1024))));
        assert_eq!(parse_size(b"2k-3k"), Some((Some(2048), Some(3072))));
        assert_eq!(parse_size(b"500"), Some((Some(500), None)));
        assert_eq!(parse_size(b"abc"), None);
    }

    #[test]
    fn parse_terms() {
        let q = Query::parse("  a  EXT:PDF ");
        assert_eq!(q.terms.len(), 2);
        assert!(matches!(q.terms[0], Term::Text(_)));
        assert!(matches!(q.terms[1], Term::Ext(_)));
    }

    #[test]
    fn scoring_order() {
        assert_eq!(score_text(b"abc.txt", b"abc.txt"), Some(1000));
        assert!(score_text(b"abc.txt", b"abc") > score_text(b"xabc.txt", b"abc"));
        assert_eq!(score_text(b"abc.txt", b"zzz"), None);
    }

    #[test]
    fn extension_filter() {
        assert!(has_extension(b"a.PDF", b"pdf"));
        assert!(!has_extension(b"a.pdfx", b"pdf"));
        assert!(!has_extension(b"a", b"pdf"));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(score_text(b"WeChat.exe", b"wechat.exe"), Some(1000));
        assert_eq!(find_ci(b"xxABCyy", b"abc"), Some(2));
        assert_eq!(find_ci(b"xxabyy", b"abc"), None);
    }
}
