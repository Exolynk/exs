//! Built-in value, collection, iteration, garbage-collection, and clone integration tests.

mod support;

use exs_abi::{ErrorSeverity, ExsValue};
use support::{execute_source, execute_source_with_inputs};

#[path = "runtime_values/cloning.rs"]
mod cloning;
#[path = "runtime_values/collections.rs"]
mod collections;
#[path = "runtime_values/control_flow.rs"]
mod control_flow;
#[path = "runtime_values/formatting.rs"]
mod formatting;
#[path = "runtime_values/garbage_collection.rs"]
mod garbage_collection;
#[path = "runtime_values/iteration.rs"]
mod iteration;
#[path = "runtime_values/primitives.rs"]
mod primitives;
