fn twice(value) {
    ret value * 2;
}

fn main(input) {
    let value = twice(21);
    if value == 42 {
        ret value;
    } else {
        ret 0;
    }
}
