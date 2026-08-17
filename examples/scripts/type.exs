/// A user which is doing and storing some awesome stuff
type User {
    name: String,
    age: Int,
    data,
}

impl User {
    /// Return the name of the user
    fn name(self) {
        ret self.name;
    }
}

trait Test {
    /// Auto Test return
    fn auto_test(self) -> String {
        ret "Auto Test";
    }

    /// Function to be implemented
    fn man_test() -> String;
}

impl Test for User {
    /// Manual Test return
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
