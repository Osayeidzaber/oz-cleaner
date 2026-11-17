//! Minimal NTFS MFT record parser (D-08). Operates only on byte slices and does
//! no I/O. Input is untrusted: every read is bounds-checked and bad records
//! return None or partial data, never a panic.
#![forbid(unsafe_code)]

pub const FILE_ATTR_REPARSE: u32 = 0x400;
pub const FILE_ATTR_HIDDEN: u32 = 0x2;
pub const FILE_ATTR_SYSTEM: u32 = 0x4;

const ATTR_STD: u32 = 0x10;
const ATTR_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_REPARSE: u32 = 0xC0;
const ATTR_END: u32 = 0xFFFF_FFFF;

pub const TAG_MOUNT_POINT: u32 = 0xA000_0003;
pub const TAG_SYMLINK: u32 = 0xA000_000C;

#[derive(Debug, Clone, PartialEq)]
pub struct FileName {
    pub parent: u64,
    pub name: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Record {
    pub number: u64,
    pub is_dir: bool,
    /// Non-zero for extension records: their attributes belong to this record.
    pub base: u64,
    pub names: Vec<FileName>,
    pub modified: i64,
    pub attrs: u32,
    pub has_std: bool,
    pub logical: u64,
    pub allocated: u64,
    pub has_ads: bool,
    pub reparse_tag: u32,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Applies the update sequence array. Returns false if the record is torn.
pub fn apply_fixups(rec: &mut [u8], sector: usize) -> bool {
    let (Some(off), Some(count)) = (u16_at(rec, 4), u16_at(rec, 6)) else { return false };
    let (off, count) = (off as usize, count as usize);
    if count < 1 || sector < 2 || off + count * 2 > rec.len() {
        return false;
    }
    let (u0, u1) = (rec[off], rec[off + 1]);
    for i in 1..count {
        let end = i * sector;
        if end > rec.len() {
            return false;
        }
        if rec[end - 2] != u0 || rec[end - 1] != u1 {
            return false;
        }
        rec[end - 2] = rec[off + i * 2];
        rec[end - 1] = rec[off + i * 2 + 1];
    }
    true
}

/// Parses a fixed-up record. Returns None for free or invalid records.
pub fn parse_record(b: &[u8], number: u64) -> Option<Record> {
    if b.get(0..4)? != b"FILE" {
        return None;
    }
    let flags = u16_at(b, 22)?;
    if flags & 1 == 0 {
        return None;
    }
    let mut r = Record {
        number,
        is_dir: flags & 2 != 0,
        base: u64_at(b, 32)? & 0x0000_FFFF_FFFF_FFFF,
        ..Default::default()
    };
    let mut o = u16_at(b, 20)? as usize;
    // win32 name preferred over DOS-only names; POSIX kept.
    for _ in 0..512 {
        let ty = u32_at(b, o)?;
        if ty == ATTR_END {
            break;
        }
        let len = u32_at(b, o + 4)? as usize;
        if len < 16 || o + len > b.len() {
            break;
        }
        let a = &b[o..o + len];
        let non_resident = a[8] != 0;
        let name_len = a[9];
        match (ty, non_resident) {
            (ATTR_STD, false) => {
                if let Some(v) = resident_value(a) {
                    r.modified = u64_at(v, 8).unwrap_or(0) as i64;
                    r.attrs = u32_at(v, 32).unwrap_or(0);
                    r.has_std = true;
                }
            }
            (ATTR_NAME, false) => {
                if let Some(v) = resident_value(a) {
                    if let Some(n) = parse_name(v) {
                        r.names.push(n);
                    }
                }
            }
            (ATTR_DATA, _) => add_data(&mut r, a, non_resident, name_len),
            // Directory index blocks occupy real clusters; count them on disk.
            (0xA0, true) => {
                if u64_at(a, 16) == Some(0) {
                    r.allocated = r.allocated.saturating_add(u64_at(a, 40).unwrap_or(0));
                }
            }
            (ATTR_REPARSE, false) => {
                if let Some(v) = resident_value(a) {
                    r.reparse_tag = u32_at(v, 0).unwrap_or(0);
                }
            }
            _ => {}
        }
        o += len;
    }
    Some(r)
}

fn resident_value(a: &[u8]) -> Option<&[u8]> {
    let vlen = u32_at(a, 16)? as usize;
    let voff = u16_at(a, 20)? as usize;
    a.get(voff..voff.checked_add(vlen)?)
}

fn parse_name(v: &[u8]) -> Option<FileName> {
    let parent = u64_at(v, 0)? & 0x0000_FFFF_FFFF_FFFF;
    let len = *v.get(64)? as usize;
    let ns = *v.get(65)?;
    if ns == 2 {
        return None; // DOS 8.3 alias of a name present elsewhere
    }
    let raw = v.get(66..66 + len * 2)?;
    let wide: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(FileName { parent, name: String::from_utf16_lossy(&wide) })
}

fn add_data(r: &mut Record, a: &[u8], non_resident: bool, name_len: u8) {
    if name_len != 0 {
        r.has_ads = true;
    }
    if non_resident {
        // Only the first extent (starting VCN 0) carries sizes.
        if u64_at(a, 16) != Some(0) {
            return;
        }
        let aflags = u16_at(a, 12).unwrap_or(0);
        let compressed_or_sparse = aflags & 0x8001 != 0;
        let real = u64_at(a, 48).unwrap_or(0);
        let alloc = if compressed_or_sparse {
            u64_at(a, 64).unwrap_or(0)
        } else {
            u64_at(a, 40).unwrap_or(0)
        };
        r.logical = r.logical.saturating_add(real);
        r.allocated = r.allocated.saturating_add(alloc);
    } else if let Some(v) = resident_value(a) {
        // Resident data lives inside the MFT record and allocates no clusters.
        r.logical = r.logical.saturating_add(v.len() as u64);
    }
}

/// Decodes a non-resident attribute's data runs into (lcn, clusters) pairs.
pub fn decode_runs(runs: &[u8]) -> Option<Vec<(i64, u64)>> {
    let mut out = Vec::new();
    let (mut i, mut lcn) = (0usize, 0i64);
    while i < runs.len() {
        let h = runs[i];
        if h == 0 {
            return Some(out);
        }
        let (ll, ol) = ((h & 0xF) as usize, (h >> 4) as usize);
        if ll == 0 || ll > 8 || ol > 8 {
            return None;
        }
        let lb = runs.get(i + 1..i + 1 + ll)?;
        let ob = runs.get(i + 1 + ll..i + 1 + ll + ol)?;
        let mut len = 0u64;
        for (k, v) in lb.iter().enumerate() {
            len |= (*v as u64) << (8 * k);
        }
        if ol > 0 {
            let mut d = 0i64;
            for (k, v) in ob.iter().enumerate() {
                d |= (*v as i64) << (8 * k);
            }
            if ob[ol - 1] & 0x80 != 0 && ol < 8 {
                d |= -1i64 << (8 * ol);
            }
            lcn = lcn.checked_add(d)?;
            out.push((lcn, len));
        } else {
            out.push((-1, len)); // sparse run
        }
        i += 1 + ll + ol;
    }
    Some(out)
}

/// Finds the unnamed $DATA runs in the fixed-up record 0 ($MFT).
pub fn mft_runs(rec0: &[u8]) -> Option<Vec<(i64, u64)>> {
    let mut o = u16_at(rec0, 20)? as usize;
    for _ in 0..64 {
        let ty = u32_at(rec0, o)?;
        if ty == ATTR_END {
            return None;
        }
        let len = u32_at(rec0, o + 4)? as usize;
        if len < 16 || o + len > rec0.len() {
            return None;
        }
        let a = &rec0[o..o + len];
        if ty == ATTR_DATA && a[8] != 0 && a[9] == 0 {
            let ro = u16_at(a, 32)? as usize;
            return decode_runs(a.get(ro..)?);
        }
        o += len;
    }
    None
}

/// Compact wire encoding used between the elevated helper and the core.
pub fn encode_record(r: &Record, out: &mut Vec<u8>) {
    out.extend_from_slice(&r.number.to_le_bytes());
    out.extend_from_slice(&r.base.to_le_bytes());
    out.extend_from_slice(&r.modified.to_le_bytes());
    out.extend_from_slice(&r.logical.to_le_bytes());
    out.extend_from_slice(&r.allocated.to_le_bytes());
    out.extend_from_slice(&r.attrs.to_le_bytes());
    out.extend_from_slice(&r.reparse_tag.to_le_bytes());
    out.push(r.is_dir as u8 | (r.has_ads as u8) << 1 | (r.has_std as u8) << 2);
    out.push(r.names.len().min(255) as u8);
    for n in r.names.iter().take(255) {
        out.extend_from_slice(&n.parent.to_le_bytes());
        let b = n.name.as_bytes();
        let l = b.len().min(u16::MAX as usize);
        out.extend_from_slice(&(l as u16).to_le_bytes());
        out.extend_from_slice(&b[..l]);
    }
}

/// Decodes one record, advancing `o`. None on truncated input.
pub fn decode_record(b: &[u8], o: &mut usize) -> Option<Record> {
    let s = *o;
    let flags = *b.get(s + 48)?;
    let count = *b.get(s + 49)? as usize;
    let mut r = Record {
        number: u64_at(b, s)?,
        base: u64_at(b, s + 8)?,
        modified: u64_at(b, s + 16)? as i64,
        logical: u64_at(b, s + 24)?,
        allocated: u64_at(b, s + 32)?,
        attrs: u32_at(b, s + 40)?,
        reparse_tag: u32_at(b, s + 44)?,
        is_dir: flags & 1 != 0,
        has_ads: flags & 2 != 0,
        has_std: flags & 4 != 0,
        names: Vec::with_capacity(count),
    };
    let mut p = s + 50;
    for _ in 0..count {
        let parent = u64_at(b, p)?;
        let l = u16_at(b, p + 8)? as usize;
        let name = std::str::from_utf8(b.get(p + 10..p + 10 + l)?).ok()?.to_string();
        r.names.push(FileName { parent, name });
        p += 10 + l;
    }
    *o = p;
    Some(r)
}

#[cfg(test)]
mod tests {
    #[test]
    fn wire_roundtrip() {
        let r = super::Record {
            number: 9,
            base: 2,
            modified: 77,
            logical: 5,
            allocated: 4096,
            attrs: 3,
            reparse_tag: 8,
            is_dir: true,
            has_ads: true,
            has_std: true,
            names: vec![super::FileName { parent: 5, name: "dé.txt".into() }],
        };
        let mut b = Vec::new();
        super::encode_record(&r, &mut b);
        let mut o = 0;
        assert_eq!(super::decode_record(&b, &mut o).unwrap(), r);
        assert_eq!(o, b.len());
        let mut o = 0;
        assert!(super::decode_record(&b[..b.len() - 1], &mut o).is_none());
    }

    use super::*;

    fn name_attr(parent: u64, name: &str, ns: u8) -> Vec<u8> {
        let w: Vec<u16> = name.encode_utf16().collect();
        let mut v = vec![0u8; 66 + w.len() * 2];
        v[0..8].copy_from_slice(&parent.to_le_bytes());
        v[64] = w.len() as u8;
        v[65] = ns;
        for (i, c) in w.iter().enumerate() {
            v[66 + i * 2..68 + i * 2].copy_from_slice(&c.to_le_bytes());
        }
        resident(0x30, &v)
    }
    fn resident(ty: u32, val: &[u8]) -> Vec<u8> {
        let total = (24 + val.len() + 7) & !7;
        let mut a = vec![0u8; total];
        a[0..4].copy_from_slice(&ty.to_le_bytes());
        a[4..8].copy_from_slice(&(total as u32).to_le_bytes());
        a[16..20].copy_from_slice(&(val.len() as u32).to_le_bytes());
        a[20..22].copy_from_slice(&24u16.to_le_bytes());
        a[24..24 + val.len()].copy_from_slice(val);
        a
    }
    fn nonres(real: u64, alloc: u64, total: u64, flags: u16, named: bool) -> Vec<u8> {
        let mut a = vec![0u8; 72];
        a[0..4].copy_from_slice(&0x80u32.to_le_bytes());
        a[4..8].copy_from_slice(&72u32.to_le_bytes());
        a[8] = 1;
        a[9] = named as u8;
        a[12..14].copy_from_slice(&flags.to_le_bytes());
        a[40..48].copy_from_slice(&alloc.to_le_bytes());
        a[48..56].copy_from_slice(&real.to_le_bytes());
        a[64..72].copy_from_slice(&total.to_le_bytes());
        a
    }
    fn record(flags: u16, attrs: &[Vec<u8>]) -> Vec<u8> {
        let mut b = vec![0u8; 1024];
        b[0..4].copy_from_slice(b"FILE");
        b[20..22].copy_from_slice(&56u16.to_le_bytes());
        b[22..24].copy_from_slice(&flags.to_le_bytes());
        let mut o = 56;
        for a in attrs {
            b[o..o + a.len()].copy_from_slice(a);
            o += a.len();
        }
        b[o..o + 4].copy_from_slice(&ATTR_END.to_le_bytes());
        b
    }

    #[test]
    fn hard_links_and_dos_alias() {
        let b = record(1, &[name_attr(5, "LONGNA~1.TXT", 2), name_attr(5, "long name.txt", 1), name_attr(9, "second.txt", 1)]);
        let r = parse_record(&b, 40).unwrap();
        assert_eq!(r.names.len(), 2);
        assert_eq!(r.names[0].name, "long name.txt");
        assert_eq!(r.names[1].parent, 9);
    }

    #[test]
    fn sizes_normal_compressed_and_ads() {
        let r = parse_record(&record(1, &[nonres(10000, 12288, 999, 0, false)]), 1).unwrap();
        assert_eq!((r.logical, r.allocated), (10000, 12288));
        let r = parse_record(&record(1, &[nonres(10000, 12288, 4096, 0x8000, false)]), 1).unwrap();
        assert_eq!(r.allocated, 4096);
        let r = parse_record(&record(1, &[nonres(100, 4096, 0, 0, false), nonres(50, 4096, 0, 0, true)]), 1).unwrap();
        assert_eq!(r.allocated, 8192);
        assert!(r.has_ads);
        let r = parse_record(&record(1, &[resident(0x80, &[1, 2, 3])]), 1).unwrap();
        assert_eq!((r.logical, r.allocated), (3, 0));
    }

    #[test]
    fn free_and_bad_records() {
        assert!(parse_record(&record(0, &[]), 1).is_none());
        assert!(parse_record(&[0u8; 1024], 1).is_none());
        assert!(parse_record(b"FILE", 1).is_none());
    }

    #[test]
    fn fixups() {
        let mut b = vec![0u8; 1024];
        b[4..6].copy_from_slice(&48u16.to_le_bytes());
        b[6..8].copy_from_slice(&3u16.to_le_bytes());
        b[48] = 0xAA;
        b[49] = 0xBB;
        b[50] = 1;
        b[51] = 2;
        b[52] = 3;
        b[53] = 4;
        b[510] = 0xAA;
        b[511] = 0xBB;
        b[1022] = 0xAA;
        b[1023] = 0xBB;
        assert!(apply_fixups(&mut b, 512));
        assert_eq!(&b[510..512], &[1, 2]);
        assert_eq!(&b[1022..1024], &[3, 4]);
        let mut t = b.clone();
        t[48] = 0x11;
        assert!(!apply_fixups(&mut t, 512));
    }

    #[test]
    fn runs() {
        // len 0x18 at lcn 0x5634, then len 0x10 at -0x10 relative, then sparse len 5
        let r = decode_runs(&[0x21, 0x18, 0x34, 0x56, 0x11, 0x10, 0xF0, 0x01, 0x05, 0x00]).unwrap();
        assert_eq!(r, vec![(0x5634, 0x18), (0x5624, 0x10), (-1, 5)]);
        assert!(decode_runs(&[0x99]).is_none());
    }

    /// Deterministic corruption test: must never panic.
    #[test]
    fn fuzz_never_panics() {
        let base = record(3, &[name_attr(5, "x.txt", 1), nonres(1, 2, 3, 0, false), resident(0x10, &[0; 48])]);
        let mut s: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for _ in 0..200_000 {
            let mut b = base.clone();
            for _ in 0..(next() % 12) {
                let i = (next() as usize) % b.len().min(240);
                b[i] = next() as u8;
            }
            let cut = if next() % 8 == 0 { (next() as usize) % b.len() } else { b.len() };
            let n = parse_record(&b[..cut], 1);
            let _ = n;
            let mut c = b.clone();
            let _ = apply_fixups(&mut c, 512);
            let _ = mft_runs(&b[..cut]);
            let _ = decode_runs(&b[..cut.min(64)]);
        }
    }
}
