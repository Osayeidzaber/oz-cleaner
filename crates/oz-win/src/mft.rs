//! Fast scan: reads the $MFT from a raw volume handle. Read-only (A-1): the
//! handle is opened with GENERIC_READ and is never used to write (A-2).
use crate::wide;
use oz_model::{flag, IndexBuilder, NodeId, NONE, ROOT};
use oz_ntfs::{apply_fixups, mft_runs, parse_record, Record, FILE_ATTR_REPARSE, TAG_MOUNT_POINT, TAG_SYMLINK};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Ioctl::*;
use windows_sys::Win32::System::IO::DeviceIoControl;

struct Vol(HANDLE);
impl Drop for Vol {
    fn drop(&mut self) {
        // SAFETY: owned, open handle.
        unsafe { CloseHandle(self.0) };
    }
}

impl Vol {
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<usize, String> {
        let mut pos = 0i64;
        // SAFETY: open handle; out pointers valid.
        if unsafe { SetFilePointerEx(self.0, off as i64, &mut pos, FILE_BEGIN) } == 0 {
            return Err(format!("seek failed: {}", last()));
        }
        let mut n = 0u32;
        // SAFETY: buffer valid for its length.
        let ok = unsafe { ReadFile(self.0, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(format!("read failed: {}", last()));
        }
        Ok(n as usize)
    }
}

fn last() -> u32 {
    // SAFETY: no preconditions.
    unsafe { GetLastError() }
}

/// Streams every in-use record to `sink`. Requires administrator rights.
pub fn read_records(letter: char, cancel: &AtomicBool, mut sink: impl FnMut(Record)) -> Result<u64, String> {
    let path = wide(&format!(r"\\.\{letter}:"));
    // SAFETY: path is NUL-terminated.
    let h = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_NO_BUFFERING | FILE_FLAG_SEQUENTIAL_SCAN,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(format!("open volume failed: {}", last()));
    }
    let vol = Vol(h);

    // SAFETY: zeroed POD struct filled by the call.
    let mut nv: NTFS_VOLUME_DATA_BUFFER = unsafe { std::mem::zeroed() };
    let mut ret = 0u32;
    // SAFETY: output buffer matches the given size.
    let ok = unsafe {
        DeviceIoControl(
            vol.0,
            FSCTL_GET_NTFS_VOLUME_DATA,
            std::ptr::null(),
            0,
            (&mut nv as *mut NTFS_VOLUME_DATA_BUFFER).cast(),
            std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>() as u32,
            &mut ret,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(format!("not an NTFS volume or query failed: {}", last()));
    }
    let cluster = nv.BytesPerCluster as u64;
    let sector = nv.BytesPerSector as usize;
    let rec_size = nv.BytesPerFileRecordSegment as usize;
    if cluster == 0 || sector == 0 || rec_size == 0 || rec_size % sector != 0 {
        return Err("unexpected volume geometry".into());
    }

    let mut aligned = AlignedBuf::new(1024 * 1024);
    // Record 0 describes where the rest of the MFT lives.
    let start = nv.MftStartLcn as u64 * cluster;
    let need = rec_size.div_ceil(sector) * sector;
    vol.read_at(start, &mut aligned.as_mut()[..need])?;
    let mut rec0 = aligned.as_mut()[..rec_size].to_vec();
    if !apply_fixups(&mut rec0, sector) {
        return Err("MFT record 0 failed fixup".into());
    }
    let runs = mft_runs(&rec0).ok_or("could not decode MFT runs")?;

    let mut number = 0u64;
    let mut parsed = 0u64;
    let chunk_size = aligned.len() - aligned.len() % rec_size.max(sector);
    let mut tmp = vec![0u8; rec_size];
    for (lcn, clusters) in runs {
        let run_bytes = clusters * cluster;
        if lcn < 0 {
            number += run_bytes / rec_size as u64;
            continue;
        }
        let mut done = 0u64;
        while done + rec_size as u64 <= run_bytes {
            if cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let want = (run_bytes - done).min(chunk_size as u64) as usize;
            let want = want - want % sector;
            let got = vol.read_at(lcn as u64 * cluster + done, &mut aligned.as_mut()[..want])?;
            if got < rec_size {
                return Err("short read".into());
            }
            let usable = got - got % rec_size;
            for c in aligned.as_mut()[..usable].chunks_exact(rec_size) {
                if c[0..4] == *b"FILE" {
                    tmp.copy_from_slice(c);
                    if apply_fixups(&mut tmp, sector) {
                        if let Some(r) = parse_record(&tmp, number) {
                            parsed += 1;
                            sink(r);
                        }
                    }
                }
                number += 1;
            }
            done += usable as u64;
        }
    }
    Ok(parsed)
}

/// Sector-aligned buffer required by FILE_FLAG_NO_BUFFERING.
struct AlignedBuf {
    v: Vec<u8>,
    start: usize,
    len: usize,
}
impl AlignedBuf {
    fn new(len: usize) -> Self {
        let v = vec![0u8; len + 4096];
        let start = v.as_ptr().align_offset(4096);
        Self { v, start, len }
    }
    fn len(&self) -> usize {
        self.len
    }
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.v[self.start..self.start + self.len]
    }
}

/// Merges records (any order, including extension records) into a tree.
/// Hard links count once, against the first name.
#[derive(Default)]
pub struct Assembler {
    slots: Vec<Option<Box<Record>>>,
    pub count: u64,
}

impl Assembler {
    pub fn add(&mut self, r: Record) {
        self.count += 1;
        let target = if r.base != 0 { r.base } else { r.number } as usize;
        if target >= 1 << 32 {
            return;
        }
        if self.slots.len() <= target {
            self.slots.resize_with(target + 1, || None);
        }
        match &mut self.slots[target] {
            Some(s) => {
                if r.base == 0 {
                    s.number = r.number;
                    s.is_dir = r.is_dir;
                    if r.has_std {
                        s.modified = r.modified;
                        s.attrs = r.attrs;
                        s.has_std = true;
                    }
                    s.base = 0;
                }
                s.names.extend(r.names);
                s.logical = s.logical.saturating_add(r.logical);
                s.allocated = s.allocated.saturating_add(r.allocated);
                s.has_ads |= r.has_ads;
                if r.reparse_tag != 0 {
                    s.reparse_tag = r.reparse_tag;
                }
            }
            slot @ None => *slot = Some(Box::new(r)),
        }
    }

    pub fn build(&self, root_name: &str) -> IndexBuilder {
        const ROOT_RECORD: usize = 5;
        const BADCLUS: usize = 8;
        let mut b = IndexBuilder::new(root_name);
        let mut first = vec![NONE; self.slots.len()];
        if self.slots.len() > ROOT_RECORD {
            first[ROOT_RECORD] = ROOT;
        }
        let mut links: Vec<(NodeId, u64)> = Vec::new();
        for (no, s) in self.slots.iter().enumerate() {
            let Some(r) = s else { continue };
            if no == ROOT_RECORD || r.base != 0 {
                continue;
            }
            let mut f = 0u8;
            if r.is_dir {
                f |= flag::DIR;
            }
            if r.attrs & FILE_ATTR_REPARSE != 0 {
                if r.reparse_tag == TAG_MOUNT_POINT || r.reparse_tag == TAG_SYMLINK {
                    f |= flag::LINK;
                } else if r.reparse_tag & 0xF000_0000 == 0x9000_0000 {
                    f |= flag::PLACEHOLDER;
                }
            }
            if r.has_ads {
                f |= flag::ADS;
            }
            if no < 24 {
                f |= flag::SYSTEM;
            }
            let (lg, al) = if no == BADCLUS { (0, 0) } else { (r.logical, r.allocated) };
            for (i, n) in r.names.iter().enumerate() {
                let (fl, l, a) = if i == 0 { (f, lg, al) } else { (f | flag::HARDLINK, 0, 0) };
                let id = b.add(NONE, &n.name, l, a, r.modified, fl);
                if i == 0 {
                    first[no] = id;
                }
                links.push((id, n.parent));
            }
        }
        for (id, parent) in links {
            let p = first.get(parent as usize).copied().unwrap_or(NONE);
            b.set_parent(id, p);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oz_ntfs::FileName;

    fn rec(no: u64, base: u64, dir: bool, names: &[(u64, &str)], alloc: u64) -> Record {
        Record {
            number: no,
            base,
            is_dir: dir,
            has_std: base == 0,
            allocated: alloc,
            logical: alloc,
            names: names.iter().map(|(p, n)| FileName { parent: *p, name: n.to_string() }).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn assembles_any_order_with_extensions_links_and_orphans() {
        let mut a = Assembler::default();
        a.add(rec(40, 30, false, &[], 500)); // extension before base
        a.add(rec(30, 0, false, &[(20, "file.bin")], 100));
        a.add(rec(20, 0, true, &[(5, "dir")], 0));
        a.add(rec(5, 0, true, &[(5, ".")], 0));
        a.add(rec(50, 0, false, &[(5, "one"), (20, "two")], 1000)); // hard link
        a.add(rec(60, 0, false, &[(999, "orphan")], 7));
        let mut j = rec(70, 0, true, &[(5, "junction")], 0);
        j.attrs = FILE_ATTR_REPARSE;
        j.reparse_tag = TAG_MOUNT_POINT;
        a.add(j);
        let ix = a.build("C:\\").build();
        assert_eq!(ix.allocated(ROOT), 1600);
        assert_eq!(ix.orphan_allocated, 7);
        let names: Vec<_> = ix.children(ROOT).iter().map(|&c| ix.name(c)).collect();
        assert_eq!(names[0], "one");
        let jn = ix.children(ROOT).iter().copied().find(|&c| ix.name(c) == "junction").unwrap();
        assert!(ix.flags(jn) & flag::LINK != 0);
    }

    #[test]
    fn badclus_is_excluded() {
        let mut a = Assembler::default();
        a.add(rec(5, 0, true, &[(5, ".")], 0));
        a.add(rec(8, 0, false, &[(5, "$BadClus")], 9_999_999));
        assert_eq!(a.build("C:\\").build().allocated(ROOT), 0);
    }
}
