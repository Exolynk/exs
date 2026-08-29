use super::shared::*;
use super::source::{parse, render_enum_page, render_trait_page, render_type_page};
use super::*;
use crate::loaded_project::LoadedSource;

/// Returns every documented source-visible standard-library type.
#[must_use]
pub fn standard_library_types() -> Vec<StandardType> {
    vec![
        StandardType {
            name: "Any",
            description: "`Any` accepts every ExS value. It is the implicit contract when a parameter or return annotation is omitted, and is useful when a function deliberately forwards values without narrowing their type.",
            usage: "fn main(value: Any) -> Any {\n    ret value;\n}",
            methods: &[],
        },
        StandardType {
            name: "None",
            description: "`None` is the single absence value. It is used for missing optional results, empty mutations, and Object reads whose key does not exist; ExS has no `null` source literal.",
            usage: "fn main() -> None {\n    let absent = None;\n    ret absent;\n}",
            methods: &[],
        },
        StandardType {
            name: "Duration",
            description: "`Duration` is a normal prelude type representing a non-negative interval as normalized `seconds` and `nanoseconds` Int fields. Create values through its static factories before passing them to `Host::sleep`.",
            usage: "fn main() -> None {\n    let pause = Duration::milliseconds(250);\n    Host::sleep(pause);\n    ret None;\n}",
            methods: &[
                StandardMethod {
                    signature: "as_seconds() -> Int",
                    description: "Returns the normalized whole-second component. Any fractional second remains available through the `nanoseconds` field.",
                    example: "let seconds = Duration::milliseconds(1500).as_seconds(); // 1",
                },
                StandardMethod {
                    signature: "as_milliseconds() -> Int | Error",
                    description: "Returns the total duration truncated to whole milliseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let milliseconds = Duration::nanoseconds(1_500_000).as_milliseconds(); // 1",
                },
                StandardMethod {
                    signature: "as_microseconds() -> Int | Error",
                    description: "Returns the total duration truncated to whole microseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let microseconds = Duration::nanoseconds(1_500).as_microseconds(); // 1",
                },
                StandardMethod {
                    signature: "as_nanoseconds() -> Int | Error",
                    description: "Returns the total duration as exact nanoseconds. IntOverflowError is returned when the exact total cannot fit in an Int.",
                    example: "let nanoseconds = Duration::milliseconds(1).as_nanoseconds(); // 1_000_000",
                },
            ],
        },
        StandardType {
            name: "Error",
            description: "`Error` is a structured language failure value. Operations return it instead of throwing, and functions that may return an Error should include `Error` in their return contract or use the implicit `Any` contract.",
            usage: "fn main() -> Int | Error {\n    ret Error(\"DivisionByZeroError\", \"cannot divide by zero\", None);\n}",
            methods: &[
                StandardMethod {
                    signature: "kind() -> String",
                    description: "Returns the stable machine-readable category assigned when the Error was created. Use it to distinguish expected failures without parsing a human-facing message.",
                    example: "let error = Error(\"MissingValue\", \"value is required\", None);\nlet category = error.kind();",
                },
                StandardMethod {
                    signature: "message() -> String",
                    description: "Returns the human-readable explanation stored in the Error. The message is intended for diagnostics and user-facing reporting, not control-flow classification.",
                    example: "let error = Error(\"MissingValue\", \"value is required\", None);\nlet text = error.message();",
                },
                StandardMethod {
                    signature: "data() -> Any",
                    description: "Returns the language value attached to the Error. Runtime operations use this to retain the invalid input, index, or other relevant context that caused the failure.",
                    example: "let error = Error(\"InvalidInput\", \"age must be positive\", -1);\nlet invalid_age = error.data();",
                },
                StandardMethod {
                    signature: "cause() -> Error | None",
                    description: "Returns a related prior Error or value when one is present. Errors created directly with `Error(...)` have no cause and therefore return None.",
                    example: "let error = Error(\"Example\", \"no prior failure\", None);\nlet previous = error.cause();",
                },
            ],
        },
        StandardType {
            name: "Bool",
            description: "`Bool` has exactly the values `true` and `false`. Conditions require Bool explicitly; ExS does not apply implicit truthiness to numbers, strings, collections, or None.",
            usage: "fn main() {\n    let ready = true;\n    if ready {\n        Host::call(\"println\", \"ready\");\n    }\n}",
            methods: &[],
        },
        StandardType {
            name: "Int",
            description: "`Int` is a signed 64-bit exact integer. It supports numeric operators and reports `IntOverflowError` when an operation overflows the ExS integer range.",
            usage: "fn main() -> Int {\n    let quantity = 42;\n    ret quantity + 8;\n}",
            methods: &[
                StandardMethod {
                    signature: "abs() -> Int",
                    description: "Returns the non-negative magnitude of the receiver. The smallest representable Int has no representable positive counterpart, so calling `abs()` on it returns `IntOverflowError`.",
                    example: "let change = -42;\nlet magnitude = change.abs(); // 42",
                },
                StandardMethod {
                    signature: "div_euclid(other: Int) -> Int | Error",
                    description: "Returns the Euclidean quotient of two Int values. The result satisfies `self == other * quotient + self.rem_euclid(other)`. A zero divisor returns DivisionByZeroError and the only overflowing pair, the smallest Int divided by -1, returns IntOverflowError.",
                    example: "let seconds = 1001.div_euclid(1000); // 1",
                },
                StandardMethod {
                    signature: "rem_euclid(other: Int) -> Int | Error",
                    description: "Returns the non-negative Euclidean remainder of two Int values. A zero divisor returns DivisionByZeroError and the only overflowing pair, the smallest Int divided by -1, returns IntOverflowError.",
                    example: "let milliseconds = 1001.rem_euclid(1000); // 1",
                },
            ],
        },
        StandardType {
            name: "Float",
            description: "`Float` uses IEEE-754 binary64 values, including infinities, signed zero, and NaN. Mixed arithmetic promotes the other numeric operand to Float.",
            usage: "fn main() -> Float {\n    let price = 19.95;\n    ret price * 1.19;\n}",
            methods: &[
                StandardMethod {
                    signature: "abs() -> Float",
                    description: "Returns the non-negative floating-point magnitude. It preserves Float semantics for signed zero, infinities, and NaN.",
                    example: "let delta = -1.5;\nlet magnitude = delta.abs(); // 1.5",
                },
                StandardMethod {
                    signature: "floor() -> Float",
                    description: "Rounds down to the greatest integral Float that is not greater than the receiver. The result remains Float so it composes with floating-point arithmetic.",
                    example: "let page = 3.8;\nlet first_index = page.floor(); // 3.0",
                },
                StandardMethod {
                    signature: "ceil() -> Float",
                    description: "Rounds up to the least integral Float that is not less than the receiver. The result remains Float.",
                    example: "let pages = 3.2;\nlet required = pages.ceil(); // 4.0",
                },
                StandardMethod {
                    signature: "round() -> Float",
                    description: "Rounds to the nearest integral Float. Exact halfway values are rounded away from zero, so `1.5` becomes `2.0` and `-1.5` becomes `-2.0`.",
                    example: "let rating = 4.5;\nlet displayed = rating.round(); // 5.0",
                },
            ],
        },
        StandardType {
            name: "String",
            description: "`String` is an immutable UTF-8 sequence. `length()` operates on Unicode scalar values rather than UTF-8 byte positions; use `for` to iterate scalar Strings.",
            usage: "fn main() -> String {\n    let greeting = \"Hello\";\n    ret greeting;\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of Unicode scalar values in the String. This is not the UTF-8 byte length, so a single emoji scalar counts as one.",
                    example: "let symbol = \"🙂\";\nlet count = symbol.length(); // 1",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the String contains no Unicode scalar values and false otherwise. It does not trim or normalize the String.",
                    example: "let input = \"\";\nif input.is_empty() {\n    Host::call(\"println\", \"missing input\");\n}",
                },
            ],
        },
        StandardType {
            name: "Bytes",
            description: "`Bytes` is an immutable sequence of raw octets. It is the binary-safe value used for arbitrary host data; indexing and iteration yield Int values from 0 through 255.",
            usage: "fn main() -> Bytes | Error {\n    let header = b\"EXS\";\n    ret header.concat(Bytes::from_list([0, 1])?);\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of octets in the Bytes value.",
                    example: "let payload = b\"abc\";\nlet count = payload.length(); // 3",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the Bytes value contains no octets. It does not mutate the receiver.",
                    example: "let payload = b\"\";\nlet empty = payload.is_empty(); // true",
                },
                StandardMethod {
                    signature: "to_list() -> List",
                    description: "Returns a new List whose elements are the receiver octets as Int values in source order.",
                    example: "let values = b\"AB\".to_list(); // [65, 66]",
                },
                StandardMethod {
                    signature: "slice(start: Int, end: Int) -> Bytes | Error",
                    description: "Returns a new Bytes value covering the half-open range from start through end. Both indexes must be non-negative and the range must lie inside the receiver, otherwise IndexError is returned.",
                    example: "let middle = b\"abcd\".slice(1, 3); // b\"bc\"",
                },
                StandardMethod {
                    signature: "concat(other: Bytes) -> Bytes | Error",
                    description: "Returns a new Bytes value with other appended. A non-Bytes argument returns TypeError.",
                    example: "let message = b\"hello, \".concat(b\"world\");",
                },
                StandardMethod {
                    signature: "decode_utf8() -> String | Error",
                    description: "Decodes the receiver as UTF-8. Invalid byte sequences return EncodingError rather than replacing or dropping octets.",
                    example: "let text = b\"hello\".decode_utf8(); // \"hello\"",
                },
            ],
        },
        StandardType {
            name: "List",
            description: "`List` is a mutable ordered collection. Variables and closure captures preserve List identity, so mutations through one alias are visible through every alias of the same List.",
            usage: "fn main() -> Int {\n    let items = [\"Ada\", \"Lin\"];\n    ret items.push(\"Mia\");\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the current number of elements. The count changes immediately after List mutations such as `push`, `pop`, `insert`, `remove`, and `clear`.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet count = items.length(); // 2",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the List has no elements. It does not mutate the List.",
                    example: "let queue = [];\nif queue.is_empty() {\n    Host::call(\"println\", \"queue is empty\");\n}",
                },
                StandardMethod {
                    signature: "push(value) -> Int",
                    description: "Appends one value to the end of the List and returns the new element count. The operation mutates the existing List rather than allocating a replacement.",
                    example: "let items = [\"Ada\"];\nlet count = items.push(\"Lin\"); // 2",
                },
                StandardMethod {
                    signature: "pop() -> Any | None",
                    description: "Removes and returns the final element. Calling `pop()` on an empty List leaves it unchanged and returns None.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet last = items.pop(); // \"Lin\"",
                },
                StandardMethod {
                    signature: "insert(index: Int, value) -> None | Error",
                    description: "Inserts one value before the zero-based index and returns None. The index may equal the current length to append; invalid indexes return `IndexError`.",
                    example: "let items = [\"Ada\", \"Mia\"];\nitems.insert(1, \"Lin\"); // [\"Ada\", \"Lin\", \"Mia\"]",
                },
                StandardMethod {
                    signature: "remove(index: Int) -> Any | Error",
                    description: "Removes and returns the element at a zero-based index. Invalid indexes return `IndexError` and leave the List unchanged.",
                    example: "let items = [\"Ada\", \"Lin\"];\nlet removed = items.remove(0); // \"Ada\"",
                },
                StandardMethod {
                    signature: "clear() -> None",
                    description: "Removes every element from the existing List and returns None. Aliases continue to refer to the now-empty same List.",
                    example: "let items = [1, 2, 3];\nitems.clear();\nlet empty = items.is_empty(); // true",
                },
            ],
        },
        StandardType {
            name: "Object",
            description: "`Object` is a mutable insertion-ordered mapping from String keys to values. Dot properties and bracket access operate on the same ordered collection.",
            usage: "fn main() -> Object {\n    let user = { name: \"Ada\" };\n    user.role = \"admin\";\n    ret user;\n}",
            methods: &[
                StandardMethod {
                    signature: "length() -> Int",
                    description: "Returns the number of present keys. Replacing an existing key does not increase the count; creating or deleting a key does.",
                    example: "let user = { name: \"Ada\" };\nlet count = user.length(); // 1",
                },
                StandardMethod {
                    signature: "is_empty() -> Bool",
                    description: "Returns true when the Object has no keys. It does not mutate the Object.",
                    example: "let options = {};\nif options.is_empty() {\n    Host::call(\"println\", \"using defaults\");\n}",
                },
                StandardMethod {
                    signature: "has(key: String) -> Bool | Error",
                    description: "Returns whether a String key is present. A non-String key returns `TypeError` rather than coercing the key.",
                    example: "let user = { name: \"Ada\" };\nlet has_name = user.has(\"name\"); // true",
                },
                StandardMethod {
                    signature: "delete(key: String) -> Any | None | Error",
                    description: "Removes a String key and returns its previous value. When the key is absent, it returns None; a non-String key returns `TypeError`.",
                    example: "let user = { name: \"Ada\" };\nlet name = user.delete(\"name\"); // \"Ada\"",
                },
                StandardMethod {
                    signature: "keys() -> List",
                    description: "Returns a new List of String keys in insertion order. Changing the returned List does not change the Object.",
                    example: "let user = { name: \"Ada\", role: \"admin\" };\nlet keys = user.keys(); // [\"name\", \"role\"]",
                },
                StandardMethod {
                    signature: "values() -> List",
                    description: "Returns a new shallow List of values in the same insertion order as `keys()`. The values themselves retain their original identity.",
                    example: "let user = { name: \"Ada\", role: \"admin\" };\nlet values = user.values(); // [\"Ada\", \"admin\"]",
                },
            ],
        },
        StandardType {
            name: "Fn",
            description: "`Fn` is the callable closure contract used in annotations. A closure captures lexical bindings and can be called with its declared parameter count.",
            usage: "fn apply(function: Fn, value: Int) -> Int {\n    ret function(value);\n}\n\nfn main() -> Int {\n    let increment = (value) => { ret value + 1; };\n    ret apply(increment, 1);\n}",
            methods: &[],
        },
    ]
}

/// Generates runtime-owned and source-prelude declaration pages for the standard library.
/// Generates runtime-owned and source-prelude declaration pages for the standard library.
pub(super) fn standard_pages() -> Result<Vec<DocumentationPage>, String> {
    let prelude_modules = standard_prelude_modules()?;
    let prelude_type_names = prelude_modules
        .iter()
        .flat_map(|(module, _)| {
            module
                .types
                .iter()
                .map(|declaration| declaration.name.name.as_str())
        })
        .collect::<Vec<_>>();
    let prelude_enum_names = prelude_modules
        .iter()
        .flat_map(|(module, _)| {
            module
                .enums
                .iter()
                .map(|declaration| declaration.name.name.as_str())
        })
        .collect::<Vec<_>>();
    let prelude_trait_names = prelude_modules
        .iter()
        .flat_map(|(module, _)| {
            module
                .traits
                .iter()
                .map(|declaration| declaration.name.name.as_str())
        })
        .collect::<Vec<_>>();
    let types = standard_library_types()
        .into_iter()
        .filter(|type_info| !prelude_type_names.contains(&type_info.name))
        .collect::<Vec<_>>();
    let functions = standard_library_functions();
    let namespaces = standard_library_namespaces();
    let traits = codegen_standard::traits()
        .iter()
        .filter(|descriptor| !prelude_trait_names.contains(&descriptor.name))
        .collect::<Vec<_>>();
    let enums = codegen_standard::enums()
        .iter()
        .filter(|descriptor| !prelude_enum_names.contains(&descriptor.name))
        .collect::<Vec<_>>();
    let type_names = types
        .iter()
        .map(|type_info| type_info.name)
        .chain(prelude_type_names.iter().copied())
        .collect::<Vec<_>>();
    let trait_names = traits
        .iter()
        .map(|descriptor| descriptor.name)
        .chain(prelude_trait_names.iter().copied())
        .collect::<Vec<_>>();
    let enum_names = enums
        .iter()
        .map(|descriptor| descriptor.name)
        .chain(prelude_enum_names.iter().copied())
        .collect::<Vec<_>>();
    let mut pages = vec![DocumentationPage {
        path: "modules/std/index.md".to_owned(),
        markdown: render_standard_index(
            &type_names,
            functions,
            namespaces,
            &enum_names,
            &trait_names,
        ),
    }];
    for type_info in &types {
        pages.push(DocumentationPage {
            path: format!("modules/std/types/{}.md", slug(type_info.name)),
            markdown: if type_info.name == "Error" {
                render_standard_error_type(type_info)
            } else {
                render_standard_type(type_info)
            },
        });
    }
    for trait_info in traits {
        pages.push(standard_trait_page(trait_info));
    }
    for enum_info in enums {
        pages.push(DocumentationPage {
            path: format!("modules/std/enums/{}.md", slug(enum_info.name)),
            markdown: render_standard_enum(enum_info),
        });
    }
    for namespace in namespaces {
        pages.push(standard_namespace_page(namespace));
    }
    for (module, source) in &prelude_modules {
        pages.extend(standard_prelude_pages(module, source));
    }
    Ok(pages)
}

/// Parses every bundled ExS prelude source for standard-library documentation rendering.
fn standard_prelude_modules() -> Result<Vec<(Module<'static>, LoadedSource)>, String> {
    crate::prelude::source_inputs()
        .into_iter()
        .map(|source| {
            let module = parse(source.source_id, source.text)?;
            Ok((
                module,
                LoadedSource {
                    source_id: source.source_id.to_owned(),
                    display_path: source.source_id.to_owned(),
                    text: source.text.to_owned(),
                },
            ))
        })
        .collect()
}

/// Builds standard-library declaration pages from one bundled ExS prelude source.
fn standard_prelude_pages(module: &Module<'_>, source: &LoadedSource) -> Vec<DocumentationPage> {
    let mut pages = Vec::new();
    for declaration in &module.types {
        pages.push(DocumentationPage {
            path: format!("modules/std/types/{}.md", slug(&declaration.name.name)),
            markdown: render_type_page(module, declaration, source),
        });
    }
    for declaration in &module.enums {
        pages.push(DocumentationPage {
            path: format!("modules/std/enums/{}.md", slug(&declaration.name.name)),
            markdown: render_enum_page(module, declaration, source),
        });
    }
    for declaration in &module.traits {
        pages.push(DocumentationPage {
            path: format!("modules/std/traits/{}.md", slug(&declaration.name.name)),
            markdown: render_trait_page(declaration, source),
        });
    }
    pages
}

/// Renders the synthetic standard-library module index.
fn render_standard_index(
    types: &[&str],
    functions: &[StandardFunction],
    namespaces: &[StandardNamespace],
    enums: &[&str],
    traits: &[&str],
) -> String {
    let mut output = String::from(
        "# Module `std`\n\nBuilt-in standard items are globally available in ExS source and may also be written with the `std::` qualifier. Importing `std` is not required or allowed.\n\n## Types\n\n",
    );
    for type_name in types {
        output.push_str(&format!(
            "- [`{}`](types/{}.md)\n",
            type_name,
            slug(type_name)
        ));
    }
    if !namespaces.is_empty() {
        output.push_str("\n## Namespaces\n\n");
        for namespace in namespaces {
            output.push_str(&format!(
                "- [`{}`](namespaces/{}.md)\n",
                namespace.name,
                slug(namespace.name)
            ));
        }
    }
    if !enums.is_empty() {
        output.push_str("\n## Enums\n\n");
        for enum_name in enums {
            output.push_str(&format!(
                "- [`{}`](enums/{}.md)\n",
                enum_name,
                slug(enum_name)
            ));
        }
    }
    output.push_str("\n## Traits\n\n");
    for trait_name in traits {
        output.push_str(&format!(
            "- [`{}`](traits/{}.md)\n",
            trait_name,
            slug(trait_name)
        ));
    }
    output.push_str("\n## Functions\n\n");
    for function in functions {
        if function.name == "Error" {
            output.push_str("- [`Error`](types/error.md#constructor)\n");
        } else {
            render_standard_function(&mut output, function, "###");
        }
    }
    output
}

/// Builds one dedicated compiler-owned standard-trait API page.
fn standard_trait_page(descriptor: &StandardTraitDescriptor) -> DocumentationPage {
    let implementations = descriptor
        .implemented_by
        .iter()
        .map(|type_name| {
            let directory = if codegen_standard::enum_descriptor(type_name).is_some() {
                "enums"
            } else {
                "types"
            };
            format!(
                "- [`std::{type_name}`](../{directory}/{}.md)",
                slug(type_name)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let methods = descriptor
        .methods
        .iter()
        .map(|method| {
            format!(
                "### `{}`\n\n```exs\n{}\n```\n\n{}",
                method.name, method.signature, method.description
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let method_heading = if descriptor.methods.len() == 1 {
        "Required Method"
    } else {
        "Required Methods"
    };
    DocumentationPage {
        path: format!("modules/std/traits/{}.md", slug(descriptor.name)),
        markdown: format!(
            "# Trait `std::{}`\n\n{}\n\n## {method_heading}\n\n{methods}\n\n## Built-in Implementations\n\n{implementations}\n\n## Usage\n\n```exs\n{}\n```\n",
            descriptor.name, descriptor.description, descriptor.usage,
        ),
    }
}

/// Renders one compiler-owned standard enum page.
fn render_standard_enum(descriptor: &StandardEnumDescriptor) -> String {
    let mut output = format!(
        "# Enum `std::{}`\n\n{}\n\n```exs\nenum {} {{\n",
        descriptor.name, descriptor.description, descriptor.name
    );
    for variant in descriptor.variants {
        output.push_str(&format!("    {variant},\n"));
    }
    output.push_str("}\n```\n\n## Usage\n\n```exs\n");
    output.push_str(descriptor.usage);
    output.push_str("\n```\n");
    let traits = codegen_standard::traits()
        .iter()
        .filter(|trait_info| trait_info.implemented_by.contains(&descriptor.name));
    if traits.clone().next().is_some() {
        output.push_str("\n## Implemented Methods\n\n");
        for trait_info in traits {
            render_standard_trait_implementation(&mut output, trait_info);
        }
    }
    output.push_str("\n## Runtime Methods\n\n");
    render_clone_method(&mut output, descriptor.name);
    output
}

/// Renders the Error type page with its source-level constructor.
fn render_standard_error_type(type_info: &StandardType) -> String {
    let mut output = render_standard_type(type_info);
    if let Some(constructor) = standard_library_functions()
        .iter()
        .find(|function| function.name == "Error")
    {
        output.push_str("\n## Constructor\n\n```exs\n");
        output.push_str(constructor.signature);
        output.push_str("\n```\n\n");
        output.push_str(constructor.description);
        output.push_str("\n\n```exs\n");
        output.push_str(&script_example(constructor.example));
        output.push_str("\n```\n");
    }
    output
}

/// Renders one synthetic standard-library type page.
fn render_standard_type(type_info: &StandardType) -> String {
    let mut output = format!(
        "# Type `std::{}`\n\n{}\n\n",
        type_info.name, type_info.description
    );
    output.push_str("```exs\n");
    output.push_str(&format!("type {}\n```\n", type_info.name));
    output.push_str("\n## Usage\n\n```exs\n");
    output.push_str(type_info.usage);
    output.push_str("\n```\n");
    let traits = codegen_standard::traits()
        .iter()
        .filter(|descriptor| descriptor.implemented_by.contains(&type_info.name))
        .collect::<Vec<_>>();
    output.push_str("\n## Implemented Methods\n\n");
    for descriptor in traits {
        render_standard_trait_implementation(&mut output, descriptor);
    }
    for method in type_info.methods {
        output.push_str(&format!(
            "### `{}`\n\n{}\n\n```exs\n{}\n```\n\n",
            method.signature,
            method.description,
            script_example(method.example)
        ));
    }
    let static_functions = standard_library_type_static_functions(type_info.name);
    if !static_functions.is_empty() {
        output.push_str("## Static Functions\n\n");
        for function in static_functions {
            render_standard_function(&mut output, function, "###");
        }
    }
    render_clone_method(&mut output, type_info.name);
    output
}

/// Renders one standard static function under a caller-selected Markdown heading level.
fn render_standard_function(output: &mut String, function: &StandardFunction, heading: &str) {
    output.push_str(&format!(
        "{heading} `{}`\n\n{}\n\n```exs\n{}\n```\n\n",
        function.signature,
        function.description,
        script_example(function.example)
    ));
}

/// Renders the automatic runtime-owned deep clone method for one source-visible type.
pub(super) fn render_clone_method(output: &mut String, type_name: &str) {
    output.push_str(&format!(
        "### `clone() -> {type_name} | Error`\n\nCreates a synchronous deep copy of this value's reachable mutable graph. Lists, Objects, nominal values, enum payloads, Errors, Cells, and Closures are copied while preserving aliases and cycles inside the copy; immutable values such as None, Bool, Int, Float, String, and Bytes are reused. `clone()` never mutates its source, cannot be overridden, and returns `CloneError` when a reachable host-owned resource cannot be cloned.\n\n```exs\nfn main(value: {type_name}) -> {type_name} | Error {{\n    ret value.clone();\n}}\n```\n\n"
    ));
}

/// Renders one built-in trait implementation with the same method detail as nominal pages.
fn render_standard_trait_implementation(output: &mut String, descriptor: &StandardTraitDescriptor) {
    output.push_str(&format!(
        "### Trait [`{}`](../traits/{}.md)\n\n",
        descriptor.name,
        slug(descriptor.name)
    ));
    for method in descriptor.methods {
        output.push_str(&format!("#### `{}`\n\n", method.name));
        output.push_str(method.description);
        output.push_str("\n\n```exs\n");
        output.push_str(method.signature.trim_end_matches(';'));
        output.push_str(" { ... }\n```\n\n");
    }
}

/// Renders one method-body example as an independently runnable ExS script.
fn script_example(body: &str) -> String {
    let mut output = String::from("fn main() {\n");
    for line in body.lines() {
        output.push_str("    ");
        output.push_str(line);
        output.push('\n');
    }
    output.push('}');
    output
}

/// Builds one synthetic standard-library namespace page.
fn standard_namespace_page(namespace: &StandardNamespace) -> DocumentationPage {
    let mut markdown = format!(
        "# Namespace `std::{}`\n\n{}\n\n## Functions\n\n",
        namespace.name, namespace.description
    );
    for function in namespace.functions {
        render_standard_function(&mut markdown, function, "###");
    }
    DocumentationPage {
        path: format!("modules/std/namespaces/{}.md", slug(namespace.name)),
        markdown,
    }
}
