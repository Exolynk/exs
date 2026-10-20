//! Rust counterpart of the Exolynk-style record round trip.
//!
//! Hydrates host Objects into Rust structs and enums, processes them and dehydrates them back
//! into the identical host Object shape the ExS script produces.

use exs_guest::{ExsValue, Vec};

use crate::basic::field;
use crate::datetime::DateTime;

/// Lifecycle status of one order.
#[derive(Clone, Copy, PartialEq)]
enum Status {
    /// Not yet paid.
    Open,
    /// Paid.
    Paid,
    /// Cancelled.
    Cancelled,
}

/// Kind of one order line item.
enum ItemKind {
    /// A product without surcharge.
    Product,
    /// A service billed per hour.
    Service(i64),
}

/// One order line item.
struct Item {
    /// Stock keeping unit.
    sku: String,
    /// Ordered quantity.
    quantity: i64,
    /// Unit price.
    price: i64,
    /// Product or billed service.
    kind: ItemKind,
}

/// One monetary amount.
struct Money {
    /// Amount in minor units.
    amount: i64,
    /// Currency code.
    currency: String,
}

/// One order with its typed fields.
struct Order {
    /// Record identifier.
    id: i64,
    /// Display name.
    name: String,
    /// Lifecycle status.
    status: Status,
    /// Creation timestamp.
    created: DateTime,
    /// Order total.
    total: Money,
    /// Free-form tags.
    tags: Vec<String>,
    /// Line items.
    items: Vec<Item>,
}

/// Returns one String field.
fn string(value: &ExsValue, key: &str) -> Option<String> {
    match field(value, key)? {
        ExsValue::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// Returns one Int field.
fn int(value: &ExsValue, key: &str) -> Option<i64> {
    match field(value, key)? {
        ExsValue::Int(number) => Some(*number),
        _ => None,
    }
}

/// Parses Exolynk's `date time offset` timestamp text.
fn hydrate_datetime(value: &str) -> Option<DateTime> {
    let mut parts = value.split(' ');
    let (date, time, offset) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    DateTime::parse_rfc3339(&format!("{date}T{time}{}", offset.get(..6)?))
}

/// Converts one status text into a status.
fn hydrate_status(value: &str) -> Option<Status> {
    match value {
        "open" => Some(Status::Open),
        "paid" => Some(Status::Paid),
        "cancelled" => Some(Status::Cancelled),
        _ => None,
    }
}

/// Converts one status into its text.
fn dehydrate_status(status: Status) -> &'static str {
    match status {
        Status::Open => "open",
        Status::Paid => "paid",
        Status::Cancelled => "cancelled",
    }
}

/// Converts one host line item.
fn hydrate_item(raw: &ExsValue) -> Option<Item> {
    let kind = match field(field(raw, "kind")?, "Service") {
        Some(ExsValue::Int(hours)) => ItemKind::Service(*hours),
        _ => ItemKind::Product,
    };
    Some(Item {
        sku: string(raw, "sku")?,
        quantity: int(raw, "quantity")?,
        price: int(raw, "price")?,
        kind,
    })
}

/// Converts one host record into an order.
fn hydrate_order(raw: &ExsValue) -> Option<Order> {
    let ExsValue::List(raw_items) = field(raw, "items")? else {
        return None;
    };
    let items = raw_items
        .iter()
        .map(hydrate_item)
        .collect::<Option<Vec<_>>>()?;
    let ExsValue::List(raw_tags) = field(raw, "tags")? else {
        return None;
    };
    let tags = raw_tags
        .iter()
        .map(|tag| match tag {
            ExsValue::String(text) => Some(text.clone()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let created = string(field(raw, "created")?, "Datetime")?;
    let money = field(field(raw, "total")?, "Currency")?;
    Some(Order {
        id: int(raw, "id")?,
        name: string(raw, "name")?,
        status: hydrate_status(&string(raw, "status")?)?,
        created: hydrate_datetime(&created)?,
        total: Money {
            amount: int(money, "amount")?,
            currency: string(money, "currency")?,
        },
        tags,
        items,
    })
}

/// Recomputes the total, pays large open orders, moves the timestamp and tags the order.
fn process(mut order: Order) -> Order {
    let amount = order
        .items
        .iter()
        .map(|item| {
            let surcharge = match item.kind {
                ItemKind::Product => 0,
                ItemKind::Service(hours) => hours * 1500,
            };
            item.quantity * item.price + surcharge
        })
        .sum();
    order.total.amount = amount;
    if order.status == Status::Open && amount > 10_000 {
        order.status = Status::Paid;
    }
    order.created = order.created.add_seconds(86_400);
    order.tags.push(format!("processed-{}", order.id));
    order
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

/// Converts one order back into the host record shape.
fn dehydrate_order(order: Order) -> ExsValue {
    let items = order
        .items
        .into_iter()
        .map(|item| {
            let kind = match item.kind {
                ItemKind::Product => object(vec![("Product", ExsValue::None)]),
                ItemKind::Service(hours) => object(vec![("Service", ExsValue::Int(hours))]),
            };
            object(vec![
                ("sku", ExsValue::String(item.sku)),
                ("quantity", ExsValue::Int(item.quantity)),
                ("price", ExsValue::Int(item.price)),
                ("kind", kind),
            ])
        })
        .collect();
    object(vec![
        ("id", ExsValue::Int(order.id)),
        ("name", ExsValue::String(order.name)),
        (
            "status",
            ExsValue::String(dehydrate_status(order.status).to_owned()),
        ),
        (
            "created",
            object(vec![(
                "Datetime",
                ExsValue::String(order.created.to_rfc3339()),
            )]),
        ),
        (
            "total",
            object(vec![(
                "Currency",
                object(vec![
                    ("amount", ExsValue::Int(order.total.amount)),
                    ("currency", ExsValue::String(order.total.currency)),
                ]),
            )]),
        ),
        (
            "tags",
            ExsValue::List(order.tags.into_iter().map(ExsValue::String).collect()),
        ),
        ("items", ExsValue::List(items)),
    ])
}

/// Hydrates, processes and dehydrates every supplied record.
pub(crate) async fn record_roundtrip_20(inputs: Vec<ExsValue>) -> ExsValue {
    let Some(ExsValue::List(records)) = inputs.first() else {
        return ExsValue::None;
    };
    let mut output = Vec::with_capacity(records.len());
    for raw in records {
        match hydrate_order(raw) {
            Some(order) => output.push(dehydrate_order(process(order))),
            None => return ExsValue::None,
        }
    }
    ExsValue::List(output)
}
