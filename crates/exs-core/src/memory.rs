//! Access to the linear memory holding all heap objects and VM state.

/// Byte-addressed little-endian access to one linear memory.
///
/// Addresses are `u32` offsets. Implementations may panic or trap on out-of-bounds access;
/// the allocator and collector only touch memory they carved from [`Memory::size`].
pub trait Memory {
    /// Reads one byte.
    fn load_u8(&self, address: u32) -> u8;
    /// Reads one little-endian `u16`.
    fn load_u16(&self, address: u32) -> u16;
    /// Reads one little-endian `u32`.
    fn load_u32(&self, address: u32) -> u32;
    /// Reads one little-endian `u64`.
    fn load_u64(&self, address: u32) -> u64;
    /// Writes one byte.
    fn store_u8(&mut self, address: u32, value: u8);
    /// Writes one little-endian `u16`.
    fn store_u16(&mut self, address: u32, value: u16);
    /// Writes one little-endian `u32`.
    fn store_u32(&mut self, address: u32, value: u32);
    /// Writes one little-endian `u64`.
    fn store_u64(&mut self, address: u32, value: u64);
    /// Sets `length` bytes starting at `address` to `value`.
    fn fill(&mut self, address: u32, length: u32, value: u8);
    /// Copies `length` bytes from `source` to `destination`; the ranges may overlap.
    fn copy(&mut self, destination: u32, source: u32, length: u32);
    /// Returns the current memory size in bytes.
    fn size(&self) -> u64;
    /// Grows the memory by `pages` 64 KiB pages and returns whether it succeeded.
    fn grow(&mut self, pages: u32) -> bool;
    /// Borrows `length` bytes starting at `address`.
    fn bytes(&self, address: u32, length: u32) -> &[u8];
}

/// The linear memory of the running Wasm module, accessed through plain loads and stores.
#[cfg(target_arch = "wasm32")]
pub struct LinearMemory {
    /// Prevents construction outside [`LinearMemory::new`].
    _private: (),
}

#[cfg(target_arch = "wasm32")]
impl LinearMemory {
    /// Returns access to memory 0 of the running module.
    ///
    /// # Safety
    ///
    /// Callers must only pass addresses of memory that holds heap or VM state laid out by
    /// `exs-layout`, so writes never overlap Rust-owned data.
    #[must_use]
    pub const unsafe fn new() -> Self {
        Self { _private: () }
    }
}

#[cfg(target_arch = "wasm32")]
impl Memory for LinearMemory {
    fn load_u8(&self, address: u32) -> u8 {
        // SAFETY: the address lies in module memory per `LinearMemory::new`; Wasm traps
        // instead of reading outside the memory.
        unsafe { (address as usize as *const u8).read() }
    }

    fn load_u16(&self, address: u32) -> u16 {
        // SAFETY: as in `load_u8`; heap fields are naturally aligned.
        unsafe { (address as usize as *const u16).read() }
    }

    fn load_u32(&self, address: u32) -> u32 {
        // SAFETY: as in `load_u8`; heap fields are naturally aligned.
        unsafe { (address as usize as *const u32).read() }
    }

    fn load_u64(&self, address: u32) -> u64 {
        // SAFETY: as in `load_u8`; heap fields are naturally aligned.
        unsafe { (address as usize as *const u64).read() }
    }

    fn store_u8(&mut self, address: u32, value: u8) {
        // SAFETY: as in `load_u8`.
        unsafe { (address as usize as *mut u8).write(value) }
    }

    fn store_u16(&mut self, address: u32, value: u16) {
        // SAFETY: as in `load_u16`.
        unsafe { (address as usize as *mut u16).write(value) }
    }

    fn store_u32(&mut self, address: u32, value: u32) {
        // SAFETY: as in `load_u32`.
        unsafe { (address as usize as *mut u32).write(value) }
    }

    fn store_u64(&mut self, address: u32, value: u64) {
        // SAFETY: as in `load_u64`.
        unsafe { (address as usize as *mut u64).write(value) }
    }

    fn fill(&mut self, address: u32, length: u32, value: u8) {
        // SAFETY: as in `load_u8`; the range lies inside one carved heap allocation.
        unsafe { core::ptr::write_bytes(address as usize as *mut u8, value, length as usize) }
    }

    fn copy(&mut self, destination: u32, source: u32, length: u32) {
        // SAFETY: as in `fill`; `copy` permits overlapping ranges.
        unsafe {
            core::ptr::copy(
                source as usize as *const u8,
                destination as usize as *mut u8,
                length as usize,
            );
        }
    }

    fn size(&self) -> u64 {
        core::arch::wasm32::memory_size(0) as u64 * u64::from(exs_layout::PAGE_SIZE)
    }

    fn grow(&mut self, pages: u32) -> bool {
        core::arch::wasm32::memory_grow(0, pages as usize) != usize::MAX
    }

    fn bytes(&self, address: u32, length: u32) -> &[u8] {
        // SAFETY: as in `load_u8`; the range lies inside one heap object, which no write
        // changes while the slice is borrowed, because writes need `&mut self`.
        unsafe { core::slice::from_raw_parts(address as usize as *const u8, length as usize) }
    }
}

/// A read-only view of a module's memory for code outside the module, such as the runner.
///
/// Every read is bounds-checked: reads outside the view return 0 and [`Memory::bytes`] an
/// empty slice. Writes are ignored and the view never grows, so walks over memory the module
/// controls cannot touch anything else.
pub struct SliceMemory<'a> {
    /// The viewed bytes.
    data: &'a [u8],
}

impl<'a> SliceMemory<'a> {
    /// Returns a view of `data`.
    #[must_use]
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    /// Returns the `N` bytes at `address`, or zeros when they lie outside the view.
    fn array<const N: usize>(&self, address: u32) -> [u8; N] {
        let start = address as usize;
        start
            .checked_add(N)
            .and_then(|end| self.data.get(start..end))
            .and_then(|bytes| bytes.try_into().ok())
            .unwrap_or([0; N])
    }
}

impl Memory for SliceMemory<'_> {
    fn load_u8(&self, address: u32) -> u8 {
        self.array::<1>(address)[0]
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
        self.data.len() as u64
    }

    fn grow(&mut self, _pages: u32) -> bool {
        false
    }

    fn bytes(&self, address: u32, length: u32) -> &[u8] {
        let start = address as usize;
        start
            .checked_add(length as usize)
            .and_then(|end| self.data.get(start..end))
            .unwrap_or(&[])
    }
}
