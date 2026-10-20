//! Heap pages and their headers.

/// Byte size of one heap page, equal to one Wasm memory page.
pub const PAGE_SIZE: u32 = 65_536;

/// Byte size of [`PageHeader`]; cells and large objects start right after it.
pub const PAGE_HEADER_SIZE: u32 = 16;

/// Use of one heap page, stored in [`PageHeader::kind`].
#[repr(u32)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageKind {
    /// The first page of a run of free pages.
    Free = 0,
    /// A page split into equal cells of one size class.
    Class = 1,
    /// The first page of a run holding one large object.
    Large = 2,
    /// The first page of a run holding collector-internal storage such as the mark stack.
    System = 3,
}

impl PageKind {
    /// Returns the kind stored in one page header, or `None` for an unknown value.
    #[must_use]
    pub const fn from_u32(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::Free,
            1 => Self::Class,
            2 => Self::Large,
            3 => Self::System,
            _ => return None,
        })
    }
}

/// The header at the start of every page run.
#[repr(C)]
pub struct PageHeader {
    /// One [`PageKind`].
    pub kind: u32,
    /// Number of pages in this run, 1 for class pages.
    pub run_pages: u32,
    /// Size-class index for class pages; next free run for free runs.
    pub detail: u32,
    /// Live cells after the last sweep, for class pages.
    pub live: u32,
}

const _: () = assert!(core::mem::size_of::<PageHeader>() == PAGE_HEADER_SIZE as usize);
