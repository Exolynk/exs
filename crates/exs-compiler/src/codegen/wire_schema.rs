//! Static wire schemas of annotated `let` and `for` bindings, the record trees of
//! `exs_layout::WireContract` the core's decoder walks like v1's schema strings.
//!
//! Records are built children first and shared by content among the static arrays.

use crate::codegen::types::{NominalKind, NominalType, TypeContract};
use crate::diagnostic::{CompileDiagnostics, SourceSpan};

use super::function::{FunctionCompiler, error, unsupported};

/// Returns whether a binding annotated with `contract` needs the core's decoder, because it
/// may convert Lists, Objects or enums rather than only test built-in type flags.
pub(super) fn needs_decode(contract: &TypeContract) -> bool {
    contract.list_item.is_some()
        || !contract.nominal_type_ids.is_empty()
        || !contract.enum_type_ids.is_empty()
}

impl<'a> FunctionCompiler<'a, '_> {
    /// Returns the address of the static wire schema of `contract`.
    pub(super) fn wire_schema(
        &self,
        contract: &TypeContract,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        self.wire_contract(contract, &mut Vec::new(), span)
    }

    /// Lays out the `WireContract` of `contract`, with `path` holding the nominal types being
    /// expanded, which a recursive type would repeat.
    fn wire_contract(
        &self,
        contract: &TypeContract,
        path: &mut Vec<u32>,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        let list = match &contract.list_item {
            Some(item) => self.wire_contract(item, path, span)?,
            None => 0,
        };
        let nominal = |kind: NominalKind| {
            contract.nominal_type_ids.iter().filter_map(move |id| {
                self.context
                    .types
                    .nominal_types()
                    .find(|nominal| nominal.id == *id && nominal.kind == kind)
            })
        };
        let object = match nominal(NominalKind::Object).next() {
            Some(object) => self.wire_object(object, path, span)?,
            None => 0,
        };
        let mut enums = Vec::new();
        for enumeration in nominal(NominalKind::Enum) {
            enums.extend(self.wire_enum(enumeration, path, span)?);
        }
        let enum_count = (enums.len() / 4) as u32;
        let enums = if enums.is_empty() {
            0
        } else {
            self.static_array(enums)
        };
        Ok(self.static_array(vec![
            contract.builtin_mask,
            list,
            object,
            enum_count,
            enums,
            0,
        ]))
    }

    /// Enters nominal type `id` on `path`, rejecting a type that contains itself.
    fn enter(
        &self,
        id: u32,
        path: &mut Vec<u32>,
        span: SourceSpan<'a>,
    ) -> Result<(), CompileDiagnostics<'a>> {
        if path.contains(&id) {
            return Err(unsupported(span, "annotations of recursive types"));
        }
        path.push(id);
        Ok(())
    }

    /// Lays out the `WireObject` of the nominal Object type `object`.
    fn wire_object(
        &self,
        object: &NominalType,
        path: &mut Vec<u32>,
        span: SourceSpan<'a>,
    ) -> Result<u32, CompileDiagnostics<'a>> {
        self.enter(object.id, path, span)?;
        let fields = object
            .fields
            .iter()
            .map(|field| (field.name.as_str(), &field.contract));
        let (count, fields) = self.wire_fields(fields, path, span)?;
        path.pop();
        Ok(self.static_array(vec![object.id, count, fields, 0]))
    }

    /// Returns the four words of the `WireEnum` of the enum type `enumeration`.
    fn wire_enum(
        &self,
        enumeration: &NominalType,
        path: &mut Vec<u32>,
        span: SourceSpan<'a>,
    ) -> Result<[u32; 4], CompileDiagnostics<'a>> {
        self.enter(enumeration.id, path, span)?;
        let mut variants: Vec<_> = self
            .context
            .types
            .enum_variants()
            .filter(|variant| variant.type_id == enumeration.id)
            .collect();
        variants.sort_by_key(|variant| variant.index);
        let mut words = Vec::with_capacity(variants.len() * 6);
        for variant in &variants {
            let fields = variant
                .field_names
                .iter()
                .map(String::as_str)
                .zip(&variant.fields);
            let (count, fields) = self.wire_fields(fields, path, span)?;
            let [low, high] = self.static_name(&variant.name, span)?;
            words.extend([low, high, variant.index, count, fields, 0]);
        }
        path.pop();
        let count = variants.len() as u32;
        let variants = if words.is_empty() {
            0
        } else {
            self.static_array(words)
        };
        Ok([enumeration.id, count, variants, 0])
    }

    /// Lays out the `WireField` array of `fields`, returning its length and address.
    fn wire_fields<'f>(
        &self,
        fields: impl Iterator<Item = (&'f str, &'f TypeContract)>,
        path: &mut Vec<u32>,
        span: SourceSpan<'a>,
    ) -> Result<(u32, u32), CompileDiagnostics<'a>> {
        let mut words = Vec::new();
        for (name, contract) in fields {
            let contract = self.wire_contract(contract, path, span)?;
            let [low, high] = self.static_name(name, span)?;
            words.extend([low, high, contract, 0]);
        }
        let count = (words.len() / 4) as u32;
        if words.is_empty() {
            return Ok((0, 0));
        }
        Ok((count, self.static_array(words)))
    }

    /// Returns the two words of the raw `ValueRef` of the static String `name`.
    fn static_name(
        &self,
        name: &str,
        span: SourceSpan<'a>,
    ) -> Result<[u32; 2], CompileDiagnostics<'a>> {
        let raw = self
            .context
            .strings
            .get(name)
            .copied()
            .ok_or_else(|| error("E0999", span, format!("missing static String `{name}`")))?;
        Ok([raw as u32, (raw >> 32) as u32])
    }
}
