//! Building the runner's values from the memory of compiled modules.
//!
//! The core library walks a value in the module's memory and reports its parts; this visitor
//! builds the [`ExsValue`]. It keeps the boundary limits: a payload budget stops the walk at
//! once, while nesting, collection and payload limits apply once the walk has shown that the
//! value can cross the boundary at all.

use exs_abi::value::{ExsStackFrame, SourcePositionId};
use exs_abi::{BoundaryLimits, ErrorSeverity, ExsError, ExsValue};
use exs_core::{Memory, Visitor, WalkError, walk_value};

/// Size of the pages within which [`Builder::copy`] borrows memory.
const PAGE_SIZE: u32 = 65_536;
/// Tag number of an Error in the compact encoding the payload size measures.
const ERROR_TAG: u64 = 60_001;
/// Tag number of an enum in the compact encoding the payload size measures.
const ENUM_TAG: u64 = 60_005;

/// Why a value could not be read.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum ReadError {
    /// The value cannot cross the boundary, with the message of v1's SerializationError.
    Unsupported(&'static str),
    /// The value exceeds the byte budget.
    TooLarge,
    /// The value exceeds the payload limit.
    PayloadLimitExceeded,
    /// The value nests deeper than the nesting limit.
    NestingLimitExceeded,
    /// A collection has more entries than the collection limit.
    CollectionLimitExceeded,
    /// The module's memory does not hold a well-formed value.
    Malformed,
}

/// Reads the value `raw` of the memory `memory`, whose VM state block is at `vm`: the walk
/// stops once the payload size exceeds `budget` bytes, and `limits` apply afterwards.
pub(crate) fn read_value<M: Memory>(
    memory: &M,
    vm: u32,
    raw: u64,
    budget: usize,
    limits: BoundaryLimits,
) -> Result<ExsValue, ReadError> {
    let mut builder = Builder {
        stack: Vec::new(),
        root: None,
        size: 0,
        budget,
        limits,
        violation: None,
    };
    match walk_value(memory, vm, raw, &mut builder) {
        Ok(()) => {}
        Err(WalkError::Unsupported) => {
            return Err(ReadError::Unsupported(exs_core::serialization_message(
                memory, raw,
            )));
        }
        Err(WalkError::Stopped(error)) => return Err(error),
    }
    if builder.size > limits.max_payload_bytes {
        return Err(ReadError::PayloadLimitExceeded);
    }
    if let Some(violation) = builder.violation {
        return Err(violation);
    }
    builder.root.ok_or(ReadError::Malformed)
}

/// Returns the byte length of a head with `argument` in the compact encoding.
const fn head_len(argument: u64) -> usize {
    match argument {
        0..=23 => 1,
        24..=0xff => 2,
        0x100..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// A container whose parts are still arriving.
enum Frame {
    /// A List of the remaining number of items.
    List(usize, Vec<ExsValue>),
    /// An Object of the remaining number of parts, keys and values alternating.
    Object(usize, Vec<ExsValue>),
    /// An enum of the remaining number of parts: identity, variant name, payload fields.
    Enum(usize, Vec<ExsValue>),
    /// An Error with its severity, origin and trace, and its arriving parts: kind, message,
    /// data and cause.
    Error {
        /// Whether it is fatal.
        fatal: bool,
        /// Its origin.
        origin: Option<SourcePositionId>,
        /// Its trace.
        trace: Vec<ExsStackFrame>,
        /// Its parts so far.
        parts: Vec<ExsValue>,
    },
}

impl Frame {
    /// Returns whether the next part is a name, not a value: an Object key, an enum's
    /// identity or variant name, or an Error's kind or message.
    fn expects_name(&self) -> bool {
        match self {
            Self::List(..) => false,
            Self::Object(_, parts) => parts.len().is_multiple_of(2),
            Self::Enum(_, parts) => parts.len() < 2,
            Self::Error { parts, .. } => parts.len() < 2,
        }
    }

    /// Returns whether every part arrived.
    fn complete(&self) -> bool {
        match self {
            Self::List(remaining, _) | Self::Object(remaining, _) | Self::Enum(remaining, _) => {
                *remaining == 0
            }
            Self::Error { parts, .. } => parts.len() == 4,
        }
    }

    /// Adds one part.
    fn push(&mut self, value: ExsValue) {
        match self {
            Self::List(remaining, parts)
            | Self::Object(remaining, parts)
            | Self::Enum(remaining, parts) => {
                *remaining = remaining.saturating_sub(1);
                parts.push(value);
            }
            Self::Error { parts, .. } => parts.push(value),
        }
    }

    /// Builds the value of the complete container.
    fn finish(self) -> Result<ExsValue, ReadError> {
        let text = |value: ExsValue| match value {
            ExsValue::String(text) => Ok(text),
            _ => Err(ReadError::Malformed),
        };
        Ok(match self {
            Self::List(_, items) => ExsValue::List(items),
            Self::Object(_, parts) => {
                let mut entries = Vec::with_capacity(parts.len() / 2);
                let mut parts = parts.into_iter();
                while let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                    entries.push((text(key)?, value));
                }
                ExsValue::Object(entries)
            }
            Self::Enum(_, parts) => {
                let mut parts = parts.into_iter();
                let type_id = text(parts.next().ok_or(ReadError::Malformed)?)?;
                let variant = text(parts.next().ok_or(ReadError::Malformed)?)?;
                ExsValue::Enum {
                    type_id,
                    variant,
                    fields: parts.collect(),
                }
            }
            Self::Error {
                fatal,
                origin,
                trace,
                parts,
            } => {
                let mut parts = parts.into_iter();
                let kind = text(parts.next().ok_or(ReadError::Malformed)?)?;
                let message = text(parts.next().ok_or(ReadError::Malformed)?)?;
                let data = parts.next().ok_or(ReadError::Malformed)?;
                // A None cause is no cause.
                let cause = match parts.next().ok_or(ReadError::Malformed)? {
                    ExsValue::None => None,
                    cause => Some(Box::new(cause)),
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
                    cause,
                })
            }
        })
    }
}

/// The visitor building one value.
struct Builder {
    /// The containers whose parts are still arriving, innermost last.
    stack: Vec<Frame>,
    /// The completed root value.
    root: Option<ExsValue>,
    /// Payload size so far.
    size: usize,
    /// The largest payload size before the walk stops.
    budget: usize,
    /// The limits that apply after the walk.
    limits: BoundaryLimits,
    /// The first nesting or collection limit the value exceeds.
    violation: Option<ReadError>,
}

impl Builder {
    /// Accounts for `count` bytes of payload.
    fn grow(&mut self, count: usize) -> Result<(), ReadError> {
        self.size = self.size.saturating_add(count);
        if self.size > self.budget {
            return Err(ReadError::TooLarge);
        }
        Ok(())
    }

    /// Records `violation` unless an earlier one was recorded.
    fn violate(&mut self, violation: ReadError) {
        self.violation.get_or_insert(violation);
    }

    /// Checks the nesting of a value about to start, unless it is a name.
    fn enter_value(&mut self) {
        let is_name = self.stack.last().is_some_and(Frame::expects_name);
        if !is_name && self.stack.len() + 1 > self.limits.max_nesting {
            self.violate(ReadError::NestingLimitExceeded);
        }
    }

    /// Checks the entry count of a collection.
    fn collection(&mut self, count: u32) {
        if count as usize > self.limits.max_collection_entries {
            self.violate(ReadError::CollectionLimitExceeded);
        }
    }

    /// Adds a complete value to its container, completing containers in turn.
    fn complete(&mut self, value: ExsValue) -> Result<(), ReadError> {
        let mut value = value;
        loop {
            let Some(frame) = self.stack.last_mut() else {
                self.root = Some(value);
                return Ok(());
            };
            frame.push(value);
            if !frame.complete() {
                return Ok(());
            }
            let frame = self.stack.pop().ok_or(ReadError::Malformed)?;
            value = frame.finish()?;
        }
    }

    /// Starts `frame`, which may already be complete.
    fn start(&mut self, frame: Frame) -> Result<(), ReadError> {
        if frame.complete() {
            return self.complete(frame.finish()?);
        }
        self.stack.push(frame);
        Ok(())
    }

    /// Copies the `len` bytes at `address` in chunks that stay inside one 64 KiB page, so a
    /// memory that borrows only within pages still copies whole chunks.
    fn copy<M: Memory>(memory: &M, address: u32, len: u32) -> Vec<u8> {
        let mut copied = Vec::with_capacity(len as usize);
        let mut address = address;
        let mut left = len;
        while left > 0 {
            let chunk = left.min(PAGE_SIZE - address % PAGE_SIZE);
            let bytes = memory.bytes(address, chunk);
            if bytes.len() == chunk as usize {
                copied.extend_from_slice(bytes);
            } else {
                copied
                    .extend((0..chunk).map(|offset| memory.load_u8(address.wrapping_add(offset))));
            }
            address = address.wrapping_add(chunk);
            left -= chunk;
        }
        copied
    }
}

impl<M: Memory> Visitor<M> for Builder {
    type Error = ReadError;

    fn none(&mut self) -> Result<(), ReadError> {
        self.enter_value();
        self.grow(1)?;
        self.complete(ExsValue::None)
    }

    fn boolean(&mut self, value: bool) -> Result<(), ReadError> {
        self.enter_value();
        self.grow(1)?;
        self.complete(ExsValue::Bool(value))
    }

    fn int(&mut self, value: i64) -> Result<(), ReadError> {
        self.enter_value();
        let magnitude = if value < 0 {
            (-1 - value).cast_unsigned()
        } else {
            value.cast_unsigned()
        };
        self.grow(head_len(magnitude))?;
        self.complete(ExsValue::Int(value))
    }

    fn float(&mut self, value: f64) -> Result<(), ReadError> {
        self.enter_value();
        self.grow(9)?;
        self.complete(ExsValue::Float(value))
    }

    fn string(&mut self, memory: &M, address: u32, len: u32) -> Result<(), ReadError> {
        self.enter_value();
        self.grow(head_len(u64::from(len)) + len as usize)?;
        let text = String::from_utf8(Self::copy(memory, address, len))
            .map_err(|_| ReadError::Malformed)?;
        self.complete(ExsValue::String(text))
    }

    fn bytes(&mut self, memory: &M, address: u32, len: u32) -> Result<(), ReadError> {
        self.enter_value();
        self.grow(head_len(u64::from(len)) + len as usize)?;
        self.complete(ExsValue::Bytes(Self::copy(memory, address, len)))
    }

    fn list(&mut self, count: u32) -> Result<(), ReadError> {
        self.enter_value();
        self.collection(count);
        self.grow(head_len(u64::from(count)))?;
        self.start(Frame::List(count as usize, Vec::new()))
    }

    fn object(&mut self, count: u32) -> Result<(), ReadError> {
        self.enter_value();
        self.collection(count);
        self.grow(head_len(u64::from(count)))?;
        let parts = (count as usize).saturating_mul(2);
        self.start(Frame::Object(parts, Vec::new()))
    }

    fn enumeration(&mut self, count: u32) -> Result<(), ReadError> {
        self.enter_value();
        self.collection(count);
        self.grow(head_len(ENUM_TAG) + 1 + head_len(u64::from(count)))?;
        let parts = (count as usize).saturating_add(2);
        self.start(Frame::Enum(parts, Vec::new()))
    }

    fn error(
        &mut self,
        memory: &M,
        fatal: bool,
        origin: u32,
        trace: u32,
        count: u32,
    ) -> Result<(), ReadError> {
        self.enter_value();
        self.collection(count);
        self.grow(head_len(ERROR_TAG) + 1 + 7 + 1 + head_len(u64::from(count)))?;
        let origin = if origin == 0 {
            self.grow(1)?;
            None
        } else {
            self.grow(head_len(u64::from(origin)))?;
            Some(SourcePositionId(origin))
        };
        let mut frames = Vec::new();
        for index in 0..count {
            let entry = memory.load_u64(trace.wrapping_add(index.wrapping_mul(8)));
            let frame = ExsStackFrame {
                function_id: entry as u32,
                call_site: SourcePositionId((entry >> 32) as u32),
            };
            self.grow(
                1 + head_len(u64::from(frame.function_id)) + head_len(u64::from(frame.call_site.0)),
            )?;
            frames.push(frame);
        }
        self.start(Frame::Error {
            fatal,
            origin,
            trace: frames,
            parts: Vec::new(),
        })
    }

    fn no_cause(&mut self) -> Result<(), ReadError> {
        self.grow(1)?;
        self.complete(ExsValue::None)
    }
}

/// Returns v1's SerializationError with `message`: fatal for a result, recoverable for
/// host-call arguments.
pub(crate) fn serialization_error(message: &str, fatal: bool) -> ExsError {
    ExsError {
        severity: if fatal {
            ErrorSeverity::Fatal
        } else {
            ErrorSeverity::Recoverable
        },
        kind: "SerializationError".to_owned(),
        message: message.to_owned(),
        data: Box::new(ExsValue::None),
        origin: None,
        trace: Vec::new(),
        cause: None,
    }
}
