//! The static type table the core library reads to name the fields of typed Objects and the
//! identities and variants of enums.
//!
//! The table is an array of `exs_layout::TypeEntry`s indexed by nominal type id, followed by
//! the name arrays they point to, each a run of raw `ValueRef`s of static Strings.

use std::collections::{HashMap, HashSet};

use exs_layout::TypeEntry;

use crate::codegen::types::{NominalKind, TypeRegistry};

/// Encoded type table at a fixed address.
pub(super) struct TypeTable {
    /// Address of the first entry.
    pub(super) base: u32,
    /// Number of entries.
    pub(super) count: u32,
    /// Encoded entries and field-name arrays, 8-byte aligned.
    pub(super) bytes: Vec<u8>,
}

impl TypeTable {
    /// Returns the field names of the `used` nominal Object types and the identities, variant
    /// names and payload names of the `used` enums, which must all be static Strings.
    pub(super) fn field_names<'t>(
        types: &'t TypeRegistry,
        used: &'t HashSet<u32>,
    ) -> impl Iterator<Item = &'t str> {
        let fields = types
            .nominal_types()
            .filter(|nominal| nominal.kind == NominalKind::Object && used.contains(&nominal.id))
            .flat_map(|nominal| nominal.fields.iter().map(|field| field.name.as_str()));
        let enums = types
            .enum_variants()
            .filter(|variant| used.contains(&variant.type_id))
            .flat_map(|variant| {
                [variant.type_identity.as_str(), variant.name.as_str()]
                    .into_iter()
                    .chain(variant.field_names.iter().map(String::as_str))
            });
        fields.chain(enums)
    }

    /// Returns the variant names of the enum with type id `type_id` in declaration order.
    fn variant_names(types: &TypeRegistry, type_id: u32) -> Vec<&str> {
        let mut variants: Vec<_> = types
            .enum_variants()
            .filter(|variant| variant.type_id == type_id)
            .collect();
        variants.sort_by_key(|variant| variant.index);
        variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect()
    }

    /// Lays out the table for `types` at `base`, naming the fields, variants and enum
    /// identities of the `used` types with the static Strings in `strings`; other entries stay
    /// empty. Returns `None` when an address leaves linear memory or a name is missing.
    pub(super) fn build(
        types: &TypeRegistry,
        used: &HashSet<u32>,
        strings: &HashMap<String, u64>,
        base: u32,
    ) -> Option<Self> {
        let base = base.next_multiple_of(8);
        let count = types
            .nominal_types()
            .map(|nominal| nominal.id + 1)
            .max()
            .unwrap_or_default();
        let entry_size = size_of::<TypeEntry>() as u32;
        let mut entries = vec![(0_u32, 0_u32, 0_u64); count as usize];
        let mut names = Vec::new();
        let names_base = base.checked_add(count.checked_mul(entry_size)?)?;
        for nominal in types
            .nominal_types()
            .filter(|nominal| used.contains(&nominal.id))
        {
            let (listed, identity): (Vec<&str>, u64) = match (&nominal.kind, &nominal.enum_type_id)
            {
                (NominalKind::Enum, Some(identity)) => (
                    Self::variant_names(types, nominal.id),
                    *strings.get(identity)?,
                ),
                _ => (
                    nominal
                        .fields
                        .iter()
                        .map(|field| field.name.as_str())
                        .collect(),
                    0,
                ),
            };
            if listed.is_empty() && identity == 0 {
                continue;
            }
            let address = names_base.checked_add(u32::try_from(names.len()).ok()?)?;
            for name in &listed {
                names.extend(strings.get(*name)?.to_le_bytes());
            }
            *entries.get_mut(nominal.id as usize)? =
                (u32::try_from(listed.len()).ok()?, address, identity);
        }
        let mut bytes = Vec::with_capacity(entries.len() * entry_size as usize + names.len());
        for (field_count, field_names, identity) in entries {
            bytes.extend(field_count.to_le_bytes());
            bytes.extend(field_names.to_le_bytes());
            bytes.extend(identity.to_le_bytes());
        }
        bytes.extend(names);
        Some(Self { base, count, bytes })
    }

    /// Returns the first address after the table.
    pub(super) fn end(&self) -> u32 {
        self.base + self.bytes.len() as u32
    }
}
