fn main(input) {
    let o = {
        a: 12,
        b: 12.12,
        c: "test",
        d: ["a", "b", "c"],
        i: input
    };

    o.a = 11;
    o["b"] = 11.11;

    let d = o.d;
    d[0] = "A";

    ret o;
}
