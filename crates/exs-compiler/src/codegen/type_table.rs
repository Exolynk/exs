//! The static type table the core library reads to name the fields of typed Objects and the
//! identities and variants of enums.
//!
//! The table is an array of `exs_layout::TypeEntry`s, one per used nominal type sorted by type
//! id, followed by the name arrays they point to, each a run of raw `ValueRef`s of static
//! Strings. Programs with many unused types pay nothing for them.

use std::collections::{HashMap, HashSet};

use exs_layout::TypeEntry;

use crate::codegen::types::{NominalKind, TypeRegistry};

/// The nominal types whose equality or rendering an implementation in the program decides.
pub(super) struct CustomBehavior {
    /// Type ids with an `Eq` or `Compare` implementation.
    pub(super) equality: HashSet<u32>,
    /// Type ids with a `Debug` or `ToString` implementation.
    pub(super) rendering: HashSet<u32>,
}

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
            .flat_map(|nominal| {
                core::iter::once(nominal.name.as_str())
                    .chain(nominal.fields.iter().map(|field| field.name.as_str()))
            });
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

    /// Lays out the table of the `used` types at `base`, naming the fields, variants and enum
    /// identities with the static Strings in `strings`. Returns `None` when an address leaves
    /// linear memory or a name is missing.
    pub(super) fn build(
        types: &TypeRegistry,
        used: &HashSet<u32>,
        custom: &CustomBehavior,
        strings: &HashMap<String, u64>,
        base: u32,
    ) -> Option<Self> {
        let base = base.next_multiple_of(8);
        let mut nominals: Vec<_> = types
            .nominal_types()
            .filter(|nominal| used.contains(&nominal.id))
            .collect();
        nominals.sort_by_key(|nominal| nominal.id);
        let count = u32::try_from(nominals.len()).ok()?;
        let entry_size = size_of::<TypeEntry>() as u32;
        let mut entries = Vec::with_capacity(nominals.len());
        let mut names = Vec::new();
        let names_base = base.checked_add(count.checked_mul(entry_size)?)?;
        for nominal in nominals {
            let (listed, identity, kind): (Vec<&str>, u64, u32) =
                match (&nominal.kind, &nominal.enum_type_id) {
                    (NominalKind::Enum, Some(identity)) => (
                        Self::variant_names(types, nominal.id),
                        *strings.get(identity)?,
                        exs_layout::TYPE_ENTRY_ENUM,
                    ),
                    _ => (
                        nominal
                            .fields
                            .iter()
                            .map(|field| field.name.as_str())
                            .collect(),
                        *strings.get(&nominal.name)?,
                        exs_layout::TYPE_ENTRY_OBJECT,
                    ),
                };
            let mut kind = kind;
            if custom.equality.contains(&nominal.id) {
                kind |= exs_layout::TYPE_ENTRY_CUSTOM_EQUALITY;
            }
            if custom.rendering.contains(&nominal.id) {
                kind |= exs_layout::TYPE_ENTRY_CUSTOM_RENDERING;
            }
            let address = names_base.checked_add(u32::try_from(names.len()).ok()?)?;
            for name in &listed {
                names.extend(strings.get(*name)?.to_le_bytes());
            }
            entries.push((
                u32::try_from(listed.len()).ok()?,
                address,
                identity,
                nominal.id,
                kind,
            ));
        }
        let mut bytes = Vec::with_capacity(entries.len() * entry_size as usize + names.len());
        for (field_count, field_names, identity, type_id, kind) in entries {
            bytes.extend(field_count.to_le_bytes());
            bytes.extend(field_names.to_le_bytes());
            bytes.extend(identity.to_le_bytes());
            bytes.extend(type_id.to_le_bytes());
            bytes.extend(kind.to_le_bytes());
        }
        bytes.extend(names);
        Some(Self { base, count, bytes })
    }

    /// Returns the first address after the table.
    pub(super) fn end(&self) -> u32 {
        self.base + self.bytes.len() as u32
    }
}
