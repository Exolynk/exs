fn main() {
    let y = 12;
    test((x) => {host.call("println", x, y);});

    ret true;
}

fn test(f: Fn) {
    f("Test");
}
