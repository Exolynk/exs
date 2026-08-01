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
}

fn main() {
    ret Color::new_trans();
}
