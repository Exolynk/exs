//! Serde conversion tests for host-safe ExS values.

use exs_abi::{Bytes, ExsValue};
use serde::{Deserialize, Serialize};

/// A representative host DTO containing each supported recursive wire shape.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct Vehicle {
    /// Stable vehicle identifier.
    id: String,
    /// Human-readable registration number.
    registration: String,
    /// Current vehicle lifecycle state.
    status: VehicleStatus,
    /// ISO-8601 maintenance records.
    visits: Vec<ServiceVisit>,
    /// Optional operator-provided note.
    note: Option<String>,
    /// Binary diagnostic attachment.
    attachment: Bytes,
}

/// A lifecycle state transferred as a tagged object.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
enum VehicleStatus {
    /// Vehicle may be scheduled.
    Available,
    /// Vehicle is unavailable until its given date.
    InService { until: String },
}

/// One completed maintenance visit.
#[derive(Debug, Deserialize, PartialEq, Serialize)]
struct ServiceVisit {
    /// ISO-8601 date.
    performed_on: String,
    /// Work completed during the visit.
    actions: Vec<String>,
}

/// Preserves structured DTOs, optional fields, generic lists, tagged enums, and byte buffers.
#[test]
fn round_trips_complex_dto_through_exs_values() {
    let vehicle = Vehicle {
        id: "vh_0042".into(),
        registration: "ZH 421 742".into(),
        status: VehicleStatus::InService {
            until: "2026-09-04".into(),
        },
        visits: vec![ServiceVisit {
            performed_on: "2026-08-20".into(),
            actions: vec!["replace brake pads".into(), "calibrate radar".into()],
        }],
        note: None,
        attachment: Bytes::new(vec![0, 1, 2, 255]),
    };

    let wire = ExsValue::from_serialize(&vehicle).unwrap();
    assert_eq!(
        wire,
        ExsValue::Object(vec![
            ("id".into(), ExsValue::String("vh_0042".into())),
            ("registration".into(), ExsValue::String("ZH 421 742".into()),),
            (
                "status".into(),
                ExsValue::Object(vec![
                    ("$variant".into(), ExsValue::String("InService".into())),
                    ("until".into(), ExsValue::String("2026-09-04".into())),
                ]),
            ),
            (
                "visits".into(),
                ExsValue::List(vec![ExsValue::Object(vec![
                    ("performed_on".into(), ExsValue::String("2026-08-20".into()),),
                    (
                        "actions".into(),
                        ExsValue::List(vec![
                            ExsValue::String("replace brake pads".into()),
                            ExsValue::String("calibrate radar".into()),
                        ]),
                    ),
                ])]),
            ),
            ("note".into(), ExsValue::None),
            ("attachment".into(), ExsValue::Bytes(vec![0, 1, 2, 255])),
        ])
    );
    assert_eq!(wire.into_deserialize::<Vehicle>().unwrap(), vehicle);
}
