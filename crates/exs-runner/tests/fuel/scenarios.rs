//! Fuel scenarios shared by the ExS fuel ceilings and the Rust baseline comparison.
//!
//! Every scenario describes one ExS script exporting `main`, its inputs and its expected
//! result. The Rust baseline guest in `examples/fuel-baseline` exports a function with the
//! scenario's name that performs the same work, so both can be measured side by side.

use exs_abi::ExsValue;

/// One fuel scenario with its ExS script, inputs and expected result.
pub struct Scenario {
    /// Stable name, also the export name of the matching Rust baseline function.
    pub name: &'static str,
    /// ExS source exporting `main`.
    pub source: &'static str,
    /// Builds the inputs passed to `main`.
    pub inputs: fn() -> Vec<ExsValue>,
    /// Builds the expected result of `main`.
    pub expected: fn() -> ExsValue,
}

/// Every scenario, in reporting order.
pub const ALL: &[Scenario] = &[
    EMPTY_MAIN,
    SHOW_TRUE,
    SHOW_COMPARE_RECORD,
    CONCAT_10_OBJECTS,
    CONCAT_100_OBJECTS,
    INT_LOOP_1000,
    ASYNC_INT_LOOP_100,
    ASYNC_EMPTY_WHILE_1000,
    ASYNC_INT_WHILE_1000,
    FOR_RANGE_1000,
    FOR_RANGE_VARIABLE_1000,
    ASYNC_CALL_LOOP_100,
    RESUMABLE_CALL_LOOP_100,
    FORMAT_LOOP_1000,
    LIST_CHURN_6000,
    SYNC_CALL_LOOP_1000,
    DATETIME_ROUNDTRIP_100,
    RECORD_ROUNDTRIP_20,
];

/// Measures the fixed overhead of entering and leaving an empty function.
pub const EMPTY_MAIN: Scenario = Scenario {
    name: "empty_main",
    source: "fn main() {}",
    inputs: no_inputs,
    expected: none_result,
};

/// Measures a show function that unconditionally returns `true`.
pub const SHOW_TRUE: Scenario = Scenario {
    name: "show_true",
    source: "fn main() -> Bool { ret true; }",
    inputs: no_inputs,
    expected: true_result,
};

/// Measures a show function comparing a string and an int of a host-supplied record object.
///
/// The ABI carries records as untyped objects, so the parameter is untyped like in real scripts.
pub const SHOW_COMPARE_RECORD: Scenario = Scenario {
    name: "show_compare_record",
    source: r#"
        fn main(record) -> Bool {
            ret record.name == "active" && record.count > 10;
        }
    "#,
    inputs: || {
        vec![ExsValue::Object(vec![
            ("name".to_owned(), ExsValue::String("active".to_owned())),
            ("count".to_owned(), ExsValue::Int(42)),
        ])]
    },
    expected: true_result,
};

/// Script concatenating the `name` field of every supplied object.
const CONCAT_SOURCE: &str = r#"
    fn main(items: List) -> String {
        let output = "";
        for item in items {
            output = output + item.name + ";";
        }
        ret output;
    }
"#;

/// Measures concatenating one field of 10 host-supplied objects.
pub const CONCAT_10_OBJECTS: Scenario = Scenario {
    name: "concat_10_objects",
    source: CONCAT_SOURCE,
    inputs: || vec![named_objects(10)],
    expected: || expected_concat(10),
};

/// Measures concatenating one field of 100 host-supplied objects.
pub const CONCAT_100_OBJECTS: Scenario = Scenario {
    name: "concat_100_objects",
    source: CONCAT_SOURCE,
    inputs: || vec![named_objects(100)],
    expected: || expected_concat(100),
};

/// Measures 1000 iterations of a loop with a comparison, additions and a multiplication.
pub const INT_LOOP_1000: Scenario = Scenario {
    name: "int_loop_1000",
    source: r#"
        fn main(limit: Int) -> Int {
            let total = 0;
            let index = 0;
            while index < limit {
                total = total + index * 3 + 1;
                index = index + 1;
            }
            ret total;
        }
    "#,
    inputs: || vec![ExsValue::Int(1000)],
    expected: || ExsValue::Int(1_499_500),
};

/// Measures 100 iterations of a resumable `for` loop with a comparison and arithmetic.
///
/// The `for` loop selects resumable lowering, exercising async comparisons.
pub const ASYNC_INT_LOOP_100: Scenario = Scenario {
    name: "async_int_loop_100",
    source: r#"
        fn main(limit: Int) -> Int {
            let total = 0;
            for index in 0..limit {
                if index < limit {
                    total = total + index * 3 + 1;
                }
            }
            ret total;
        }
    "#,
    inputs: || vec![ExsValue::Int(100)],
    expected: || ExsValue::Int(14_950),
};

/// Measures 1000 iterations of an empty `while` loop in a resumable function.
///
/// The leading host call selects resumable lowering for an otherwise empty loop.
pub const ASYNC_EMPTY_WHILE_1000: Scenario = Scenario {
    name: "async_empty_while_1000",
    source: r#"
        fn main(limit: Int) -> Int {
            Host::elapsed();
            let index = 0;
            while index < limit {
                index = index + 1;
            }
            ret index;
        }
    "#,
    inputs: || vec![ExsValue::Int(1000)],
    expected: || ExsValue::Int(1000),
};

/// Measures 1000 iterations of an integer arithmetic `while` loop in a resumable function.
pub const ASYNC_INT_WHILE_1000: Scenario = Scenario {
    name: "async_int_while_1000",
    source: r#"
        fn main(limit: Int) -> Int {
            Host::elapsed();
            let total = 0;
            let index = 0;
            while index < limit {
                total = total + index * 3 + 1;
                index = index + 1;
            }
            ret total;
        }
    "#,
    inputs: || vec![ExsValue::Int(1000)],
    expected: || ExsValue::Int(1_499_500),
};

/// Measures 1000 iterations of an empty `for` loop over a Range.
pub const FOR_RANGE_1000: Scenario = Scenario {
    name: "for_range_1000",
    source: r#"
        fn main(limit: Int) -> Int {
            let count = 0;
            for index in 0..limit {
            }
            ret count;
        }
    "#,
    inputs: || vec![ExsValue::Int(1000)],
    expected: || ExsValue::Int(0),
};

/// Measures 1000 iterations of an empty `for` loop over a Range that stays visible to the script.
pub const FOR_RANGE_VARIABLE_1000: Scenario = Scenario {
    name: "for_range_variable_1000",
    source: r#"
        fn main(limit: Int) -> Int {
            let range = 0..limit;
            for index in range {
            }
            ret range.current;
        }
    "#,
    inputs: || vec![ExsValue::Int(1000)],
    expected: || ExsValue::Int(1000),
};

/// Measures 100 calls of a small function with a `match`, which was resumable before `match`
/// lowered synchronously.
pub const ASYNC_CALL_LOOP_100: Scenario = Scenario {
    name: "async_call_loop_100",
    source: r#"
        fn bump(value: Int) -> Int {
            ret match value {
                _ => value + 1,
            };
        }

        fn main(limit: Int) -> Int {
            let total = 0;
            let index = 0;
            while index < limit {
                total = bump(total);
                index = index + 1;
            }
            ret total;
        }
    "#,
    inputs: || vec![ExsValue::Int(100)],
    expected: || ExsValue::Int(100),
};

/// Measures 100 calls of a small resumable function; any closure makes every function resumable.
pub const RESUMABLE_CALL_LOOP_100: Scenario = Scenario {
    name: "resumable_call_loop_100",
    source: r#"
        fn bump(value: Int) -> Int {
            ret value + 1;
        }

        fn callback() {
            ret () => { ret 1; };
        }

        fn main(limit: Int) -> Int {
            let total = 0;
            let index = 0;
            while index < limit {
                total = bump(total);
                index = index + 1;
            }
            ret total;
        }
    "#,
    inputs: || vec![ExsValue::Int(100)],
    expected: || ExsValue::Int(100),
};

/// Measures 1000 iterations building one formatted line with two interpolated Ints.
pub const FORMAT_LOOP_1000: Scenario = Scenario {
    name: "format_loop_1000",
    source: r#"
        fn main(limit: Int) -> Int {
            let length = 0;
            let index = 0;
            while index < limit {
                let line = f"row {index}: {index * 2};";
                length = length + line.length();
                index = index + 1;
            }
            ret length;
        }
    "#,
    inputs: || vec![ExsValue::Int(1000)],
    expected: || {
        ExsValue::Int(
            (0..1000_i64)
                .map(|index| format!("row {index}: {};", index * 2).len() as i64)
                .sum(),
        )
    },
};

/// Measures 6000 iterations allocating short-lived Lists, which triggers garbage collection.
pub const LIST_CHURN_6000: Scenario = Scenario {
    name: "list_churn_6000",
    source: r#"
        fn main(limit: Int) -> Int {
            let total = 0;
            let index = 0;
            while index < limit {
                let pair = [index, index];
                total = total + pair[0];
                index = index + 1;
            }
            ret total;
        }
    "#,
    inputs: || vec![ExsValue::Int(6000)],
    expected: || ExsValue::Int(17_997_000),
};

/// Measures 1000 calls of a small typed function in a synchronous loop.
pub const SYNC_CALL_LOOP_1000: Scenario = Scenario {
    name: "sync_call_loop_1000",
    source: r#"
        fn bump(value: Int) -> Int {
            ret value + 1;
        }

        fn main(limit: Int) -> Int {
            let total = 0;
            let index = 0;
            while index < limit {
                total = bump(total);
                index = index + 1;
            }
            ret total;
        }
    "#,
    inputs: || vec![ExsValue::Int(1000)],
    expected: || ExsValue::Int(1000),
};

/// Measures parsing 100 RFC 3339 strings into native DateTimes and formatting them back.
pub const DATETIME_ROUNDTRIP_100: Scenario = Scenario {
    name: "datetime_roundtrip_100",
    source: r#"
        fn main(values: List) -> List | Error {
            let output = [];
            for value in values {
                let text: String = value;
                let parsed = DateTime::parse_rfc3339(text)?;
                output.push(parsed.to_rfc3339()?);
            }
            ret output;
        }
    "#,
    inputs: || vec![ExsValue::List((0..100).map(datetime_input).collect())],
    expected: || ExsValue::List((0..100).map(datetime_output).collect()),
};

/// Measures an Exolynk-style round trip of 20 records through typed ExS values.
///
/// Like the Exolynk script API, the script hydrates host Objects into nominal types and enums
/// (including an enum with payload, nested Lists, a tagged `Datetime` and a tagged `Currency`),
/// processes them and dehydrates them back into the host Object shape.
pub const RECORD_ROUNDTRIP_20: Scenario = Scenario {
    name: "record_roundtrip_20",
    source: RECORD_SOURCE,
    inputs: || vec![ExsValue::List((0..20).map(record_input).collect())],
    expected: || ExsValue::List((0..20).map(record_output).collect()),
};

/// Script hydrating, processing and dehydrating Exolynk-style records.
const RECORD_SOURCE: &str = r#"
    enum Status { Open, Paid, Cancelled }
    enum ItemKind { Product, Service(hours: Int) }

    type Money { amount: Int, currency: String }
    type Item { sku: String, quantity: Int, price: Int, kind: ItemKind }
    type Order {
        id: Int,
        name: String,
        status: Status,
        created: DateTime,
        total: Money,
        tags: List,
        items: List,
    }

    fn hydrate_datetime(value: String) -> DateTime | Error {
        let parts = value.split(" ");
        if parts.length() != 3 {
            ret Error("ParseError", "DateTime must contain date, time, and UTC offset", value);
        }
        let offset = parts[2];
        let normalized = parts[0] + "T" + parts[1] + offset.slice(0, 6)?;
        ret DateTime::parse_rfc3339(normalized);
    }

    fn hydrate_status(value: String) -> Status | Error {
        if value == "open" { ret Status::Open; }
        if value == "paid" { ret Status::Paid; }
        if value == "cancelled" { ret Status::Cancelled; }
        ret Error("ValueError", "unknown order status", value);
    }

    fn dehydrate_status(status: Status) -> String {
        ret match status {
            Status::Open => "open",
            Status::Paid => "paid",
            Status::Cancelled => "cancelled",
        };
    }

    fn hydrate_item(raw: Object) -> Item | Error {
        let kind_raw: Object = raw.kind;
        let kind = ItemKind::Product;
        if kind_raw.has("Service")? {
            let hours: Int = kind_raw["Service"];
            kind = ItemKind::Service(hours);
        }
        let sku: String = raw.sku;
        let quantity: Int = raw.quantity;
        let price: Int = raw.price;
        ret Item { sku: sku, quantity: quantity, price: price, kind: kind };
    }

    fn dehydrate_kind(kind: ItemKind) -> Object {
        let hours = match kind {
            ItemKind::Product => -1,
            ItemKind::Service(value) => value,
        };
        if hours < 0 { ret { Product: None }; }
        ret { Service: hours };
    }

    fn hydrate_order(raw: Object) -> Order | Error {
        let items = [];
        for raw_item in raw.items {
            items.push(hydrate_item(raw_item)?);
        }
        let tags = [];
        for tag in raw.tags {
            let text: String = tag;
            tags.push(text);
        }
        let created_raw: Object = raw.created;
        let created_text: String = created_raw["Datetime"];
        let total_raw: Object = raw.total;
        let money_raw: Object = total_raw["Currency"];
        let amount: Int = money_raw.amount;
        let currency: String = money_raw.currency;
        let id: Int = raw.id;
        let name: String = raw.name;
        let status_text: String = raw.status;
        ret Order {
            id: id,
            name: name,
            status: hydrate_status(status_text)?,
            created: hydrate_datetime(created_text)?,
            total: Money { amount: amount, currency: currency },
            tags: tags,
            items: items,
        };
    }

    fn process(order: Order) -> Order | Error {
        let amount = 0;
        for item in order.items {
            let surcharge = match item.kind {
                ItemKind::Product => 0,
                ItemKind::Service(hours) => hours * 1500,
            };
            amount = amount + item.quantity * item.price + surcharge;
        }
        order.total = Money { amount: amount, currency: order.total.currency };
        let open = match order.status {
            Status::Open => true,
            _ => false,
        };
        if open && amount > 10000 {
            order.status = Status::Paid;
        }
        order.created = order.created.add_duration(Duration::seconds(86400)?)?;
        order.tags.push(f"processed-{order.id}");
        ret order;
    }

    fn dehydrate_order(order: Order) -> Object | Error {
        let items = [];
        for item in order.items {
            items.push({
                sku: item.sku,
                quantity: item.quantity,
                price: item.price,
                kind: dehydrate_kind(item.kind),
            });
        }
        ret {
            id: order.id,
            name: order.name,
            status: dehydrate_status(order.status),
            created: { Datetime: order.created.to_rfc3339()? },
            total: { Currency: { amount: order.total.amount, currency: order.total.currency } },
            tags: order.tags,
            items: items,
        };
    }

    fn main(records: List) -> List | Error {
        let output = [];
        for raw in records {
            let order = hydrate_order(raw)?;
            output.push(dehydrate_order(process(order)?)?);
        }
        ret output;
    }
"#;

/// Builds no inputs.
fn no_inputs() -> Vec<ExsValue> {
    Vec::new()
}

/// Builds the None result.
fn none_result() -> ExsValue {
    ExsValue::None
}

/// Builds the `true` result.
fn true_result() -> ExsValue {
    ExsValue::Bool(true)
}

/// Builds `count` host objects with a `name` field, mimicking records loaded outside the script.
fn named_objects(count: usize) -> ExsValue {
    ExsValue::List(
        (0..count)
            .map(|index| {
                object(vec![
                    ("name", ExsValue::String(format!("item{index}"))),
                    ("count", ExsValue::Int(index as i64)),
                ])
            })
            .collect(),
    )
}

/// Builds the string the concatenation script returns for `count` objects.
fn expected_concat(count: usize) -> ExsValue {
    ExsValue::String((0..count).map(|index| format!("item{index};")).collect())
}

/// UTC offsets cycled through by the DateTime round-trip inputs.
const DATETIME_OFFSETS: [&str; 4] = ["Z", "+01:00", "-05:30", "+14:00"];

/// Builds one RFC 3339 input with varying fields, fractions and offsets.
fn datetime_input(index: i64) -> ExsValue {
    let fraction = if index % 3 == 0 { ".25" } else { "" };
    ExsValue::String(format!(
        "{}{fraction}{}",
        datetime_fields(index),
        DATETIME_OFFSETS[(index % 4) as usize]
    ))
}

/// Builds the canonical RFC 3339 text `to_rfc3339` produces for one input.
fn datetime_output(index: i64) -> ExsValue {
    let fraction = if index % 3 == 0 {
        ".250000000"
    } else {
        ".000000000"
    };
    ExsValue::String(format!(
        "{}{fraction}{}",
        datetime_fields(index),
        DATETIME_OFFSETS[(index % 4) as usize]
    ))
}

/// Returns the date and time fields of one DateTime input without fraction and offset.
fn datetime_fields(index: i64) -> String {
    format!(
        "2024-{:02}-{:02}T{:02}:{:02}:{:02}",
        1 + index % 12,
        1 + index % 28,
        index % 24,
        (index * 7) % 60,
        (index * 13) % 60
    )
}

/// Order statuses cycled through by the record inputs.
const RECORD_STATUSES: [&str; 4] = ["open", "paid", "cancelled", "open"];
/// UTC offsets cycled through by the record creation timestamps.
const RECORD_OFFSETS: [&str; 3] = ["+01:00", "+00:00", "-05:30"];

/// Builds one Exolynk-style host record.
fn record_input(index: i64) -> ExsValue {
    object(vec![
        ("id", ExsValue::Int(index)),
        ("name", ExsValue::String(format!("Order {index}"))),
        (
            "status",
            ExsValue::String(RECORD_STATUSES[(index % 4) as usize].to_owned()),
        ),
        (
            "created",
            object(vec![(
                "Datetime",
                ExsValue::String(format!(
                    "{} {}",
                    record_created(index, " "),
                    RECORD_OFFSETS[(index % 3) as usize]
                )),
            )]),
        ),
        ("total", currency(0, record_currency(index))),
        ("tags", record_tags(index, false)),
        ("items", record_items(index)),
    ])
}

/// Builds the record the script returns for one input record.
fn record_output(index: i64) -> ExsValue {
    let amount = (0..3)
        .map(|item| record_item_amount(index, item))
        .sum::<i64>();
    let status = RECORD_STATUSES[(index % 4) as usize];
    let status = if status == "open" && amount > 10_000 {
        "paid"
    } else {
        status
    };
    let offset = match RECORD_OFFSETS[(index % 3) as usize] {
        "+00:00" => "Z",
        offset => offset,
    };
    object(vec![
        ("id", ExsValue::Int(index)),
        ("name", ExsValue::String(format!("Order {index}"))),
        ("status", ExsValue::String(status.to_owned())),
        (
            "created",
            object(vec![(
                "Datetime",
                ExsValue::String(format!(
                    "{}.000000000{offset}",
                    record_created_next_day(index)
                )),
            )]),
        ),
        ("total", currency(amount, record_currency(index))),
        ("tags", record_tags(index, true)),
        ("items", record_items(index)),
    ])
}

/// Returns one record's creation date and time joined by `separator`.
///
/// Days stay at or below 27, so adding one day never changes the month.
fn record_created(index: i64, separator: &str) -> String {
    format!(
        "2024-{:02}-{:02}{separator}{:02}:{:02}:00",
        1 + index % 12,
        1 + index % 27,
        index % 24,
        (index * 7) % 60
    )
}

/// Returns one record's creation timestamp one day later in RFC 3339 form without fraction.
fn record_created_next_day(index: i64) -> String {
    format!(
        "2024-{:02}-{:02}T{:02}:{:02}:00",
        1 + index % 12,
        2 + index % 27,
        index % 24,
        (index * 7) % 60
    )
}

/// Returns one record's currency code.
fn record_currency(index: i64) -> &'static str {
    if index % 2 == 0 { "EUR" } else { "CHF" }
}

/// Builds one record's tags, with the processing tag when `processed`.
fn record_tags(index: i64, processed: bool) -> ExsValue {
    let mut tags = vec![
        ExsValue::String(format!("t{index}")),
        ExsValue::String(format!("group{}", index % 3)),
    ];
    if processed {
        tags.push(ExsValue::String(format!("processed-{index}")));
    }
    ExsValue::List(tags)
}

/// Builds one record's three line items; the last one is a service with billed hours.
fn record_items(index: i64) -> ExsValue {
    ExsValue::List(
        (0..3)
            .map(|item| {
                let kind = if item == 2 {
                    object(vec![(
                        "Service",
                        ExsValue::Int(record_service_hours(index)),
                    )])
                } else {
                    object(vec![("Product", ExsValue::None)])
                };
                object(vec![
                    ("sku", ExsValue::String(format!("SKU-{index}-{item}"))),
                    ("quantity", ExsValue::Int(record_quantity(index, item))),
                    ("price", ExsValue::Int(record_price(index, item))),
                    ("kind", kind),
                ])
            })
            .collect(),
    )
}

/// Returns one line item's quantity.
fn record_quantity(index: i64, item: i64) -> i64 {
    item + 1 + index % 4
}

/// Returns one line item's unit price.
fn record_price(index: i64, item: i64) -> i64 {
    250 * (item + 1) + index * 10
}

/// Returns the billed hours of one record's service item.
fn record_service_hours(index: i64) -> i64 {
    index % 5 + 1
}

/// Returns one line item's total including the service surcharge.
fn record_item_amount(index: i64, item: i64) -> i64 {
    let surcharge = if item == 2 {
        record_service_hours(index) * 1500
    } else {
        0
    };
    record_quantity(index, item) * record_price(index, item) + surcharge
}

/// Builds one tagged Exolynk Currency object.
fn currency(amount: i64, code: &str) -> ExsValue {
    object(vec![(
        "Currency",
        object(vec![
            ("amount", ExsValue::Int(amount)),
            ("currency", ExsValue::String(code.to_owned())),
        ]),
    )])
}

/// Builds one host Object from borrowed keys.
fn object(entries: Vec<(&str, ExsValue)>) -> ExsValue {
    ExsValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}
