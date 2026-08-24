fn main() {
    ret None;
}

fn twice(value: Int) -> Int {
    ret value * 2;
}

test "assert_eq compares computed values" {
    assert_eq(twice(21), 42, "twice must double its input");
}

test "assert validates boolean conditions" {
    assert(twice(2) > 3, "twice of two must be greater than three");
}

test "simple test" {
    assert(true);
}
