//! Standard scan: parallel, unelevated directory walk (D-09). Uses
//! FileIdBothDirectoryInfo, which returns allocated size and file ID without
//! opening each file. Links are recorded, never entered. Cloud-only folders are
//! not entered, so scanning does not trigger hydration.
use crate::long_wide;
use oz_model::{flag, IndexBuilder, NodeId, ROOT};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;

const ATTR_DIR: u32 = 0x10;
const ATTR_REPARSE: u32 = 0x400;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_HIDDEN: u32 = 0x2;
const ATTR_RECALL: u32 = 0x40000 | 0x400000 | 0x1000;
const TAG_MOUNT: u32 = 0xA000_0003;
const TAG_SYMLINK: u32 = 0xA000_000C;

pub struct Shared {
    pub builder: Mutex<State>,
    pub cancel: AtomicBool,
    pub items: AtomicU64,
    pub bytes: AtomicU64,
}

pub struct State {
    pub b: IndexBuilder,
    seen: HashSet<u64>,
    pub unread: Vec<String>,
}

impl Shared {
    pub fn new(root: &str) -> Arc<Self> {
        let name = root.trim_end_matches('\\');
        let name = if name.len() == 2 {
            format!("{name}\\")
        } else {
            name.to_string()
        };
        Arc::new(Self {
            builder: Mutex::new(State {
                b: IndexBuilder::new(&name),
                seen: HashSet::new(),
                unread: Vec::new(),
            }),
            cancel: AtomicBool::new(false),
            items: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
        })
    }
}

struct Entry {
    name: String,
    logical: u64,
    allocated: u64,
    modified: i64,
    attrs: u32,
    tag: u32,
    file_id: u64,
}

/// Reads one directory. Err carries the Win32 error.
fn read_dir(path: &str, out: &mut Vec<Entry>) -> Result<(), u32> {
    let w = long_wide(path);
    // SAFETY: path is NUL-terminated; handle is closed below on every path.
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        // SAFETY: no preconditions.
        return Err(unsafe { GetLastError() });
    }
    let mut buf = vec![0u64; 16 * 1024]; // 128 KB, 8-byte aligned
    let bytes = buf.len() * 8;
    let mut class = FileIdBothDirectoryRestartInfo;
    let mut result = Ok(());
    loop {
        // SAFETY: buffer pointer and size are valid and aligned.
        let ok = unsafe {
            GetFileInformationByHandleEx(h, class, buf.as_mut_ptr().cast(), bytes as u32)
        };
        class = FileIdBothDirectoryInfo;
        if ok == 0 {
            // SAFETY: no preconditions.
            let e = unsafe { GetLastError() };
            if e != ERROR_NO_MORE_FILES {
                result = Err(e);
            }
            break;
        }
        // SAFETY: reinterpret initialized u64 storage as bytes of the same length.
        let b: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast(), bytes) };
        parse_batch(b, out);
    }
    // SAFETY: h is an open handle.
    unsafe { CloseHandle(h) };
    result
}

/// Parses packed FILE_ID_BOTH_DIR_INFO records.
fn parse_batch(b: &[u8], out: &mut Vec<Entry>) {
    let mut o = 0usize;
    loop {
        let rd64 = |p: usize| {
            b.get(p..p + 8)
                .map(|s| u64::from_le_bytes(s.try_into().unwrap()))
        };
        let rd32 = |p: usize| {
            b.get(p..p + 4)
                .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
        };
        let (
            Some(next),
            Some(modified),
            Some(eof),
            Some(alloc),
            Some(attrs),
            Some(nlen),
            Some(ea),
            Some(id),
        ) = (
            rd32(o),
            rd64(o + 24),
            rd64(o + 40),
            rd64(o + 48),
            rd32(o + 56),
            rd32(o + 60),
            rd32(o + 64),
            rd64(o + 96),
        )
        else {
            return;
        };
        let Some(nb) = b.get(o + 104..o + 104 + nlen as usize) else {
            return;
        };
        let wide: Vec<u16> = nb
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let name = String::from_utf16_lossy(&wide);
        if name != "." && name != ".." {
            out.push(Entry {
                name,
                logical: eof,
                allocated: alloc,
                modified: modified as i64,
                attrs,
                // For reparse points this field carries the reparse tag.
                tag: if attrs & ATTR_REPARSE != 0 { ea } else { 0 },
                file_id: id,
            });
        }
        if next == 0 {
            return;
        }
        o += next as usize;
    }
}

struct Queue {
    q: Mutex<(Vec<(NodeId, String)>, usize)>,
    cv: Condvar,
}

/// Blocks until the walk completes or is cancelled.
pub fn run(shared: Arc<Shared>, root: &str, threads: usize) {
    let root = if root.ends_with('\\') {
        root.to_string()
    } else {
        format!("{root}\\")
    };
    let queue = Arc::new(Queue {
        q: Mutex::new((vec![(ROOT, root)], 0)),
        cv: Condvar::new(),
    });
    let mut handles = Vec::new();
    for _ in 0..threads.max(1) {
        let (s, q) = (shared.clone(), queue.clone());
        handles.push(std::thread::spawn(move || worker(&s, &q)));
    }
    for h in handles {
        let _ = h.join();
    }
}

fn worker(s: &Shared, q: &Queue) {
    let mut entries = Vec::new();
    loop {
        let job = {
            let mut g = q.q.lock().unwrap();
            loop {
                if s.cancel.load(Ordering::Relaxed) {
                    g.0.clear();
                    q.cv.notify_all();
                    return;
                }
                if let Some(j) = g.0.pop() {
                    g.1 += 1;
                    break j;
                }
                if g.1 == 0 {
                    q.cv.notify_all();
                    return;
                }
                g = q.cv.wait(g).unwrap();
            }
        };
        entries.clear();
        let res = read_dir(&job.1, &mut entries);
        let mut next = Vec::new();
        {
            let mut st = s.builder.lock().unwrap();
            if res.is_err() {
                st.b.add_flags(job.0, flag::UNREAD);
                st.unread.push(job.1.clone());
            }
            for e in &entries {
                let is_dir = e.attrs & ATTR_DIR != 0;
                let mut f = 0u8;
                if is_dir {
                    f |= flag::DIR;
                }
                let is_link = e.tag == TAG_MOUNT || e.tag == TAG_SYMLINK;
                if is_link {
                    f |= flag::LINK;
                }
                if e.attrs & ATTR_RECALL != 0 {
                    f |= flag::PLACEHOLDER;
                }
                if e.attrs & ATTR_SYSTEM != 0 && e.attrs & ATTR_HIDDEN != 0 {
                    f |= flag::SYSTEM;
                }
                let (mut lg, mut al) = (e.logical, e.allocated);
                if is_dir {
                    (lg, al) = (0, 0);
                } else if !st.seen.insert(e.file_id) && e.file_id != 0 {
                    f |= flag::HARDLINK;
                    (lg, al) = (0, 0);
                }
                let id = st.b.add(job.0, &e.name, lg, al, e.modified, f);
                s.bytes.fetch_add(al, Ordering::Relaxed);
                if is_dir && !is_link && f & flag::PLACEHOLDER == 0 {
                    next.push((id, format!("{}{}\\", job.1, e.name)));
                }
            }
            s.items.fetch_add(entries.len() as u64, Ordering::Relaxed);
        }
        let mut g = q.q.lock().unwrap();
        g.0.append(&mut next);
        g.1 -= 1;
        q.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scans_long_paths_hardlinks_and_does_not_follow_junctions() {
        let root = std::env::temp_dir().join(format!("oz-walk-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let r = root.to_str().unwrap().to_string();
        fs::write(root.join("a.bin"), vec![1u8; 100_000]).unwrap();
        fs::hard_link(root.join("a.bin"), root.join("a-link.bin")).unwrap();

        // Outside folder, reached only through a junction: must not be counted.
        let outside = std::env::temp_dir().join(format!("oz-walk-out-{}", std::process::id()));
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("big.bin"), vec![1u8; 500_000]).unwrap();
        crate::fsops::create_junction(root.join("j").to_str().unwrap(), outside.to_str().unwrap())
            .unwrap();

        // Long path: over 300 characters.
        let mut deep = crate::long_path(&r);
        for _ in 0..30 {
            deep.push_str("\\0123456789");
        }
        fs::create_dir_all(&deep).unwrap();
        fs::write(format!("{deep}\\deep.bin"), vec![1u8; 50_000]).unwrap();
        assert!(deep.len() > 300);

        let s = Shared::new(&r);
        run(s.clone(), &r, 4);
        let ix = s.builder.lock().unwrap().b.build();
        // 100k once for the hard link pair + 50k deep; junction excluded.
        assert_eq!(ix.logical(ROOT), 150_000);
        let names: Vec<_> = ix
            .children(ROOT)
            .iter()
            .map(|&c| ix.name(c).to_string())
            .collect();
        assert!(names.contains(&"j".to_string()));
        let j = ix
            .children(ROOT)
            .iter()
            .copied()
            .find(|&c| ix.name(c) == "j")
            .unwrap();
        assert!(ix.flags(j) & flag::LINK != 0);
        assert!(ix.children(j).is_empty());
        assert!(ix.allocated(ROOT) >= 150_000);

        crate::fsops::remove_link(root.join("j").to_str().unwrap()).unwrap();
        crate::fsops::remove_tree(&r, &|| false).unwrap();
        fs::remove_dir_all(&outside).unwrap();
    }
}
