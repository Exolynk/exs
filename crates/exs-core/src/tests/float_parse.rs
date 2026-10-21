//! Tests that the table-free Float parser matches `f64::from_str` bit for bit.

use std::format;
use std::string::String;
use std::vec::Vec;

use crate::methods::parse_f64;

/// Asserts that `text` parses exactly like `f64::from_str`, including rejections.
fn check(text: &str) {
    let expected = text.parse::<f64>().ok().map(f64::to_bits);
    let actual = parse_f64(text.as_bytes()).map(f64::to_bits);
    assert_eq!(actual, expected, "parsing {text:?}");
}

/// A small deterministic pseudo-random generator.
struct XorShift(u64);

impl XorShift {
    /// Returns the next pseudo-random number.
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Matches the standard parser on syntax edge cases and hard rounding cases.
#[test]
fn matches_std_on_edge_cases() {
    for text in [
        "0",
        "-0",
        "+0",
        "0.0",
        "00.00e5",
        "1",
        "1.",
        ".5",
        "+.5",
        "-.5",
        ".",
        "",
        "+",
        "-",
        "e5",
        "1e",
        "1e+",
        "1e-",
        "1e5",
        "1E5",
        "1.5e-3",
        "1.e3",
        ".e3",
        "1..2",
        "1.2.3",
        " 1",
        "1 ",
        "1_0",
        "0x10",
        "inf",
        "+inf",
        "-inf",
        "INF",
        "Infinity",
        "-infinity",
        "infin",
        "nan",
        "-nan",
        "NaN",
        "+NaN",
        "nan1",
        "inf.0",
        "1e308",
        "1.7976931348623157e308",
        "1.7976931348623158e308",
        "1.7976931348623159e308",
        "1e309",
        "2e-308",
        "2.2250738585072014e-308",
        "2.2250738585072011e-308",
        "4.9406564584124654e-324",
        "2.4703282292062327e-324",
        "2.4703282292062328e-324",
        "2.4703282292062327e-325",
        "1e-400",
        "1e400",
        "9007199254740993",
        "9007199254740992.5",
        "9007199254740993.0000000000000000001",
        "0.1",
        "0.2",
        "0.3",
        "123456789012345678901234567890",
        "1e99999999999999999999",
        "1e-99999999999999999999",
        "0e99999999999999999999",
        "2.47032822920623272088284396434110686182529901307162382212792841250337753635104375932649918180817996189898282347722858865463328355177969898199387398005390939063150356595155702263922908583924491051844359318028499365361525003193704576782492193656236698636584807570015857692699037063119282795585513329278343384093519780155312465972635795746227664652728272200563740064854999770965994704540208281662262378573934507363390079677619305775067401763246736009689513405355374585166611342237666786041621596804619144672918403005300575308490487653917113865916462395249126236538818796362393732804238910186723484976682350898633885879256283027559956575244555072551893136908362547791869486679949683240497058210285131854513962138377228261454376934125320985913276672363281255e-324",
    ] {
        check(text);
    }
    let long_one = format!("1.{}1", "0".repeat(900));
    check(&long_one);
    let halfway = format!("9007199254740993{}", "0".repeat(850));
    check(&halfway);
    check(&format!("{halfway}1"));
}

/// Matches the standard parser on random decimal texts and on the round-trip renderings of
/// random binary64 values and their rounding neighbors.
#[test]
fn matches_std_on_random_inputs() {
    let mut random = XorShift(0x2545_F491_4F6C_DD1D);
    for _ in 0..200_000 {
        let bits = random.next();
        let value = f64::from_bits(bits);
        check(&format!("{value}"));
        check(&format!("{value:e}"));
        check(&format!("{value:.17e}"));
        // Texts just below, at and above the midpoint toward the next value probe rounding.
        let next = f64::from_bits(bits.wrapping_add(1));
        if value.is_finite() && next.is_finite() {
            let midpoint = format!("{:.30e}", value / 2.0 + next / 2.0);
            check(&midpoint);
        }
    }
    let alphabet: Vec<char> = "0123456789.eE+-".chars().collect();
    for _ in 0..200_000 {
        let length = 1 + (random.next() % 24) as usize;
        let text: String = (0..length)
            .map(|_| alphabet[(random.next() % alphabet.len() as u64) as usize])
            .collect();
        check(&text);
        let digits = 1 + (random.next() % 40) as usize;
        let mantissa: String = (0..digits)
            .map(|_| char::from(b'0' + (random.next() % 10) as u8))
            .collect();
        let exponent = (random.next() % 700) as i64 - 350;
        check(&format!("{mantissa}e{exponent}"));
        check(&format!("0.{mantissa}e{exponent}"));
    }
}
