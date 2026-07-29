fn depth0() {
    let data = {info1: "Hello", info2: "World"};
    ret Error("UserError", "This is a manual created user error", data);
}

fn depth1() {
    ret depth2();
}

fn depth2() {
    ret None?;
}

fn main(input) {
    let a = "Test"?;
    let b = Ok("Test")?;

    ret depth0()?;
}
