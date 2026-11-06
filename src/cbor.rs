//! RFC 8949 core-deterministic CBOR for opaque value leaves.
//!
//! Map keys are ordered lexicographically by their encoded bytes, not by the
//! older length-first CBOR ordering. Decoding verifies the original bytes, so
//! it never silently accepts an alternate representation of the same value.

use std::{collections::HashSet, fmt};

use half::f16;

use crate::limits::MAX_VALUE_DEPTH;

/// An invalid, non-deterministic, or unsupported CBOR value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CborError(String);

impl fmt::Display for CborError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn error(message: impl Into<String>) -> CborError {
    CborError(message.into())
}

impl std::error::Error for CborError {}

/// A CBOR data item. Maps retain arbitrary CBOR keys, including compound keys.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    /// CBOR major type 0.
    Unsigned(u64),
    /// CBOR major type 1, with the stored argument `n` representing `-1-n`.
    Negative(u64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Tag(u64, Box<Value>),
    Simple(u8),
    Float(f64),
}

/// Encode one data item using RFC 8949 core-deterministic rules.
///
/// CBOR NaNs are normalized to binary16 `0x7e00`; unlike float text leaves,
/// signed zero is preserved. Duplicate keys, including signed-zero variants,
/// are an error.
pub(crate) fn encode(value: &Value) -> Result<Vec<u8>, CborError> {
    let mut output = Vec::new();
    encode_item(value, &mut output, 0)?;
    Ok(output)
}

/// Decode exactly one core-deterministic CBOR item.
pub(crate) fn decode(bytes: &[u8]) -> Result<Value, CborError> {
    let mut parser = Parser { bytes, offset: 0 };
    let value = parser.item(0)?;
    if parser.offset != bytes.len() {
        return Err(error("trailing bytes after CBOR item"));
    }
    Ok(value)
}

/// Validate an opaque CBOR payload without changing it.
///
/// Only the `value`-gated schema read/write paths call this; it is the
/// pass-through used to validate opaque CBOR without materializing it.
#[cfg_attr(not(feature = "value"), allow(dead_code))]
pub(crate) fn validate(bytes: &[u8]) -> Result<(), CborError> {
    decode(bytes).map(|_| ())
}

fn head(output: &mut Vec<u8>, major: u8, value: u64) {
    let prefix = major << 5;
    match value {
        0..=23 => output.push(prefix | value as u8),
        24..=0xff => output.extend_from_slice(&[prefix | 24, value as u8]),
        0x100..=0xffff => {
            output.push(prefix | 25);
            output.extend_from_slice(&(value as u16).to_be_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            output.push(prefix | 26);
            output.extend_from_slice(&(value as u32).to_be_bytes());
        }
        _ => {
            output.push(prefix | 27);
            output.extend_from_slice(&value.to_be_bytes());
        }
    }
}

fn encode_float(value: f64, output: &mut Vec<u8>) {
    if value.is_nan() {
        output.extend_from_slice(&[0xf9, 0x7e, 0x00]);
        return;
    }
    let half = f16::from_f64(value);
    if half.to_f64().to_bits() == value.to_bits() {
        output.push(0xf9);
        output.extend_from_slice(&half.to_bits().to_be_bytes());
        return;
    }
    let single = value as f32;
    if f64::from(single).to_bits() == value.to_bits() {
        output.push(0xfa);
        output.extend_from_slice(&single.to_bits().to_be_bytes());
        return;
    }
    output.push(0xfb);
    output.extend_from_slice(&value.to_bits().to_be_bytes());
}

fn validate_tag(tag: u64, value: &Value) -> Result<(), CborError> {
    if matches!(tag, 2 | 3) {
        match value {
            Value::Bytes(bytes) if bytes.len() > 8 && bytes.first() != Some(&0) => {}
            Value::Bytes(_) => return Err(error("non-canonical CBOR bignum")),
            _ => return Err(error("CBOR bignum requires a byte string")),
        }
    }
    Ok(())
}

fn contains_negative_zero(value: &Value) -> bool {
    match value {
        Value::Float(number) => number.to_bits() == (-0.0f64).to_bits(),
        Value::Array(items) => items.iter().any(contains_negative_zero),
        Value::Map(entries) => entries
            .iter()
            .any(|(key, value)| contains_negative_zero(key) || contains_negative_zero(value)),
        Value::Tag(_, inner) => contains_negative_zero(inner),
        _ => false,
    }
}

// RFC 8949 map-key equivalence identifies both signs of zero, including in
// compound keys. Keep the original encoding for ordering and output.
fn key_identity(value: &Value, depth: usize) -> Result<Vec<u8>, CborError> {
    fn normalize_zero(value: &mut Value) {
        match value {
            Value::Float(number) if number.to_bits() == (-0.0f64).to_bits() => *number = 0.0,
            Value::Array(items) => {
                for item in items {
                    normalize_zero(item);
                }
            }
            Value::Map(entries) => {
                for (key, value) in entries {
                    normalize_zero(key);
                    normalize_zero(value);
                }
            }
            Value::Tag(_, inner) => normalize_zero(inner),
            _ => {}
        }
    }

    let mut normalized = value.clone();
    normalize_zero(&mut normalized);
    let mut identity = Vec::new();
    encode_item(&normalized, &mut identity, depth)?;
    Ok(identity)
}

fn encode_item(value: &Value, output: &mut Vec<u8>, depth: usize) -> Result<(), CborError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(error("maximum CBOR nesting depth exceeded"));
    }
    match value {
        Value::Unsigned(value) => head(output, 0, *value),
        Value::Negative(value) => head(output, 1, *value),
        Value::Bytes(bytes) => {
            head(output, 2, bytes.len() as u64);
            output.extend_from_slice(bytes);
        }
        Value::Text(text) => {
            head(output, 3, text.len() as u64);
            output.extend_from_slice(text.as_bytes());
        }
        Value::Array(items) => {
            head(output, 4, items.len() as u64);
            for item in items {
                encode_item(item, output, depth + 1)?;
            }
        }
        Value::Map(items) => {
            let mut entries = Vec::with_capacity(items.len());
            let mut normalized = Vec::new();
            for (key, value) in items {
                let mut encoded = Vec::new();
                encode_item(key, &mut encoded, depth + 1)?;
                if contains_negative_zero(key) {
                    normalized.push(key_identity(key, depth + 1)?);
                }
                entries.push((encoded, value));
            }
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
                return Err(error("duplicate CBOR map key"));
            }
            if !normalized.is_empty() {
                let mut identities = HashSet::new();
                identities.extend(entries.iter().map(|(key, _)| key.as_slice()));
                for identity in &normalized {
                    if !identities.insert(identity) {
                        return Err(error("duplicate CBOR map key"));
                    }
                }
            }
            head(output, 5, entries.len() as u64);
            for (key, value) in entries {
                output.extend_from_slice(&key);
                encode_item(value, output, depth + 1)?;
            }
        }
        Value::Tag(tag, value) => {
            validate_tag(*tag, value)?;
            head(output, 6, *tag);
            encode_item(value, output, depth + 1)?;
        }
        Value::Simple(simple @ 0..=23) => output.push(0xe0 | simple),
        Value::Simple(simple @ 32..=u8::MAX) => output.extend_from_slice(&[0xf8, *simple]),
        Value::Simple(_) => return Err(error("reserved CBOR simple value")),
        Value::Float(value) => encode_float(*value, output),
    }
    Ok(())
}

struct Parser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Parser<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], CborError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(error("CBOR length overflow"))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or(error("truncated CBOR item"))?;
        self.offset = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, CborError> {
        self.take(1)?
            .first()
            .copied()
            .ok_or(error("truncated CBOR item"))
    }

    fn argument(&mut self, additional: u8) -> Result<u64, CborError> {
        let (value, minimum) = match additional {
            0..=23 => return Ok(u64::from(additional)),
            24 => (u64::from(self.byte()?), 24),
            25 => (
                u64::from(u16::from_be_bytes(
                    self.take(2)?
                        .try_into()
                        .map_err(|_error| error("truncated CBOR argument"))?,
                )),
                0x100,
            ),
            26 => (
                u64::from(u32::from_be_bytes(
                    self.take(4)?
                        .try_into()
                        .map_err(|_error| error("truncated CBOR argument"))?,
                )),
                0x1_0000,
            ),
            27 => (
                u64::from_be_bytes(
                    self.take(8)?
                        .try_into()
                        .map_err(|_error| error("truncated CBOR argument"))?,
                ),
                0x1_0000_0000,
            ),
            31 => return Err(error("indefinite-length CBOR is not deterministic")),
            _ => return Err(error("reserved CBOR additional information")),
        };
        if value < minimum {
            return Err(error("non-minimal CBOR argument"));
        }
        Ok(value)
    }

    fn len(&mut self, additional: u8) -> Result<usize, CborError> {
        usize::try_from(self.argument(additional)?).map_err(|_error| error("CBOR length overflow"))
    }

    fn item(&mut self, depth: usize) -> Result<Value, CborError> {
        if depth > MAX_VALUE_DEPTH {
            return Err(error("maximum CBOR nesting depth exceeded"));
        }
        let initial = self.byte()?;
        let major = initial >> 5;
        let additional = initial & 0x1f;
        match major {
            0 => Ok(Value::Unsigned(self.argument(additional)?)),
            1 => Ok(Value::Negative(self.argument(additional)?)),
            2 => {
                let len = self.len(additional)?;
                Ok(Value::Bytes(self.take(len)?.to_vec()))
            }
            3 => {
                let len = self.len(additional)?;
                let bytes = self.take(len)?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|_error| error("invalid UTF-8 in CBOR text"))?;
                Ok(Value::Text(text.to_owned()))
            }
            4 => {
                let len = self.len(additional)?;
                if len > self.bytes.len() - self.offset {
                    return Err(error("truncated CBOR array"));
                }
                let mut values = Vec::new();
                for _ in 0..len {
                    values.push(self.item(depth + 1)?);
                }
                Ok(Value::Array(values))
            }
            5 => {
                let len = self.len(additional)?;
                if len > (self.bytes.len() - self.offset) / 2 {
                    return Err(error("truncated CBOR map"));
                }
                let mut entries = Vec::new();
                let mut previous: Option<&[u8]> = None;
                let mut identities = HashSet::new();
                for _ in 0..len {
                    let start = self.offset;
                    let key = self.item(depth + 1)?;
                    let encoded = self
                        .bytes
                        .get(start..self.offset)
                        .ok_or(error("invalid CBOR map key"))?;
                    if previous.is_some_and(|prior| prior >= encoded) {
                        return Err(error("duplicate or out-of-order CBOR map key"));
                    }
                    identities.insert(encoded.to_vec());
                    if contains_negative_zero(&key)
                        && !identities.insert(key_identity(&key, depth + 1)?)
                    {
                        return Err(error("duplicate CBOR map key"));
                    }
                    previous = Some(encoded);
                    let value = self.item(depth + 1)?;
                    entries.push((key, value));
                }
                Ok(Value::Map(entries))
            }
            6 => {
                let tag = self.argument(additional)?;
                let value = self.item(depth + 1)?;
                validate_tag(tag, &value)?;
                Ok(Value::Tag(tag, Box::new(value)))
            }
            7 => self.simple_or_float(additional),
            _ => Err(error("invalid CBOR major type")),
        }
    }

    fn simple_or_float(&mut self, additional: u8) -> Result<Value, CborError> {
        match additional {
            0..=23 => Ok(Value::Simple(additional)),
            24 => {
                let value = self.byte()?;
                if value < 32 {
                    return Err(error("non-minimal or reserved CBOR simple value"));
                }
                Ok(Value::Simple(value))
            }
            25..=27 => {
                let mut original = vec![0xe0 | additional];
                let value = match additional {
                    25 => {
                        let bytes = self.take(2)?;
                        original.extend_from_slice(bytes);
                        f16::from_bits(u16::from_be_bytes(
                            bytes
                                .try_into()
                                .map_err(|_error| error("truncated CBOR float"))?,
                        ))
                        .to_f64()
                    }
                    26 => {
                        let bytes = self.take(4)?;
                        original.extend_from_slice(bytes);
                        f64::from(f32::from_bits(u32::from_be_bytes(
                            bytes
                                .try_into()
                                .map_err(|_error| error("truncated CBOR float"))?,
                        )))
                    }
                    27 => {
                        let bytes = self.take(8)?;
                        original.extend_from_slice(bytes);
                        f64::from_bits(u64::from_be_bytes(
                            bytes
                                .try_into()
                                .map_err(|_error| error("truncated CBOR float"))?,
                        ))
                    }
                    _ => return Err(error("invalid CBOR float")),
                };
                let mut canonical = Vec::new();
                encode_float(value, &mut canonical);
                if canonical != original {
                    return Err(error("non-canonical CBOR float"));
                }
                Ok(Value::Float(value))
            }
            31 => Err(error("unexpected CBOR break")),
            _ => Err(error("reserved CBOR additional information")),
        }
    }
}

/// Encode a reflected value, preserving non-string map keys and 128-bit integers.
pub(crate) fn encode_peek(peek: facet::Peek<'_, '_>) -> Result<Vec<u8>, CborError> {
    encode(&from_peek(peek, 0)?)
}

fn unsigned(value: u128) -> Value {
    if let Ok(value) = u64::try_from(value) {
        return Value::Unsigned(value);
    }
    let bytes = value.to_be_bytes();
    let first = bytes
        .iter()
        .position(|&byte| byte != 0)
        .unwrap_or(bytes.len());
    Value::Tag(2, Box::new(Value::Bytes(bytes[first..].to_vec())))
}

fn signed(value: i128) -> Value {
    if value >= 0 {
        return unsigned(value as u128);
    }
    let argument = (-1 - value) as u128;
    if let Ok(argument) = u64::try_from(argument) {
        return Value::Negative(argument);
    }
    let bytes = argument.to_be_bytes();
    let first = bytes
        .iter()
        .position(|&byte| byte != 0)
        .unwrap_or(bytes.len());
    Value::Tag(3, Box::new(Value::Bytes(bytes[first..].to_vec())))
}

fn from_peek(peek: facet::Peek<'_, '_>, depth: usize) -> Result<Value, CborError> {
    use facet::{Def, ScalarType, StructKind, Type, UserType};

    if depth > MAX_VALUE_DEPTH {
        return Err(error("maximum CBOR nesting depth exceeded"));
    }
    let peek = peek.innermost_peek();
    let shape = peek.shape();
    if shape.is_type::<facet_value::Value>() {
        return from_dynamic(
            peek.get::<facet_value::Value>()
                .map_err(|e| error(e.to_string()))?,
            depth,
        );
    }
    if let Some(scalar) = shape.scalar_type() {
        macro_rules! get {
            ($t:ty) => {
                *peek.get::<$t>().map_err(|e| error(e.to_string()))?
            };
        }
        return Ok(match scalar {
            ScalarType::Unit => Value::Simple(22),
            ScalarType::Bool => Value::Simple(if get!(bool) { 21 } else { 20 }),
            ScalarType::Char => Value::Text(get!(char).to_string()),
            ScalarType::Str | ScalarType::String | ScalarType::CowStr => Value::Text(
                peek.as_str()
                    .ok_or_else(|| error("unreadable CBOR text"))?
                    .to_owned(),
            ),
            ScalarType::F32 => Value::Float(f64::from(get!(f32))),
            ScalarType::F64 => Value::Float(get!(f64)),
            ScalarType::U8 => unsigned(u128::from(get!(u8))),
            ScalarType::U16 => unsigned(u128::from(get!(u16))),
            ScalarType::U32 => unsigned(u128::from(get!(u32))),
            ScalarType::U64 => unsigned(u128::from(get!(u64))),
            ScalarType::U128 => unsigned(get!(u128)),
            ScalarType::USize => unsigned(get!(usize) as u128),
            ScalarType::I8 => signed(i128::from(get!(i8))),
            ScalarType::I16 => signed(i128::from(get!(i16))),
            ScalarType::I32 => signed(i128::from(get!(i32))),
            ScalarType::I64 => signed(i128::from(get!(i64))),
            ScalarType::I128 => signed(get!(i128)),
            ScalarType::ISize => signed(get!(isize) as i128),
            _ => {
                return Err(error(format!(
                    "unsupported CBOR scalar {}",
                    shape.type_identifier
                )));
            }
        });
    }
    match shape.def {
        Def::Option(_) => {
            let option = peek.into_option().map_err(|e| error(e.to_string()))?;
            match option.value() {
                None => Ok(Value::Simple(22)),
                Some(inner) => {
                    let value = from_peek(inner, depth + 1)?;
                    if matches!(value, Value::Simple(22)) {
                        return Err(error("CBOR option cannot distinguish Some(null) from None"));
                    }
                    Ok(value)
                }
            }
        }
        Def::List(_) | Def::Array(_) | Def::Slice(_) => {
            let list = peek.into_list_like().map_err(|e| error(e.to_string()))?;
            if crate::classify::is_byte_seq(shape) {
                if let Some(bytes) = list.as_bytes() {
                    return Ok(Value::Bytes(bytes.to_vec()));
                }
                let mut bytes = Vec::with_capacity(list.len());
                for value in list.iter() {
                    bytes.push(*value.get::<u8>().map_err(|e| error(e.to_string()))?);
                }
                return Ok(Value::Bytes(bytes));
            }
            let mut values = Vec::with_capacity(list.len());
            for value in list.iter() {
                values.push(from_peek(value, depth + 1)?);
            }
            Ok(Value::Array(values))
        }
        Def::Map(_) => {
            let map = peek.into_map().map_err(|e| error(e.to_string()))?;
            let mut values = Vec::with_capacity(map.len());
            for (key, value) in map.iter() {
                values.push((from_peek(key, depth + 1)?, from_peek(value, depth + 1)?));
            }
            Ok(Value::Map(values))
        }
        _ => match shape.ty {
            Type::User(UserType::Struct(st)) => {
                let structure = peek.into_struct().map_err(|e| error(e.to_string()))?;
                match st.kind {
                    StructKind::Unit => Ok(Value::Simple(22)),
                    StructKind::Tuple | StructKind::TupleStruct => {
                        let mut values = Vec::with_capacity(st.fields.len());
                        for index in 0..st.fields.len() {
                            values.push(from_peek(
                                structure.field(index).map_err(|e| error(e.to_string()))?,
                                depth + 1,
                            )?);
                        }
                        Ok(Value::Array(values))
                    }
                    StructKind::Struct => {
                        let mut values = Vec::with_capacity(st.fields.len());
                        for (index, field) in st.fields.iter().enumerate() {
                            values.push((
                                Value::Text(field.effective_name().to_owned()),
                                from_peek(
                                    structure.field(index).map_err(|e| error(e.to_string()))?,
                                    depth + 1,
                                )?,
                            ));
                        }
                        Ok(Value::Map(values))
                    }
                }
            }
            Type::User(UserType::Enum(_)) => {
                let enumeration = peek.into_enum().map_err(|e| error(e.to_string()))?;
                let variant = enumeration
                    .active_variant()
                    .map_err(|e| error(e.to_string()))?;
                let mut fields = Vec::with_capacity(variant.data.fields.len());
                for index in 0..variant.data.fields.len() {
                    let field = enumeration
                        .field(index)
                        .map_err(|e| error(e.to_string()))?
                        .ok_or_else(|| error("missing CBOR enum field"))?;
                    fields.push(from_peek(field, depth + 2)?);
                }
                let payload = match variant.data.kind {
                    StructKind::Unit => Value::Simple(22),
                    StructKind::Tuple | StructKind::TupleStruct if fields.len() == 1 => fields
                        .pop()
                        .ok_or_else(|| error("missing CBOR enum field"))?,
                    StructKind::Tuple | StructKind::TupleStruct => Value::Array(fields),
                    StructKind::Struct => Value::Map(
                        variant
                            .data
                            .fields
                            .iter()
                            .zip(fields)
                            .map(|(field, value)| {
                                (Value::Text(field.effective_name().to_owned()), value)
                            })
                            .collect(),
                    ),
                };
                Ok(Value::Map(vec![(
                    Value::Text(variant.effective_name().to_owned()),
                    payload,
                )]))
            }
            _ => Err(error(format!(
                "unsupported CBOR type {}",
                shape.type_identifier
            ))),
        },
    }
}

fn from_dynamic(value: &facet_value::Value, depth: usize) -> Result<Value, CborError> {
    if depth > MAX_VALUE_DEPTH {
        return Err(error("maximum CBOR nesting depth exceeded"));
    }
    if value.is_null() {
        return Ok(Value::Simple(22));
    }
    if let Some(value) = value.as_bool() {
        return Ok(Value::Simple(if value { 21 } else { 20 }));
    }
    if let Some(number) = value.as_number() {
        return if number.is_float() {
            Ok(Value::Float(number.to_f64_lossy()))
        } else if let Some(value) = number.to_i128() {
            Ok(signed(value))
        } else if let Some(value) = number.to_u128() {
            Ok(unsigned(value))
        } else {
            Err(error("unrepresentable CBOR number"))
        };
    }
    if let Some(text) = value.as_string() {
        return Ok(Value::Text(text.as_str().to_owned()));
    }
    if let Some(character) = value.as_char() {
        return Ok(Value::Text(character.to_string()));
    }
    if let Some(bytes) = value.as_bytes() {
        return Ok(Value::Bytes(bytes.as_slice().to_vec()));
    }
    if let Some(array) = value.as_array() {
        return array
            .as_slice()
            .iter()
            .map(|value| from_dynamic(value, depth + 1))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    if let Some(object) = value.as_object() {
        return object
            .iter()
            .map(|(key, value)| {
                Ok((
                    Value::Text(key.as_str().to_owned()),
                    from_dynamic(value, depth + 1)?,
                ))
            })
            .collect::<Result<Vec<_>, CborError>>()
            .map(Value::Map);
    }
    Err(error("unsupported dynamic CBOR value"))
}

/// Populate an existing `facet` partial value from one canonical CBOR item.
pub(crate) fn decode_into<'facet>(
    partial: facet::Partial<'facet, true>,
    bytes: &[u8],
) -> Result<facet::Partial<'facet, true>, CborError> {
    let value = decode(bytes)?;
    let partial = into_partial(partial, &value, 0)?;
    // A target map's equality can merge distinct CBOR keys. Re-encoding the
    // built outer value must still be checked by its caller where available.
    Ok(partial)
}

fn integer(value: &Value) -> Option<(bool, u128)> {
    match value {
        Value::Unsigned(value) => Some((false, u128::from(*value))),
        Value::Negative(value) => Some((true, u128::from(*value))),
        Value::Tag(tag @ (2 | 3), inner) => {
            let Value::Bytes(bytes) = inner.as_ref() else {
                return None;
            };
            if bytes.len() <= 8 || bytes.len() > 16 || bytes.first() == Some(&0) {
                return None;
            }
            let mut result = 0u128;
            for byte in bytes {
                result = (result << 8) | u128::from(*byte);
            }
            Some((*tag == 3, result))
        }
        _ => None,
    }
}

fn integer_text(value: &Value) -> Option<String> {
    let (negative, argument) = integer(value)?;
    if negative {
        // The negative argument for i128::MIN is i128::MAX.
        let magnitude_minus_one = i128::try_from(argument).ok()?;
        Some((-1 - magnitude_minus_one).to_string())
    } else {
        Some(argument.to_string())
    }
}

fn value_to_dynamic(value: &Value, depth: usize) -> Result<facet_value::Value, CborError> {
    use facet_value::{VArray, VObject};

    if depth > MAX_VALUE_DEPTH {
        return Err(error("maximum CBOR nesting depth exceeded"));
    }
    if let Some((negative, argument)) = integer(value) {
        return if negative {
            let argument =
                i128::try_from(argument).map_err(|_error| error("CBOR integer outside i128"))?;
            Ok(facet_value::Value::from(-1 - argument))
        } else {
            Ok(facet_value::Value::from(argument))
        };
    }
    match value {
        Value::Simple(20) => Ok(false.into()),
        Value::Simple(21) => Ok(true.into()),
        Value::Simple(22) => Ok(facet_value::Value::NULL),
        Value::Float(value) => Ok((*value).into()),
        Value::Text(value) => Ok(value.clone().into()),
        Value::Bytes(value) => Ok(value.clone().into()),
        Value::Array(values) => {
            let mut result = VArray::new();
            for value in values {
                result.push(value_to_dynamic(value, depth + 1)?);
            }
            Ok(result.into())
        }
        Value::Map(values) => {
            let mut result = VObject::new();
            for (key, value) in values {
                let Value::Text(key) = key else {
                    return Err(error(
                        "non-string CBOR map key cannot become a dynamic object",
                    ));
                };
                if result
                    .insert(key.as_str(), value_to_dynamic(value, depth + 1)?)
                    .is_some()
                {
                    return Err(error("duplicate dynamic CBOR object key"));
                }
            }
            Ok(result.into())
        }
        _ => Err(error("CBOR value cannot be represented as a dynamic value")),
    }
}

fn into_partial<'facet>(
    partial: facet::Partial<'facet, true>,
    value: &Value,
    depth: usize,
) -> Result<facet::Partial<'facet, true>, CborError> {
    use facet::{Def, ScalarType, StructKind, Type, UserType};

    if depth > MAX_VALUE_DEPTH {
        return Err(error("maximum CBOR nesting depth exceeded"));
    }
    let shape = partial.shape();
    if shape.is_type::<facet_value::Value>() {
        return partial
            .set(value_to_dynamic(value, depth)?)
            .map_err(|e| error(e.to_string()));
    }
    if shape.scalar_type() == Some(ScalarType::CowStr) {
        let Value::Text(text) = value else {
            return Err(error("expected CBOR text"));
        };
        return partial
            .set(std::borrow::Cow::<'facet, str>::Owned(text.clone()))
            .map_err(|e| error(e.to_string()));
    }
    if matches!(shape.def, Def::Pointer(_)) {
        let partial = partial
            .begin_smart_ptr()
            .map_err(|e| error(e.to_string()))?;
        if partial.is_building_smart_ptr_slice() {
            return Err(error("CBOR smart pointer slices are unsupported"));
        }
        return into_partial(partial, value, depth + 1)?
            .end()
            .map_err(|e| error(e.to_string()));
    }
    if shape.inner.is_some() && shape.vtable.has_try_borrow_inner() {
        let partial = partial.begin_inner().map_err(|e| error(e.to_string()))?;
        return into_partial(partial, value, depth + 1)?
            .end()
            .map_err(|e| error(e.to_string()));
    }
    if let Some(scalar) = shape.scalar_type() {
        return match scalar {
            ScalarType::Unit if matches!(value, Value::Simple(22)) => {
                partial.set(()).map_err(|e| error(e.to_string()))
            }
            ScalarType::Bool => match value {
                Value::Simple(20) => partial.set(false).map_err(|e| error(e.to_string())),
                Value::Simple(21) => partial.set(true).map_err(|e| error(e.to_string())),
                _ => Err(error("expected CBOR boolean")),
            },
            ScalarType::F32 => match value {
                Value::Float(number) => {
                    let result = *number as f32;
                    if !number.is_nan() && f64::from(result).to_bits() != number.to_bits() {
                        return Err(error("CBOR float is not representable as f32"));
                    }
                    partial.set(result).map_err(|e| error(e.to_string()))
                }
                _ => Err(error("expected CBOR float")),
            },
            ScalarType::F64 => match value {
                Value::Float(number) => partial.set(*number).map_err(|e| error(e.to_string())),
                _ => Err(error("expected CBOR float")),
            },
            ScalarType::Str | ScalarType::String | ScalarType::Char => match value {
                Value::Text(text) => partial
                    .parse_from_str(text)
                    .map_err(|e| error(e.to_string())),
                _ => Err(error("expected CBOR text")),
            },
            ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::I128
            | ScalarType::ISize
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::U128
            | ScalarType::USize => {
                let text = integer_text(value)
                    .ok_or_else(|| error("expected representable CBOR integer"))?;
                partial
                    .parse_from_str(&text)
                    .map_err(|e| error(e.to_string()))
            }
            _ => Err(error(format!(
                "unsupported CBOR scalar {}",
                shape.type_identifier
            ))),
        };
    }
    match shape.def {
        Def::Option(_) => {
            if matches!(value, Value::Simple(22)) {
                return partial.set_default().map_err(|e| error(e.to_string()));
            }
            let partial = partial.begin_some().map_err(|e| error(e.to_string()))?;
            into_partial(partial, value, depth + 1)?
                .end()
                .map_err(|e| error(e.to_string()))
        }
        Def::List(_) | Def::Array(_) => {
            if crate::classify::is_byte_seq(shape) {
                let Value::Bytes(bytes) = value else {
                    return Err(error("expected CBOR byte string"));
                };
                if shape.is_type::<Vec<u8>>() {
                    return partial.set(bytes.clone()).map_err(|e| error(e.to_string()));
                }
                match shape.def {
                    Def::Array(array) => {
                        if array.n != bytes.len() {
                            return Err(error("CBOR byte array length mismatch"));
                        }
                        let mut partial = partial.init_array().map_err(|e| error(e.to_string()))?;
                        for (index, byte) in bytes.iter().enumerate() {
                            partial = partial
                                .begin_nth_field(index)
                                .map_err(|e| error(e.to_string()))?;
                            partial = partial.set(*byte).map_err(|e| error(e.to_string()))?;
                            partial = partial.end().map_err(|e| error(e.to_string()))?;
                        }
                        return Ok(partial);
                    }
                    Def::List(_) => {
                        let mut partial = partial.init_list().map_err(|e| error(e.to_string()))?;
                        for byte in bytes {
                            partial = partial
                                .begin_list_item()
                                .map_err(|e| error(e.to_string()))?;
                            partial = partial.set(*byte).map_err(|e| error(e.to_string()))?;
                            partial = partial.end().map_err(|e| error(e.to_string()))?;
                        }
                        return Ok(partial);
                    }
                    _ => return Err(error("unsupported CBOR byte sequence")),
                }
            }
            let Value::Array(values) = value else {
                return Err(error("expected CBOR array"));
            };
            match shape.def {
                Def::List(_) => {
                    let mut partial = partial.init_list().map_err(|e| error(e.to_string()))?;
                    for value in values {
                        partial = partial
                            .begin_list_item()
                            .map_err(|e| error(e.to_string()))?;
                        partial = into_partial(partial, value, depth + 1)?
                            .end()
                            .map_err(|e| error(e.to_string()))?;
                    }
                    Ok(partial)
                }
                Def::Array(array) => {
                    if array.n != values.len() {
                        return Err(error("CBOR array length mismatch"));
                    }
                    let mut partial = partial.init_array().map_err(|e| error(e.to_string()))?;
                    for (index, value) in values.iter().enumerate() {
                        partial = partial
                            .begin_nth_field(index)
                            .map_err(|e| error(e.to_string()))?;
                        partial = into_partial(partial, value, depth + 1)?
                            .end()
                            .map_err(|e| error(e.to_string()))?;
                    }
                    Ok(partial)
                }
                _ => Err(error("unsupported CBOR sequence")),
            }
        }
        Def::Map(_) => {
            let Value::Map(values) = value else {
                return Err(error("expected CBOR map"));
            };
            let mut partial = partial.init_map().map_err(|e| error(e.to_string()))?;
            for (key, value) in values {
                partial = partial.begin_key().map_err(|e| error(e.to_string()))?;
                partial = into_partial(partial, key, depth + 1)?
                    .end()
                    .map_err(|e| error(e.to_string()))?;
                partial = partial.begin_value().map_err(|e| error(e.to_string()))?;
                partial = into_partial(partial, value, depth + 1)?
                    .end()
                    .map_err(|e| error(e.to_string()))?;
            }
            Ok(partial)
        }
        _ => match shape.ty {
            Type::User(UserType::Struct(st)) => match st.kind {
                StructKind::Unit if matches!(value, Value::Simple(22)) => {
                    partial.set_default().map_err(|e| error(e.to_string()))
                }
                StructKind::Tuple | StructKind::TupleStruct => {
                    fill_tuple(partial, value, st.fields.len(), depth)
                }
                StructKind::Struct => fill_fields(partial, value, st.fields, depth),
                _ => Err(error("expected CBOR null for unit struct")),
            },
            Type::User(UserType::Enum(enumeration)) => {
                let Value::Map(entries) = value else {
                    return Err(error("expected CBOR enum map"));
                };
                let [(Value::Text(name), payload)] = entries.as_slice() else {
                    return Err(error("expected one CBOR enum variant"));
                };
                let mut matching = enumeration
                    .variants
                    .iter()
                    .enumerate()
                    .filter(|(_, variant)| variant.effective_name() == name);
                let (index, variant) = matching
                    .next()
                    .ok_or_else(|| error("unknown CBOR enum variant"))?;
                if matching.next().is_some() {
                    return Err(error("duplicate CBOR enum variant name"));
                }
                let partial = partial
                    .select_nth_variant(index)
                    .map_err(|e| error(e.to_string()))?;
                match variant.data.kind {
                    StructKind::Unit if matches!(payload, Value::Simple(22)) => Ok(partial),
                    StructKind::Tuple | StructKind::TupleStruct
                        if variant.data.fields.len() == 1 =>
                    {
                        let partial = partial
                            .begin_nth_field(0)
                            .map_err(|e| error(e.to_string()))?;
                        into_partial(partial, payload, depth + 2)?
                            .end()
                            .map_err(|e| error(e.to_string()))
                    }
                    StructKind::Tuple | StructKind::TupleStruct => {
                        fill_tuple(partial, payload, variant.data.fields.len(), depth + 1)
                    }
                    StructKind::Struct => {
                        fill_fields(partial, payload, variant.data.fields, depth + 1)
                    }
                    _ => Err(error("expected CBOR null for unit variant")),
                }
            }
            _ => Err(error(format!(
                "unsupported CBOR type {}",
                shape.type_identifier
            ))),
        },
    }
}

fn fill_tuple<'facet>(
    mut partial: facet::Partial<'facet, true>,
    value: &Value,
    len: usize,
    depth: usize,
) -> Result<facet::Partial<'facet, true>, CborError> {
    let Value::Array(values) = value else {
        return Err(error("expected CBOR tuple array"));
    };
    if values.len() != len {
        return Err(error("CBOR tuple length mismatch"));
    }
    for (index, value) in values.iter().enumerate() {
        partial = partial
            .begin_nth_field(index)
            .map_err(|e| error(e.to_string()))?;
        partial = into_partial(partial, value, depth + 1)?
            .end()
            .map_err(|e| error(e.to_string()))?;
    }
    Ok(partial)
}

fn fill_fields<'facet>(
    mut partial: facet::Partial<'facet, true>,
    value: &Value,
    fields: &'static [facet::Field],
    depth: usize,
) -> Result<facet::Partial<'facet, true>, CborError> {
    let Value::Map(values) = value else {
        return Err(error("expected CBOR struct map"));
    };
    if values.len() != fields.len() {
        return Err(error("CBOR struct field count mismatch"));
    }
    let mut visited = vec![false; fields.len()];
    for (key, value) in values {
        let Value::Text(name) = key else {
            return Err(error("expected CBOR text field name"));
        };
        let mut matching = fields
            .iter()
            .enumerate()
            .filter(|(_, field)| field.effective_name() == name);
        let (index, _) = matching
            .next()
            .ok_or_else(|| error("unknown CBOR struct field"))?;
        if matching.next().is_some() || visited[index] {
            return Err(error("duplicate CBOR struct field"));
        }
        visited[index] = true;
        partial = partial
            .begin_nth_field(index)
            .map_err(|e| error(e.to_string()))?;
        partial = into_partial(partial, value, depth + 1)?
            .end()
            .map_err(|e| error(e.to_string()))?;
    }
    Ok(partial)
}

#[cfg(test)]
mod tests {
    use super::{Value, decode, decode_into, encode, encode_peek, validate};
    use facet::{Facet, Partial, Peek};
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    fn assert_roundtrip<T>(value: &T)
    where
        T: for<'a> Facet<'a> + PartialEq + std::fmt::Debug,
    {
        let bytes = encode_peek(Peek::new(value)).expect("encode reflected value");
        validate(&bytes).expect("encoded bytes are canonical");
        let partial = Partial::alloc::<T>().expect("allocate reflected value");
        let partial = decode_into(partial, &bytes).expect("decode reflected value");
        let heap = partial.build().expect("build reflected value");
        assert_eq!(encode_peek(heap.peek()).expect("re-encode"), bytes);
        assert_eq!(&heap.materialize::<T>().expect("materialize"), value);
    }

    #[test]
    fn integer_boundaries_and_tags() {
        for (value, bytes) in [
            (Value::Unsigned(0), vec![0x00]),
            (Value::Unsigned(23), vec![0x17]),
            (Value::Unsigned(24), vec![0x18, 0x18]),
            (Value::Unsigned(255), vec![0x18, 0xff]),
            (Value::Unsigned(256), vec![0x19, 0x01, 0x00]),
            (Value::Unsigned(65_536), vec![0x1a, 0x00, 0x01, 0x00, 0x00]),
            (Value::Negative(0), vec![0x20]),
            (Value::Negative(24), vec![0x38, 0x18]),
            (
                Value::Tag(32, Box::new(Value::Text("x".into()))),
                vec![0xd8, 0x20, 0x61, b'x'],
            ),
        ] {
            assert_eq!(encode(&value).unwrap(), bytes);
            assert_eq!(decode(&bytes).unwrap(), value);
        }
        assert_roundtrip(&u64::MAX);
        assert_roundtrip(&i64::MIN);
        assert_roundtrip(&u128::MAX);
        assert_roundtrip(&i128::MIN);
        assert_roundtrip(&(u128::from(u64::MAX) + 1));
        assert_roundtrip(&(-i128::from(u64::MAX) - 2));
    }

    #[test]
    fn floats_use_shortest_exact_width_and_canonical_nan() {
        for (value, bytes) in [
            (0.0, vec![0xf9, 0x00, 0x00]),
            (-0.0, vec![0xf9, 0x80, 0x00]),
            (1.5, vec![0xf9, 0x3e, 0x00]),
            (65_504.0, vec![0xf9, 0x7b, 0xff]),
            (65_536.0, vec![0xfa, 0x47, 0x80, 0x00, 0x00]),
            (
                f64::from(f32::from_bits(1)),
                vec![0xfa, 0x00, 0x00, 0x00, 0x01],
            ),
            (f64::INFINITY, vec![0xf9, 0x7c, 0x00]),
            (f64::NEG_INFINITY, vec![0xf9, 0xfc, 0x00]),
            (f64::from_bits(1), vec![0xfb, 0, 0, 0, 0, 0, 0, 0, 1]),
        ] {
            assert_eq!(encode(&Value::Float(value)).unwrap(), bytes);
            let Value::Float(decoded) = decode(&bytes).unwrap() else {
                panic!("expected float")
            };
            assert_eq!(decoded.to_bits(), value.to_bits());
        }
        for bits in [
            0x7ff8_0000_0000_0000,
            0xfff0_0000_0000_0001,
            0x7ff0_0000_0000_0001,
        ] {
            assert_eq!(
                encode(&Value::Float(f64::from_bits(bits))).unwrap(),
                [0xf9, 0x7e, 0x00]
            );
        }
        let Value::Float(nan) = decode(&[0xf9, 0x7e, 0x00]).unwrap() else {
            panic!("expected float")
        };
        assert!(nan.is_nan());
    }

    #[test]
    fn map_order_is_core_bytewise_not_length_first() {
        let value = Value::Map(vec![
            (Value::Negative(0), Value::Simple(22)),
            (Value::Unsigned(24), Value::Simple(21)),
            (Value::Unsigned(1), Value::Simple(20)),
        ]);
        let bytes = [0xa3, 0x01, 0xf4, 0x18, 0x18, 0xf5, 0x20, 0xf6];
        assert_eq!(encode(&value).unwrap(), bytes);
        assert_eq!(encode(&decode(&bytes).unwrap()).unwrap(), bytes);
        assert!(validate(&[0xa2, 0x20, 0xf6, 0x18, 0x18, 0xf6]).is_err());
        assert!(validate(&[0xa2, 0x01, 0xf4, 0x01, 0xf5]).is_err());
        let duplicate = Value::Map(vec![
            (Value::Unsigned(1), Value::Simple(20)),
            (Value::Unsigned(1), Value::Simple(21)),
        ]);
        assert!(encode(&duplicate).is_err());
        let duplicate_nan = Value::Map(vec![
            (Value::Float(f64::NAN), Value::Simple(20)),
            (
                Value::Float(f64::from_bits(0xfff8_0000_0000_0001)),
                Value::Simple(21),
            ),
        ]);
        assert!(encode(&duplicate_nan).is_err());
    }

    #[test]
    fn rejects_invalid_noncanonical_and_trailing_data() {
        let invalid: &[&[u8]] = &[
            &[],
            &[0x00, 0x00],
            &[0x18, 0x17],
            &[0x19, 0x00, 0xff],
            &[0x38, 0x00],
            &[0x5f, 0xff],
            &[0x7f, 0xff],
            &[0x9f, 0xff],
            &[0xbf, 0xff],
            &[0xff],
            &[0x1c],
            &[0x1d],
            &[0x1e],
            &[0xf8, 0x14],
            &[0xf8, 0x18],
            &[0xfc],
            &[0x61, 0xff],
            &[0x62, b'a'],
            &[0x42, 0x00],
            &[0x82, 0x00],
            &[0xa1, 0x00],
            &[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
            &[0xfa, 0x3f, 0xc0, 0x00, 0x00],
            &[0xfb, 0x3f, 0xf8, 0, 0, 0, 0, 0, 0],
            &[0xf9, 0x7e, 0x01],
            &[0xf9, 0xfe, 0x00],
            &[0xfa, 0x7f, 0xc0, 0, 0],
        ];
        for bytes in invalid {
            assert!(validate(bytes).is_err(), "accepted {bytes:02x?}");
        }
        let mut nested = vec![0x81; crate::limits::MAX_VALUE_DEPTH + 1];
        nested.push(0xf6);
        assert!(validate(&nested).is_err());
    }

    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Facet)]
    struct Pair {
        number: i32,
        label: String,
    }

    #[derive(Debug, PartialEq, Facet)]
    #[repr(u8)]
    enum Example {
        Empty,
        One(u128),
        Two(bool, String),
        Named { alpha: u8, omega: Vec<u8> },
    }

    #[test]
    fn facet_roundtrips_maps_arrays_enums_and_dynamic_values() {
        let value = BTreeMap::from([
            (
                Pair {
                    number: -2,
                    label: "a".into(),
                },
                Some(vec![1u8, 2, 3]),
            ),
            (
                Pair {
                    number: 24,
                    label: "b".into(),
                },
                None,
            ),
        ]);
        assert_roundtrip(&value);
        for value in [
            Example::Empty,
            Example::One(u128::MAX),
            Example::Two(true, "x".into()),
            Example::Named {
                alpha: 4,
                omega: vec![0, 255],
            },
        ] {
            assert_roundtrip(&value);
        }
        assert_roundtrip(&[1u8, 2, 3]);
        assert_roundtrip(&vec![Some(1i64), None, Some(-2)]);
        let dynamic = facet_value::value!({"alpha": [1, 2], "omega": true});
        assert_roundtrip(&dynamic);
        assert!(encode_peek(Peek::new(&Some(None::<u8>))).is_err());
    }

    proptest! {
        #[test]
        fn arbitrary_input_that_is_accepted_reencodes_identically(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
            if let Ok(value) = decode(&bytes) {
                prop_assert_eq!(encode(&value).unwrap(), bytes);
            }
        }

        #[test]
        fn float_bits_roundtrip_with_nan_normalization(bits in any::<u64>()) {
            let value = f64::from_bits(bits);
            let encoded = encode(&Value::Float(value)).unwrap();
            let Value::Float(decoded) = decode(&encoded).unwrap() else { panic!("expected float") };
            if value.is_nan() {
                prop_assert!(decoded.is_nan());
                prop_assert_eq!(&encoded, &[0xf9, 0x7e, 0x00]);
            } else {
                prop_assert_eq!(decoded.to_bits(), bits);
            }
            prop_assert_eq!(encode(&Value::Float(decoded)).unwrap(), encoded);
        }

        #[test]
        fn integers_roundtrip_and_reencode(value in any::<i128>()) {
            let bytes = encode_peek(Peek::new(&value)).unwrap();
            let partial = Partial::alloc::<i128>().unwrap();
            let heap = decode_into(partial, &bytes).unwrap().build().unwrap();
            prop_assert_eq!(encode_peek(heap.peek()).unwrap(), bytes);
            prop_assert_eq!(heap.materialize::<i128>().unwrap(), value);
        }
    }
}
