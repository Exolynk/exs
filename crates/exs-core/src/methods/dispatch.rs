//! Dispatch of built-in receiver methods by their `exs_layout::BuiltinMethod` index.

use exs_layout::BuiltinMethod;

use crate::bytes::bytes_object;
use crate::lists::list_object;
use crate::memory::Memory;
use crate::methods::bytes::Relation as BytesRelation;
use crate::methods::clone::deep_clone;
use crate::methods::numbers::type_error;
use crate::methods::numbers::{
    abs, boolean, clamp, euclidean, float_predicate, float_unary, gcd_or_lcm, int_unary,
    min_or_max, pow, signum, to_float, to_int,
};
use crate::methods::objects::Collect;
use crate::methods::strings::{Relation, Trim};
use crate::methods::values::{ErrorPart, error_part, is_empty, length, operator, render};
use crate::methods::{bytes, lists, objects, strings};
use crate::numbers::Arithmetic;
use crate::strings::string_object;
use exs_layout::ValueRef;

/// Calls the built-in method `method` on `receiver` with up to two arguments; all three
/// values must be rooted.
///
/// Returns `None` when memory is exhausted or `method` is one the core library does not
/// implement yet, which the compiler never emits.
pub fn call_builtin(
    memory: &mut impl Memory,
    vm: u32,
    method: BuiltinMethod,
    receiver: u64,
    first: u64,
    second: u64,
) -> Option<u64> {
    let float = |memory: &mut _, operation: fn(f64) -> f64, message| {
        float_unary(memory, vm, receiver, operation, message)
    };
    let predicate = |memory: &mut _, test: fn(f64) -> bool, message| {
        float_predicate(memory, vm, receiver, test, message)
    };
    match method {
        BuiltinMethod::Add => operator(memory, vm, Some(Arithmetic::Add), receiver, first),
        BuiltinMethod::Sub => operator(memory, vm, Some(Arithmetic::Subtract), receiver, first),
        BuiltinMethod::Mul => operator(memory, vm, Some(Arithmetic::Multiply), receiver, first),
        BuiltinMethod::Div => operator(memory, vm, None, receiver, first),
        BuiltinMethod::DivEuclid => euclidean(memory, vm, receiver, first, false),
        BuiltinMethod::RemEuclid => euclidean(memory, vm, receiver, first, true),
        BuiltinMethod::Rem => crate::numbers::remainder(memory, vm, receiver, first),
        BuiltinMethod::ToString => render(memory, vm, receiver),
        BuiltinMethod::Debug => match crate::render::debug_text(memory, vm, receiver)? {
            text if text == exs_layout::ValueRef::NONE.raw() => render(memory, vm, receiver),
            text => Some(text),
        },
        BuiltinMethod::Abs => abs(memory, vm, receiver),
        BuiltinMethod::Floor => float(memory, libm::floor, "floor requires a Float receiver"),
        BuiltinMethod::Ceil => float(memory, libm::ceil, "ceil requires a Float receiver"),
        BuiltinMethod::Round => float(memory, libm::round, "round requires a Float receiver"),
        BuiltinMethod::Trunc => float(memory, libm::trunc, "trunc requires a Float receiver"),
        BuiltinMethod::Fract => float(
            memory,
            |value| value - libm::trunc(value),
            "fract requires a Float receiver",
        ),
        BuiltinMethod::Sqrt => float(memory, libm::sqrt, "sqrt requires a Float receiver"),
        BuiltinMethod::Signum => int_unary(
            memory,
            vm,
            receiver,
            signum,
            "signum requires an Int receiver",
        ),
        BuiltinMethod::IsEven => int_unary(
            memory,
            vm,
            receiver,
            |value| boolean(value % 2 == 0),
            "is_even requires an Int receiver",
        ),
        BuiltinMethod::IsOdd => int_unary(
            memory,
            vm,
            receiver,
            |value| boolean(value % 2 != 0),
            "is_odd requires an Int receiver",
        ),
        BuiltinMethod::Gcd => gcd_or_lcm(memory, vm, receiver, first, false),
        BuiltinMethod::Lcm => gcd_or_lcm(memory, vm, receiver, first, true),
        BuiltinMethod::Pow => pow(memory, vm, receiver, first),
        BuiltinMethod::Min => min_or_max(memory, vm, receiver, first, false),
        BuiltinMethod::Max => min_or_max(memory, vm, receiver, first, true),
        BuiltinMethod::Clamp => clamp(memory, vm, receiver, first, second),
        BuiltinMethod::ToFloat => to_float(memory, vm, receiver),
        BuiltinMethod::ToInt => to_int(memory, vm, receiver),
        BuiltinMethod::IsNan => predicate(memory, f64::is_nan, "is_nan requires a Float receiver"),
        BuiltinMethod::IsFinite => predicate(
            memory,
            f64::is_finite,
            "is_finite requires a Float receiver",
        ),
        BuiltinMethod::IsInfinite => predicate(
            memory,
            f64::is_infinite,
            "is_infinite requires a Float receiver",
        ),
        BuiltinMethod::Length => length(memory, vm, receiver),
        BuiltinMethod::IsEmpty => is_empty(memory, vm, receiver),
        BuiltinMethod::Kind => error_part(memory, vm, receiver, ErrorPart::Kind),
        BuiltinMethod::Message => error_part(memory, vm, receiver, ErrorPart::Message),
        BuiltinMethod::Data => error_part(memory, vm, receiver, ErrorPart::Data),
        BuiltinMethod::Cause => error_part(memory, vm, receiver, ErrorPart::Cause),
        BuiltinMethod::Push => lists::push(memory, vm, receiver, first),
        BuiltinMethod::Pop => lists::pop(memory, vm, receiver),
        BuiltinMethod::Get if list_object(receiver).is_some() => {
            lists::get(memory, vm, receiver, first)
        }
        BuiltinMethod::Get if objects::is_object(memory, receiver) => {
            objects::get(memory, vm, receiver, first)
        }
        BuiltinMethod::Get => type_error(
            memory,
            vm,
            "get requires a List or Object receiver",
            receiver,
        ),
        BuiltinMethod::First => lists::end(memory, vm, receiver, false),
        BuiltinMethod::Last => lists::end(memory, vm, receiver, true),
        BuiltinMethod::Extend => lists::extend(memory, vm, receiver, first),
        BuiltinMethod::Reverse => lists::reverse(memory, vm, receiver, false),
        BuiltinMethod::Reversed => lists::reverse(memory, vm, receiver, true),
        BuiltinMethod::IndexOf => lists::index_of(memory, vm, receiver, first, false),
        BuiltinMethod::LastIndexOf => lists::index_of(memory, vm, receiver, first, true),
        BuiltinMethod::Join => lists::join(memory, vm, receiver, first),
        BuiltinMethod::Insert => lists::insert(memory, vm, receiver, first, second),
        BuiltinMethod::Remove => lists::remove(memory, vm, receiver, first),
        BuiltinMethod::Clear => {
            if let Some(list) = list_object(receiver) {
                lists::clear(memory, list);
            } else if objects::is_object(memory, receiver) {
                return objects::clear(memory, vm, receiver);
            } else {
                return type_error(
                    memory,
                    vm,
                    "clear requires a List or Object receiver",
                    receiver,
                );
            }
            Some(ValueRef::NONE.raw())
        }
        BuiltinMethod::Has => objects::has(memory, vm, receiver, first),
        BuiltinMethod::Delete => objects::delete(memory, vm, receiver, first),
        BuiltinMethod::Keys => objects::collect(memory, vm, receiver, Collect::Keys),
        BuiltinMethod::Values => objects::collect(memory, vm, receiver, Collect::Values),
        BuiltinMethod::Entries => objects::collect(memory, vm, receiver, Collect::Entries),
        BuiltinMethod::GetOr => objects::get_or(memory, vm, receiver, first, second),
        BuiltinMethod::Merge => objects::merge(memory, vm, receiver, first),
        BuiltinMethod::Contains if bytes_object(receiver).is_some() => {
            bytes::relation(memory, vm, receiver, first, BytesRelation::Contains)
        }
        BuiltinMethod::Contains if string_object(receiver).is_some() => {
            strings::relation(memory, vm, receiver, first, Relation::Contains)
        }
        BuiltinMethod::Contains if list_object(receiver).is_some() => {
            lists::contains(memory, vm, receiver, first)
        }
        BuiltinMethod::Contains => type_error(
            memory,
            vm,
            "contains requires a Bytes, List, or String receiver",
            receiver,
        ),
        BuiltinMethod::Slice if bytes_object(receiver).is_some() => {
            bytes::slice(memory, vm, receiver, first, second)
        }
        BuiltinMethod::Slice if string_object(receiver).is_some() => {
            strings::slice(memory, vm, receiver, first, second)
        }
        BuiltinMethod::Slice if list_object(receiver).is_some() => {
            lists::slice(memory, vm, receiver, first, second)
        }
        BuiltinMethod::Slice => type_error(
            memory,
            vm,
            "slice requires a Bytes, List, or String receiver",
            receiver,
        ),
        BuiltinMethod::StartsWith if bytes_object(receiver).is_some() => {
            bytes::relation(memory, vm, receiver, first, BytesRelation::StartsWith)
        }
        BuiltinMethod::StartsWith if string_object(receiver).is_some() => {
            strings::relation(memory, vm, receiver, first, Relation::StartsWith)
        }
        BuiltinMethod::StartsWith => type_error(
            memory,
            vm,
            "starts_with requires a Bytes or String receiver",
            receiver,
        ),
        BuiltinMethod::EndsWith if bytes_object(receiver).is_some() => {
            bytes::relation(memory, vm, receiver, first, BytesRelation::EndsWith)
        }
        BuiltinMethod::EndsWith if string_object(receiver).is_some() => {
            strings::relation(memory, vm, receiver, first, Relation::EndsWith)
        }
        BuiltinMethod::EndsWith => type_error(
            memory,
            vm,
            "ends_with requires a Bytes or String receiver",
            receiver,
        ),
        BuiltinMethod::Repeat if bytes_object(receiver).is_some() => {
            bytes::repeat(memory, vm, receiver, first)
        }
        BuiltinMethod::Repeat if string_object(receiver).is_some() => {
            strings::repeat(memory, vm, receiver, first)
        }
        BuiltinMethod::Repeat => type_error(
            memory,
            vm,
            "repeat requires a Bytes or String receiver",
            receiver,
        ),
        BuiltinMethod::Trim => strings::trim(memory, vm, receiver, Trim::Both),
        BuiltinMethod::TrimStart => strings::trim(memory, vm, receiver, Trim::Start),
        BuiltinMethod::TrimEnd => strings::trim(memory, vm, receiver, Trim::End),
        BuiltinMethod::Replace => strings::replace(memory, vm, receiver, first, second),
        BuiltinMethod::Split => strings::split(memory, vm, receiver, first),
        BuiltinMethod::Clone => deep_clone(memory, vm, receiver),
        BuiltinMethod::Compare => crate::values::compare_method(memory, vm, receiver, first),
        BuiltinMethod::ToList => bytes::to_list(memory, vm, receiver),
        BuiltinMethod::Concat => bytes::concat(memory, vm, receiver, first),
        BuiltinMethod::DecodeUtf8 => bytes::decode_utf8(memory, vm, receiver),
        BuiltinMethod::EncodeUtf8 => bytes::from_utf8(memory, vm, receiver),
        BuiltinMethod::ToHex => bytes::to_hex(memory, vm, receiver),
        BuiltinMethod::ToLowercase => strings::change_case(memory, vm, receiver, false),
        BuiltinMethod::ToUppercase => strings::change_case(memory, vm, receiver, true),
        BuiltinMethod::ToBase64 => bytes::to_base64(memory, vm, receiver),
        _ => None,
    }
}
