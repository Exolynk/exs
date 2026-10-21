//! Tests that String case mapping matches `str::to_lowercase` and `str::to_uppercase`.

use std::string::String;
use std::vec::Vec;

use exs_abi::BuiltinMethod;
use exs_layout::CASE_TABLES;
use exs_value::ValueRef;

use super::execution::Execution;
use super::heap::VM;
use crate::{Memory, call_builtin, set_case_tables};

/// Address of the case tables in these tests.
const TABLES: u32 = 4096;

/// Returns an execution holding the case tables.
fn execution() -> Execution {
    let mut execution = Execution::new();
    for (offset, byte) in (0_u32..).zip(CASE_TABLES.iter()) {
        execution.memory.store_u8(TABLES + offset, *byte);
    }
    set_case_tables(&mut execution.memory, VM, TABLES);
    execution
}

/// Asserts that both case mappings of `text` match the standard library.
fn check(execution: &mut Execution, text: &str) {
    let value = execution.string(text);
    let none = ValueRef::NONE.raw();
    for (method, expected) in [
        (BuiltinMethod::ToLowercase, text.to_lowercase()),
        (BuiltinMethod::ToUppercase, text.to_uppercase()),
    ] {
        let mapped = call_builtin(&mut execution.memory, VM, method, value, none, none)
            .expect("memory has room");
        assert_eq!(execution.text(mapped), expected, "{method:?} of {text:?}");
        let object = crate::string_object(mapped).expect("a String");
        let chars = execution.memory.load_u32(object + 4);
        assert_eq!(chars as usize, expected.chars().count());
    }
}

/// Maps texts with final and non-final sigma, special casing and ignorable characters.
#[test]
fn matches_std_on_texts() {
    let mut execution = execution();
    for text in [
        "",
        "ASCII Text",
        "ÄÖÜ ß straße",
        "ΣΑΣ",
        "ΑΣ",
        "Σ",
        "ΑΣ Α",
        "Α'Σ",
        "ΑΣ'",
        "ΑΣ'Β",
        "Α.Σ.",
        "ΌΣΟΣ",
        "ﬁ ŉ İ ǅ",
        "ΑΣ\u{301}",
        "\u{301}Σ",
        "x\u{345}Σ",
        "ꭰ Ꭰ",
        "𐐀𐐨",
    ] {
        check(&mut execution, text);
    }
}

/// Maps random texts drawn from cased, ignorable and sigma characters.
#[test]
fn matches_std_on_random_texts() {
    let alphabet: Vec<char> = "aZΣσςΑα'.\u{301}\u{345} ßİﬁǅ𐐀1-\u{2E}\u{3000}ΌΣΟ"
        .chars()
        .collect();
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut execution = execution();
    for _ in 0..20_000 {
        let length = (next() % 8) as usize;
        let text: String = (0..length)
            .map(|_| alphabet[(next() % alphabet.len() as u64) as usize])
            .collect();
        check(&mut execution, &text);
    }
}
