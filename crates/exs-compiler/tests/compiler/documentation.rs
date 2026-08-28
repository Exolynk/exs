use super::*;

/// Generates language and API pages for reachable imported modules.
#[test]
fn generates_markdown_api_documentation() {
    let mut resolver = TestResolver {
        sources: HashMap::from([(
            "./math.exs".to_owned(),
            "/// Adds two integers.\nfn add(left: Int, right: Int) -> Int { ret left + right; }\n\n/// A coordinate.\ntype Point { value: Int }\n\nimpl Point { fn coordinate(self) -> Int { ret self.value; } }\n\n/// A display color.\nenum Color { Rgb(red: Int, green: Int, blue: Int), Transparent, }\n\nimpl Color { fn channels(self) -> Int { ret 3; } }".to_owned(),
        )]),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./main.exs",
            text: "import \"./math.exs\" as math;\n/// Runs the program.\nfn main() -> Int { ret math::add(20, 22); }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation,
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    assert!(documentation.index.contains("## Language"));
    assert!(
        documentation
            .index
            .contains("[`std`](modules/std/index.md)")
    );
    let main = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/00-main/index.md")
        .unwrap_or_else(|| panic!("missing main module page"));
    assert!(main.markdown.contains("[module](../01-math/index.md)"));
    let math = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/index.md")
        .unwrap_or_else(|| panic!("missing math module index"));
    assert!(math.markdown.starts_with("# Module `./math.exs`"));
    assert!(math.markdown.contains("[`add`](fn/add.md)"));
    assert!(math.markdown.contains("[`Point`](types/point.md)"));
    assert!(math.markdown.contains("[`Color`](enums/color.md)"));
    assert!(!math.markdown.contains("## Implementations"));
    let add = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/fn/add.md")
        .unwrap_or_else(|| panic!("missing function page"));
    assert!(add.markdown.contains("Adds two integers."));
    let point = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/types/point.md")
        .unwrap_or_else(|| panic!("missing Point type page"));
    assert!(point.markdown.contains("## Implemented Methods"));
    assert!(point.markdown.contains("coordinate"));
    assert!(point.markdown.contains("## Runtime Methods"));
    assert!(point.markdown.contains("clone() -> Point | Error"));
    let color = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/01-math/enums/color.md")
        .unwrap_or_else(|| panic!("missing Color enum page"));
    assert!(
        color
            .markdown
            .contains("Rgb(red: Int, green: Int, blue: Int)")
    );
    assert!(color.markdown.contains("## Implemented Methods"));
    assert!(color.markdown.contains("clone() -> Color | Error"));
    let host = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/namespaces/host.md")
        .unwrap_or_else(|| panic!("missing std Host namespace page"));
    assert!(host.markdown.contains("Host::call(name, arguments...)"));
    assert!(
        host.markdown
            .contains("Host::stream(name, arguments...) -> HostStream | Error")
    );
    assert!(
        host.markdown
            .contains("Host::sleep(duration: Duration) -> None")
    );
    assert!(host.markdown.contains("Host::now() -> DateTime"));
    assert!(host.markdown.contains("Host::elapsed() -> Duration"));
    assert!(host.markdown.contains("# Namespace `std::Host`"));
    let duration = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/duration.md")
        .unwrap_or_else(|| panic!("missing std Duration type page"));
    assert!(duration.markdown.contains("## Implemented Methods"));
    assert!(
        duration
            .markdown
            .contains("fn milliseconds(value: Int) -> Duration | Error")
    );
    let date_time = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/datetime.md")
        .unwrap_or_else(|| panic!("missing std DateTime type page"));
    assert!(
        date_time
            .markdown
            .contains("optional IANA time-zone metadata")
    );
    assert!(
        date_time
            .markdown
            .contains("fn duration_since(self, earlier: DateTime)")
    );
    assert!(
        duration
            .markdown
            .contains("Creates an exact Duration from a non-negative millisecond count.")
    );
    assert!(
        duration
            .markdown
            .contains("fn nanoseconds(value: Int) -> Duration | Error")
    );
    assert!(
        duration
            .markdown
            .contains("fn microseconds(value: Int) -> Duration | Error")
    );
    assert!(
        duration
            .markdown
            .contains("fn seconds(value: Int) -> Duration | Error")
    );
    assert!(
        duration
            .markdown
            .contains("fn as_nanoseconds(self) -> Int | Error")
    );
    let iterator_step = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/enums/iteratorstep.md")
        .unwrap_or_else(|| panic!("missing std IteratorStep enum page"));
    assert!(
        iterator_step
            .markdown
            .contains("The result of advancing one Iterator.")
    );
    assert!(iterator_step.markdown.contains("Item(value: Any)"));
    let iterator = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/traits/iterator.md")
        .unwrap_or_else(|| panic!("missing std Iterator trait page"));
    assert!(
        iterator
            .markdown
            .contains("fn next(self) -> IteratorStep | Error;")
    );
    let host_stream = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/hoststream.md")
        .unwrap_or_else(|| panic!("missing std HostStream type page"));
    assert!(
        host_stream
            .markdown
            .contains("A host-backed pull stream that implements Iterator.")
    );
    assert!(
        host_stream
            .markdown
            .contains("Trait [`Iterator`](../traits/iterator.md)")
    );
    let standard = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/index.md")
        .unwrap_or_else(|| panic!("missing std module page"));
    assert!(standard.markdown.contains("[`Host`](namespaces/host.md)"));
    assert!(
        !standard
            .markdown
            .contains("[`Duration`](namespaces/duration.md)")
    );
    assert!(!standard.markdown.contains("[`type`]"));
    assert!(!standard.markdown.contains("[`len`]"));
    assert!(
        standard
            .markdown
            .contains("[`Duration`](types/duration.md)")
    );
    assert!(
        standard
            .markdown
            .contains("[`HostStream`](types/hoststream.md)")
    );
    assert!(
        standard
            .markdown
            .contains("[`IteratorStep`](enums/iteratorstep.md)")
    );
    assert!(
        standard
            .markdown
            .contains("[`Iterator`](traits/iterator.md)")
    );
    assert!(standard.markdown.contains("`std::` qualifier"));
    assert!(standard.markdown.contains("[`Add`](traits/add.md)"));
    assert!(standard.markdown.contains("[`Sub`](traits/sub.md)"));
    assert!(standard.markdown.contains("[`Mul`](traits/mul.md)"));
    assert!(standard.markdown.contains("[`Div`](traits/div.md)"));
    assert!(standard.markdown.contains("[`Compare`](traits/compare.md)"));
    assert!(
        standard
            .markdown
            .contains("[`Ordering`](enums/ordering.md)")
    );
    let test_namespace = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/namespaces/test.md")
        .unwrap_or_else(|| panic!("missing std test namespace page"));
    assert!(test_namespace.markdown.contains("# Namespace `std::test`"));
    assert!(
        test_namespace
            .markdown
            .contains("std::test::assert(condition: Bool")
    );
    assert!(
        test_namespace
            .markdown
            .contains("std::test::assert_eq(actual: Any, expected: Any")
    );
    assert!(documentation.pages.iter().all(|page| !matches!(
        page.path.as_str(),
        "modules/std/fn/type.md"
            | "modules/std/fn/len.md"
            | "modules/std/namespaces/std.md"
            | "modules/std/namespaces/duration.md"
    )));
    assert!(
        documentation
            .pages
            .iter()
            .all(|page| !page.path.contains("__exs_"))
    );
    let error = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/error.md")
        .unwrap_or_else(|| panic!("missing std Error type page"));
    assert!(error.markdown.contains("## Constructor"));
    let any = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/any.md")
        .unwrap_or_else(|| panic!("missing std Any page"));
    assert!(any.markdown.contains("clone() -> Any | Error"));
    assert!(any.markdown.contains("preserving aliases and cycles"));
    assert!(error.markdown.contains("Error(kind, message, data)"));
    assert!(error.markdown.contains("### `kind() -> String`"));
    assert!(error.markdown.contains("machine-readable category"));
    assert!(error.markdown.contains("fn main()"));
    assert!(
        documentation
            .pages
            .iter()
            .all(|page| page.path != "modules/std/fn/error.md")
    );
    let list = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/list.md")
        .unwrap_or_else(|| panic!("missing standard List page"));
    assert!(list.markdown.contains("### `push(value) -> Int`"));
    assert!(list.markdown.contains("### `length() -> Int`"));
    assert!(list.markdown.contains("mutates the existing List"));
    assert!(list.markdown.contains("let items = [\"Ada\"];"));
    let float = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/float.md")
        .unwrap_or_else(|| panic!("missing standard Float page"));
    assert!(float.markdown.contains("### `round() -> Float`"));
    let add_trait = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/traits/add.md")
        .unwrap_or_else(|| panic!("missing std Add trait page"));
    assert!(
        add_trait
            .markdown
            .contains("fn add(self, other: Any) -> Any;")
    );
    assert!(
        add_trait
            .markdown
            .contains("Vector { value: 20 } + Vector { value: 22 }")
    );
    assert!(add_trait.markdown.contains("## Built-in Implementations"));
    assert!(
        add_trait
            .markdown
            .contains("[`std::String`](../types/string.md)")
    );
    let string = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/string.md")
        .unwrap_or_else(|| panic!("missing standard String page"));
    assert!(string.markdown.contains("## Implemented Methods"));
    assert!(string.markdown.contains("Trait [`Add`](../traits/add.md)"));
    assert!(string.markdown.contains("#### `add`"));
    assert!(
        string
            .markdown
            .contains("Adds the receiver to the evaluated `other` operand.")
    );
    let div_trait = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/traits/div.md")
        .unwrap_or_else(|| panic!("missing std Div trait page"));
    assert!(
        div_trait
            .markdown
            .contains("fn div(self, other: Any) -> Any;")
    );
    let integer = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/types/int.md")
        .unwrap_or_else(|| panic!("missing standard Int page"));
    assert!(integer.markdown.contains("Trait [`Sub`](../traits/sub.md)"));
    assert!(integer.markdown.contains("Trait [`Mul`](../traits/mul.md)"));
    assert!(integer.markdown.contains("Trait [`Div`](../traits/div.md)"));
    assert!(
        integer
            .markdown
            .contains("Trait [`Compare`](../traits/compare.md)")
    );
    assert!(
        integer
            .markdown
            .contains("Trait [`ToString`](../traits/tostring.md)")
    );
    assert!(
        integer
            .markdown
            .contains("Trait [`Debug`](../traits/debug.md)")
    );
    assert!(integer.markdown.contains("clone() -> Int | Error"));
    let ordering = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/enums/ordering.md")
        .unwrap_or_else(|| panic!("missing standard Ordering enum page"));
    assert!(ordering.markdown.contains("Ordering::Unordered"));
    assert!(ordering.markdown.contains("clone() -> Ordering | Error"));
    let compare_trait = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/traits/compare.md")
        .unwrap_or_else(|| panic!("missing std Compare trait page"));
    assert!(
        compare_trait
            .markdown
            .contains("fn compare(self, other: Any) -> Ordering;")
    );
    let to_string_trait = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/traits/tostring.md")
        .unwrap_or_else(|| panic!("missing std ToString trait page"));
    assert!(
        to_string_trait
            .markdown
            .contains("fn to_string(self) -> String;")
    );
    let debug_trait = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/std/traits/debug.md")
        .unwrap_or_else(|| panic!("missing std Debug trait page"));
    assert!(debug_trait.markdown.contains("fn debug(self) -> String;"));
}

/// Links a standard trait implementation and inherits its method documentation on an enum page.
#[test]
fn documents_standard_add_implementations_on_nominal_pages() {
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./add.exs",
            text: "enum Abc { A, B, } impl Add for Abc { fn add(self, other: Any) -> Any { ret self; } } fn main() { ret Abc::A; }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation,
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    let abc = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/00-add/enums/abc.md")
        .unwrap_or_else(|| panic!("missing enum page"));
    assert!(
        abc.markdown
            .contains("Trait [`Add`](../../std/traits/add.md)")
    );
    assert!(
        abc.markdown
            .contains("Adds the receiver to the evaluated `other` operand.")
    );
}

/// Prefers an implementation method's own documentation over inherited trait documentation.
#[test]
fn prefers_implementation_documentation_over_standard_add_documentation() {
    let mut resolver = TestResolver {
        sources: HashMap::new(),
    };
    let documentation = match document_with_resolver(
        SourceInput {
            source_id: "./custom-add.exs",
            text: "type Counter {} impl Add for Counter {\n/// Adds one application-specific count.\nfn add(self, other: Any) -> Any { ret self; } } fn main() { ret Counter {}; }",
        },
        &mut resolver,
    ) {
        Ok(documentation) => documentation,
        Err(error) => panic!("documentation generation failed: {error}"),
    };
    let counter = documentation
        .pages
        .iter()
        .find(|page| page.path == "modules/00-custom-add/types/counter.md")
        .unwrap_or_else(|| panic!("missing Counter type page"));
    assert!(
        counter
            .markdown
            .contains("Adds one application-specific count.")
    );
    assert!(
        !counter
            .markdown
            .contains("Adds the receiver to the evaluated `other` operand.")
    );
}
