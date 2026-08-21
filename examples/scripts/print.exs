fn main() {
    Host::call("println", "Start Script");
    adding();
    ret true;
}

fn adding() {
    let v = 13 + 13.3;
    Host::call("println", "Calculation", v);
}
