enum Abc {
    A,
    B,
    C
}

impl Abc {
    fn ts(self) {
        ret match self {
            Abc::A => "A",
            Abc::B => "B",
            Abc::C => "C",
        };
    }
}

impl Add for Abc {
    fn add(self, other: Any) -> Any {
        let ss = self.ts();
        ret ss.add(other);
    }
}

fn test(inp: Add) {
    ret inp.add(2);
}

fn main() {
    ret test(1);
    //let abc = Abc::B;
    //ret abc + " <-- Value";
}
