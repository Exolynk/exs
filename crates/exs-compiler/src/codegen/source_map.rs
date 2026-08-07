//! Compact source-position metadata emitted with one compiled ExS module.

use std::collections::HashMap;
use std::fmt;

use wasmparser::{Parser, Payload};

use crate::ast::{AssignmentTarget, Block, Expression, Module, ObjectProperty, Statement};
use crate::diagnostic::SourceSpan;

/// Custom-section name containing compact source position records.
pub(super) const SOURCE_MAP_SECTION: &str = "exs.source.map";
/// Custom-section name containing optional source text.
pub(super) const SOURCES_SECTION: &str = "exs.sources";

/// One compiler-assigned source position table for a single source unit.
pub(super) struct SourceMap<'a> {
    source_ids: Vec<&'a str>,
    entries: Vec<SourceSpan<'a>>,
    ids: HashMap<(&'a str, u32, u32), u32>,
    functions: Vec<String>,
}

impl<'a> SourceMap<'a> {
    /// Assigns compact non-zero position identifiers to all spans in one parsed module.
    pub(super) fn collect(module: &Module<'a>) -> Self {
        let mut source_map = Self {
            source_ids: Vec::new(),
            entries: Vec::new(),
            ids: HashMap::new(),
            functions: module
                .functions
                .iter()
                .map(|function| function.name.name.clone())
                .chain(module.implementations.iter().flat_map(|implementation| {
                    implementation.methods.iter().map(|method| {
                        format!("{}::{}", implementation.type_name.name, method.name.name)
                    })
                }))
                .collect(),
        };
        for declaration in &module.types {
            source_map.insert(declaration.span);
            source_map.insert(declaration.name.span);
            for field in &declaration.fields {
                source_map.insert(field.span);
                source_map.insert(field.name.span);
            }
        }
        for declaration in &module.enums {
            source_map.insert(declaration.span);
            source_map.insert(declaration.name.span);
            for variant in &declaration.variants {
                source_map.insert(variant.span);
                source_map.insert(variant.name.span);
                for field in &variant.fields {
                    source_map.insert(field.span);
                    source_map.insert(field.name.span);
                    if let Some(annotation) = &field.type_annotation {
                        source_map.insert(annotation.span);
                        for member in &annotation.members {
                            source_map.insert(member.span);
                        }
                    }
                }
            }
        }
        for declaration in &module.traits {
            source_map.insert(declaration.span);
            source_map.insert(declaration.name.span);
            for method in &declaration.methods {
                source_map.insert(method.span);
                source_map.insert(method.name.span);
                for parameter in &method.parameters {
                    source_map.insert(parameter.name.span);
                    if let Some(annotation) = &parameter.type_annotation {
                        source_map.insert(annotation.span);
                        for member in &annotation.members {
                            source_map.insert(member.span);
                        }
                    }
                }
                if let Some(annotation) = &method.return_type {
                    source_map.insert(annotation.span);
                    for member in &annotation.members {
                        source_map.insert(member.span);
                    }
                }
                if let Some(body) = &method.body {
                    source_map.collect_block(body);
                }
            }
        }
        for function in &module.functions {
            source_map.collect_function(function);
        }
        for implementation in &module.implementations {
            source_map.insert(implementation.span);
            source_map.insert(implementation.type_name.span);
            for method in &implementation.methods {
                source_map.collect_function(method);
            }
        }
        source_map
    }

    /// Returns the compact identifier assigned to one source span.
    pub(super) fn id(&self, span: SourceSpan<'a>) -> Option<u32> {
        self.ids
            .get(&(span.source_id, span.start_byte, span.end_byte))
            .copied()
    }

    /// Encodes the `exs.source.map` binary format.
    pub(super) fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        if self.source_ids.len() == 1 {
            bytes.extend_from_slice(b"EXSMAP2\0");
            append_u32(&mut bytes, self.source_ids[0].len());
        } else {
            bytes.extend_from_slice(b"EXSMAP3\0");
            append_u32(&mut bytes, self.source_ids.len());
        }
        append_u32(&mut bytes, self.entries.len());
        append_u32(&mut bytes, self.functions.len());
        if self.source_ids.len() == 1 {
            bytes.extend_from_slice(self.source_ids[0].as_bytes());
        } else {
            for source_id in &self.source_ids {
                append_u32(&mut bytes, source_id.len());
                bytes.extend_from_slice(source_id.as_bytes());
            }
        }
        for span in &self.entries {
            if self.source_ids.len() > 1 {
                let source = self
                    .source_ids
                    .iter()
                    .position(|source_id| *source_id == span.source_id)
                    .unwrap_or_default();
                append_u32(&mut bytes, source);
            }
            append_u32(&mut bytes, span.start_byte as usize);
            append_u32(&mut bytes, span.end_byte as usize);
        }
        for (identifier, name) in self.functions.iter().enumerate() {
            append_u32(&mut bytes, identifier);
            append_u32(&mut bytes, name.len());
            bytes.extend_from_slice(name.as_bytes());
        }
        bytes
    }

    /// Encodes the `exs.sources` binary format for one optional embedded source unit.
    pub(super) fn encode_source(&self, sources: &[crate::SourceInput<'a>]) -> Vec<u8> {
        let mut bytes = Vec::new();
        if self.source_ids.len() == 1 {
            let source = sources
                .iter()
                .find(|source| source.source_id == self.source_ids[0]);
            let Some(source) = source else {
                return bytes;
            };
            bytes.extend_from_slice(b"EXSSRC1\0");
            append_u32(&mut bytes, source.source_id.len());
            append_u32(&mut bytes, source.text.len());
            bytes.extend_from_slice(source.source_id.as_bytes());
            bytes.extend_from_slice(source.text.as_bytes());
        } else {
            bytes.extend_from_slice(b"EXSSRC2\0");
            append_u32(&mut bytes, self.source_ids.len());
            for source_id in &self.source_ids {
                let source = sources.iter().find(|source| source.source_id == *source_id);
                let Some(source) = source else {
                    return bytes;
                };
                append_u32(&mut bytes, source.source_id.len());
                append_u32(&mut bytes, source.text.len());
                bytes.extend_from_slice(source.source_id.as_bytes());
                bytes.extend_from_slice(source.text.as_bytes());
            }
        }
        bytes
    }

    /// Adds one span when it has not already received an identifier.
    fn insert(&mut self, span: SourceSpan<'a>) {
        let key = (span.source_id, span.start_byte, span.end_byte);
        if self.ids.contains_key(&key) {
            return;
        }
        let identifier = u32::try_from(self.entries.len())
            .ok()
            .and_then(|index| index.checked_add(1));
        if let Some(identifier) = identifier {
            if !self.source_ids.contains(&span.source_id) {
                self.source_ids.push(span.source_id);
                self.source_ids.sort_unstable();
            }
            self.entries.push(span);
            self.ids.insert(key, identifier);
        }
    }

    /// Collects all spans reachable from one block.
    fn collect_block(&mut self, block: &Block<'a>) {
        self.insert(block.span);
        for statement in &block.statements {
            self.collect_statement(statement);
        }
    }

    /// Collects all spans reachable from one direct function declaration.
    fn collect_function(&mut self, function: &crate::ast::FunctionDeclaration<'a>) {
        self.insert(function.span);
        self.insert(function.name.span);
        for parameter in &function.parameters {
            self.insert(parameter.name.span);
            if let Some(annotation) = &parameter.type_annotation {
                self.insert(annotation.span);
                for member in &annotation.members {
                    self.insert(member.span);
                }
            }
        }
        if let Some(annotation) = &function.return_type {
            self.insert(annotation.span);
            for member in &annotation.members {
                self.insert(member.span);
            }
        }
        self.collect_block(&function.body);
    }

    /// Collects all spans reachable from one statement.
    fn collect_statement(&mut self, statement: &Statement<'a>) {
        match statement {
            Statement::Let { name, value, span } => {
                self.insert(*span);
                self.insert(name.span);
                self.collect_expression(value);
            }
            Statement::Assign {
                target,
                value,
                span,
            } => {
                self.insert(*span);
                self.collect_target(target);
                self.collect_expression(value);
            }
            Statement::Return { value, span } => {
                self.insert(*span);
                if let Some(value) = value {
                    self.collect_expression(value);
                }
            }
            Statement::Block { block, span } => {
                self.insert(*span);
                self.collect_block(block);
            }
            Statement::If {
                condition,
                then_block,
                else_branch,
                span,
            } => {
                self.insert(*span);
                self.collect_expression(condition);
                self.collect_block(then_block);
                if let Some(else_branch) = else_branch {
                    match else_branch {
                        crate::ast::ElseBranch::Block(block) => self.collect_block(block),
                        crate::ast::ElseBranch::If(statement) => self.collect_statement(statement),
                    }
                }
            }
            Statement::While {
                condition,
                body,
                span,
            } => {
                self.insert(*span);
                self.collect_expression(condition);
                self.collect_block(body);
            }
            Statement::For {
                binding,
                iterable,
                body,
                span,
            } => {
                self.insert(*span);
                self.insert(binding.span);
                self.collect_expression(iterable);
                self.collect_block(body);
            }
            Statement::Break { span } | Statement::Continue { span } => self.insert(*span),
            Statement::Expression { expression, span } => {
                self.insert(*span);
                self.collect_expression(expression);
            }
        }
    }

    /// Collects all spans reachable from one assignment target.
    fn collect_target(&mut self, target: &AssignmentTarget<'a>) {
        match target {
            AssignmentTarget::Variable(identifier) => self.insert(identifier.span),
            AssignmentTarget::Index {
                receiver,
                index,
                span,
            } => {
                self.insert(*span);
                self.collect_expression(receiver);
                self.collect_expression(index);
            }
            AssignmentTarget::Property {
                receiver,
                property,
                span,
            } => {
                self.insert(*span);
                self.insert(property.span);
                self.collect_expression(receiver);
            }
        }
    }

    /// Collects all spans reachable from one expression.
    fn collect_expression(&mut self, expression: &Expression<'a>) {
        match expression {
            Expression::Integer(_, span)
            | Expression::Float(_, span)
            | Expression::String(_, span)
            | Expression::Bool(_, span)
            | Expression::None(span) => self.insert(*span),
            Expression::IsError { value, span } | Expression::Propagate { value, span } => {
                self.insert(*span);
                self.collect_expression(value);
            }
            Expression::List { elements, span } => {
                self.insert(*span);
                for element in elements {
                    self.collect_expression(element);
                }
            }
            Expression::Object { properties, span }
            | Expression::TypedObject {
                properties, span, ..
            } => {
                self.insert(*span);
                for property in properties {
                    self.collect_property(property);
                }
            }
            Expression::Match { value, arms, span } => {
                self.insert(*span);
                self.collect_expression(value);
                for arm in arms {
                    self.insert(arm.span);
                    match &arm.pattern {
                        crate::ast::MatchPattern::Variant {
                            type_name,
                            variant,
                            bindings,
                            span,
                        } => {
                            self.insert(*span);
                            self.insert(type_name.span);
                            self.insert(variant.span);
                            for binding in bindings {
                                self.insert(binding.span);
                            }
                        }
                        crate::ast::MatchPattern::Wildcard(span) => self.insert(*span),
                    }
                    match &arm.body {
                        crate::ast::MatchArmBody::Expression(value) => {
                            self.collect_expression(value);
                        }
                        crate::ast::MatchArmBody::Block(block) => self.collect_block(block),
                    }
                }
            }
            Expression::Variable(identifier) => self.insert(identifier.span),
            Expression::Closure {
                parameters,
                body,
                span,
            } => {
                self.insert(*span);
                for parameter in parameters {
                    self.insert(parameter.name.span);
                }
                self.collect_block(body);
            }
            Expression::ParallelStatic { tasks, span } => {
                self.insert(*span);
                for task in tasks {
                    self.collect_expression(task);
                }
            }
            Expression::ParallelDynamic { functions, span } => {
                self.insert(*span);
                self.collect_expression(functions);
            }
            Expression::Unary { operand, span, .. } => {
                self.insert(*span);
                self.collect_expression(operand);
            }
            Expression::Binary {
                left, right, span, ..
            } => {
                self.insert(*span);
                self.collect_expression(left);
                self.collect_expression(right);
            }
            Expression::Call {
                callee,
                arguments,
                span,
            } => {
                self.insert(*span);
                self.insert(callee.span);
                for argument in arguments {
                    self.collect_expression(argument);
                }
            }
            Expression::HostCall {
                name,
                arguments,
                span,
            } => {
                self.insert(*span);
                self.collect_expression(name);
                for argument in arguments {
                    self.collect_expression(argument);
                }
            }
            Expression::MethodCall {
                receiver,
                method,
                arguments,
                span,
            } => {
                self.insert(*span);
                self.insert(method.span);
                self.collect_expression(receiver);
                for argument in arguments {
                    self.collect_expression(argument);
                }
            }
            Expression::StaticMethodCall {
                type_name,
                method,
                arguments,
                span,
            } => {
                self.insert(*span);
                self.insert(type_name.span);
                self.insert(method.span);
                for argument in arguments {
                    self.collect_expression(argument);
                }
            }
            Expression::Index {
                receiver,
                index,
                span,
            } => {
                self.insert(*span);
                self.collect_expression(receiver);
                self.collect_expression(index);
            }
            Expression::Property {
                receiver,
                property,
                span,
            } => {
                self.insert(*span);
                self.insert(property.span);
                self.collect_expression(receiver);
            }
        }
    }

    /// Collects the spans attached to one object property.
    fn collect_property(&mut self, property: &ObjectProperty<'a>) {
        self.insert(property.key_span);
        self.insert(property.span);
        self.collect_expression(&property.value);
    }
}

/// Appends one bounded length or byte offset in little-endian form.
fn append_u32(bytes: &mut Vec<u8>, value: usize) {
    let value = u32::try_from(value).unwrap_or(u32::MAX);
    bytes.extend_from_slice(&value.to_le_bytes());
}

/// One source range resolved from a compiler-assigned position identifier.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SourcePosition {
    /// Human-readable source identity.
    pub source_id: String,
    /// Inclusive UTF-8 byte offset of the source range.
    pub start_byte: u32,
    /// Exclusive UTF-8 byte offset of the source range.
    pub end_byte: u32,
}

/// One generated function name resolved from a compiler-assigned function identifier.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct FunctionDebugInfo {
    /// Stable generated function identifier.
    pub function_id: u32,
    /// Source-level function name.
    pub name: String,
}

/// One optional embedded source unit from a resolved module graph.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct EmbeddedSource {
    /// Canonical source identity.
    pub source_id: String,
    /// Complete UTF-8 source text.
    pub source: String,
}

/// Debug metadata decoded from ExS custom sections in one linked Wasm module.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ModuleDebugInfo {
    /// Human-readable source identity shared by all current source positions.
    pub source_id: String,
    /// Position records ordered by one-based `SourcePositionId`.
    pub positions: Vec<SourcePosition>,
    /// Source-level function names keyed by generated function identifier.
    pub functions: Vec<FunctionDebugInfo>,
    /// Optional UTF-8 source text embedded in the module.
    pub source: Option<String>,
    /// All embedded sources when compilation included a resolved module graph.
    pub sources: Vec<EmbeddedSource>,
}

impl ModuleDebugInfo {
    /// Resolves one compiler-assigned source position identifier.
    #[must_use]
    pub fn position(&self, identifier: exs_abi::SourcePositionId) -> Option<&SourcePosition> {
        let index = usize::try_from(identifier.0.checked_sub(1)?).ok()?;
        self.positions.get(index)
    }

    /// Resolves one generated function identifier to its source-level name.
    #[must_use]
    pub fn function_name(&self, identifier: u32) -> Option<&str> {
        self.functions
            .iter()
            .find(|function| function.function_id == identifier)
            .map(|function| function.name.as_str())
    }

    /// Returns embedded text for one source identity, when source embedding was requested.
    #[must_use]
    pub fn source_for(&self, source_id: &str) -> Option<&str> {
        self.sources
            .iter()
            .find(|source| source.source_id == source_id)
            .map(|source| source.source.as_str())
            .or_else(|| {
                (self.source_id == source_id)
                    .then_some(self.source.as_deref())
                    .flatten()
            })
    }
}

/// Failure while reading ExS debugging metadata from a Wasm module.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DebugInfoError {
    /// The input is not a valid Wasm module.
    InvalidWasm,
    /// The module does not contain `exs.source.map` metadata.
    MissingSourceMap,
    /// An ExS metadata section has an unsupported version.
    UnsupportedVersion,
    /// An ExS metadata section is malformed or inconsistent.
    Malformed,
}

impl fmt::Display for DebugInfoError {
    /// Formats one metadata-reading failure for CLI and embedding callers.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWasm => formatter.write_str("invalid Wasm module"),
            Self::MissingSourceMap => formatter.write_str("missing exs.source.map section"),
            Self::UnsupportedVersion => formatter.write_str("unsupported ExS source-map version"),
            Self::Malformed => formatter.write_str("malformed ExS debugging metadata"),
        }
    }
}

impl std::error::Error for DebugInfoError {}

/// Decodes ExS debug metadata from one linked Wasm module.
///
/// # Errors
///
/// Returns an error when the Wasm module or its ExS metadata sections are invalid.
pub fn read_debug_info(wasm: &[u8]) -> Result<ModuleDebugInfo, DebugInfoError> {
    let mut source_map = None;
    let mut source = None;
    for payload in Parser::new(0).parse_all(wasm) {
        let payload = payload.map_err(|_| DebugInfoError::InvalidWasm)?;
        if let Payload::CustomSection(section) = payload {
            match section.name() {
                SOURCE_MAP_SECTION => source_map = Some(section.data().to_vec()),
                SOURCES_SECTION => source = Some(section.data().to_vec()),
                _ => {}
            }
        }
    }
    let source_map = source_map.ok_or(DebugInfoError::MissingSourceMap)?;
    let mut info = decode_source_map(&source_map)?;
    if let Some(source) = source {
        let sources = decode_sources(&source)?;
        if !sources
            .iter()
            .any(|source| source.source_id == info.source_id)
        {
            return Err(DebugInfoError::Malformed);
        }
        info.source = sources
            .iter()
            .find(|source| source.source_id == info.source_id)
            .map(|source| source.source.clone());
        info.sources = sources;
    }
    Ok(info)
}

/// Decodes the version-two compact ExS source-map payload.
fn decode_source_map(bytes: &[u8]) -> Result<ModuleDebugInfo, DebugInfoError> {
    let mut reader = MetadataReader::new(bytes);
    let version = reader.take(8)?;
    let (source_ids, position_count, function_count) = if version == b"EXSMAP2\0" {
        let source_id_length = reader.length()?;
        let position_count = reader.length()?;
        let function_count = reader.length()?;
        (
            vec![reader.string(source_id_length)?],
            position_count,
            function_count,
        )
    } else if version == b"EXSMAP3\0" {
        let source_count = reader.length()?;
        let position_count = reader.length()?;
        let function_count = reader.length()?;
        let mut source_ids = Vec::new();
        for _ in 0..source_count {
            let source_id_length = reader.length()?;
            source_ids.push(reader.string(source_id_length)?);
        }
        (source_ids, position_count, function_count)
    } else {
        return Err(DebugInfoError::UnsupportedVersion);
    };
    let source_id = source_ids
        .first()
        .cloned()
        .ok_or(DebugInfoError::Malformed)?;
    let mut positions = Vec::new();
    for _ in 0..position_count {
        let position_source = if source_ids.len() == 1 {
            source_id.clone()
        } else {
            source_ids
                .get(reader.length()?)
                .cloned()
                .ok_or(DebugInfoError::Malformed)?
        };
        positions.push(SourcePosition {
            source_id: position_source,
            start_byte: reader.u32()?,
            end_byte: reader.u32()?,
        });
    }
    let mut functions = Vec::new();
    for _ in 0..function_count {
        let function_id = reader.u32()?;
        let name_length = reader.length()?;
        functions.push(FunctionDebugInfo {
            function_id,
            name: reader.string(name_length)?,
        });
    }
    reader.finish()?;
    Ok(ModuleDebugInfo {
        source_id,
        positions,
        functions,
        source: None,
        sources: Vec::new(),
    })
}

/// Decodes the optional embedded-source custom-section payload.
fn decode_sources(bytes: &[u8]) -> Result<Vec<EmbeddedSource>, DebugInfoError> {
    let mut reader = MetadataReader::new(bytes);
    let version = reader.take(8)?;
    let source_count = if version == b"EXSSRC1\0" {
        1
    } else if version == b"EXSSRC2\0" {
        reader.length()?
    } else {
        return Err(DebugInfoError::UnsupportedVersion);
    };
    let mut sources = Vec::new();
    for _ in 0..source_count {
        let source_id_length = reader.length()?;
        let source_length = reader.length()?;
        sources.push(EmbeddedSource {
            source_id: reader.string(source_id_length)?,
            source: reader.string(source_length)?,
        });
    }
    reader.finish()?;
    Ok(sources)
}

/// Bounds-checked reader for fixed-format ExS metadata sections.
struct MetadataReader<'a> {
    /// Full metadata section payload.
    bytes: &'a [u8],
    /// Current byte offset within the payload.
    position: usize,
}

impl<'a> MetadataReader<'a> {
    /// Starts reading at the first byte of one metadata payload.
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    /// Reads one little-endian unsigned 32-bit value.
    fn u32(&mut self) -> Result<u32, DebugInfoError> {
        let bytes = self.take(4)?;
        let bytes: [u8; 4] = bytes.try_into().map_err(|_| DebugInfoError::Malformed)?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Reads one bounded section length.
    fn length(&mut self) -> Result<usize, DebugInfoError> {
        usize::try_from(self.u32()?).map_err(|_| DebugInfoError::Malformed)
    }

    /// Reads one UTF-8 byte range with a previously decoded length.
    fn string(&mut self, length: usize) -> Result<String, DebugInfoError> {
        String::from_utf8(self.take(length)?.to_vec()).map_err(|_| DebugInfoError::Malformed)
    }

    /// Returns exactly the requested bytes and advances the reader.
    fn take(&mut self, length: usize) -> Result<&'a [u8], DebugInfoError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(DebugInfoError::Malformed)?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .ok_or(DebugInfoError::Malformed)?;
        self.position = end;
        Ok(bytes)
    }

    /// Verifies that the reader consumed the complete payload.
    fn finish(&self) -> Result<(), DebugInfoError> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err(DebugInfoError::Malformed)
        }
    }
}
