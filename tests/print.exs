fn main() {
    host.call("println", "Start Script");
    adding();
    ret true;
}

fn adding() {
    let v = 13 + 13.3;
    host.call("println", "Calculation", v);
}
