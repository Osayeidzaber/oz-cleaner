//! Read-only queries over a finished index.
use oz_model::{flag, type_class, Index, NodeId, ROOT, TYPE_NAMES};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Clone)]
pub struct Row {
    pub id: u32,
    pub name: String,
    pub depth: u16,
    pub alloc: u64,
    pub logical: u64,
    pub files: u32,
    pub modified: i64,
    pub flags: u8,
    pub kids: bool,
    pub expanded: bool,
    pub pct: f32,
}

pub fn row(ix: &Index, id: NodeId, depth: u16, expanded: bool) -> Row {
    let p = ix.parent(id);
    let parent_alloc = if id == ROOT || !ix.contains(p) { ix.allocated(id) } else { ix.allocated(p) };
    Row {
        id,
        name: ix.name(id).to_string(),
        depth,
        alloc: ix.allocated(id),
        logical: ix.logical(id),
        files: ix.files(id),
        modified: ix.modified(id),
        flags: ix.flags(id),
        kids: !ix.children(id).is_empty(),
        expanded,
        pct: if parent_alloc == 0 { 0.0 } else { (ix.allocated(id) as f64 / parent_alloc as f64 * 100.0) as f32 },
    }
}

pub fn sorted_children(ix: &Index, id: NodeId, sort: &str) -> Vec<NodeId> {
    let mut v = ix.children(id).to_vec();
    match sort {
        "logical" => v.sort_by_key(|&c| std::cmp::Reverse(ix.logical(c))),
        "files" => v.sort_by_key(|&c| std::cmp::Reverse(ix.files(c))),
        "modified" => v.sort_by_key(|&c| std::cmp::Reverse(ix.modified(c))),
        "name" => v.sort_by_cached_key(|&c| ix.name(c).to_lowercase()),
        _ => {}
    }
    v
}

#[derive(Serialize)]
pub struct Page {
    pub total: u32,
    pub offset: u32,
    pub rows: Vec<Row>,
}

/// Flattens the expanded tree and returns one window of it.
pub fn page(ix: &Index, root: NodeId, expanded: &HashSet<u32>, sort: &str, offset: u32, limit: u32) -> Page {
    let mut rows = Vec::new();
    let mut total = 0u32;
    let mut stack: Vec<(NodeId, u16)> = sorted_children(ix, root, sort).into_iter().rev().map(|c| (c, 0)).collect();
    while let Some((id, d)) = stack.pop() {
        let is_open = expanded.contains(&id);
        if total >= offset && (rows.len() as u32) < limit {
            rows.push(row(ix, id, d, is_open));
        }
        total += 1;
        if is_open {
            let kids = sorted_children(ix, id, sort);
            stack.extend(kids.into_iter().rev().map(|c| (c, d + 1)));
        }
    }
    Page { total, offset, rows }
}

const DAY: i64 = 864_000_000_000;

#[derive(Default, Debug, PartialEq)]
pub struct Filter {
    text: Vec<String>,
    ext: Vec<String>,
    min_size: Option<u64>,
    max_size: Option<u64>,
    min_age: Option<i64>,
    max_age: Option<i64>,
    from: Option<i64>,
    to: Option<i64>,
    pub valid: bool,
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.to_ascii_lowercase();
    let t = s.trim_end_matches('b');
    let (n, m) = match t.chars().last()? {
        'k' => (&t[..t.len() - 1], 1u64 << 10),
        'm' => (&t[..t.len() - 1], 1 << 20),
        'g' => (&t[..t.len() - 1], 1 << 30),
        't' => (&t[..t.len() - 1], 1 << 40),
        _ => (t, 1),
    };
    n.parse::<f64>().ok().map(|v| (v * m as f64) as u64)
}

fn parse_age(s: &str) -> Option<i64> {
    let s = s.to_ascii_lowercase();
    let (n, m) = match s.chars().last()? {
        'd' => (&s[..s.len() - 1], 1i64),
        'w' => (&s[..s.len() - 1], 7),
        'm' => (&s[..s.len() - 1], 30),
        'y' => (&s[..s.len() - 1], 365),
        _ => (s.as_str(), 1),
    };
    n.parse::<f64>().ok().map(|v| (v * m as f64) as i64 * DAY)
}

pub fn parse_filter(q: &str) -> Filter {
    let mut f = Filter { valid: true, ..Default::default() };
    for t in q.split_whitespace() {
        let l = t.to_lowercase();
        if let Some(e) = l.strip_prefix("ext:") {
            f.ext.extend(e.split(',').map(|x| x.trim_start_matches('.').to_string()));
        } else if let Some(v) = l.strip_prefix("size>") {
            f.min_size = parse_size(v);
            f.valid &= f.min_size.is_some();
        } else if let Some(v) = l.strip_prefix("size<") {
            f.max_size = parse_size(v);
            f.valid &= f.max_size.is_some();
        } else if let Some(v) = l.strip_prefix("age>") {
            f.min_age = parse_age(v);
            f.valid &= f.min_age.is_some();
        } else if let Some(v) = l.strip_prefix("age<") {
            f.max_age = parse_age(v);
            f.valid &= f.max_age.is_some();
        } else if let Some(v) = l.strip_prefix("from:") {
            f.from = v.parse().ok();
        } else if let Some(v) = l.strip_prefix("to:") {
            f.to = v.parse().ok();
        } else {
            f.text.push(l);
        }
    }
    f
}

#[derive(Serialize)]
pub struct SearchResult {
    pub total: u32,
    pub bytes: u64,
    pub rows: Vec<SearchRow>,
    pub ids: Vec<u32>,
    pub valid: bool,
}

#[derive(Serialize)]
pub struct SearchRow {
    pub row: Row,
    pub path: String,
}

pub fn search(ix: &Index, q: &str, limit: usize, now: i64) -> SearchResult {
    let f = parse_filter(q);
    let mut hits: Vec<NodeId> = Vec::new();
    if f.valid {
        let attr_only = f.text.is_empty();
        for id in ix.ids() {
            if id == ROOT {
                continue;
            }
            let dir = ix.is_dir(id);
            if attr_only && dir {
                continue; // attribute filters describe files
            }
            let sz = ix.allocated(id);
            if f.min_size.is_some_and(|m| sz < m) || f.max_size.is_some_and(|m| sz > m) {
                continue;
            }
            let age = now - ix.modified(id);
            if f.min_age.is_some_and(|m| age < m) || f.max_age.is_some_and(|m| age > m) {
                continue;
            }
            if f.from.is_some_and(|t| ix.modified(id) < t) || f.to.is_some_and(|t| ix.modified(id) >= t) {
                continue;
            }
            if !f.ext.is_empty() && !ix.ext(id).is_some_and(|e| f.ext.contains(&e)) {
                continue;
            }
            if !f.text.is_empty() {
                let name = ix.name(id).to_lowercase();
                let needs_path = f.text.iter().any(|t| t.contains('\\'));
                let hay = if needs_path { ix.path(id).to_lowercase() } else { name };
                if !f.text.iter().all(|t| hay.contains(t.as_str())) {
                    continue;
                }
            }
            hits.push(id);
        }
    }
    hits.sort_by_key(|&i| std::cmp::Reverse(ix.allocated(i)));
    // Files only in the byte total, so nested matches don't double-count.
    let bytes = hits.iter().filter(|&&i| !ix.is_dir(i)).map(|&i| ix.allocated(i)).sum();
    SearchResult {
        total: hits.len() as u32,
        bytes,
        rows: hits.iter().take(limit).map(|&i| SearchRow { row: row(ix, i, 0, false), path: ix.path(i) }).collect(),
        ids: hits.iter().take(200_000).copied().collect(),
        valid: f.valid,
    }
}

#[derive(Serialize)]
pub struct TopEntry {
    pub id: u32,
    pub name: String,
    pub path: String,
    pub alloc: u64,
    pub files: u32,
}

pub fn top_files(ix: &Index, limit: usize) -> Vec<TopEntry> {
    let mut v: Vec<NodeId> = ix.ids().filter(|&i| !ix.is_dir(i)).collect();
    v.sort_unstable_by_key(|&i| std::cmp::Reverse(ix.allocated(i)));
    v.truncate(limit);
    v.iter().map(|&i| entry(ix, i, ix.allocated(i), 1)).collect()
}

/// Ranks folders by the files directly inside them, which avoids nested
/// double-counting (F-10).
pub fn top_folders(ix: &Index, limit: usize) -> Vec<TopEntry> {
    let mut v: Vec<(NodeId, u64, u32)> = ix
        .ids()
        .filter(|&i| ix.is_dir(i))
        .map(|i| {
            let (mut s, mut n) = (0u64, 0u32);
            for &c in ix.children(i) {
                if !ix.is_dir(c) {
                    s += ix.allocated(c);
                    n += 1;
                }
            }
            (i, s, n)
        })
        .collect();
    v.sort_unstable_by_key(|x| std::cmp::Reverse(x.1));
    v.truncate(limit);
    v.iter().map(|&(i, s, n)| entry(ix, i, s, n)).collect()
}

fn entry(ix: &Index, i: NodeId, alloc: u64, files: u32) -> TopEntry {
    TopEntry { id: i, name: ix.name(i).to_string(), path: ix.path(i), alloc, files }
}

#[derive(Serialize)]
pub struct TypeEntry {
    pub ext: String,
    pub class: String,
    pub alloc: u64,
    pub files: u32,
}

pub fn top_types(ix: &Index, limit: usize) -> Vec<TypeEntry> {
    let mut m: HashMap<String, (u64, u32)> = HashMap::new();
    for i in ix.ids() {
        if ix.is_dir(i) {
            continue;
        }
        let e = m.entry(ix.ext(i).unwrap_or_default()).or_default();
        e.0 += ix.allocated(i);
        e.1 += 1;
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_unstable_by_key(|x| std::cmp::Reverse(x.1 .0));
    v.truncate(limit);
    v.into_iter()
        .map(|(ext, (a, n))| TypeEntry {
            class: TYPE_NAMES[type_class(Some(&ext)) as usize].to_string(),
            ext,
            alloc: a,
            files: n,
        })
        .collect()
}

#[derive(Serialize)]
pub struct Week {
    pub start: i64,
    pub bytes: u64,
    pub files: u32,
}

/// Bytes last modified per week, newest last (F-13).
pub fn timeline(ix: &Index, weeks: usize, now: i64) -> Vec<Week> {
    const WEEK: i64 = 7 * DAY;
    let end = now - now.rem_euclid(WEEK) + WEEK;
    let mut out: Vec<Week> = (0..weeks).map(|i| Week { start: end - WEEK * (weeks - i) as i64, bytes: 0, files: 0 }).collect();
    let first = out[0].start;
    for i in ix.ids() {
        if ix.is_dir(i) {
            continue;
        }
        let t = ix.modified(i);
        // Anything older than the grid lands in its first cell.
        let idx = if t < first { 0 } else { ((t - first) / WEEK) as usize };
        if let Some(w) = out.get_mut(idx) {
            w.bytes += ix.allocated(i);
            w.files += 1;
        }
    }
    out
}

pub fn find_path(ix: &Index, path: &str) -> Option<NodeId> {
    let root = ix.name(ROOT).trim_end_matches('\\').to_lowercase();
    let p = path.trim_end_matches('\\').to_lowercase();
    let rest = p.strip_prefix(&root)?;
    let rest = rest.strip_prefix('\\').unwrap_or(rest);
    if rest.is_empty() {
        return Some(ROOT);
    }
    let mut cur = ROOT;
    for seg in rest.split('\\') {
        cur = *ix.children(cur).iter().find(|&&c| ix.name(c).eq_ignore_ascii_case(seg) || ix.name(c).to_lowercase() == seg)?;
    }
    Some(cur)
}

#[derive(Serialize)]
pub struct DevFolder {
    pub id: u32,
    pub path: String,
    pub name: String,
    pub alloc: u64,
    pub modified: i64,
}

/// Rebuildable-looking folders, offered for Review only when a project marker
/// sits beside or inside them (F-33). Nested ones are ignored.
pub fn dev_folders(ix: &Index, limit: usize) -> Vec<DevFolder> {
    let mut out = Vec::new();
    for id in ix.ids() {
        if !ix.is_dir(id) || ix.flags(id) & flag::LINK != 0 || ix.allocated(id) < 1 << 20 {
            continue;
        }
        let name = ix.name(id);
        let Some(markers) = oz_rules::dev_folder_marker(name) else { continue };
        let p = ix.parent(id);
        let has = |dir: NodeId| ix.children(dir).iter().any(|&c| markers.iter().any(|m| ix.name(c).eq_ignore_ascii_case(m)));
        if !(has(p) || has(id)) {
            continue;
        }
        let path = ix.path(id);
        let lower = path.to_lowercase();
        let nested = ["\\node_modules\\", "\\.venv\\", "\\target\\"].iter().any(|m| lower[..lower.len() - name.len()].contains(m));
        if nested {
            continue;
        }
        out.push(DevFolder { id, name: name.to_string(), path, alloc: ix.allocated(id), modified: ix.modified(id) });
    }
    out.sort_unstable_by_key(|d| std::cmp::Reverse(d.alloc));
    out.truncate(limit);
    out
}

/// Folder sizes down to a fixed depth, for snapshots and compare (F-14).
pub fn folder_sizes(ix: &Index, max_depth: u16, min_bytes: u64) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![(ROOT, 0u16)];
    while let Some((id, d)) = stack.pop() {
        out.push((ix.path(id), ix.allocated(id)));
        if d < max_depth {
            for &c in ix.children(id) {
                if ix.is_dir(c) && ix.allocated(c) >= min_bytes {
                    stack.push((c, d + 1));
                }
            }
        }
    }
    out
}

pub fn csv_field(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oz_model::IndexBuilder;

    fn ix() -> Index {
        let mut b = IndexBuilder::new("C:\\");
        let d = b.add(ROOT, "proj", 0, 0, 100, flag::DIR);
        b.add(d, "package.json", 1, 4096, 100, 0);
        let nm = b.add(d, "node_modules", 0, 0, 100, flag::DIR);
        b.add(nm, "x.js", 1 << 21, 1 << 21, 100, 0);
        let nn = b.add(nm, "node_modules", 0, 0, 100, flag::DIR);
        b.add(nn, "package.json", 1 << 21, 1 << 21, 100, 0);
        b.add(ROOT, "movie.MKV", 5 << 30, 5 << 30, 100, 0);
        b.build()
    }

    #[test]
    fn filters() {
        let f = parse_filter("ext:iso,.mkv size>1g age>1y foo");
        assert_eq!(f.ext, vec!["iso", "mkv"]);
        assert_eq!(f.min_size, Some(1 << 30));
        assert_eq!(f.min_age, Some(365 * DAY));
        assert_eq!(f.text, vec!["foo"]);
        assert!(!parse_filter("size>abc").valid);
    }

    #[test]
    fn search_works() {
        let i = ix();
        let r = search(&i, "ext:mkv size>1g", 10, 100);
        assert_eq!(r.total, 1);
        assert_eq!(r.rows[0].row.name, "movie.MKV");
        assert_eq!(search(&i, "PROJ", 10, 100).total, 1);
        assert_eq!(search(&i, "proj\\package", 10, 100).total, 1);
        assert_eq!(search(&i, "age>1d", 10, DAY * 400).total, 4);
        assert_eq!(search(&i, "size>abc", 10, 100).valid, false);
    }

    #[test]
    fn paging_and_expansion() {
        let i = ix();
        let mut e = HashSet::new();
        assert_eq!(page(&i, ROOT, &e, "alloc", 0, 50).total, 2);
        e.insert(1);
        let p = page(&i, ROOT, &e, "alloc", 0, 50);
        assert_eq!(p.total, 4);
        assert_eq!(p.rows[1].name, "proj");
        assert_eq!(p.rows[2].depth, 1);
        assert_eq!(page(&i, ROOT, &e, "name", 0, 1).rows[0].name, "movie.MKV");
        assert_eq!(page(&i, ROOT, &e, "alloc", 3, 50).rows.len(), 1);
    }

    #[test]
    fn dev_folder_needs_marker_and_skips_nested() {
        let v = dev_folders(&ix(), 10);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].path, r"C:\proj\node_modules");
    }

    #[test]
    fn lookups() {
        let i = ix();
        assert_eq!(find_path(&i, r"c:\PROJ\node_modules"), Some(3));
        assert_eq!(find_path(&i, "C:\\"), Some(ROOT));
        assert_eq!(find_path(&i, r"D:\proj"), None);
        assert_eq!(top_files(&i, 1)[0].name, "movie.MKV");
        assert_eq!(top_types(&i, 1)[0].ext, "mkv");
        assert_eq!(top_folders(&i, 1)[0].name, "C:\\");
        assert_eq!(csv_field("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn timeline_buckets() {
        let t = timeline(&ix(), 4, 1000 * DAY);
        assert_eq!(t.len(), 4);
        assert_eq!(t[0].files, 4);
        assert!(t.windows(2).all(|w| w[0].start < w[1].start));
    }
}
