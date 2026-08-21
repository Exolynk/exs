fn main() {
    let y = 12;
    test((x) => { Host::call("println", x, y); });
    ret true;
}

fn test(f: Fn) {
    f("Test");
}
