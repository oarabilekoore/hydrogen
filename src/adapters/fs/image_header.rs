use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Zero-dependency binary header parser for PNG, GIF, WEBP, and JPEG dimensions.
pub fn get_image_dimensions(path: &Path) -> Option<(u32, u32)> {
    let mut f = File::open(path).ok()?;
    let mut head = [0u8; 32];
    let n = f.read(&mut head).ok()?;
    if n < 16 {
        return None;
    }

    // PNG
    if &head[..8] == b"\x89PNG\r\n\x1a\n" && n >= 24 {
        let w = u32::from_be_bytes(head[16..20].try_into().ok()?);
        let h = u32::from_be_bytes(head[20..24].try_into().ok()?);
        return Some((w, h));
    }

    // GIF
    if &head[..6] == b"GIF87a" || &head[..6] == b"GIF89a" {
        let w = u16::from_le_bytes(head[6..8].try_into().ok()?) as u32;
        let h = u16::from_le_bytes(head[8..10].try_into().ok()?) as u32;
        return Some((w, h));
    }

    // WEBP
    if &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        let chunk = &head[12..16];
        if chunk == b"VP8X" && n >= 30 {
            let w = 1
                + (u32::from(head[24]) | (u32::from(head[25]) << 8) | (u32::from(head[26]) << 16));
            let h = 1
                + (u32::from(head[27]) | (u32::from(head[28]) << 8) | (u32::from(head[29]) << 16));
            return Some((w, h));
        }
        if chunk == b"VP8 " && n >= 30 {
            let w = u16::from_le_bytes(head[26..28].try_into().ok()?) as u32 & 0x3FFF;
            let h = u16::from_le_bytes(head[28..30].try_into().ok()?) as u32 & 0x3FFF;
            return Some((w, h));
        }
        if chunk == b"VP8L" && n >= 25 {
            let (b0, b1, b2, b3) = (
                u32::from(head[21]),
                u32::from(head[22]),
                u32::from(head[23]),
                u32::from(head[24]),
            );
            let w = 1 + (((b1 & 0x3F) << 8) | b0);
            let h = 1 + (((b3 & 0x0F) << 10) | (b2 << 2) | ((b1 & 0xC0) >> 6));
            return Some((w, h));
        }
    }

    // JPEG
    if &head[..2] == b"\xff\xd8" {
        f.seek(SeekFrom::Start(2)).ok()?;
        let mut byte = [0u8; 1];
        loop {
            if f.read(&mut byte).ok()? == 0 {
                break;
            }
            while byte[0] == 0xFF {
                if f.read(&mut byte).ok()? == 0 {
                    return None;
                }
            }
            let marker = byte[0];
            if matches!(marker, 0xD8 | 0xD9 | 0x01) || (0xD0..=0xD7).contains(&marker) {
                continue;
            }
            let mut len_buf = [0u8; 2];
            if f.read_exact(&mut len_buf).is_err() {
                break;
            }
            let seg_len = u16::from_be_bytes(len_buf) as i64;
            if matches!(
                marker,
                0xC0 | 0xC1
                    | 0xC2
                    | 0xC3
                    | 0xC5
                    | 0xC6
                    | 0xC7
                    | 0xC9
                    | 0xCA
                    | 0xCB
                    | 0xCD
                    | 0xCE
                    | 0xCF
            ) {
                let mut sof = [0u8; 5];
                if f.read_exact(&mut sof).is_ok() {
                    let h = u16::from_be_bytes([sof[1], sof[2]]) as u32;
                    let w = u16::from_be_bytes([sof[3], sof[4]]) as u32;
                    return Some((w, h));
                }
                break;
            }
            if seg_len < 2 || f.seek(SeekFrom::Current(seg_len - 2)).is_err() {
                break;
            }
        }
    }

    None
}
