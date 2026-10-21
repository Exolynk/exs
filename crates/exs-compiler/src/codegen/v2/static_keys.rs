//! Static key arrays of Object literals, from which the core builds each literal's shape once.

use std::collections::HashMap;

/// Encoded key arrays at a fixed address.
pub(super) struct StaticKeys {
    /// Address of the first array.
    pub(super) base: u32,
    /// Raw `ValueRef`s of the static key Strings of every array, back to back.
    pub(super) bytes: Vec<u8>,
    /// Address of the array of every key sequence.
    pub(super) addresses: HashMap<Vec<String>, u32>,
}

impl StaticKeys {
    /// Lays out one array per key sequence in `lists` at `base`, naming keys with the static
    /// Strings in `strings`. Returns `None` when an address leaves linear memory or a key is
    /// missing.
    pub(super) fn build(
        lists: Vec<Vec<String>>,
        strings: &HashMap<String, u64>,
        base: u32,
    ) -> Option<Self> {
        let base = base.next_multiple_of(8);
        let mut bytes = Vec::new();
        let mut addresses = HashMap::new();
        for keys in lists {
            let address = base.checked_add(u32::try_from(bytes.len()).ok()?)?;
            for key in &keys {
                bytes.extend(strings.get(key)?.to_le_bytes());
            }
            addresses.insert(keys, address);
        }
        Some(Self {
            base,
            bytes,
            addresses,
        })
    }

    /// Returns the first address after the arrays.
    pub(super) fn end(&self) -> u32 {
        self.base + self.bytes.len() as u32
    }
}
