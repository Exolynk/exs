//! A visitor that builds the runner's value from a walk, for the boundary tests.

use std::boxed::Box;
use std::string::String;
use std::vec::Vec;

use exs_abi::cbor::{ExsStackFrame, SourcePositionId};
use exs_abi::{ErrorSeverity, ExsError, ExsValue};

use crate::{Memory, Visitor, WalkError, walk_value};

/// Walks `raw` of the VM at `vm` and returns the value it reports.
pub(super) fn walked<M: Memory>(memory: &M, vm: u32, raw: u64) -> Result<ExsValue, WalkError<()>> {
    let mut builder = Builder::default();
    walk_value(memory, vm, raw, &mut builder)?;
    builder.value.ok_or(WalkError::Stopped(()))
}

/// One composite value whose parts are still arriving.
enum Frame {
    /// A List of the given item count.
    List(usize, Vec<ExsValue>),
    /// An Object of the given part count, keys alternating with values.
    Object(usize, Vec<ExsValue>),
    /// An enum of the given part count: identity, variant, then fields.
    Enum(usize, Vec<ExsValue>),
    /// An Error with its severity, origin and trace, and its kind, message, data and cause.
    Error(
        bool,
        Option<SourcePositionId>,
        Vec<ExsStackFrame>,
        Vec<ExsValue>,
    ),
}

/// Builds the walked value from its parts.
#[derive(Default)]
struct Builder {
    /// The composites being built, innermost last.
    frames: Vec<Frame>,
    /// The finished value.
    value: Option<ExsValue>,
}

impl Builder {
    /// Adds the finished `value` to the innermost composite, finishing every composite that
    /// it completes.
    fn complete(&mut self, mut value: ExsValue) -> Result<(), ()> {
        loop {
            let Some(frame) = self.frames.last_mut() else {
                self.value = Some(value);
                return Ok(());
            };
            let done = match frame {
                Frame::List(count, parts)
                | Frame::Object(count, parts)
                | Frame::Enum(count, parts) => {
                    parts.push(value);
                    parts.len() == *count
                }
                Frame::Error(_, _, _, parts) => {
                    parts.push(value);
                    parts.len() == 4
                }
            };
            if !done {
                return Ok(());
            }
            value = match self.frames.pop().ok_or(())? {
                Frame::List(_, items) => ExsValue::List(items),
                Frame::Object(_, parts) => {
                    let mut entries = Vec::new();
                    let mut parts = parts.into_iter();
                    while let (Some(ExsValue::String(key)), Some(value)) =
                        (parts.next(), parts.next())
                    {
                        entries.push((key, value));
                    }
                    ExsValue::Object(entries)
                }
                Frame::Enum(_, parts) => {
                    let mut parts = parts.into_iter();
                    let (Some(ExsValue::String(type_id)), Some(ExsValue::String(variant))) =
                        (parts.next(), parts.next())
                    else {
                        return Err(());
                    };
                    ExsValue::Enum {
                        type_id,
                        variant,
                        fields: parts.collect(),
                    }
                }
                Frame::Error(fatal, origin, trace, parts) => {
                    let mut parts = parts.into_iter();
                    let (
                        Some(ExsValue::String(kind)),
                        Some(ExsValue::String(message)),
                        Some(data),
                        Some(cause),
                    ) = (parts.next(), parts.next(), parts.next(), parts.next())
                    else {
                        return Err(());
                    };
                    ExsValue::Error(ExsError {
                        severity: if fatal {
                            ErrorSeverity::Fatal
                        } else {
                            ErrorSeverity::Recoverable
                        },
                        kind,
                        message,
                        data: Box::new(data),
                        origin,
                        trace,
                        cause: match cause {
                            ExsValue::None => None,
                            cause => Some(Box::new(cause)),
                        },
                    })
                }
            };
        }
    }

    /// Starts `frame`, finishing it at once when it has no parts.
    fn start(&mut self, frame: Frame) -> Result<(), ()> {
        let empty = matches!(&frame, Frame::List(0, _) | Frame::Object(0, _));
        self.frames.push(frame);
        if empty {
            return match self.frames.pop().ok_or(())? {
                Frame::Object(..) => self.complete(ExsValue::Object(Vec::new())),
                _ => self.complete(ExsValue::List(Vec::new())),
            };
        }
        Ok(())
    }
}

/// Returns the `len` bytes at `address`.
fn copy<M: Memory>(memory: &M, address: u32, len: u32) -> Vec<u8> {
    (0..len)
        .map(|offset| memory.load_u8(address + offset))
        .collect()
}

impl<M: Memory> Visitor<M> for Builder {
    type Error = ();

    fn none(&mut self) -> Result<(), ()> {
        self.complete(ExsValue::None)
    }

    fn boolean(&mut self, value: bool) -> Result<(), ()> {
        self.complete(ExsValue::Bool(value))
    }

    fn int(&mut self, value: i64) -> Result<(), ()> {
        self.complete(ExsValue::Int(value))
    }

    fn float(&mut self, value: f64) -> Result<(), ()> {
        self.complete(ExsValue::Float(value))
    }

    fn string(&mut self, memory: &M, address: u32, len: u32) -> Result<(), ()> {
        let text = String::from_utf8(copy(memory, address, len)).map_err(|_| ())?;
        self.complete(ExsValue::String(text))
    }

    fn bytes(&mut self, memory: &M, address: u32, len: u32) -> Result<(), ()> {
        self.complete(ExsValue::Bytes(copy(memory, address, len)))
    }

    fn list(&mut self, count: u32) -> Result<(), ()> {
        self.start(Frame::List(count as usize, Vec::new()))
    }

    fn object(&mut self, count: u32) -> Result<(), ()> {
        self.start(Frame::Object(count as usize * 2, Vec::new()))
    }

    fn enumeration(&mut self, count: u32) -> Result<(), ()> {
        self.start(Frame::Enum(count as usize + 2, Vec::new()))
    }

    fn error(
        &mut self,
        memory: &M,
        fatal: bool,
        origin: u32,
        trace: u32,
        count: u32,
    ) -> Result<(), ()> {
        let trace = (0..count)
            .map(|index| {
                let entry = memory.load_u64(trace + index * 8);
                ExsStackFrame {
                    function_id: entry as u32,
                    call_site: SourcePositionId((entry >> 32) as u32),
                }
            })
            .collect();
        let origin = (origin != 0).then_some(SourcePositionId(origin));
        self.start(Frame::Error(fatal, origin, trace, Vec::new()))
    }

    fn no_cause(&mut self) -> Result<(), ()> {
        self.complete(ExsValue::None)
    }
}
