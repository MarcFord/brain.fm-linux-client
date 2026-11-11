//! A read-only reader for the LevelDB files Chromium keeps `localStorage` in.
//!
//! Only enough of the format to recover `(key, value)` records: the
//! write-ahead log (`*.log`) and the sorted tables (`*.ldb`). Nothing here
//! opens the database properly, takes its lock, or replays anything — the
//! browser may be running, and all we want is to look.
//!
//! Why parse at all, rather than grep the files for token-shaped bytes? A
//! token on its own says nothing about where it came from. `localStorage`
//! for every site the user has visited lives in the same files, and a JWT
//! that merely *looks* like Brain.fm's could belong to any of them. Handing
//! such a token to Brain.fm would leak another site's credential. The record
//! key names the origin that wrote the value, and that is the only thing
//! that can be trusted to say whose token it is.
//!
//! Format references: `doc/log_format.md` and `doc/table_format.md` in the
//! LevelDB source tree.

use std::path::Path;

/// A live record. Deletions are dropped during parsing.
pub struct Record {
    pub key: Vec<u8>,
    pub value: Vec<u8>,
}

/// Every live record in every log and table file of a database directory.
///
/// Files are read as a snapshot, with no attention to which the manifest
/// still considers current. An obsolete table waiting to be unlinked may
/// therefore contribute stale records; callers that care about freshness
/// must judge it from the record itself.
pub fn read_dir(dir: &Path) -> Vec<Record> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = entry.path();
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        let Ok(bytes) = std::fs::read(&p) else {
            continue;
        };
        match ext {
            "log" => read_log(&bytes, &mut out),
            // `.sst` is the name older LevelDB builds used for the same file.
            "ldb" | "sst" => read_table(&bytes, &mut out),
            _ => {}
        }
    }
    out
}

/// Decode a LEB128 varint, returning the value and the position after it.
fn varint(b: &[u8], mut p: usize) -> Option<(u64, usize)> {
    let mut v: u64 = 0;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(p)?;
        p += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((v, p));
        }
    }
    None
}

fn varint_usize(b: &[u8], p: usize) -> Option<(usize, usize)> {
    let (v, p) = varint(b, p)?;
    Some((usize::try_from(v).ok()?, p))
}

// --- write-ahead log -------------------------------------------------------

const LOG_BLOCK: usize = 32 * 1024;
const LOG_HEADER: usize = 7;

/// Parse a `*.log` file: 32 KiB blocks of framed fragments, each logical
/// record being a write batch.
pub fn read_log(bytes: &[u8], out: &mut Vec<Record>) {
    let mut pending: Vec<u8> = Vec::new();
    let mut pos = 0;

    while pos + LOG_HEADER <= bytes.len() {
        // A fragment never straddles a block; the tail of a block too short
        // for a header is zero padding.
        let block_left = LOG_BLOCK - pos % LOG_BLOCK;
        if block_left < LOG_HEADER {
            pos += block_left;
            continue;
        }
        let len = usize::from(u16::from_le_bytes([bytes[pos + 4], bytes[pos + 5]]));
        let kind = bytes[pos + 6];
        let start = pos + LOG_HEADER;
        let end = start + len;
        if end > bytes.len() || len > block_left - LOG_HEADER {
            break;
        }
        let frag = &bytes[start..end];

        match kind {
            // Zero-type, zero-length fragments are space the writer
            // preallocated and has not reached; skip them.
            0 if len == 0 => {}
            1 => {
                read_batch(frag, out);
                pending.clear();
            }
            2 => {
                pending.clear();
                pending.extend_from_slice(frag);
            }
            3 => pending.extend_from_slice(frag),
            4 => {
                pending.extend_from_slice(frag);
                read_batch(&pending, out);
                pending.clear();
            }
            // Anything else is a torn write or a format we do not know.
            // Stop rather than invent records out of whatever follows.
            _ => break,
        }
        pos = end;
    }
}

/// A write batch: 8-byte sequence, 4-byte count, then `count` entries of
/// `kind, key, [value]`. Kind 1 is a put, kind 0 a delete.
fn read_batch(b: &[u8], out: &mut Vec<Record>) {
    if b.len() < 12 {
        return;
    }
    let count = u32::from_le_bytes([b[8], b[9], b[10], b[11]]);
    let mut p = 12;
    for _ in 0..count {
        let Some(&kind) = b.get(p) else { return };
        p += 1;
        let Some((klen, after)) = varint_usize(b, p) else {
            return;
        };
        let Some(key) = b.get(after..after + klen) else {
            return;
        };
        p = after + klen;
        match kind {
            1 => {
                let Some((vlen, after)) = varint_usize(b, p) else {
                    return;
                };
                let Some(value) = b.get(after..after + vlen) else {
                    return;
                };
                p = after + vlen;
                out.push(Record {
                    key: key.to_vec(),
                    value: value.to_vec(),
                });
            }
            0 => {}
            _ => return,
        }
    }
}

// --- sorted tables ---------------------------------------------------------

const TABLE_MAGIC: u64 = 0xdb47_7524_8b80_fb57;
const FOOTER_LEN: usize = 48;
const BLOCK_TRAILER: usize = 5;

/// Parse a `*.ldb` file: a footer locates the index block, the index block
/// locates every data block, and each data block is a run of
/// prefix-compressed entries keyed by `user_key ++ (sequence << 8 | kind)`.
pub fn read_table(bytes: &[u8], out: &mut Vec<Record>) {
    if bytes.len() < FOOTER_LEN {
        return;
    }
    let footer = &bytes[bytes.len() - FOOTER_LEN..];
    let magic = u64::from_le_bytes(footer[40..48].try_into().expect("8 bytes"));
    if magic != TABLE_MAGIC {
        return;
    }
    // metaindex handle first (unused), then the index handle.
    let Some((_, p)) = handle(footer, 0) else {
        return;
    };
    let Some(((idx_off, idx_len), _)) = handle(footer, p) else {
        return;
    };
    let Some(index) = read_block(bytes, idx_off, idx_len) else {
        return;
    };

    for (_, value) in block_entries(&index) {
        let Some(((off, len), _)) = handle(&value, 0) else {
            continue;
        };
        let Some(data) = read_block(bytes, off, len) else {
            continue;
        };
        for (ikey, value) in block_entries(&data) {
            // Internal key: user key followed by an 8-byte trailer whose low
            // byte is the kind. 1 is a live value; 0 a tombstone.
            let Some(split) = ikey.len().checked_sub(8) else {
                continue;
            };
            if ikey[split] == 1 {
                out.push(Record {
                    key: ikey[..split].to_vec(),
                    value,
                });
            }
        }
    }
}

/// A block handle: `(offset, size)`, both varints.
fn handle(b: &[u8], p: usize) -> Option<((usize, usize), usize)> {
    let (off, p) = varint_usize(b, p)?;
    let (len, p) = varint_usize(b, p)?;
    Some(((off, len), p))
}

/// The contents of one block, decompressed. Every block is followed by a
/// one-byte compression type and a four-byte checksum.
fn read_block(bytes: &[u8], off: usize, len: usize) -> Option<Vec<u8>> {
    let data = bytes.get(off..off.checked_add(len)?)?;
    let kind = *bytes.get(off + len)?;
    if bytes.len() < off + len + BLOCK_TRAILER {
        return None;
    }
    match kind {
        0 => Some(data.to_vec()),
        1 => snap::raw::Decoder::new().decompress_vec(data).ok(),
        // Zstd and whatever comes after it: not something Chromium writes
        // today. Skipping the block loses records; guessing would invent them.
        _ => None,
    }
}

/// Walk a block's entries in order, reconstructing each prefix-compressed
/// key. The block ends with a restart array we do not need: reading from
/// the first entry, every key is reachable by accumulation.
fn block_entries(block: &[u8]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out = Vec::new();
    if block.len() < 4 {
        return out;
    }
    let n_restarts = u32::from_le_bytes(block[block.len() - 4..].try_into().expect("4 bytes"));
    let Some(entries_end) = (block.len() - 4).checked_sub(4 * n_restarts as usize) else {
        return out;
    };
    let entries = &block[..entries_end];

    let mut key: Vec<u8> = Vec::new();
    let mut p = 0;
    while p < entries.len() {
        let Some((shared, q)) = varint_usize(entries, p) else {
            break;
        };
        let Some((unshared, q)) = varint_usize(entries, q) else {
            break;
        };
        let Some((vlen, q)) = varint_usize(entries, q) else {
            break;
        };
        if shared > key.len() {
            break;
        }
        let Some(delta) = entries.get(q..q + unshared) else {
            break;
        };
        let Some(value) = entries.get(q + unshared..q + unshared + vlen) else {
            break;
        };
        key.truncate(shared);
        key.extend_from_slice(delta);
        out.push((key.clone(), value.to_vec()));
        p = q + unshared + vlen;
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn put_varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return;
            }
            out.push(byte | 0x80);
        }
    }

    /// Build a `*.log` file holding one batch of puts and deletes, as the
    /// LevelDB writer would, split into fragments of at most `frag` bytes.
    pub(crate) fn log_file(entries: &[(&[u8], Option<&[u8]>)], frag: usize) -> Vec<u8> {
        let mut batch = Vec::new();
        batch.extend_from_slice(&7u64.to_le_bytes());
        batch.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (k, v) in entries {
            match v {
                Some(v) => {
                    batch.push(1);
                    put_varint(&mut batch, k.len() as u64);
                    batch.extend_from_slice(k);
                    put_varint(&mut batch, v.len() as u64);
                    batch.extend_from_slice(v);
                }
                None => {
                    batch.push(0);
                    put_varint(&mut batch, k.len() as u64);
                    batch.extend_from_slice(k);
                }
            }
        }

        let chunks: Vec<&[u8]> = batch.chunks(frag).collect();
        let mut file = Vec::new();
        for (i, c) in chunks.iter().enumerate() {
            let kind = match (i == 0, i + 1 == chunks.len()) {
                (true, true) => 1,
                (true, false) => 2,
                (false, false) => 3,
                (false, true) => 4,
            };
            file.extend_from_slice(&[0, 0, 0, 0]); // checksum, unverified
            file.extend_from_slice(&(c.len() as u16).to_le_bytes());
            file.push(kind);
            file.extend_from_slice(c);
        }
        file
    }

    /// Build a block in the on-disk layout, with a restart every 2 entries
    /// so prefix compression actually happens. Returns the raw contents,
    /// without trailer.
    fn block(entries: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut b = Vec::new();
        let mut restarts = Vec::new();
        let mut prev: Vec<u8> = Vec::new();
        for (i, (k, v)) in entries.iter().enumerate() {
            let shared = if i % 2 == 0 {
                restarts.push(b.len() as u32);
                0
            } else {
                prev.iter()
                    .zip(k.iter())
                    .take_while(|(a, b)| a == b)
                    .count()
            };
            put_varint(&mut b, shared as u64);
            put_varint(&mut b, (k.len() - shared) as u64);
            put_varint(&mut b, v.len() as u64);
            b.extend_from_slice(&k[shared..]);
            b.extend_from_slice(v);
            prev = k.to_vec();
        }
        for r in &restarts {
            b.extend_from_slice(&r.to_le_bytes());
        }
        b.extend_from_slice(&(restarts.len() as u32).to_le_bytes());
        b
    }

    fn internal_key(user: &[u8], seq: u64, kind: u8) -> Vec<u8> {
        let mut k = user.to_vec();
        k.extend_from_slice(&((seq << 8) | u64::from(kind)).to_le_bytes());
        k
    }

    /// Build a `*.ldb` file with one data block, optionally Snappy-compressed.
    pub(crate) fn table_file(entries: &[(&[u8], u8, &[u8])], compress: bool) -> Vec<u8> {
        let keyed: Vec<(Vec<u8>, &[u8])> = entries
            .iter()
            .enumerate()
            .map(|(i, (k, kind, v))| (internal_key(k, 100 - i as u64, *kind), *v))
            .collect();
        let refs: Vec<(&[u8], &[u8])> = keyed.iter().map(|(k, v)| (k.as_slice(), *v)).collect();
        let data = block(&refs);

        let mut file = Vec::new();
        let data_off = 0;
        let data_len = if compress {
            let c = snap::raw::Encoder::new().compress_vec(&data).unwrap();
            file.extend_from_slice(&c);
            file.push(1);
            c.len()
        } else {
            file.extend_from_slice(&data);
            file.push(0);
            data.len()
        };
        file.extend_from_slice(&[0, 0, 0, 0]);

        let mut h = Vec::new();
        put_varint(&mut h, data_off as u64);
        put_varint(&mut h, data_len as u64);
        let last_key = internal_key(b"~", 0, 1);
        let index = block(&[(&last_key, &h)]);
        let index_off = file.len();
        file.extend_from_slice(&index);
        file.push(0);
        file.extend_from_slice(&[0, 0, 0, 0]);

        let meta_off = file.len();
        let meta = block(&[]);
        file.extend_from_slice(&meta);
        file.push(0);
        file.extend_from_slice(&[0, 0, 0, 0]);

        let mut footer = Vec::new();
        put_varint(&mut footer, meta_off as u64);
        put_varint(&mut footer, meta.len() as u64);
        put_varint(&mut footer, index_off as u64);
        put_varint(&mut footer, index.len() as u64);
        footer.resize(40, 0);
        footer.extend_from_slice(&TABLE_MAGIC.to_le_bytes());
        file.extend_from_slice(&footer);
        file
    }

    #[test]
    fn log_yields_puts_and_drops_deletes() {
        let file = log_file(
            &[
                (b"_https://a.example\x00\x01k", Some(b"\x01v1")),
                (b"_https://b.example\x00\x01k", None),
                (b"_https://c.example\x00\x01k", Some(b"\x01v3")),
            ],
            usize::MAX,
        );
        let mut out = Vec::new();
        read_log(&file, &mut out);
        let keys: Vec<&[u8]> = out.iter().map(|r| r.key.as_slice()).collect();
        assert_eq!(
            keys,
            vec![
                b"_https://a.example\x00\x01k".as_slice(),
                b"_https://c.example\x00\x01k".as_slice()
            ]
        );
        assert_eq!(out[1].value, b"\x01v3");
    }

    #[test]
    fn log_reassembles_fragmented_batches() {
        let value = vec![b'x'; 300];
        let file = log_file(&[(b"_https://a.example\x00\x01big", Some(&value))], 64);
        let mut out = Vec::new();
        read_log(&file, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].value, value);
    }

    #[test]
    fn log_stops_at_garbage_instead_of_inventing_records() {
        let mut file = log_file(&[(b"k", Some(b"v"))], usize::MAX);
        file.extend_from_slice(&[0, 0, 0, 0, 0xff, 0xff, 9, 1, 2, 3]);
        let mut out = Vec::new();
        read_log(&file, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn table_yields_live_records_with_user_keys() {
        for compress in [false, true] {
            let file = table_file(
                &[
                    (b"_https://a.example\x00\x01k", 1, b"\x01live"),
                    (b"_https://a.example\x00\x01z", 0, b""),
                    (b"_https://b.example\x00\x01k", 1, b"\x01other"),
                ],
                compress,
            );
            let mut out = Vec::new();
            read_table(&file, &mut out);
            let got: Vec<(&[u8], &[u8])> = out
                .iter()
                .map(|r| (r.key.as_slice(), r.value.as_slice()))
                .collect();
            assert_eq!(
                got,
                vec![
                    (
                        b"_https://a.example\x00\x01k".as_slice(),
                        b"\x01live".as_slice()
                    ),
                    (
                        b"_https://b.example\x00\x01k".as_slice(),
                        b"\x01other".as_slice()
                    ),
                ],
                "compress={compress}"
            );
        }
    }

    #[test]
    fn table_without_magic_is_ignored() {
        let mut out = Vec::new();
        read_table(&[0u8; 100], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn varint_decodes_multi_byte_values() {
        let mut b = Vec::new();
        put_varint(&mut b, 300);
        assert_eq!(varint(&b, 0), Some((300, 2)));
        assert_eq!(varint(&[0x80], 0), None, "truncated");
    }
}
