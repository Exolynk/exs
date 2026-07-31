//! Integration tests for the runner host-function registry.

use std::task::{Context, Poll, Waker};

use exs_runner::{
    ErrorSeverity, ExsError, ExsValue, HostCall, HostCborError, HostFunctionRegistry,
    RegistryError, decode_arguments, encode_result,
};

/// Polls an immediately ready host future without adding an executor dependency.
fn complete(mut future: exs_runner::HostFuture) -> ExsValue {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("test host future unexpectedly remained pending"),
    }
}

/// Decodes ordered CBOR arguments and encodes a host-function result.
#[test]
fn converts_host_payloads_through_the_exs_cbor_boundary() {
    let request = ExsValue::List(vec![ExsValue::Int(1), ExsValue::String("Ada".to_owned())]);
    let bytes = request.to_cbor().unwrap();
    assert_eq!(
        decode_arguments(&bytes),
        Ok(vec![ExsValue::Int(1), ExsValue::String("Ada".to_owned())])
    );

    let result = ExsValue::Bool(true);
    let encoded = encode_result(&result).unwrap();
    assert_eq!(ExsValue::from_cbor(&encoded), Ok(result));
}

/// Rejects host requests whose valid CBOR root is not an argument List.
#[test]
fn rejects_non_list_host_arguments() {
    let bytes = ExsValue::Int(1).to_cbor().unwrap();
    assert_eq!(
        decode_arguments(&bytes),
        Err(HostCborError::ArgumentsMustBeList)
    );
}

/// Preserves argument order and language Errors through a synchronous host implementation.
#[test]
fn starts_synchronous_host_functions() {
    let mut registry = HostFunctionRegistry::new();
    registry
        .register_sync("join", |arguments: Vec<ExsValue>| {
            assert_eq!(arguments, vec![ExsValue::String("Ada".to_owned())]);
            ExsValue::Error(ExsError {
                severity: ErrorSeverity::Recoverable,
                kind: "HostFailure".to_owned(),
                message: "not available".to_owned(),
                data: Box::new(ExsValue::None),
                origin: None,
                trace: Vec::new(),
                cause: None,
            })
        })
        .unwrap();

    let HostCall::Ready(ExsValue::Error(error)) = registry
        .start("join", vec![ExsValue::String("Ada".to_owned())])
        .unwrap()
    else {
        panic!("synchronous host call did not return its language Error");
    };
    assert_eq!(error.kind, "HostFailure");
}

/// Starts an asynchronous host function without polling it in the registration path.
#[test]
fn starts_asynchronous_host_functions() {
    let mut registry = HostFunctionRegistry::new();
    registry
        .register_async("increment", |arguments: Vec<ExsValue>| async move {
            let [ExsValue::Int(value)] = arguments.as_slice() else {
                return ExsValue::None;
            };
            ExsValue::Int(value + 1)
        })
        .unwrap();

    let HostCall::Pending(future) = registry
        .start("increment", vec![ExsValue::Int(41)])
        .unwrap()
    else {
        panic!("asynchronous host call completed during registration");
    };
    assert_eq!(complete(future), ExsValue::Int(42));
}

/// Rejects duplicate and missing static host names without replacing registered functions.
#[test]
fn rejects_duplicate_and_unknown_host_names() {
    let mut registry = HostFunctionRegistry::new();
    registry
        .register_sync("value", |_| ExsValue::Int(1))
        .unwrap();
    assert_eq!(
        registry.register_sync("value", |_| ExsValue::Int(2)),
        Err(RegistryError::DuplicateName("value".to_owned()))
    );
    assert!(matches!(
        registry.start("missing", Vec::new()),
        Err(RegistryError::UnknownName(name)) if name == "missing"
    ));

    let HostCall::Ready(value) = registry.start("value", Vec::new()).unwrap() else {
        panic!("registered synchronous host function unexpectedly became pending");
    };
    assert_eq!(value, ExsValue::Int(1));
}
