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
    /// and 64 nesting levels.
    ///
    /// Each nesting level costs the parser and later passes several kilobytes of stack in an
    /// optimized build and far more in a debug build, so the nesting limit stays well below
    /// what a 2 MiB thread stack holds.
    fn default() -> Self {
        Self {
            max_source_bytes: 1024 * 1024,
            max_total_source_bytes: 8 * 1024 * 1024,
            max_modules: 512,
            max_nesting: 64,
        }
    }
}
