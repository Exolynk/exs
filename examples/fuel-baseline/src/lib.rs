//! Rust fuel baseline for the ExS fuel scenarios.
//!
//! Every export performs the same work as the ExS scenario of the same name in
//! `crates/exs-runner/tests/fuel/scenarios.rs` and returns the same result, so the comparison
//! binary can measure both through the identical runner ABI. Loops route their state through
//! `black_box` so the optimizer keeps the per-iteration work instead of folding it away.

mod basic;
mod datetime;
mod records;

use basic::{
    async_call_loop_100, async_empty_while_1000, async_int_loop_100, async_int_while_1000,
    concat_10_objects, concat_100_objects, empty_main, for_range_1000, for_range_variable_1000,
    format_loop_1000, int_loop_1000, list_churn_6000, show_compare_record, show_true,
    sync_call_loop_1000,
};
use datetime::datetime_roundtrip_100;
use records::record_roundtrip_20;

exs_guest::export!(
    empty_main,
    show_true,
    show_compare_record,
    concat_10_objects,
    concat_100_objects,
    int_loop_1000,
    async_int_loop_100,
    async_empty_while_1000,
    async_int_while_1000,
    for_range_1000,
    for_range_variable_1000,
    async_call_loop_100,
    format_loop_1000,
    list_churn_6000,
    sync_call_loop_1000,
    datetime_roundtrip_100,
    record_roundtrip_20,
);
