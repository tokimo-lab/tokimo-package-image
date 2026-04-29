//! Extract embedded JPEG previews from DNG/TIFF-based RAW camera files.
//!
//! DNG files store raw sensor data alongside embedded JPEG previews at
//! various resolutions. This module parses the TIFF IFD chain and returns
//! the largest embedded JPEG — no demosaicing required, fast and memory-light.

const TAG_IMAGE_WIDTH: u16 = 0x0100;
const TAG_IMAGE_LENGTH: u16 = 0x0101;
const TAG_COMPRESSION: u16 = 0x0103;
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x0117;
const TAG_JPEG_IF_OFFSET: u16 = 0x0201;
const TAG_JPEG_IF_BYTE_COUNT: u16 = 0x0202;
const TAG_SUB_IFDS: u16 = 0x014A;

const COMPRESSION_OLD_JPEG: u16 = 6;
const COMPRESSION_JPEG: u16 = 7;

#[derive(Clone, Copy)]
enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    fn u16(self, buf: &[u8], off: usize) -> Option<u16> {
        let b = buf.get(off..off + 2)?;
        Some(match self {
            Self::Little => u16::from_le_bytes([b[0], b[1]]),
            Self::Big => u16::from_be_bytes([b[0], b[1]]),
        })
    }

    fn u32(self, buf: &[u8], off: usize) -> Option<u32> {
        let b = buf.get(off..off + 4)?;
        Some(match self {
            Self::Little => u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
            Self::Big => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        })
    }
}

struct JpegCandidate {
    offset: usize,
    length: usize,
    pixels: u64,
}

fn read_value(buf: &[u8], bo: ByteOrder, typ: u16, val_off: usize) -> Option<u32> {
    match typ {
        1 | 2 => buf.get(val_off).copied().map(u32::from),
        3 => bo.u16(buf, val_off).map(u32::from),
        _ => bo.u32(buf, val_off),
    }
}

fn read_values(buf: &[u8], bo: ByteOrder, typ: u16, val_off: usize, count: usize) -> Option<Vec<u32>> {
    let elem_size = match typ {
        1 | 2 => 1,
        3 => 2,
        _ => 4,
    };
    let total = count * elem_size;
    let data_off = if total <= 4 {
        val_off
    } else {
        bo.u32(buf, val_off)? as usize
    };

    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        let off = data_off + i * elem_size;
        let v = match typ {
            1 | 2 => u32::from(buf.get(off).copied()?),
            3 => u32::from(bo.u16(buf, off)?),
            _ => bo.u32(buf, off)?,
        };
        values.push(v);
    }
    Some(values)
}

fn parse_ifd(
    buf: &[u8],
    bo: ByteOrder,
    ifd_offset: usize,
    candidates: &mut Vec<JpegCandidate>,
    sub_ifd_offsets: &mut Vec<usize>,
) -> Option<usize> {
    let count = bo.u16(buf, ifd_offset)? as usize;
    if ifd_offset + 2 + count * 12 + 4 > buf.len() {
        return None;
    }

    let mut width: u32 = 0;
    let mut height: u32 = 0;
    let mut compression: u16 = 0;
    let mut jpeg_offset: u32 = 0;
    let mut jpeg_length: u32 = 0;
    let mut strip_offsets: Vec<u32> = Vec::new();
    let mut strip_byte_counts: Vec<u32> = Vec::new();

    for i in 0..count {
        let entry = ifd_offset + 2 + i * 12;
        let tag = bo.u16(buf, entry)?;
        let typ = bo.u16(buf, entry + 2)?;
        let cnt = bo.u32(buf, entry + 4)? as usize;
        let val_off = entry + 8;

        match tag {
            TAG_IMAGE_WIDTH => width = read_value(buf, bo, typ, val_off)?,
            TAG_IMAGE_LENGTH => height = read_value(buf, bo, typ, val_off)?,
            TAG_COMPRESSION => compression = read_value(buf, bo, typ, val_off)? as u16,
            TAG_JPEG_IF_OFFSET => jpeg_offset = read_value(buf, bo, typ, val_off)?,
            TAG_JPEG_IF_BYTE_COUNT => jpeg_length = read_value(buf, bo, typ, val_off)?,
            TAG_STRIP_OFFSETS => strip_offsets = read_values(buf, bo, typ, val_off, cnt)?,
            TAG_STRIP_BYTE_COUNTS => strip_byte_counts = read_values(buf, bo, typ, val_off, cnt)?,
            TAG_SUB_IFDS => {
                for off in read_values(buf, bo, typ, val_off, cnt)? {
                    sub_ifd_offsets.push(off as usize);
                }
            }
            _ => {}
        }
    }

    let is_jpeg = compression == COMPRESSION_JPEG || compression == COMPRESSION_OLD_JPEG;
    if is_jpeg {
        if jpeg_offset > 0 && jpeg_length > 0 {
            let off = jpeg_offset as usize;
            let len = jpeg_length as usize;
            if off + len <= buf.len() {
                candidates.push(JpegCandidate {
                    offset: off,
                    length: len,
                    pixels: u64::from(width) * u64::from(height),
                });
            }
        } else if strip_offsets.len() == 1 && strip_byte_counts.len() == 1 {
            let off = strip_offsets[0] as usize;
            let len = strip_byte_counts[0] as usize;
            if off + len <= buf.len() {
                candidates.push(JpegCandidate {
                    offset: off,
                    length: len,
                    pixels: u64::from(width) * u64::from(height),
                });
            }
        }
    }

    let next_off = ifd_offset + 2 + count * 12;
    let next = bo.u32(buf, next_off)? as usize;
    Some(next)
}

/// Extract the largest embedded JPEG preview from a DNG/TIFF RAW file.
pub fn extract_raw_preview(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 8 {
        return None;
    }

    let bo = match (data[0], data[1]) {
        (0x49, 0x49) => ByteOrder::Little,
        (0x4D, 0x4D) => ByteOrder::Big,
        _ => return None,
    };

    if bo.u16(data, 2)? != 42 {
        return None;
    }

    let mut candidates: Vec<JpegCandidate> = Vec::new();
    let mut sub_ifd_offsets: Vec<usize> = Vec::new();

    // Walk main IFD chain
    let mut ifd_offset = bo.u32(data, 4)? as usize;
    let mut safety = 0;
    while ifd_offset > 0 && ifd_offset < data.len() && safety < 20 {
        safety += 1;
        ifd_offset = parse_ifd(data, bo, ifd_offset, &mut candidates, &mut sub_ifd_offsets).unwrap_or(0);
    }

    // Walk SubIFDs
    let mut i = 0;
    while i < sub_ifd_offsets.len() && i < 50 {
        let off = sub_ifd_offsets[i];
        i += 1;
        if off > 0 && off < data.len() {
            let mut nested = Vec::new();
            parse_ifd(data, bo, off, &mut candidates, &mut nested);
            for ns in nested {
                if sub_ifd_offsets.len() < 50 {
                    sub_ifd_offsets.push(ns);
                }
            }
        }
    }

    if candidates.is_empty() {
        return None;
    }

    candidates.sort_by(|a, b| b.pixels.cmp(&a.pixels).then(b.length.cmp(&a.length)));
    let best = &candidates[0];

    if best.offset + best.length <= data.len() && best.length >= 2 {
        Some(data[best.offset..best.offset + best.length].to_vec())
    } else {
        None
    }
}

/// Check if a file extension is a TIFF-based RAW format that may contain embedded JPEG previews.
pub fn is_raw_with_preview(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "dng" | "cr2" | "nef" | "arw" | "orf" | "rw2" | "pef" | "srw" | "raf" | "cr3"
    )
}
