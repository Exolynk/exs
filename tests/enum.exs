/// Test Color enum
enum Color {
    Rgb(r: Int, g: Int, b: Int),
    Name(value),
    Transparent,
}

impl Color {
    /// Return the transparent color
    fn new_trans() -> Color {
        ret Color::Transparent;
    }

    fn as_number(self) -> Int {
        ret match self {
            Color::Rgb(r, g, b) => r + g + b,
            Color::Name(s) => s.length(),
            Color::Transparent => {ret -1;}
        };
    }
}

fn main() {
    let c = Color::new_trans();

    ret c.as_number();
}
