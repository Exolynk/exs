use super::*;

impl<'a> Parser<'a> {
    pub(super) fn import_declaration(
        &mut self,
    ) -> Result<ImportDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Import, "expected `import`")?
            .span;
        let token = self.advance().clone();
        let path = match token.kind {
            TokenKind::String(path) => path,
            _ => {
                return Err(self.error(token.span, "E0113", "expected string path after `import`"));
            }
        };
        let alias = if self.matches(&TokenKind::As) {
            Some(self.identifier("expected namespace after `as`")?)
        } else {
            None
        };
        let end = self
            .expect_simple(TokenKind::Semicolon, "expected `;` after import")?
            .span;
        Ok(ImportDeclaration {
            path,
            alias,
            span: start.through(end),
        })
    }

    /// Parses one nominal enum declaration and its zero-or-more-field variants.
    pub(super) fn enum_declaration(
        &mut self,
    ) -> Result<EnumDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Enum, "expected `enum` at module level")?
            .span;
        let name = self.identifier("expected enum name after `enum`")?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after enum name")?;
        let mut variants = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let variant_name = self.identifier("expected enum variant name")?;
            let mut fields = Vec::new();
            let mut end = variant_name.span;
            if self.matches(&TokenKind::LeftParen) {
                while !self.check(&TokenKind::RightParen) && !self.at_end() {
                    let field_start = self.peek().span;
                    let field_name = self.identifier("expected variant payload field name")?;
                    let type_annotation = if self.matches(&TokenKind::Colon) {
                        Some(self.type_annotation()?)
                    } else {
                        None
                    };
                    let field_end = type_annotation
                        .as_ref()
                        .map_or(field_name.span, |annotation| annotation.span);
                    fields.push(TypeField {
                        name: field_name,
                        type_annotation,
                        span: field_start.through(field_end),
                    });
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
                end = self
                    .expect_simple(TokenKind::RightParen, "expected `)` after variant fields")?
                    .span;
            }
            variants.push(EnumVariant {
                span: variant_name.span.through(end),
                name: variant_name,
                fields,
            });
            if !self.matches(&TokenKind::Comma) {
                if !self.check(&TokenKind::RightBrace) {
                    return Err(self.error(
                        self.peek().span,
                        "E0103",
                        "expected `,` or `}` after enum variant",
                    ));
                }
                break;
            }
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after enum variants")?
            .span;
        Ok(EnumDeclaration {
            name,
            variants,
            span: start.through(end),
        })
    }

    /// Parses one `use namespace::{name as alias}` declaration.
    pub(super) fn use_declaration(&mut self) -> Result<UseDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self.expect_simple(TokenKind::Use, "expected `use`")?.span;
        let namespace = self.identifier("expected namespace after `use`")?;
        self.expect_simple(TokenKind::DoubleColon, "expected `::` after use namespace")?;
        let mut items = Vec::new();
        if self.matches(&TokenKind::LeftBrace) {
            loop {
                let name = self.identifier("expected imported declaration name")?;
                let alias = if self.matches(&TokenKind::As) {
                    Some(self.identifier("expected alias after `as`")?)
                } else {
                    None
                };
                self.type_names.insert(
                    alias
                        .as_ref()
                        .map_or_else(|| name.name.clone(), |alias| alias.name.clone()),
                );
                items.push(UseItem { name, alias });
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightBrace) {
                    break;
                }
            }
            self.expect_simple(TokenKind::RightBrace, "expected `}` after use list")?;
        } else {
            let name = self.identifier("expected imported declaration name")?;
            let alias = if self.matches(&TokenKind::As) {
                Some(self.identifier("expected alias after `as`")?)
            } else {
                None
            };
            self.type_names.insert(
                alias
                    .as_ref()
                    .map_or_else(|| name.name.clone(), |alias| alias.name.clone()),
            );
            items.push(UseItem { name, alias });
        }
        let end = self
            .expect_simple(TokenKind::Semicolon, "expected `;` after use declaration")?
            .span;
        Ok(UseDeclaration {
            namespace,
            items,
            span: start.through(end),
        })
    }

    pub(super) fn function(
        &mut self,
        visibility: FunctionVisibility,
    ) -> Result<FunctionDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Fn, "expected `fn` at module level")?
            .span;
        self.function_from_start(start, visibility)
    }

    /// Parses one named source test declaration.
    pub(super) fn test_declaration(
        &mut self,
    ) -> Result<TestDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self.advance().span;
        let token = self.advance().clone();
        let description = match token.kind {
            TokenKind::String(description) => description,
            _ => {
                return Err(self.error(
                    token.span,
                    "E0113",
                    "expected string test description after `test`",
                ));
            }
        };
        let body = self.block()?;
        Ok(TestDeclaration {
            description,
            span: start.through(body.span),
            body,
        })
    }

    /// Parses one function after its `fn` keyword was consumed.
    pub(super) fn function_from_start(
        &mut self,
        start: SourceSpan<'a>,
        visibility: FunctionVisibility,
    ) -> Result<FunctionDeclaration<'a>, CompileDiagnostic<'a>> {
        let (name, parameters, return_type) = self.function_header_from_start()?;
        let body = self.block()?;
        Ok(FunctionDeclaration {
            visibility,
            name,
            parameters,
            return_type,
            span: start.through(body.span),
            body,
        })
    }

    /// Parses the shared signature portion of a function or trait method.
    pub(super) fn function_header_from_start(
        &mut self,
    ) -> Result<
        (
            Identifier<'a>,
            Vec<Parameter<'a>>,
            Option<TypeAnnotation<'a>>,
        ),
        CompileDiagnostic<'a>,
    > {
        let name = self.identifier("expected function name")?;
        self.expect_simple(TokenKind::LeftParen, "expected `(` after function name")?;
        let mut parameters = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                let name = self.identifier("expected parameter name")?;
                let type_annotation = if self.matches(&TokenKind::Colon) {
                    Some(self.type_annotation()?)
                } else {
                    None
                };
                let variadic = self.matches(&TokenKind::Ellipsis);
                parameters.push(Parameter {
                    name,
                    type_annotation,
                    variadic,
                });
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightParen) {
                    break;
                }
            }
        }
        self.expect_simple(TokenKind::RightParen, "expected `)` after parameters")?;
        let return_type = if self.matches(&TokenKind::Arrow) {
            Some(self.type_annotation()?)
        } else {
            None
        };
        Ok((name, parameters, return_type))
    }

    /// Parses one nominal Object type declaration.
    pub(super) fn type_declaration(
        &mut self,
    ) -> Result<TypeDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Type, "expected `type` at module level")?
            .span;
        let name = self.identifier("expected type name after `type`")?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after type name")?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let field_start = self.peek().span;
            let name = self.identifier("expected type field name")?;
            let type_annotation = if self.matches(&TokenKind::Colon) {
                Some(self.type_annotation()?)
            } else {
                None
            };
            let end = type_annotation
                .as_ref()
                .map_or(name.span, |annotation| annotation.span);
            fields.push(TypeField {
                name,
                type_annotation,
                span: field_start.through(end),
            });
            if !self.matches(&TokenKind::Comma) {
                if !self.check(&TokenKind::RightBrace) {
                    return Err(self.error(
                        self.peek().span,
                        "E0103",
                        "expected `,` or `}` after type field",
                    ));
                }
                break;
            }
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after type fields")?
            .span;
        Ok(TypeDeclaration {
            name,
            fields,
            span: start.through(end),
        })
    }

    /// Parses one trait declaration with required signatures and default method bodies.
    pub(super) fn trait_declaration(
        &mut self,
    ) -> Result<TraitDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Trait, "expected `trait` at module level")?
            .span;
        let name = self.identifier("expected trait name after `trait`")?;
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after trait name")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let method_start = self
                .expect_simple(TokenKind::Fn, "expected `fn` inside trait block")?
                .span;
            let (name, parameters, return_type) = self.function_header_from_start()?;
            let (body, end) = if self.matches(&TokenKind::Semicolon) {
                (None, self.previous().span)
            } else {
                let body = self.block()?;
                let end = body.span;
                (Some(body), end)
            };
            methods.push(TraitMethodDeclaration {
                name,
                parameters,
                return_type,
                body,
                span: method_start.through(end),
            });
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after trait methods")?
            .span;
        Ok(TraitDeclaration {
            name,
            methods,
            span: start.through(end),
        })
    }

    /// Parses one inherent or trait implementation block for a nominal type.
    pub(super) fn implementation(&mut self) -> Result<ImplDeclaration<'a>, CompileDiagnostic<'a>> {
        let start = self
            .expect_simple(TokenKind::Impl, "expected `impl` at module level")?
            .span;
        let first_name = self.qualified_identifier("expected trait or type name after `impl`")?;
        let (trait_name, type_name) = if self.matches(&TokenKind::For) {
            (
                Some(first_name),
                self.qualified_identifier("expected type name after `for")?,
            )
        } else {
            (None, first_name)
        };
        self.expect_simple(TokenKind::LeftBrace, "expected `{` after impl type name")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.at_end() {
            let method_start = self
                .expect_simple(TokenKind::Fn, "expected `fn` inside impl block")?
                .span;
            methods.push(self.function_from_start(method_start, FunctionVisibility::Private)?);
        }
        let end = self
            .expect_simple(TokenKind::RightBrace, "expected `}` after impl methods")?
            .span;
        Ok(ImplDeclaration {
            trait_name,
            type_name,
            methods,
            span: start.through(end),
        })
    }

    /// Parses one non-empty `Type | Type` annotation in a function boundary position.
    pub(super) fn type_annotation(&mut self) -> Result<TypeAnnotation<'a>, CompileDiagnostic<'a>> {
        let first = self.type_name()?;
        let mut members = vec![first];
        while self.matches(&TokenKind::Pipe) {
            members.push(self.type_name()?);
        }
        let span = members
            .first()
            .map_or_else(|| SourceSpan::empty("<unknown>"), |member| member.span)
            .through(
                members
                    .last()
                    .map_or_else(|| SourceSpan::empty("<unknown>"), |member| member.span),
            );
        Ok(TypeAnnotation { members, span })
    }

    /// Parses one source-visible type name.
    pub(super) fn type_name(&mut self) -> Result<TypeName<'a>, CompileDiagnostic<'a>> {
        let (mut name, mut span) = self.type_name_component("expected type name")?;
        if self.matches(&TokenKind::DoubleColon) {
            let (member, member_span) =
                self.type_name_component("expected type name after `::`")?;
            name.push_str("::");
            name.push_str(&member);
            span = span.through(member_span);
        }
        let argument = if self.matches(&TokenKind::Less) {
            let argument = self.type_annotation()?;
            let end = self
                .expect_simple(
                    TokenKind::Greater,
                    "expected `>` after generic type argument",
                )?
                .span;
            span = span.through(end);
            Some(Box::new(argument))
        } else {
            None
        };
        Ok(TypeName {
            name,
            argument,
            span,
        })
    }

    /// Parses one identifier or reserved built-in type-name token.
    pub(super) fn type_name_component(
        &mut self,
        message: &str,
    ) -> Result<(String, SourceSpan<'a>), CompileDiagnostic<'a>> {
        let token = self.advance().clone();
        let name = match token.kind {
            TokenKind::Identifier(name) => name,
            TokenKind::None => "None".to_owned(),
            TokenKind::Error => "Error".to_owned(),
            _ => return Err(self.error(token.span, "E0111", message)),
        };
        Ok((name, token.span))
    }
}
