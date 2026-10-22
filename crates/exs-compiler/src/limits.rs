//! Resource limits of one compilation.

/// Bounds that keep the compilation of untrusted source finite before any runner sandbox
/// exists: the size of each source file and of all of them, the number of modules an entry
/// imports, and how deeply expressions, blocks and type annotations nest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileLimits {
    /// Maximum bytes of one source file.
    pub max_source_bytes: usize,
    /// Maximum bytes of all source files of one compilation together.
    pub max_total_source_bytes: usize,
    /// Maximum number of source files, the entry included.
    pub max_modules: usize,
    /// Maximum nesting depth of expressions, blocks and type annotations.
    pub max_nesting: usize,
}

impl Default for CompileLimits {
    /// Returns limits suited to untrusted source: 1 MiB per file, 8 MiB in total, 512 files
    /// and 32 nesting levels.
    ///
    /// Each nesting level costs the passes after parsing about 4 KiB of stack in an optimized
    /// build and several times that in a debug build. With 32 levels an optimized build keeps
    /// at least seven times that headroom on a 1 MiB stack, and a debug build fits a 2 MiB
    /// thread.
    fn default() -> Self {
        Self {
            max_source_bytes: 1024 * 1024,
            max_total_source_bytes: 8 * 1024 * 1024,
            max_modules: 512,
            max_nesting: 32,
        }
    }
}
