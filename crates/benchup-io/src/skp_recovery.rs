use std::{
    collections::BTreeSet,
    io::{self, Cursor, Read},
};
use zip::{ZipWriter, read::read_zipfile_from_stream};

const LOCAL_HEADER: &[u8; 4] = b"PK\x03\x04";
const MAX_RECOVERED_BYTES: u64 = 256 * 1024 * 1024;

/// Rebuild an unreadable modern SKP directory from intact local ZIP records.
/// BenchUp uses geometry and base materials, so only copy entries it needs.
pub(crate) fn recover(bytes: &[u8]) -> Option<Vec<u8>> {
    if !bytes.starts_with(&[0xff, 0xfe, 0xff, 0x0e]) {
        return None;
    }
    let start = bytes[..bytes.len().min(4096)]
        .windows(4)
        .position(|window| window == LOCAL_HEADER)?;
    let mut entries = BTreeSet::new();
    let mut remaining = MAX_RECOVERED_BYTES;
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    for (offset, _) in bytes[start..]
        .windows(4)
        .enumerate()
        .filter(|(_, window)| *window == LOCAL_HEADER)
    {
        let mut record = &bytes[start + offset..];
        let Some(header) = record.get(..30) else {
            continue;
        };
        let flags = u16::from_le_bytes([header[6], header[7]]);
        let name_len = u16::from_le_bytes([header[26], header[27]]) as usize;
        let Some(name) = record.get(30..30 + name_len) else {
            continue;
        };
        // Encryption and data descriptors prevent verification from the local
        // header alone. Keep deflate level and UTF-8 flags.
        if flags & !0x0806 != 0 || !needed(name) {
            continue;
        }
        let raw_record = record;
        let Ok(Some(mut entry)) = read_zipfile_from_stream(&mut record) else {
            continue;
        };
        let size = entry.size();
        let compressed = entry.compressed_size();
        if size == 0 || compressed == 0 || compressed > raw_record.len() as u64 {
            continue;
        }
        remaining = remaining.checked_sub(size)?;
        // Reading to EOF checks the CRC. The extra byte also detects any
        // mismatch between the declared size and the decompressed payload.
        if io::copy(&mut (&mut entry).take(size + 1), &mut io::sink()).ok() != Some(size) {
            continue;
        }
        if !entries.insert(name) {
            if name == b"model.dat" {
                return None;
            }
            continue;
        }
        let mut raw = raw_record;
        archive
            .raw_copy_file(read_zipfile_from_stream(&mut raw).ok()??)
            .ok()?;
    }
    if !entries.contains(b"model.dat".as_slice()) {
        return None;
    }
    let zip = archive.finish().ok()?.into_inner();
    let mut recovered = bytes[..start].to_vec();
    recovered.extend(zip);
    (recovered.len() <= crate::interchange::MAX_SKP_BYTES).then_some(recovered)
}

fn needed(name: &[u8]) -> bool {
    name == b"model.dat"
        || name == b"meta/meta.dat"
        || (name.starts_with(b"materials/") && name.ends_with(b"/material.xml"))
}
