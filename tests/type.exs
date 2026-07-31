type User {
    name: String,
    age: Int,
    data,
}

impl User {
    fn name(self) {
        ret self.name;
    }
}

trait Test {
    fn auto_test(self) -> String {
        ret "Auto Test";
    }

    fn man_test() -> String;
}

impl Test for User {
    fn man_test() -> String {
        ret "Manual test";
    }
}


fn main() {
    let u = User {
        name: "Robert",
        age: 121,
        data: "myData"
    };

    ret u.auto_test();
}
