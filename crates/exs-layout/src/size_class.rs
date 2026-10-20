//! Cell sizes of the small-object allocator.

/// Cell byte sizes, each including the object header and a multiple of 8.
pub const SIZE_CLASSES: [u32; CLASS_COUNT] = [
    16, 24, 32, 48, 64, 96, 128, 192, 256, 384, 512, 768, 1024, 1536, 2048,
];

/// Number of size classes.
pub const CLASS_COUNT: usize = 15;

/// Largest object size served from size-class pages; larger objects get their own pages.
pub const MAX_CLASS_SIZE: u32 = SIZE_CLASSES[CLASS_COUNT - 1];

/// Returns the smallest class whose cells hold `size` bytes, or `None` for large objects.
#[must_use]
pub const fn class_for_size(size: u32) -> Option<usize> {
    let mut class = 0;
    while class < CLASS_COUNT {
        if size <= SIZE_CLASSES[class] {
            return Some(class);
        }
        class += 1;
    }
    None
}

const _: () = {
    let mut class = 0;
    while class < CLASS_COUNT {
        assert!(SIZE_CLASSES[class].is_multiple_of(8) && SIZE_CLASSES[class] >= 16);
        class += 1;
    }
};
