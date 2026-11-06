//! Nominal types, enums, traits, closures, and typed values at the boundary.

type Point {
    x: Int,
    y: Int,
}

impl Point {
    fn new(x: Int, y: Int) -> Point {
        ret Point { x: x, y: y };
    }

    fn norm(self) -> Int {
        ret self.x * self.x + self.y * self.y;
    }
}

impl Add for Point {
    fn add(self, other: Any) -> Any {
        ret Point::new(self.x + other.x, self.y + other.y);
    }
}

enum Shape {
    Circle(radius: Int),
    Square(side: Int),
    Empty,
}

trait Area {
    fn area(self) -> Int;

    fn doubled(self) -> Int {
        ret self.area() * 2;
    }
}

impl Area for Shape {
    fn area(self) -> Int {
        ret match self {
            Shape::Circle(radius) => 3 * radius * radius,
            Shape::Square(side) => side * side,
            Shape::Empty => 0,
        };
    }
}

fn sum(values: Int...) -> Int {
    let total = 0;
    for value in values {
        total = total + value;
    }
    ret total;
}

fn main() {
    let count = 0;
    let bump = () => {
        count = count + 1;
    };
    bump();
    bump();
    let twice = (value: Int) => value * 2;
    ret [
        (Point::new(1, 2) + Point::new(2, 2)).norm(),
        Shape::Circle(2).doubled(),
        Shape::Square(3).area(),
        Shape::Empty.area(),
        Point::new(1, 2).to_string(),
        Shape::Circle(2).to_string(),
        sum(),
        sum(1, 2, 3),
        count,
        twice(21),
        Point::new(1, 2) == Point::new(1, 2),
        Point::new(1, 2) is Point,
    ];
}

/// Returns a typed value, which reaches the host with the identity of its type.
fn point() -> Point {
    ret Point::new(3, 4);
}

/// Returns an enum value.
fn circle() -> Shape {
    ret Shape::Circle(2);
}

/// Takes a typed value from the host and moves it.
fn moved(point: Point) -> Point {
    ret point + Point::new(1, 1);
}

/// Takes an enum value from the host.
fn area_of(shape: Shape) -> Int {
    ret shape.area();
}
