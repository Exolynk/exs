/// Renders a composite or Bytes in ExS literal syntax for `to_string`, `debug` and formatted
/// strings:
/// `[1, "a"]`, `{ name: "Ada" }`, `Point { x: 1 }`, `Color::Rgb(255, 0, 0)` and
/// `Error("Kind", "message", data)`. Parts render through their type's `Debug`, else
/// `ToString`, implementation where it has one; a part that contains itself renders as `...`.
private fn __exs_render(value: Any) -> String | Error {
    let text = __exs_debug_text(value);
    if text is String {
        ret text;
    }
    ret __exs_render_composite(value, []);
}

/// Renders one part of a composite: through its type's `Debug` or `ToString` implementation,
/// as a literal, or part by part.
private fn __exs_render_part(value: Any, seen: List) -> String | Error {
    let custom = __exs_custom_render(value);
    if custom == 1 {
        ret value.debug();
    }
    if custom == 2 {
        ret value.to_string();
    }
    let text = __exs_debug_text(value);
    if text is String {
        ret text;
    }
    ret __exs_render_composite(value, seen);
}

/// Renders a composite unless it is already being rendered further up.
private fn __exs_render_composite(value: Any, seen: List) -> String | Error {
    for item in seen {
        if __exs_same(item, value) {
            ret "...";
        }
    }
    seen.push(value);
    let text = __exs_render_parts(value, seen);
    seen.pop();
    ret text;
}

/// Renders the parts of a composite in its literal syntax.
private fn __exs_render_parts(value: Any, seen: List) -> String | Error {
    if value is List {
        ret "[" + __exs_render_list(value, seen)? + "]";
    }
    if value is Error {
        let parts = __exs_parts(value);
        ret "Error(" + __exs_render_list([parts[0], parts[1], parts[2]], seen)? + ")";
    }
    let name = __exs_type_name(value);
    let parts = __exs_parts(value);
    if parts is None {
        if value.length() == 0 {
            ret "{}";
        }
        let entries = [];
        for key in value.keys() {
            entries.push(__exs_key_text(key) + ": " + __exs_render_part(value[key], seen)?);
        }
        ret "{ " + entries.join(", ") + " }";
    }
    let fields = __exs_field_names(value);
    if fields is None {
        if parts.length() == 0 {
            ret name;
        }
        ret name + "(" + __exs_render_list(parts, seen)? + ")";
    }
    if fields.length() == 0 {
        ret name + " {}";
    }
    let entries = [];
    let index = 0;
    while index < fields.length() {
        entries.push(fields[index] + ": " + __exs_render_part(parts[index], seen)?);
        index = index + 1;
    }
    ret name + " { " + entries.join(", ") + " }";
}

/// Renders the items of a List separated by commas.
private fn __exs_render_list(items: List, seen: List) -> String | Error {
    let texts = [];
    for item in items {
        texts.push(__exs_render_part(item, seen)?);
    }
    ret texts.join(", ");
}
