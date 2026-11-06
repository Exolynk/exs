//! A project with an import, an application library and the json and url libraries.

import "./helper.exs" as helper;

use app::{answer as respond};
use helper::double;

fn main() {
    let parsed = json::parse("{\"a\":[1,2.5,null,true]}")?;
    ret [
        helper::double(4),
        double(5),
        helper::LIMIT,
        app::answer(),
        respond(),
        app::Record { id: 1 }.id,
        json::stringify({ name: "Ada", values: [1, None] })?,
        parsed,
        url::encode("a b&c")?,
        url::decode("a%20b")?,
    ];
}
