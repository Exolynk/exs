//! Literals, operators, collections and control flow.

fn main() {
    let items = [3, 1, 2];
    items.sort();
    items.push(4);
    let person = { name: "Ada" };
    person.age = 36;
    let total = 0;
    for value in 1..=4 {
        if value == 3 {
            continue;
        }
        total = total + value;
    }
    let letters = [];
    for letter in "héy" {
        letters.push(letter);
    }
    let count = 0;
    while count < 5 {
        count = count + 1;
        if count == 3 {
            break;
        }
    }
    ret [
        7 % 3,
        -1 % 24,
        1 + 2.5,
        7 / 2,
        2_000 * 3,
        "a" + 1,
        f"{1 + 1} {{x}} {person.name}",
        "Ada".to_uppercase(),
        "a,b".split(","),
        b"hi",
        items,
        person,
        total,
        letters,
        count,
        true && !false,
        1 == 1.0,
        [1, [2]] == [1, [2]],
        (0..3).to_string(),
        [1, "a"].to_string(),
        5 is Int,
        None,
    ];
}

/// Returns its input, so that every value kind crosses the boundary in both directions.
fn identity(value) {
    ret value;
}

/// Finishes without a value.
fn nothing() {}

/// Reads a field of a host value whose type this script does not declare.
fn field_of(value) {
    ret value.v;
}

/// Sends a value to the host and returns the response.
fn through_host(value) {
    ret Host::call("echo", value);
}
