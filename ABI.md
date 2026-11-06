# ExS ABI

The ABI is the contract between a WebAssembly module and the runner that executes it: which functions each side provides, how a call proceeds, and how values cross between them. A module built for ABI 29 keeps running on every later runner, whichever language it comes from; see [COMPATIBILITY.md](COMPATIBILITY.md).

This document describes **ABI version 29**. A module that follows it can be written in any language that compiles to `wasm32`. In Rust, `exs-guest` implements it; the names and numbers below are defined in the `exs-abi` crate.

Everything else is private to a module, such as its heap, garbage collector, and value representation. A runner never reads a module's memory except at the addresses this contract gives it.

## Overview

All addresses and lengths are 32-bit. All numbers are little-endian.

```text
Runner                                       Module
__exs_abi_version()                    ->    the version
__exs_input_alloc(length)              ->    an address
writes the input image there
__exs_start_<function>(address, length) ->   runs
                                       <-    __exs_host_call_start(...)
returns READY, the response is copied  ->    keeps running
returns PENDING                        ->    returns STATUS_PENDING
... waits for the host function ...
__exs_input_alloc(length), writes the response image there
__exs_resume_host(id, address, length) ->    runs on
                                             returns STATUS_COMPLETE
__exs_result_image()                   ->    address and length of the result image
```

## What a module exports

| Export | Signature | Purpose |
| --- | --- | --- |
| `memory` | memory | The linear memory. |
| `__exs_abi_version` | `() -> i32` | The ABI version of the module: 29. A runner calls it first. |
| `__exs_input_alloc` | `(length: i32) -> i32` | Reserves `length` bytes and returns their address, where the runner writes an image. |
| `__exs_start_<function>` | `(address: i32, length: i32) -> i32` | Starts the entry point `<function>` with the input image and returns a status. |
| `__exs_resume_host` | `(call_id: i64, address: i32, length: i32) -> i32` | Continues after the host call `call_id` completed with the response image, and returns a status. |
| `__exs_result_image` | `() -> i64` | After `STATUS_COMPLETE`: where the result image lies, with the address in the low 32 bits and the length in the high 32 bits. |
| `__exs_cancel` | `() -> ()` | Cancels a suspended execution and releases what it holds. |

The input image holds a List with one element per input value.

Status values:

| Name | Value | Meaning |
| --- | --- | --- |
| `STATUS_PENDING` | 1 | Suspended until the runner resumes a host call that returned `HOST_CALL_PENDING`. |
| `STATUS_COMPLETE` | 2 | Finished, and the result image is available. A language Error is a completed result. |
| `STATUS_CANCELLED` | 3 | The execution was cancelled. |

A trap ends the execution with a technical failure in the runner, for example on running out of memory or fuel.

## What a runner provides

A module imports these. A runner refuses a module that does not import both functions of module `runner`.

| Module | Import | Signature | Purpose |
| --- | --- | --- | --- |
| `exs` | `__exs_host_call_start` | `(call_id: i64, name: i32, name_length: i32, request: i32, request_length: i32, source_position: i32) -> i32` | Starts one host call. |
| `exs` | `__exs_host_call_response_len` | `(call_id: i64) -> i32` | The byte length of a ready response. |
| `exs` | `__exs_host_call_response_copy` | `(call_id: i64, address: i32, length: i32) -> i32` | Copies a ready response to `address`, and returns 0 on success. |
| `runner` | `__runner_task_acquire` | `() -> i32` | Acquires a permit for one language task, and returns 0 on success. |
| `runner` | `__runner_task_release` | `() -> i32` | Releases a permit, and returns 0 on success. |

### Host calls

A module has one generic way to call the host: `name` is the UTF-8 name of the operation, and `request` is an image holding a List of its arguments. Host functions are not separate Wasm imports. So a new host function needs no ABI change, and a module calling a function the host lacks still starts, and receives a recoverable `HostFunctionNotFound` Error as the response.

The module chooses `call_id`, which must be unique among its active calls. `source_position` is the position the runner reports for an Error it creates, or -1 for none. The result says how the call continues:

| Name | Value | Meaning |
| --- | --- | --- |
| `HOST_CALL_READY` | 0 | The response is ready. Read it with `__exs_host_call_response_len` and `__exs_host_call_response_copy`. |
| `HOST_CALL_PENDING` | 1 | The module keeps running other tasks, or returns `STATUS_PENDING`. The runner delivers the response through `__exs_resume_host`. |
| `HOST_CALL_FATAL` | 2 | The call failed at the technical layer of the runner. |

A failing host function is not a technical failure: its response is an ordinary image of an Error.

### Built-in operations

Every runner implements these operations under reserved host names. They use `Duration` with Int fields `seconds` and `nanoseconds` (`0..1_000_000_000`), and `DateTime` with Int fields `unix_seconds`, `nanoseconds`, and `utc_offset_seconds`, and `timezone` as a String or None. An argument of these types may be an Object or the `Typed` value of the standard type, and a response is an Object. An invalid request or a failure is a response holding an Error.

| Name | Arguments | Response |
| --- | --- | --- |
| `__exs.host.sleep` | a `Duration` | Unit |
| `__exs.host.now` | none | a `DateTime` |
| `__exs.host.elapsed` | none | a `Duration`: the time since the execution began |
| `__exs.host.datetime_in_timezone` | a `DateTime`, a time zone String | a `DateTime` |
| `__exs.host.datetime_from_components` | year, month, day, hour, minute, second, and nanosecond as Ints, a time zone String | a `DateTime` |
| `__exs.host.stream_open` | a stream name String, then the arguments of the stream | an Int handle |
| `__exs.host.stream_next` | an Int handle | an `IteratorStep` enum: variant `Item` with one field, or `Done` |

Values of standard types carry these identities: `<std>/iterator.exs::IteratorStep`, `<std>/datetime.exs::DateTime`, and `<std>/duration.exs::Duration`.

## Images

Every value crosses as an image: a by-value, acyclic sequence of records. A value is copied, so a value referenced twice crosses as two copies, and a cyclic value cannot cross. The writer of an image controls its layout, but a reader checks everything, because an image may come from an untrusted module.

An image starts with an 8-byte header: the magic number `EXSI` in ASCII as a `u32`, then the `u32` offset of the root record. Offsets are relative to the image start. Records are aligned to 8 bytes, and their payloads are padded to 8 bytes. Records may appear in any order and may be shared by several parents.

Each record starts with a `u32` kind and a `u32` argument:

| Kind | Record | Argument | Payload |
| --- | --- | --- | --- |
| 0 | None | 0 | |
| 11 | Unit | 0 | |
| 1 | Bool | 0 or 1 | |
| 2 | Int | 0 | `i64` |
| 3 | Float | 0 | `f64` bits |
| 4 | String | byte length | `u32` character count, `u32` 0, the UTF-8 bytes |
| 5 | Bytes | length | the octets |
| 6 | List | element count | one `u32` offset per element |
| 7 | Object | entry count | per entry, a `u32` key offset (a String record) and a `u32` value offset |
| 10 | Typed | field count | `u32` identity offset (a String record), then per field a `u32` name offset (a String record) and a `u32` value offset |
| 8 | Enum | field count | `u32` identity offset and `u32` variant offset (String records), then one `u32` offset per field |
| 9 | Error | 1 if fatal, else 0 | `u32` offsets of kind, message, and data, `u32` origin + 1 or 0, `u32` cause offset or 0, `u32` trace count, then per trace entry a `u64` with the function id in the low 32 bits and the call site in the high 32 bits |

A String must be valid UTF-8 with a correct character count, because a compiled module copies Strings without validating them again. An image that breaks any rule, or holds an unknown record kind, is rejected.

### Types

A `Typed` value belongs to a nominal type, and an `Enum` value to an enum. Their identity is the source id or library path followed by the type name, such as `app::dom::Div`. A compiled module turns a `Typed` value into a value of its type when the program uses the type, taking declared fields by name. For an unknown identity, it treats the value as an Object that keeps its identity, so it reaches the host as that type again. An unknown type therefore needs no ABI change.

### Limits

A runner bounds each value with its `BoundaryLimits`: by default 2 MiB of payload, 64 levels of nesting, and 65,536 entries per collection. These are the runner's configuration and not part of the format. A compiled module adopts at most 128 levels of nesting. An image is at most 4 GiB because its offsets are 32-bit.

## Custom sections

A compiled ExS module also holds the custom section `exs.meta`, and optionally debug information. Runners do not need them to execute a module, and a module from another language can omit them.

## Changing the ABI

See the [rules for changing ExS](COMPATIBILITY.md#rules-for-changing-exs). In short, a change that an existing module could notice needs a new ABI version, and a runner keeps executing every version from `MIN_SUPPORTED_ABI_VERSION` on.
