//! Reads back the images the writer produces, for the boundary tests.

use exs_abi::{BoundaryLimits, ExsValue};

use exs_layout::{ObjectKind, PAGE_SIZE};

use super::execution::Execution;
use super::heap::VM;
use crate::image_writer::{WriteError, write_image};
use crate::vm::Vm;
use crate::{Memory, allocate};

/// Writes the image of `raw` of the VM at `vm` and returns the value the runner reads from it.
pub(super) fn written(memory: &mut impl Memory, vm: u32, raw: u64) -> Result<ExsValue, WriteError> {
    let (address, length) = write_image(memory, vm, raw)?;
    let bytes = memory.bytes(address, length);
    Ok(ExsValue::from_image_with_limits(bytes, BoundaryLimits::unrestricted()).expect("an image"))
}

/// Keeps the records already written when an image outgrows the output buffer, and reuses the
/// grown buffer for the next image.
#[test]
fn grows_the_output_buffer_and_reuses_it() {
    let mut execution = Execution::new();
    let items = (0..5_000).map(Execution::int).collect::<std::vec::Vec<_>>();
    let list = execution.list(&items);
    let expected = (0..5_000).map(ExsValue::Int).collect::<std::vec::Vec<_>>();
    assert_eq!(execution.encoded(list), ExsValue::List(expected));
    let (grown, _) = write_image(&mut execution.memory, VM, list).expect("memory has room");
    let small = execution.list(&[Execution::int(1)]);
    let (address, _) = write_image(&mut execution.memory, VM, small).expect("memory has room");
    assert_eq!(address, grown);
    assert_eq!(
        execution.encoded(small),
        ExsValue::List(std::vec![ExsValue::Int(1)])
    );
}

/// Collects garbage, keeping the value being written, when the heap leaves no room for an image.
#[test]
fn collects_garbage_when_an_image_does_not_fit() {
    /// The size the memory of an [`Execution`] may reach.
    const MEMORY: u64 = 256 * PAGE_SIZE as u64;
    /// Byte size of the large garbage objects.
    const LARGE: u32 = 1 << 20;
    /// Byte size of the small garbage objects, which fill what the large ones leave.
    const SMALL: u32 = PAGE_SIZE / 2;

    let mut execution = Execution::new();
    let text = "x".repeat(1 << 19);
    let value = execution.string(&text);
    let vm = Vm(VM);
    vm.set_collection_threshold(&mut execution.memory, u32::MAX);
    for (size, pages) in [(LARGE, 17), (SMALL, 1)] {
        while u64::from(vm.heap_end(&execution.memory)) + u64::from(pages * PAGE_SIZE) <= MEMORY {
            allocate(&mut execution.memory, VM, size, ObjectKind::String).expect("room");
        }
    }
    let collections = vm.collections(&execution.memory);
    assert_eq!(execution.encoded(value), ExsValue::String(text));
    assert_eq!(vm.collections(&execution.memory), collections + 1);
}

/// Writes the keys every Object of a shape shares once, as the runner's writer does.
#[test]
fn writes_shared_keys_once() {
    let mut execution = Execution::new();
    let record = |id| {
        ExsValue::Object(std::vec![
            ("id".into(), ExsValue::Int(id)),
            ("name".into(), ExsValue::String(std::format!("name {id}"))),
        ])
    };
    let value = ExsValue::List((0..3).map(record).collect());
    let decoded = execution.decoded(&value).expect("decodable");
    execution.root(&[decoded]);
    let (_, length) = write_image(&mut execution.memory, VM, decoded).expect("memory has room");
    let runner_image = value
        .to_image_with_limits(BoundaryLimits::unrestricted())
        .expect("encodable");
    assert_eq!(length as usize, runner_image.len());
    assert_eq!(execution.encoded(decoded), value);
}
