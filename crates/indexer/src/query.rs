//! 查询引擎：空格 AND 分词、`ext:` 过滤、并行扫描 + 有界 top-k 堆。
//!
//! 为什么不用倒排索引：3-gram 倒排对 200 万文件要 ~200MB 内存，而这里用
//! memchr 加速的线性扫描 + rayon 并行，实测几百万条也能压到 10~30ms。

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use rayon::prelude::*;
use serde::Serialize;

use crate::Index;

#[derive(Clone, Debug, Serialize)]
pub struct Hit {
    pub idx: u32,
    pub score: u32,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Clone, Debug)]
enum Term {
    Text(Vec<u8>),
    Ext(Vec<u8>),
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
                let name = &names
                    [entry.name_off as usize..entry.name_off as usize + entry.name_len as usize];
                if let Some(score) = score_entry(name, terms) {
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
            Hit {
                idx,
                score: cand.score,
                path: index.path_of(idx),
                name: index.name(idx).to_string(),
                is_dir: index.entries[idx as usize].is_dir,
                size: index.entries[idx as usize].size,
            }
        })
        .collect()
}

fn score_entry(name: &[u8], terms: &[Term]) -> Option<u32> {
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
        }
    }
    Some(score)
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
