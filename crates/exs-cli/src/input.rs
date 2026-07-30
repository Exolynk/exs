//! Parsing of command-line values passed to the ExS entry point.

use exs_runner::ExsValue;

/// Parses all positional values that follow a CLI `--` separator.
pub(crate) fn parse_arguments(arguments: &[String]) -> Result<Vec<ExsValue>, String> {
    arguments
        .iter()
        .map(|argument| ValueParser::new(argument).parse())
        .collect()
}

/// Parses one compact CLI value literal into a host-safe ExS value.
struct ValueParser<'input> {
    /// Complete argument text being parsed.
    input: &'input str,
    /// Current byte offset at a UTF-8 character boundary.
    offset: usize,
}

impl<'input> ValueParser<'input> {
    /// Prepares parsing for one raw command-line argument.
    fn new(input: &'input str) -> Self {
        Self { input, offset: 0 }
    }

    /// Parses exactly one value literal without trailing non-whitespace text.
    fn parse(mut self) -> Result<ExsValue, String> {
        self.skip_whitespace();
        if self.at_end() {
            return Ok(ExsValue::String(String::new()));
        }
        let value = self.value()?;
        self.skip_whitespace();
        if self.at_end() {
            Ok(value)
        } else {
            Err(self.error("unexpected trailing input"))
        }
    }

    /// Parses one value according to the CLI value grammar.
    fn value(&mut self) -> Result<ExsValue, String> {
        self.skip_whitespace();
        match self.peek() {
            Some('[') => self.list(),
            Some('{') => self.object(),
            Some('\'') | Some('"') => self.string(),
            Some(_) => self.bare_value(),
            None => Err(self.error("expected a value")),
        }
    }

    /// Parses one comma-separated List literal.
    fn list(&mut self) -> Result<ExsValue, String> {
        self.advance();
        let mut values = Vec::new();
        self.skip_whitespace();
        if self.consume(']') {
            return Ok(ExsValue::List(values));
        }
        loop {
            values.push(self.value()?);
            self.skip_whitespace();
            if self.consume(']') {
                return Ok(ExsValue::List(values));
            }
            if !self.consume(',') {
                return Err(self.error("expected `,` or `]` in List input"));
            }
        }
    }

    /// Parses one comma-separated Object literal with string keys.
    fn object(&mut self) -> Result<ExsValue, String> {
        self.advance();
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.consume('}') {
            return Ok(ExsValue::Object(entries));
        }
        loop {
            let key = self.object_key()?;
            self.skip_whitespace();
            if !self.consume(':') {
                return Err(self.error("expected `:` after Object input key"));
            }
            let value = self.value()?;
            entries.push((key, value));
            self.skip_whitespace();
            if self.consume('}') {
                return Ok(ExsValue::Object(entries));
            }
            if !self.consume(',') {
                return Err(self.error("expected `,` or `}` in Object input"));
            }
        }
    }

    /// Parses a quoted or bare Object key.
    fn object_key(&mut self) -> Result<String, String> {
        self.skip_whitespace();
        if matches!(self.peek(), Some('\'') | Some('"')) {
            let ExsValue::String(key) = self.string()? else {
                return Err(self.error("Object input key must be a String"));
            };
            return Ok(key);
        }
        let start = self.offset;
        while matches!(self.peek(), Some(character) if character != ':' && character != ',' && character != '}')
        {
            self.advance();
        }
        let key = self.input[start..self.offset].trim();
        if key.is_empty() {
            Err(self.error("expected an Object input key"))
        } else {
            Ok(key.to_owned())
        }
    }

    /// Parses one single- or double-quoted String literal.
    fn string(&mut self) -> Result<ExsValue, String> {
        let quote = self
            .advance()
            .ok_or_else(|| self.error("expected a quote"))?;
        let mut output = String::new();
        loop {
            let character = self
                .advance()
                .ok_or_else(|| self.error("unterminated String input"))?;
            if character == quote {
                return Ok(ExsValue::String(output));
            }
            if character != '\\' {
                output.push(character);
                continue;
            }
            let escaped = self
                .advance()
                .ok_or_else(|| self.error("unterminated String escape"))?;
            let character = match escaped {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                '0' => '\0',
                '\\' => '\\',
                '\'' => '\'',
                '"' => '"',
                _ => return Err(self.error("unsupported String escape")),
            };
            output.push(character);
        }
    }

    /// Parses a keyword, number, or bare String value.
    fn bare_value(&mut self) -> Result<ExsValue, String> {
        let start = self.offset;
        while matches!(self.peek(), Some(character) if character != ',' && character != ']' && character != '}')
        {
            self.advance();
        }
        let value = self.input[start..self.offset].trim();
        if value.is_empty() {
            return Err(self.error("expected a value"));
        }
        match value {
            "None" => Ok(ExsValue::None),
            "true" => Ok(ExsValue::Bool(true)),
            "false" => Ok(ExsValue::Bool(false)),
            _ => match value.parse::<i64>() {
                Ok(value) => Ok(ExsValue::Int(value)),
                Err(_) => match value.parse::<f64>() {
                    Ok(value) => Ok(ExsValue::Float(value)),
                    Err(_) => Ok(ExsValue::String(value.to_owned())),
                },
            },
        }
    }

    /// Skips source whitespace between structural tokens.
    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(character) if character.is_whitespace()) {
            self.advance();
        }
    }

    /// Consumes one expected structural character when present.
    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    /// Returns the character at the current parser offset.
    fn peek(&self) -> Option<char> {
        self.input[self.offset..].chars().next()
    }

    /// Advances one Unicode scalar and returns it.
    fn advance(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.offset += character.len_utf8();
        Some(character)
    }

    /// Returns whether the parser consumed all input text.
    fn at_end(&self) -> bool {
        self.offset == self.input.len()
    }

    /// Formats one input-literal diagnostic with its current byte offset.
    fn error(&self, message: &str) -> String {
        format!("invalid input value at byte {}: {message}", self.offset)
    }
}
