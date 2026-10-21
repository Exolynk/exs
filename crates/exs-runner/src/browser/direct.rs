//! The direct boundary of runtime v2 modules in the browser.
//!
//! JavaScript owns the module's memory, so the runner reads a value through a
//! [`ValueReader`]: the walk fetches the 64 KiB pages it touches on demand and keeps them for
//! the rest of the read. Inputs and host-call responses cross as images, like on the server.

use std::cell::OnceCell;

use exs_abi::{CborLimits, ExsValue};
use exs_core::Memory;

use super::bindings::ValueReader;
use crate::value_reader::{ReadError, read_value};

/// Size of one page JavaScript copies out of the module's memory.
const PAGE_SIZE: u32 = 65_536;

/// Reads the value `reader` points at within the default limits, like the browser's CBOR
/// decoding.
pub(super) fn read(reader: &ValueReader) -> Result<ExsValue, ReadError> {
    let limits = CborLimits::default();
    let memory = PagedMemory::new(reader);
    let raw = u64::from(reader.high()) << 32 | u64::from(reader.low());
    read_value(&memory, reader.vm(), raw, limits.max_payload_bytes, limits)
}

/// Encodes one value the runner passes to a v2 module as an image.
pub(super) fn image(value: &ExsValue) -> Result<Vec<u8>, String> {
    value
        .to_image_with_limits(CborLimits::unrestricted())
        .map_err(|error| format!("could not encode image: {error}"))
}

/// A read-only view of a module's memory whose pages are fetched from JavaScript on first use.
///
/// Like `SliceMemory`, reads outside the memory return 0 and writes are ignored, so a corrupted
/// heap cannot make the walk touch anything else.
struct PagedMemory<'r> {
    /// The JavaScript view of the module's memory.
    reader: &'r ValueReader,
    /// The fetched pages, each fetched at most once.
    pages: Vec<OnceCell<Box<[u8]>>>,
    /// Byte size of the memory.
    size: u64,
}

impl<'r> PagedMemory<'r> {
    /// Returns a view of the memory behind `reader`.
    fn new(reader: &'r ValueReader) -> Self {
        let size = reader.size();
        let size = if size.is_finite() && size > 0.0 {
            (size as u64).min(1 << 32)
        } else {
            0
        };
        let pages = size.div_ceil(u64::from(PAGE_SIZE)) as usize;
        Self {
            reader,
            pages: (0..pages).map(|_| OnceCell::new()).collect(),
            size,
        }
    }

    /// Returns page `index`, empty when it lies outside the memory or cannot be fetched.
    fn page(&self, index: u32) -> &[u8] {
        self.pages.get(index as usize).map_or(&[], |cell| {
            cell.get_or_init(|| {
                self.reader
                    .page(index)
                    .map(|page| page.to_vec().into_boxed_slice())
                    .unwrap_or_default()
            })
        })
    }

    /// Returns the `N` bytes at `address`, with zeros for bytes outside the memory.
    fn array<const N: usize>(&self, address: u32) -> [u8; N] {
        let offset = (address % PAGE_SIZE) as usize;
        if let Some(bytes) = self
            .page(address / PAGE_SIZE)
            .get(offset..offset + N)
            .and_then(|bytes| bytes.try_into().ok())
        {
            return bytes;
        }
        let mut bytes = [0; N];
        for (index, byte) in (0_u32..).zip(&mut bytes) {
            *byte = address
                .checked_add(index)
                .map_or(0, |address| self.load_u8(address));
        }
        bytes
    }
}

impl Memory for PagedMemory<'_> {
    fn load_u8(&self, address: u32) -> u8 {
        self.page(address / PAGE_SIZE)
            .get((address % PAGE_SIZE) as usize)
            .copied()
            .unwrap_or(0)
    }

    fn load_u16(&self, address: u32) -> u16 {
        u16::from_le_bytes(self.array(address))
    }

    fn load_u32(&self, address: u32) -> u32 {
        u32::from_le_bytes(self.array(address))
    }

    fn load_u64(&self, address: u32) -> u64 {
        u64::from_le_bytes(self.array(address))
    }

    fn store_u8(&mut self, _address: u32, _value: u8) {}

    fn store_u16(&mut self, _address: u32, _value: u16) {}

    fn store_u32(&mut self, _address: u32, _value: u32) {}

    fn store_u64(&mut self, _address: u32, _value: u64) {}

    fn fill(&mut self, _address: u32, _length: u32, _value: u8) {}

    fn copy(&mut self, _destination: u32, _source: u32, _length: u32) {}

    fn size(&self) -> u64 {
        self.size
    }

    fn grow(&mut self, _pages: u32) -> bool {
        false
    }

    /// Borrows ranges inside one page; other ranges are empty, and readers fall back to
    /// byte-wise loads.
    fn bytes(&self, address: u32, length: u32) -> &[u8] {
        let offset = (address % PAGE_SIZE) as usize;
        self.page(address / PAGE_SIZE)
            .get(offset..offset.saturating_add(length as usize))
            .unwrap_or(&[])
    }
}
