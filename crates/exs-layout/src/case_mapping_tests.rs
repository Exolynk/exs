//! Generation of the case-mapping tables from the standard library and their verification.

extern crate std;

use std::format;
use std::string::String;
use std::vec::Vec;

use crate::case_mapping::{CASE_TABLES, CASE_TABLES_UNICODE_VERSION, CaseTables};

/// Appends one base-128 varint.
fn varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Appends one zigzag-encoded signed varint.
fn signed(out: &mut Vec<u8>, value: i64) {
    varint(out, ((value << 1) ^ (value >> 63)) as u64);
}

/// Encodes one mapping as runs of single-character mappings with a constant delta and a
/// stride of 1 or 2, plus the multi-character mappings.
fn mapping(map: fn(char) -> String) -> (Vec<u8>, Vec<u8>) {
    let mut runs: Vec<(u32, u32, u32, i64)> = Vec::new();
    let mut multi = Vec::new();
    let mut previous_multi = 0_u32;
    let mut current: Option<(u32, u32, u32, i64)> = None;
    for code in 0..=0x10_FFFF_u32 {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        let mapped: Vec<char> = map(c).chars().collect();
        if mapped.len() > 1 {
            varint(&mut multi, u64::from(code - previous_multi));
            previous_multi = code;
            multi.push(mapped.len() as u8);
            for target in &mapped {
                signed(&mut multi, i64::from(u32::from(*target)) - i64::from(code));
            }
            continue;
        }
        if mapped[0] == c {
            continue;
        }
        let delta = i64::from(u32::from(mapped[0])) - i64::from(code);
        match &mut current {
            Some((start, count, stride, run_delta))
                if *run_delta == delta
                    && (code == *start + *count * *stride
                        || (*count == 1 && code - *start <= 2)) =>
            {
                if *count == 1 {
                    *stride = code - *start;
                }
                *count += 1;
            }
            _ => {
                runs.extend(current.take());
                current = Some((code, 1, 1, delta));
            }
        }
    }
    runs.extend(current);
    let mut out = Vec::new();
    let mut previous = 0_u32;
    for (start, count, stride, delta) in runs {
        varint(&mut out, u64::from(start - previous));
        varint(&mut out, (u64::from(count) << 1) | u64::from(stride - 1));
        signed(&mut out, delta);
        previous = start + (count - 1) * stride + 1;
    }
    (out, multi)
}

/// Encodes the code points satisfying `predicate` as ranges.
fn ranges(predicate: impl Fn(char) -> bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut previous = 0_u32;
    let mut start = None;
    for code in 0..=0x11_0000_u32 {
        let inside = code <= 0x10_FFFF && char::from_u32(code).is_some_and(&predicate);
        match (inside, start) {
            (true, None) => start = Some(code),
            (false, Some(first)) => {
                varint(&mut out, u64::from(first - previous));
                varint(&mut out, u64::from(code - first));
                previous = code;
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Returns whether the standard library lowercases the character at `index` of `text`, a
/// sigma, to the final form.
fn final_sigma_at(text: &str, index: usize) -> bool {
    text.to_lowercase().chars().nth(index) == Some('ς')
}

/// Returns whether `c` is Case_Ignorable, as the standard library's final-sigma rule sees it:
/// a sigma before `c` is final when nothing cased follows `c`, but not when a cased letter
/// follows it.
fn case_ignorable(c: char) -> bool {
    !final_sigma_at(&format!("AΣ{c}B"), 1) && final_sigma_at(&format!("AΣ{c}"), 1)
}

/// Returns whether `c`, which is not Case_Ignorable, is Cased: a sigma right after it is final.
fn cased(c: char) -> bool {
    let text = format!("{c}Σ");
    !case_ignorable(c) && text.to_lowercase().ends_with('ς')
}

/// Builds the table blob.
fn tables() -> Vec<u8> {
    let (lower, lower_multi) = mapping(|c| c.to_lowercase().collect());
    let (upper, upper_multi) = mapping(|c| c.to_uppercase().collect());
    let sections = [
        lower,
        lower_multi,
        upper,
        upper_multi,
        ranges(case_ignorable),
        ranges(cased),
    ];
    let mut offset = (sections.len() as u32 + 1) * 4;
    let mut blob = Vec::new();
    for section in &sections {
        blob.extend(offset.to_le_bytes());
        offset += section.len() as u32;
    }
    blob.extend(offset.to_le_bytes());
    for section in sections {
        blob.extend(section);
    }
    blob
}

/// Regenerates `case_tables.rs` from the standard library of the current toolchain.
#[test]
#[ignore = "writes the generated tables; run after a toolchain or Unicode update"]
fn generates_case_tables() {
    let blob = tables();
    let (major, minor, update) = char::UNICODE_VERSION;
    let mut source = String::from(
        "//! Generated case-mapping tables; regenerate with\n//! `cargo test -p exs-layout -- --ignored generates_case_tables`.\n\n",
    );
    source
        .push_str("/// Unicode version of the standard library the tables were generated from.\n");
    source.push_str(&format!(
        "pub const CASE_TABLES_UNICODE_VERSION: (u8, u8, u8) = ({major}, {minor}, {update});\n\n"
    ));
    source.push_str("/// The case-mapping tables described in [`crate::case_mapping`].\n");
    source.push_str(&format!(
        "pub static CASE_TABLES: [u8; {}] = [\n",
        blob.len()
    ));
    for chunk in blob.chunks(16) {
        let line: Vec<String> = chunk.iter().map(|byte| format!("{byte}")).collect();
        source.push_str(&format!("    {},\n", line.join(", ")));
    }
    source.push_str("];\n");
    let path = format!("{}/src/case_tables.rs", env!("CARGO_MANIFEST_DIR"));
    std::fs::write(path, source).expect("writable source tree");
}

/// Keeps the tables in step with the toolchain: a Unicode update requires regeneration.
#[test]
fn tables_match_the_toolchain() {
    assert_eq!(CASE_TABLES_UNICODE_VERSION, char::UNICODE_VERSION);
    assert_eq!(CASE_TABLES.as_slice(), tables().as_slice());
}

/// Classifies the characters the final-sigma rule depends on most.
#[test]
fn classifies_sigma_neighbors() {
    let tables = CaseTables::new(&CASE_TABLES);
    for c in ['Σ', 'σ', 'A', 'ß'] {
        assert!(!tables.is_case_ignorable(c) && tables.is_cased(c), "{c:?}");
    }
    for c in ['\'', '.', '\u{301}'] {
        assert!(tables.is_case_ignorable(c), "{c:?}");
    }
    assert!(!tables.is_case_ignorable(' ') && !tables.is_cased(' '));
}

/// Maps every code point exactly like the standard library; debug builds sample.
#[test]
fn maps_every_character_like_std() {
    let tables = CaseTables::new(&CASE_TABLES);
    let step = if cfg!(debug_assertions) { 61 } else { 1 };
    for code in (0..=0x10_FFFF_u32).step_by(step) {
        let Some(c) = char::from_u32(code) else {
            continue;
        };
        let mut out = [c; 3];
        let count = tables.lowercase(c, &mut out);
        assert_eq!(
            &out[..count],
            c.to_lowercase().collect::<Vec<_>>().as_slice(),
            "{c:?}"
        );
        let count = tables.uppercase(c, &mut out);
        assert_eq!(
            &out[..count],
            c.to_uppercase().collect::<Vec<_>>().as_slice(),
            "{c:?}"
        );
        let ignorable = case_ignorable(c);
        assert_eq!(tables.is_case_ignorable(c), ignorable, "{c:?}");
        if !ignorable {
            assert_eq!(tables.is_cased(c), cased(c), "{c:?}");
        }
    }
}
