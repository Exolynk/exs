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

fn test(u: User) {
    ret u.name();
}

fn main() {
    let u = User {
        name: "Robert",
        age: 121,
        data: "myData"
    };

    ret test(u);
}
