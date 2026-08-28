//! Typed Serde host-registry integration tests.

#![cfg(feature = "serde")]

use std::task::{Context, Poll, Waker};

use exs_runner::{ExsError, ExsValue, HostCall, HostFunctionRegistry, HostStreamItem};
use serde::{Deserialize, Serialize};

/// A small application response used to verify recursive typed registration.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct ServiceVisit {
    /// ISO-8601 visit date.
    performed_on: String,
    /// Completed work descriptions.
    actions: Vec<String>,
}

/// Polls one ready stream advance without adding an executor dependency.
fn next(mut future: exs_runner::HostStreamFuture) -> HostStreamItem {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(item) => item,
        Poll::Pending => panic!("typed iterator stream unexpectedly remained pending"),
    }
}

/// Adapts typed synchronous handlers, asynchronous handlers, and iterator streams at the boundary.
#[test]
fn serializes_registered_typed_handlers_automatically() {
    let visit = ServiceVisit {
        performed_on: "2026-08-20".into(),
        actions: vec!["calibrate radar".into()],
    };
    let mut registry = HostFunctionRegistry::new();
    registry
        .fn_sync("visit", {
            let visit = visit.clone();
            move |(): ()| -> Result<ServiceVisit, ExsError> { Ok(visit.clone()) }
        })
        .unwrap();
    registry
        .fn_async("visit_async", {
            let visit = visit.clone();
            move |(): ()| {
                let visit = visit.clone();
                async move { Ok::<ServiceVisit, ExsError>(visit) }
            }
        })
        .unwrap();
    registry
        .stream("visits", {
            let visit = visit.clone();
            move |(): ()| -> Result<Vec<ServiceVisit>, ExsError> { Ok(vec![visit.clone()]) }
        })
        .unwrap();

    let expected = ExsValue::Object(vec![
        ("performed_on".into(), ExsValue::String("2026-08-20".into())),
        (
            "actions".into(),
            ExsValue::List(vec![ExsValue::String("calibrate radar".into())]),
        ),
    ]);
    let HostCall::Ready(value) = registry.start("visit", Vec::new()).unwrap() else {
        panic!("typed synchronous handler unexpectedly suspended");
    };
    assert_eq!(value, expected);
    let HostCall::Pending(mut future) = registry.start("visit_async", Vec::new()).unwrap() else {
        panic!("typed asynchronous handler unexpectedly completed synchronously");
    };
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    assert_eq!(
        future.as_mut().poll(&mut context),
        Poll::Ready(expected.clone())
    );
    let mut stream = registry.open_stream("visits", Vec::new()).unwrap();
    assert!(matches!(next(stream.next()), HostStreamItem::Item(value) if value == expected));
    assert!(matches!(next(stream.next()), HostStreamItem::End));
}
