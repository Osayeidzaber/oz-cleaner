//! Scan index: struct-of-arrays tree of files and folders (D-07).
//! No Windows code lives here, so it is fully testable.

pub type NodeId = u32;
pub const ROOT: NodeId = 0;
pub const NONE: NodeId = u32::MAX;

pub mod flag {
    pub const DIR: u8 = 1;
    /// Junction, symlink or mount point. Never followed.
    pub const LINK: u8 = 2;
    /// Secondary name of a hard-linked file; contributes no size.
    pub const HARDLINK: u8 = 4;
    /// Cloud placeholder (OneDrive etc).
    pub const PLACEHOLDER: u8 = 8;
    /// Has alternate data streams included in its size.
    pub const ADS: u8 = 16;
    /// NTFS metadata file or hidden system file.
    pub const SYSTEM: u8 = 32;
    /// A folder that could not be read.
    pub const UNREAD: u8 = 64;
}

#[derive(Default, Clone)]
pub struct IndexBuilder {
    names: String,
    name_off: Vec<u32>,
    name_len: Vec<u32>,
    parent: Vec<NodeId>,
    logical: Vec<u64>,
    allocated: Vec<u64>,
    modified: Vec<i64>,
    flags: Vec<u8>,
}

impl IndexBuilder {
    pub fn new(root_name: &str) -> Self {
        let mut b = Self::default();
        b.add(NONE, root_name, 0, 0, 0, flag::DIR);
        b
    }

    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    pub fn add(&mut self, parent: NodeId, name: &str, logical: u64, allocated: u64, modified: i64, flags: u8) -> NodeId {
        let id = self.parent.len() as NodeId;
        self.name_off.push(self.names.len() as u32);
        self.name_len.push(name.len() as u32);
        self.names.push_str(name);
        self.parent.push(parent);
        self.logical.push(logical);
        self.allocated.push(allocated);
        self.modified.push(modified);
        self.flags.push(flags);
        id
    }

    /// For sources (the MFT) that see children before parents.
    pub fn set_parent(&mut self, id: NodeId, parent: NodeId) {
        self.parent[id as usize] = parent;
    }

    pub fn add_flags(&mut self, id: NodeId, f: u8) {
        self.flags[id as usize] |= f;
    }

    /// Builds an immutable index. Nodes unreachable from the root are dropped
    /// from the tree and their allocated bytes reported as `orphan_allocated`.
    /// Cycles cannot hang this: only a BFS from the root is walked.
    pub fn build(&self) -> Index {
        let n = self.parent.len();
        let mut counts = vec![0u32; n + 1];
        for (i, &p) in self.parent.iter().enumerate() {
            if i != ROOT as usize && (p as usize) < n {
                counts[p as usize + 1] += 1;
            }
        }
        for i in 0..n {
            counts[i + 1] += counts[i];
        }
        let child_start = counts;
        let mut cursor = child_start.clone();
        let total_children = child_start[n] as usize;
        let mut child_ids = vec![0u32; total_children];
        for (i, &p) in self.parent.iter().enumerate() {
            if i != ROOT as usize && (p as usize) < n {
                child_ids[cursor[p as usize] as usize] = i as u32;
                cursor[p as usize] += 1;
            }
        }

        let mut order = Vec::with_capacity(n);
        let mut seen = vec![false; n];
        if n > 0 {
            order.push(ROOT);
            seen[0] = true;
        }
        let mut head = 0;
        while head < order.len() {
            let id = order[head] as usize;
            head += 1;
            for &c in &child_ids[child_start[id] as usize..child_start[id + 1] as usize] {
                if !seen[c as usize] {
                    seen[c as usize] = true;
                    order.push(c);
                }
            }
        }

        let mut sub_logical = self.logical.clone();
        let mut sub_allocated = self.allocated.clone();
        let mut sub_files = vec![0u32; n];
        for i in 0..n {
            if self.flags[i] & flag::DIR == 0 {
                sub_files[i] = 1;
            }
        }
        let mut sub_modified = self.modified.clone();
        for &id in order.iter().rev() {
            let i = id as usize;
            if id != ROOT {
                let p = self.parent[i] as usize;
                sub_logical[p] += sub_logical[i];
                sub_allocated[p] += sub_allocated[i];
                sub_files[p] += sub_files[i];
                sub_modified[p] = sub_modified[p].max(sub_modified[i]);
            }
        }
        let orphan_allocated = (0..n).filter(|&i| !seen[i]).map(|i| self.allocated[i]).sum();

        for i in 0..n {
            let s = &mut child_ids[child_start[i] as usize..child_start[i + 1] as usize];
            s.sort_unstable_by(|&a, &b| sub_allocated[b as usize].cmp(&sub_allocated[a as usize]));
        }

        Index {
            names: self.names.clone(),
            name_off: self.name_off.clone(),
            name_len: self.name_len.clone(),
            parent: self.parent.clone(),
            allocated: self.allocated.clone(),
            modified: sub_modified,
            flags: self.flags.clone(),
            sub_logical,
            sub_allocated,
            sub_files,
            child_start,
            child_ids,
            reachable: seen,
            orphan_allocated,
        }
    }
}

pub struct Index {
    names: String,
    name_off: Vec<u32>,
    name_len: Vec<u32>,
    parent: Vec<NodeId>,
    allocated: Vec<u64>,
    modified: Vec<i64>,
    flags: Vec<u8>,
    sub_logical: Vec<u64>,
    sub_allocated: Vec<u64>,
    sub_files: Vec<u32>,
    child_start: Vec<u32>,
    child_ids: Vec<u32>,
    reachable: Vec<bool>,
    pub orphan_allocated: u64,
}

impl Index {
    pub fn len(&self) -> usize {
        self.parent.len()
    }
    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }
    pub fn contains(&self, id: NodeId) -> bool {
        (id as usize) < self.len() && self.reachable[id as usize]
    }
    pub fn name(&self, id: NodeId) -> &str {
        let o = self.name_off[id as usize] as usize;
        &self.names[o..o + self.name_len[id as usize] as usize]
    }
    pub fn parent(&self, id: NodeId) -> NodeId {
        self.parent[id as usize]
    }
    /// Children, largest on-disk first.
    pub fn children(&self, id: NodeId) -> &[NodeId] {
        let i = id as usize;
        &self.child_ids[self.child_start[i] as usize..self.child_start[i + 1] as usize]
    }
    pub fn flags(&self, id: NodeId) -> u8 {
        self.flags[id as usize]
    }
    pub fn is_dir(&self, id: NodeId) -> bool {
        self.flags(id) & flag::DIR != 0
    }
    /// Subtree on-disk bytes.
    pub fn allocated(&self, id: NodeId) -> u64 {
        self.sub_allocated[id as usize]
    }
    /// Subtree logical bytes.
    pub fn logical(&self, id: NodeId) -> u64 {
        self.sub_logical[id as usize]
    }
    pub fn files(&self, id: NodeId) -> u32 {
        self.sub_files[id as usize]
    }
    /// FILETIME of the newest item in the subtree.
    pub fn modified(&self, id: NodeId) -> i64 {
        self.modified[id as usize]
    }
    /// Own bytes, excluding children.
    pub fn own_allocated(&self, id: NodeId) -> u64 {
        self.allocated[id as usize]
    }
    pub fn path(&self, id: NodeId) -> String {
        let mut parts = Vec::new();
        let mut cur = id;
        let mut guard = 0;
        while cur != NONE && cur != ROOT && guard < 8192 {
            parts.push(self.name(cur));
            cur = self.parent(cur);
            guard += 1;
        }
        let mut out = self.name(ROOT).trim_end_matches('\\').to_string();
        for p in parts.iter().rev() {
            out.push('\\');
            out.push_str(p);
        }
        if parts.is_empty() {
            out.push('\\');
        }
        out
    }
    /// Iterate reachable node ids.
    pub fn ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        (0..self.len() as u32).filter(|&i| self.reachable[i as usize])
    }
    /// Lower-case file extension of a file node, without the dot.
    pub fn ext(&self, id: NodeId) -> Option<String> {
        if self.is_dir(id) {
            return None;
        }
        let n = self.name(id);
        let dot = n.rfind('.')?;
        if dot == 0 || dot + 1 == n.len() {
            return None;
        }
        Some(n[dot + 1..].to_ascii_lowercase())
    }
}

/// Coarse type class used by the Type lens and the type list.
pub fn type_class(ext: Option<&str>) -> u8 {
    match ext.unwrap_or("") {
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "wmv" | "m4v" | "mpg" | "mpeg" => 1,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "heic" | "raw" | "psd" | "svg" => 2,
        "mp3" | "wav" | "flac" | "aac" | "ogg" | "m4a" | "wma" => 3,
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "txt" | "md" | "csv" | "epub" => 4,
        "zip" | "7z" | "rar" | "tar" | "gz" | "xz" | "bz2" | "iso" | "cab" | "zst" => 5,
        "exe" | "dll" | "sys" | "msi" | "msix" | "appx" | "ocx" | "so" | "pak" => 6,
        "js" | "ts" | "tsx" | "jsx" | "json" | "rs" | "py" | "java" | "c" | "h" | "cpp" | "cs" | "go" | "html" | "css"
        | "class" | "jar" | "pyc" | "map" | "node" | "pdb" | "lib" | "obj" | "rlib" => 7,
        "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" | "db" | "sqlite" | "dat" | "bin" | "log" | "tmp" | "cache" => 8,
        _ => 0,
    }
}

pub const TYPE_NAMES: [&str; 9] =
    ["Other", "Video", "Images", "Audio", "Documents", "Archives", "Programs", "Code", "Data & disks"];

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Index {
        let mut b = IndexBuilder::new("C:\\");
        let a = b.add(ROOT, "a", 0, 0, 10, flag::DIR);
        b.add(a, "big.iso", 100, 104, 50, 0);
        b.add(a, "small.txt", 1, 4, 20, 0);
        b.add(ROOT, "z.bin", 200, 208, 5, 0);
        b.build()
    }

    #[test]
    fn rolls_up_and_sorts() {
        let i = sample();
        assert_eq!(i.allocated(ROOT), 316);
        assert_eq!(i.logical(ROOT), 301);
        assert_eq!(i.files(ROOT), 3);
        assert_eq!(i.name(i.children(ROOT)[0]), "z.bin");
        assert_eq!(i.modified(1), 50);
    }

    #[test]
    fn paths() {
        let i = sample();
        assert_eq!(i.path(ROOT), "C:\\");
        assert_eq!(i.path(2), "C:\\a\\big.iso");
    }

    #[test]
    fn cycle_and_orphan_are_safe() {
        let mut b = IndexBuilder::new("C:\\");
        let x = b.add(NONE, "x", 5, 5, 0, 0);
        let y = b.add(x, "y", 7, 7, 0, 0);
        b.set_parent(x, y);
        let i = b.build();
        assert_eq!(i.allocated(ROOT), 0);
        assert_eq!(i.orphan_allocated, 12);
        assert!(!i.contains(x));
    }

    #[test]
    fn ext() {
        let i = sample();
        assert_eq!(i.ext(2).as_deref(), Some("iso"));
        assert_eq!(i.ext(1), None);
        assert_eq!(type_class(Some("iso")), 5);
    }
}
