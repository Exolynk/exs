//! Static functions of built-in types that the compiler calls by id through the core library.

/// One static function of a built-in type, identified across the compiler-core boundary by its
/// index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum StaticFunction {
    /// `Int::parse(text)`.
    IntParse,
    /// `Float::parse(text)`.
    FloatParse,
    /// `Math::pi()`.
    MathPi,
    /// `Math::tau()`.
    MathTau,
    /// `Math::e()`.
    MathE,
    /// `Math::sin(x)`.
    MathSin,
    /// `Math::cos(x)`.
    MathCos,
    /// `Math::tan(x)`.
    MathTan,
    /// `Math::asin(x)`.
    MathAsin,
    /// `Math::acos(x)`.
    MathAcos,
    /// `Math::atan(x)`.
    MathAtan,
    /// `Math::exp(x)`.
    MathExp,
    /// `Math::ln(x)`.
    MathLn,
    /// `Math::log2(x)`.
    MathLog2,
    /// `Math::log10(x)`.
    MathLog10,
    /// `Math::atan2(y, x)`.
    MathAtan2,
    /// `Math::hypot(x, y)`.
    MathHypot,
}

/// Every static function in index order.
const ALL: [StaticFunction; 17] = [
    StaticFunction::IntParse,
    StaticFunction::FloatParse,
    StaticFunction::MathPi,
    StaticFunction::MathTau,
    StaticFunction::MathE,
    StaticFunction::MathSin,
    StaticFunction::MathCos,
    StaticFunction::MathTan,
    StaticFunction::MathAsin,
    StaticFunction::MathAcos,
    StaticFunction::MathAtan,
    StaticFunction::MathExp,
    StaticFunction::MathLn,
    StaticFunction::MathLog2,
    StaticFunction::MathLog10,
    StaticFunction::MathAtan2,
    StaticFunction::MathHypot,
];

impl StaticFunction {
    /// Resolves the function `type_name::name`, if it is one.
    #[must_use]
    pub fn from_path(type_name: &str, name: &str) -> Option<Self> {
        ALL.iter()
            .copied()
            .find(|function| function.path() == (type_name, name))
    }

    /// Resolves one index produced by [`StaticFunction::id`].
    #[must_use]
    pub fn from_id(id: u32) -> Option<Self> {
        ALL.get(usize::try_from(id).ok()?).copied()
    }

    /// Returns the index passed across the compiler-core boundary.
    #[must_use]
    pub const fn id(self) -> u32 {
        self as u32
    }

    /// Returns the type and function name.
    #[must_use]
    pub const fn path(self) -> (&'static str, &'static str) {
        match self {
            Self::IntParse => ("Int", "parse"),
            Self::FloatParse => ("Float", "parse"),
            Self::MathPi => ("Math", "pi"),
            Self::MathTau => ("Math", "tau"),
            Self::MathE => ("Math", "e"),
            Self::MathSin => ("Math", "sin"),
            Self::MathCos => ("Math", "cos"),
            Self::MathTan => ("Math", "tan"),
            Self::MathAsin => ("Math", "asin"),
            Self::MathAcos => ("Math", "acos"),
            Self::MathAtan => ("Math", "atan"),
            Self::MathExp => ("Math", "exp"),
            Self::MathLn => ("Math", "ln"),
            Self::MathLog2 => ("Math", "log2"),
            Self::MathLog10 => ("Math", "log10"),
            Self::MathAtan2 => ("Math", "atan2"),
            Self::MathHypot => ("Math", "hypot"),
        }
    }

    /// Returns the number of arguments.
    #[must_use]
    pub const fn arity(self) -> usize {
        match self {
            Self::MathPi | Self::MathTau | Self::MathE => 0,
            Self::MathAtan2 | Self::MathHypot => 2,
            _ => 1,
        }
    }
}
