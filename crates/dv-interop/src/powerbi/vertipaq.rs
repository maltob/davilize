//! Writers for the column files of a VertiPaq (Analysis Services tabular) model:
//! `.idf` segment data, `.idfmeta` segment metadata and `.dictionary` files.
//!
//! The layouts were reverse engineered from a model saved by Power BI Desktop.
//! Everything here writes a single segment per column that holds the whole
//! column as one bit-packed run, which is what the engine itself produces for
//! small columns.

/// Smallest data id of a column; ids 0..=2 are reserved.
pub(crate) const FIRST_ID: u32 = 3;

const CP_OPEN: &[u8] = b"<1:CP\0";
const CP_CLOSE: &[u8] = b"CP:1>\0";
const CS_OPEN: &[u8] = b"<1:CS\0";
const CS_CLOSE: &[u8] = b"CS:1>\0";
const SS_OPEN: &[u8] = b"<1:SS\0";
const SS_CLOSE: &[u8] = b"SS:1>\0";
const SDOS_OPEN: &[u8] = b"<1:SDOs\0";
const SDOS_CLOSE: &[u8] = b"SDOs:1>\0";
const CSDOS_OPEN: &[u8] = b"<1:CSDOs\0";
const CSDOS_CLOSE: &[u8] = b"CSDOs:1>\0";

/// Hybrid RLE segments, the class every data column uses.
const HYBRID_RLE: u32 = 703_066;
/// The identity sequence used by the hidden row-number column.
const SEQUENCE_CLASS: u32 = 703_067;

/// The bit widths the engine's packed classes offer, with their class ids.
const WIDTHS: [(u32, u32); 14] = [
    (1, 703_031),
    (2, 703_032),
    (3, 703_033),
    (4, 703_034),
    (5, 703_035),
    (6, 703_036),
    (7, 703_037),
    (8, 703_038),
    (9, 703_039),
    (10, 703_040),
    (12, 703_042),
    (16, 703_046),
    (21, 703_051),
    (32, 703_062),
];

/// The smallest supported width and class id that holds values `0..=max`.
fn packed_width(max: u64) -> (u32, u32) {
    let need = (u64::BITS - max.leading_zeros()).max(1);
    WIDTHS
        .iter()
        .copied()
        .find(|(w, _)| *w >= need)
        .unwrap_or(WIDTHS[WIDTHS.len() - 1])
}

fn put_u8(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_s8(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_u4(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn put_s4(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Packs `ids` (minus `min_id`) into little-endian 64-bit words of `width` bits.
fn pack(ids: &[u32], min_id: u32, width: u32) -> Vec<u64> {
    let per = (64 / width) as usize;
    let mut words = Vec::with_capacity(ids.len() / per + 1);
    for chunk in ids.chunks(per) {
        let mut w = 0u64;
        for (j, v) in chunk.iter().enumerate() {
            w |= u64::from(v - min_id) << (j as u32 * width);
        }
        words.push(w);
    }
    // The engine always allocates one word beyond the packed values.
    while words.len() < ids.len() / per + 1 {
        words.push(0);
    }
    words
}

/// What the metadata file needs to know about a column's segment.
pub(crate) struct Segment {
    pub records: usize,
    pub min_id: u32,
    pub max_id: u32,
    /// Smallest id of the dictionary ("original minimum"): 2 for dictionary
    /// columns, 3 for value-encoded ones.
    pub orig_min: u32,
    /// 0 for the row-number column, 1 for every other column.
    pub base_id: u64,
    /// `None` for the identity sequence of the row-number column.
    pub ids: Option<Vec<u32>>,
}

/// The `.idf` file: a 16-slot run table whose first slot says "all packed",
/// then the packed words.
pub(crate) fn idf(segment: &Segment) -> Vec<u8> {
    let mut out = Vec::new();
    put_u8(&mut out, 16);
    put_u4(&mut out, 0xFFFF_FFFF);
    put_u4(&mut out, segment.records as u32);
    out.extend_from_slice(&[0u8; 8 * 15]);
    match &segment.ids {
        None => put_u8(&mut out, 0),
        Some(ids) => {
            let (width, _) = packed_width(u64::from(segment.max_id - segment.min_id));
            let words = pack(ids, segment.min_id, width);
            put_u8(&mut out, words.len() as u64);
            for w in words {
                put_u8(&mut out, w);
            }
        }
    }
    out
}

/// The `.idfmeta` file describing [`idf`].
pub(crate) fn idfmeta(segment: &Segment) -> Vec<u8> {
    let sequence = segment.ids.is_none();
    let (width, class) = if sequence {
        (0, SEQUENCE_CLASS)
    } else {
        packed_width(u64::from(segment.max_id - segment.min_id))
    };
    let sub_words = if sequence {
        0
    } else {
        segment.records as u64 / u64::from(64 / width) + 1
    };
    let bookmark_bits = u64::from(usize::BITS - segment.records.leading_zeros()) + 2;
    let mut out = Vec::new();
    out.extend_from_slice(CP_OPEN);
    put_u8(&mut out, 1);
    out.extend_from_slice(CS_OPEN);
    put_u8(&mut out, segment.records as u64);
    put_u8(&mut out, segment.base_id);
    put_u4(&mut out, HYBRID_RLE);
    put_u4(&mut out, class);
    put_u8(&mut out, if sequence { 24 } else { bookmark_bits });
    put_u8(&mut out, 32);
    put_u8(&mut out, if sequence { 2 } else { 4 });
    out.push(0);
    put_u4(&mut out, 3);
    out.extend_from_slice(SS_OPEN);
    put_u8(&mut out, 0);
    put_u4(&mut out, segment.min_id);
    put_u4(&mut out, segment.max_id);
    put_u4(&mut out, segment.orig_min);
    put_s8(&mut out, -1);
    put_u8(&mut out, segment.records as u64);
    out.push(0);
    put_u8(&mut out, 0);
    put_u8(&mut out, u64::from(!sequence));
    out.extend_from_slice(SS_CLOSE);
    out.push(1);
    out.extend_from_slice(CS_OPEN);
    put_u8(&mut out, segment.records as u64);
    put_u8(&mut out, 0);
    out.push(0);
    out.extend_from_slice(CS_CLOSE);
    out.extend_from_slice(CS_CLOSE);
    out.extend_from_slice(CP_CLOSE);
    out.extend_from_slice(SDOS_OPEN);
    out.extend_from_slice(CSDOS_OPEN);
    put_s8(&mut out, 0);
    put_s8(&mut out, 16);
    out.extend_from_slice(CSDOS_OPEN);
    put_s8(&mut out, 136);
    put_s8(&mut out, sub_words as i64);
    out.extend_from_slice(CSDOS_CLOSE);
    out.extend_from_slice(CSDOS_CLOSE);
    out.extend_from_slice(SDOS_CLOSE);
    out
}

/// A string dictionary: one uncompressed page, strings in first-appearance order.
pub(crate) fn string_dictionary(strings: &[String], contains_nulls: bool) -> Vec<u8> {
    let mut text: Vec<u16> = Vec::new();
    let mut offsets = Vec::with_capacity(strings.len());
    for s in strings {
        offsets.push(text.len() as u32);
        text.extend(s.encode_utf16());
        text.push(0);
    }
    let chars = text.len() as u64;
    let longest = strings
        .iter()
        .map(|s| s.encode_utf16().count())
        .max()
        .unwrap_or(0) as i64;
    let mut out = Vec::new();
    put_s4(&mut out, 2);
    for v in [0, 8, 64, 6, -1, -1] {
        put_s4(&mut out, v);
    }
    put_s8(&mut out, strings.len() as i64);
    out.push(1);
    put_s8(&mut out, longest);
    put_s8(&mut out, 1);
    put_u8(&mut out, 0);
    out.push(u8::from(contains_nulls));
    put_u8(&mut out, 0);
    put_u8(&mut out, strings.len() as u64);
    out.push(0);
    out.extend_from_slice(&[0xdd, 0xcc, 0xbb, 0xaa]);
    put_u8(&mut out, 0);
    put_u8(&mut out, chars);
    put_u8(&mut out, chars * 2);
    for unit in &text {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out.extend_from_slice(&[0xcd, 0xab, 0xcd, 0xab]);
    put_u8(&mut out, strings.len() as u64);
    put_u4(&mut out, 8);
    for off in offsets {
        put_u4(&mut out, off);
        put_u4(&mut out, 0);
    }
    out
}

/// A dictionary of distinct doubles (dates are days since 1899-12-30).
pub(crate) fn number_dictionary(values: &[f64]) -> Vec<u8> {
    let mut out = Vec::new();
    put_s4(&mut out, 1);
    for v in [-1, 16, 64, 3, -1, -1] {
        put_s4(&mut out, v);
    }
    put_u8(&mut out, values.len() as u64);
    put_u4(&mut out, 8);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// A dictionary of distinct 64-bit integers in ascending order.
pub(crate) fn long_dictionary(values: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    put_s4(&mut out, 0);
    for v in [-1, 16, 64, 3, -1, -1] {
        put_s4(&mut out, v);
    }
    put_u8(&mut out, values.len() as u64);
    put_u4(&mut out, 8);
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// The `.idf` of a relationship index: plain bit-packed positions with no run table.
pub(crate) fn index_idf(positions: &[u32]) -> (Vec<u8>, u32) {
    let max = positions.iter().copied().max().unwrap_or(0);
    let (width, class) = packed_width(u64::from(max));
    let per = (64 / width) as usize;
    let mut words = Vec::new();
    for chunk in positions.chunks(per) {
        let mut w = 0u64;
        for (j, v) in chunk.iter().enumerate() {
            w |= u64::from(*v) << (j as u32 * width);
        }
        words.push(w);
    }
    let mut out = Vec::new();
    put_u8(&mut out, words.len() as u64);
    for w in &words {
        put_u8(&mut out, *w);
    }
    (out, class)
}

/// The `.idfmeta` of a relationship index with `records` entries.
pub(crate) fn index_idfmeta(records: usize, class: u32, words: usize) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(CP_OPEN);
    put_u8(&mut out, 1);
    out.extend_from_slice(CS_OPEN);
    put_u8(&mut out, records as u64);
    put_u8(&mut out, 0);
    put_u4(&mut out, class);
    put_u4(&mut out, 1);
    put_u4(&mut out, 0xFFFF_FFFF);
    out.extend_from_slice(SS_OPEN);
    put_u8(&mut out, 0);
    put_u4(&mut out, 2);
    put_u4(&mut out, 2);
    put_u4(&mut out, 2);
    put_s8(&mut out, -1);
    put_u8(&mut out, 0);
    out.push(0);
    put_u8(&mut out, 0);
    put_u8(&mut out, 0);
    out.extend_from_slice(SS_CLOSE);
    out.push(0);
    out.extend_from_slice(CS_CLOSE);
    out.extend_from_slice(CP_CLOSE);
    out.extend_from_slice(SDOS_OPEN);
    out.extend_from_slice(CSDOS_OPEN);
    put_s8(&mut out, 0);
    put_s8(&mut out, words as i64);
    out.extend_from_slice(CSDOS_CLOSE);
    out.extend_from_slice(SDOS_CLOSE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn widths_round_up_to_the_classes_the_engine_has() {
        assert_eq!(packed_width(3), (2, 703_032));
        assert_eq!(packed_width(19), (5, 703_035));
        assert_eq!(packed_width(3660), (12, 703_042));
        assert_eq!(packed_width(11), (4, 703_034));
        assert_eq!(packed_width(0), (1, 703_031));
        assert_eq!(packed_width(70_000), (21, 703_051));
    }

    /// The four-row region column, as Power BI Desktop wrote it.
    #[test]
    fn a_small_string_column_matches_the_files_the_engine_writes() {
        let ids = vec![3, 4, 5, 6];
        let segment = Segment {
            records: 4,
            min_id: 3,
            max_id: 6,
            orig_min: 2,
            base_id: 1,
            ids: Some(ids),
        };
        let idf_bytes = idf(&segment);
        assert_eq!(idf_bytes.len(), 152);
        assert_eq!(idf_bytes[..8], 16u64.to_le_bytes());
        assert_eq!(idf_bytes[8..16], [0xff, 0xff, 0xff, 0xff, 4, 0, 0, 0]);
        assert_eq!(
            idf_bytes[idf_bytes.len() - 8..],
            [0xe4, 0, 0, 0, 0, 0, 0, 0]
        );
        let meta = idfmeta(&segment);
        assert_eq!(meta.len(), 264);
        assert_eq!(&meta[..6], b"<1:CP\0");
        assert_eq!(meta[meta.len() - 8..], *b"SDOs:1>\0");
        let dict = string_dictionary(&strings(&["North", "South", "East", "West"]), false);
        assert_eq!(dict.len(), 199);
        assert_eq!(
            &dict[dict.len() - 48..dict.len() - 44],
            &[0xcd, 0xab, 0xcd, 0xab]
        );
    }

    #[test]
    fn a_dictionary_page_says_when_it_holds_an_empty_string() {
        let plain = string_dictionary(&strings(&["a", "b"]), false);
        let flagged = string_dictionary(&strings(&["a", ""]), true);
        // The flag is the byte after the page mask, which follows the 54-byte
        // dictionary header (type, six hash fields, count, flag, longest, pages).
        let at = 4 + 24 + 8 + 1 + 8 + 8 + 8;
        assert_eq!(plain[at], 0);
        assert_eq!(flagged[at], 1);
    }

    #[test]
    fn the_row_number_column_has_no_packed_data() {
        let segment = Segment {
            records: 20,
            min_id: 3,
            max_id: 22,
            orig_min: 2,
            base_id: 0,
            ids: None,
        };
        assert_eq!(idf(&segment).len(), 144);
        assert_eq!(idfmeta(&segment).len(), 264);
    }

    #[test]
    fn a_relationship_index_packs_positions_in_three_bits() {
        // North, South, West, East in the "to" table positions plus one.
        let (bytes, class) = index_idf(&[0, 0, 0, 1, 2, 4, 3]);
        assert_eq!(class, 703_033);
        assert_eq!(
            bytes,
            [1, 0, 0, 0, 0, 0, 0, 0, 0x00, 0x22, 0x0e, 0, 0, 0, 0, 0]
        );
        assert_eq!(index_idfmeta(7, class, 1).len(), 176);
    }

    #[test]
    fn many_values_get_one_word_more_than_needed() {
        let ids: Vec<u32> = (3..23).collect();
        let segment = Segment {
            records: 20,
            min_id: 3,
            max_id: 22,
            orig_min: 3,
            base_id: 1,
            ids: Some(ids),
        };
        // 5-bit values, 12 per word: two words for 20 values.
        assert_eq!(idf(&segment).len(), 8 + 128 + 8 + 16);
    }
}
