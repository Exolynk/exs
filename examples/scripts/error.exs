fn custom() {
    let data = {info1: "Hello", info2: "World"};
    ret Error("UserError", "This is a manual created user error", data);
}

fn type_panic() -> Int {
    ret "String";
}

fn main(input) {
    let a = "Test"?;
    //type_panic();

    ret custom()?;
}
