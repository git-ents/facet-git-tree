//! Schema-directed decoding of bare Git objects into [`facet_value::Value`].
//!
//! A schema supplies the type information absent from the bare tree. Dynamic
//! nodes remain heuristic; CBOR nodes are opaque, validated canonical bytes.

use std::collections::BTreeMap;

use facet_value::{VArray, VNumber, VObject, Value};
use gix_object::{Data, Find, Kind};

use crate::de::{
    DecodeBudget, Entries, MAX_DECODE_VISITS, find_object_with_budget, sort_by_ordinal,
    tree_entries_from_data, validate_marker,
};
use crate::error::{DeserializeError, SchemaReadError};
use crate::limits::MAX_VALUE_DEPTH;
use crate::ordinal::{self, MAX_TREE_ENTRIES};
use crate::schema::{Node, Schema, VariantKind};
use crate::{EntryKind, ObjectId};

/// Deserialize the bare object at `root` according to `doc`.
///
/// The entire schema is checked, including unused definitions and variants,
/// before any value is read. This does not read a `.schema` envelope. A CBOR
/// node reads as opaque canonical bytes and rejects invalid or trailing data.
/// An unwrapped `Value` cannot distinguish `Some(null)` from `None`, so such
/// an option is rejected rather than returning a value that re-encodes wrongly.
pub fn deserialize_value_with_schema<F: Find + ?Sized>(
    root: &ObjectId,
    doc: &Schema,
    store: &F,
) -> Result<Value, SchemaReadError> {
    validate_schema(doc)?;
    Reader {
        doc,
        store,
        budget: DecodeBudget::new(),
    }
    .read_node(root, &doc.root, 0, None)
}

/// Check the bare object at `root` against the entire schema.
pub fn validate_with_schema<F: Find + ?Sized>(
    root: &ObjectId,
    doc: &Schema,
    store: &F,
) -> Result<(), SchemaReadError> {
    deserialize_value_with_schema(root, doc, store).map(|_| ())
}

/// Validate all schema syntax and references, independently of a value walk.
///
/// Guards break reference cycles, but their contents are still checked for
/// dangling references, invalid keys, lengths and depth.
pub(crate) fn validate_schema(doc: &Schema) -> Result<(), SchemaReadError> {
    check_schema_len(doc.defs.len(), "definitions")?;
    let mut pending = vec![(&doc.root, 0usize)];
    for (name, node) in &doc.defs {
        check_schema_name(name)?;
        pending.push((node, 0));
    }

    let mut visited = 0usize;
    while let Some((node, depth)) = pending.pop() {
        if depth > MAX_VALUE_DEPTH {
            return Err(DeserializeError::MaxDepth(MAX_VALUE_DEPTH).into());
        }
        visited += 1;
        if visited > MAX_DECODE_VISITS {
            return Err(invalid_schema("schema node limit exceeded"));
        }
        let next = depth + 1;
        match node {
            Node::Ref(name) => {
                if !doc.defs.contains_key(name) {
                    return Err(SchemaReadError::UnknownRef(name.clone()));
                }
            }
            Node::List(elem) | Node::Optional(elem) => pending.push((elem, next)),
            Node::Array { elem, len } => {
                check_schema_len(*len, "array length")?;
                pending.push((elem, next));
            }
            Node::Map { key, value } => {
                if !matches!(key.as_ref(), Node::String) {
                    return Err(invalid_schema("map key must be literally String"));
                }
                pending.push((key, next));
                pending.push((value, next));
            }
            Node::Tuple(elems) => {
                check_schema_len(elems.len(), "tuple arity")?;
                pending.extend(elems.iter().map(|elem| (elem, next)));
            }
            Node::Struct(fields) => {
                check_schema_fields(fields, &mut pending, next)?;
            }
            Node::Enum(variants) => {
                check_schema_len(variants.len(), "enum variants")?;
                for (name, kind) in variants {
                    check_schema_name(name)?;
                    match kind {
                        VariantKind::Unit => {}
                        VariantKind::Tuple(elems) => {
                            check_schema_len(elems.len(), "tuple variant arity")?;
                            pending.extend(elems.iter().map(|elem| (elem, next)));
                        }
                        VariantKind::Struct(fields) => {
                            check_schema_fields(fields, &mut pending, next)?;
                        }
                    }
                }
            }
            Node::Unit
            | Node::Bool
            | Node::Char
            | Node::String
            | Node::I8
            | Node::I16
            | Node::I32
            | Node::I64
            | Node::I128
            | Node::ISize
            | Node::U8
            | Node::U16
            | Node::U32
            | Node::U64
            | Node::U128
            | Node::USize
            | Node::F32
            | Node::F64
            | Node::Bytes
            | Node::Cbor
            | Node::RawTree
            | Node::Dynamic => {}
        }
    }

    // Only edges through structs, tuples and refs are unguarded. An explicit
    // DFS handles long alias chains without using the call stack.
    enum Visit<'a> {
        Enter(&'a str),
        Exit(&'a str),
        Node(&'a Node),
    }
    let mut states: BTreeMap<&str, bool> = BTreeMap::new();
    for name in doc.defs.keys() {
        let mut stack = vec![Visit::Enter(name)];
        while let Some(visit) = stack.pop() {
            match visit {
                Visit::Enter(name) => match states.get(name) {
                    Some(true) => {}
                    Some(false) => {
                        return Err(invalid_schema(format!(
                            "unguarded reference cycle involving {name:?}"
                        )));
                    }
                    None => {
                        let node = doc
                            .defs
                            .get(name)
                            .ok_or_else(|| SchemaReadError::UnknownRef(name.to_owned()))?;
                        states.insert(name, false);
                        stack.push(Visit::Exit(name));
                        stack.push(Visit::Node(node));
                    }
                },
                Visit::Exit(name) => {
                    states.insert(name, true);
                }
                Visit::Node(node) => match node {
                    Node::Ref(name) => stack.push(Visit::Enter(name)),
                    Node::Struct(fields) => stack.extend(fields.values().map(Visit::Node)),
                    Node::Tuple(elems) => stack.extend(elems.iter().map(Visit::Node)),
                    // Each of these is a guard, or has no outgoing edges.
                    _ => {}
                },
            }
        }
    }
    Ok(())
}

fn check_schema_fields<'a>(
    fields: &'a BTreeMap<String, Node>,
    pending: &mut Vec<(&'a Node, usize)>,
    depth: usize,
) -> Result<(), SchemaReadError> {
    check_schema_len(fields.len(), "struct fields")?;
    for (name, node) in fields {
        check_schema_name(name)?;
        pending.push((node, depth));
    }
    Ok(())
}

fn check_schema_len(len: usize, what: &str) -> Result<(), SchemaReadError> {
    if len > MAX_TREE_ENTRIES {
        return Err(invalid_schema(format!(
            "{what} has {len} entries, exceeding {MAX_TREE_ENTRIES}"
        )));
    }
    Ok(())
}

fn check_schema_name(name: &str) -> Result<(), SchemaReadError> {
    crate::name::escape(name)
        .map(|_| ())
        .map_err(|error| invalid_schema(format!("invalid name {name:?}: {error}")))
}

fn invalid_schema(reason: impl std::fmt::Display) -> SchemaReadError {
    DeserializeError::Reflect(format!("invalid schema: {reason}")).into()
}

fn invalid_tree(oid: &ObjectId, reason: impl std::fmt::Display) -> SchemaReadError {
    DeserializeError::Reflect(format!("invalid tree {oid}: {reason}")).into()
}

struct Reader<'a, F: Find + ?Sized> {
    doc: &'a Schema,
    store: &'a F,
    budget: DecodeBudget,
}

impl<F: Find + ?Sized> Reader<'_, F> {
    fn load<'b>(
        &mut self,
        oid: &ObjectId,
        buf: &'b mut Vec<u8>,
        depth: usize,
        entry_kind: Option<EntryKind>,
    ) -> Result<Data<'b>, SchemaReadError> {
        Ok(find_object_with_budget(
            oid,
            buf,
            self.store,
            depth,
            &mut self.budget,
            entry_kind,
        )?)
    }

    fn read_node(
        &mut self,
        oid: &ObjectId,
        schema: &Node,
        depth: usize,
        entry_kind: Option<EntryKind>,
    ) -> Result<Value, SchemaReadError> {
        // References have no encoded tree level. Resolve aliases iteratively;
        // schema validation has already rejected non-productive cycles.
        let doc = self.doc;
        let mut schema = schema;
        let mut aliases = 0usize;
        while let Node::Ref(name) = schema {
            aliases += 1;
            if aliases > doc.defs.len() {
                return Err(invalid_schema("unguarded reference cycle"));
            }
            schema = doc
                .defs
                .get(name)
                .ok_or_else(|| SchemaReadError::UnknownRef(name.clone()))?;
        }

        let mut buf = Vec::new();
        let data = self.load(oid, &mut buf, depth, entry_kind)?;
        match schema {
            Node::Unit => {
                let entries = tree_entries(&data, oid)?;
                if !self.is_marker(&entries, depth)? {
                    return Err(SchemaReadError::MalformedUnit {
                        found: entries.len(),
                    });
                }
                Ok(Value::NULL)
            }
            Node::Bool => match text(&data, oid)? {
                "true" => Ok(Value::from(true)),
                "false" => Ok(Value::from(false)),
                other => Err(invalid_scalar("Bool", other)),
            },
            Node::Char => {
                let text = text(&data, oid)?;
                let mut chars = text.chars();
                match (chars.next(), chars.next()) {
                    (Some(value), None) => Ok(Value::from(value)),
                    _ => Err(invalid_scalar("Char", text)),
                }
            }
            Node::String => Ok(Value::from(text(&data, oid)?.to_owned())),
            Node::I8 => signed::<i8>(&data, oid, "I8"),
            Node::I16 => signed::<i16>(&data, oid, "I16"),
            Node::I32 => signed::<i32>(&data, oid, "I32"),
            Node::I64 => signed::<i64>(&data, oid, "I64"),
            Node::I128 => signed::<i128>(&data, oid, "I128"),
            Node::ISize => {
                let text = text(&data, oid)?;
                let value: i64 = canonical_integer(text, "ISize")?;
                isize::try_from(value).map_err(|_| invalid_scalar("ISize", text))?;
                Ok(VNumber::from_i128(i128::from(value)).into())
            }
            Node::U8 => unsigned::<u8>(&data, oid, "U8"),
            Node::U16 => unsigned::<u16>(&data, oid, "U16"),
            Node::U32 => unsigned::<u32>(&data, oid, "U32"),
            Node::U64 => unsigned::<u64>(&data, oid, "U64"),
            Node::U128 => unsigned::<u128>(&data, oid, "U128"),
            Node::USize => {
                let text = text(&data, oid)?;
                let value: u64 = canonical_integer(text, "USize")?;
                usize::try_from(value).map_err(|_| invalid_scalar("USize", text))?;
                Ok(VNumber::from_u128(u128::from(value)).into())
            }
            Node::F32 => {
                let text = text(&data, oid)?;
                let value =
                    crate::float::parse_f32(text).map_err(|_| invalid_scalar("F32", text))?;
                Ok(Value::from(value))
            }
            Node::F64 => {
                let text = text(&data, oid)?;
                let value =
                    crate::float::parse_f64(text).map_err(|_| invalid_scalar("F64", text))?;
                Ok(Value::from(value))
            }
            Node::Bytes => {
                require_blob(&data, oid)?;
                Ok(Value::from(data.data.to_vec()))
            }
            Node::Cbor => {
                require_blob(&data, oid)?;
                crate::cbor::validate(data.data).map_err(|error| {
                    DeserializeError::Reflect(format!("invalid CBOR blob {oid}: {error}"))
                })?;
                Ok(Value::from(data.data))
            }
            Node::Struct(fields) => {
                let entries = tree_entries(&data, oid)?;
                Ok(self.read_struct(entries, fields, depth)?.into())
            }
            Node::Tuple(elems) => {
                let entries = self.sequence(&data, oid, depth)?;
                Ok(self.read_tuple(entries, elems, depth)?.into())
            }
            Node::List(elem) => {
                let entries = self.sequence(&data, oid, depth)?;
                let mut array = VArray::new();
                for (_, child, kind) in entries {
                    array.push(self.read_node(&child, elem, depth + 1, Some(kind))?);
                }
                Ok(array.into())
            }
            Node::Array { elem, len } => {
                let entries = self.sequence(&data, oid, depth)?;
                check_length(*len, entries.len())?;
                let mut array = VArray::new();
                for (_, child, kind) in entries {
                    array.push(self.read_node(&child, elem, depth + 1, Some(kind))?);
                }
                Ok(array.into())
            }
            Node::Map { value, .. } => {
                let entries = tree_entries(&data, oid)?;
                if self.is_marker(&entries, depth)? {
                    return Ok(VObject::new().into());
                }
                let mut object = VObject::new();
                for (name, child, kind) in entries {
                    let name = decode_name(&name, oid)?;
                    let value = self.read_node(&child, value, depth + 1, Some(kind))?;
                    object.insert(name, value);
                }
                Ok(object.into())
            }
            Node::Optional(inner) => {
                let entries = tree_entries(&data, oid)?;
                if self.is_marker(&entries, depth)? {
                    return Ok(Value::NULL);
                }
                let [(name, child, kind)] = entries.as_slice() else {
                    return Err(DeserializeError::MalformedOption {
                        found: entries.len(),
                    }
                    .into());
                };
                if name != crate::marker::SOME_KEY {
                    return Err(DeserializeError::MislabeledOption { name: name.clone() }.into());
                }
                let value = self.read_node(child, inner, depth + 1, Some(*kind))?;
                if value.is_null() {
                    return Err(DeserializeError::Unsupported(
                        "Some(null) in schema-directed Value",
                    )
                    .into());
                }
                Ok(value)
            }
            Node::Enum(variants) => self.read_enum(&data, oid, variants, depth),
            Node::RawTree => {
                if data.kind != Kind::Tree {
                    return Err(DeserializeError::NotATree(*oid).into());
                }
                Ok(Value::from(oid.to_string()))
            }
            Node::Dynamic => self.read_dynamic(&data, oid, depth),
            Node::Ref(_) => Err(invalid_schema("unresolved reference")),
        }
    }

    fn is_marker(&mut self, entries: &Entries, depth: usize) -> Result<bool, SchemaReadError> {
        Ok(validate_marker(
            entries,
            self.store,
            depth,
            &mut self.budget,
        )?)
    }

    fn sequence(
        &mut self,
        data: &Data<'_>,
        oid: &ObjectId,
        depth: usize,
    ) -> Result<Entries, SchemaReadError> {
        let entries = tree_entries(data, oid)?;
        if self.is_marker(&entries, depth)? {
            return Ok(Vec::new());
        }
        sort_by_ordinal(&entries)?;
        Ok(entries)
    }

    fn read_tuple(
        &mut self,
        entries: Entries,
        elems: &[Node],
        depth: usize,
    ) -> Result<VArray, SchemaReadError> {
        check_length(elems.len(), entries.len())?;
        let mut array = VArray::new();
        for ((_, child, kind), elem) in entries.into_iter().zip(elems) {
            array.push(self.read_node(&child, elem, depth + 1, Some(kind))?);
        }
        Ok(array)
    }

    fn read_struct(
        &mut self,
        entries: Entries,
        fields: &BTreeMap<String, Node>,
        depth: usize,
    ) -> Result<VObject, SchemaReadError> {
        if self.is_marker(&entries, depth)? {
            if let Some(name) = fields.keys().next() {
                return Err(SchemaReadError::MissingField {
                    field: name.clone(),
                });
            }
            return Ok(VObject::new());
        }
        let mut object = VObject::new();
        for (encoded, child, kind) in entries {
            let name = decode_name(&encoded, &child)?;
            let field = fields
                .get(&name)
                .ok_or_else(|| SchemaReadError::UnexpectedEntry {
                    entry: name.clone(),
                })?;
            let value = self.read_node(&child, field, depth + 1, Some(kind))?;
            object.insert(name, value);
        }
        if let Some(name) = fields.keys().find(|name| !object.contains_key(name)) {
            return Err(SchemaReadError::MissingField {
                field: name.clone(),
            });
        }
        Ok(object)
    }

    fn read_enum(
        &mut self,
        data: &Data<'_>,
        oid: &ObjectId,
        variants: &BTreeMap<String, VariantKind>,
        depth: usize,
    ) -> Result<Value, SchemaReadError> {
        let (name, payload) = if data.kind == Kind::Blob {
            (decode_name(text(data, oid)?, oid)?, None)
        } else {
            let entries = tree_entries(data, oid)?;
            let [(name, child, kind)] = entries.as_slice() else {
                return Err(DeserializeError::MalformedEnum {
                    found: entries.len(),
                }
                .into());
            };
            (decode_name(name, oid)?, Some((*child, *kind)))
        };
        let kind = variants
            .get(&name)
            .ok_or_else(|| SchemaReadError::UnknownVariant {
                variant: name.clone(),
                expected: variants.keys().cloned().collect(),
            })?;
        let value = match (kind, payload) {
            (VariantKind::Unit, None) => Value::NULL,
            (VariantKind::Unit, Some(_)) => {
                return Err(DeserializeError::UnitVariantIsTree { variant: name }.into());
            }
            (_, None) => {
                return Err(DeserializeError::VariantPayloadIsBlob { variant: name }.into());
            }
            (VariantKind::Tuple(elems), Some((child, kind))) => match elems.as_slice() {
                [inner] => self.read_node(&child, inner, depth + 1, Some(kind))?,
                _ => {
                    let mut buf = Vec::new();
                    let data = self.load(&child, &mut buf, depth + 1, Some(kind))?;
                    let entries = self.sequence(&data, &child, depth + 1)?;
                    self.read_tuple(entries, elems, depth + 1)?.into()
                }
            },
            (VariantKind::Struct(fields), Some((child, kind))) => {
                let mut buf = Vec::new();
                let data = self.load(&child, &mut buf, depth + 1, Some(kind))?;
                let entries = tree_entries(&data, &child)?;
                self.read_struct(entries, fields, depth + 1)?.into()
            }
        };
        let mut object = VObject::new();
        object.insert(name, value);
        Ok(object.into())
    }

    fn read_dynamic(
        &mut self,
        data: &Data<'_>,
        oid: &ObjectId,
        depth: usize,
    ) -> Result<Value, SchemaReadError> {
        if data.kind == Kind::Blob {
            let bytes = text_bytes(data, oid)?;
            return Ok(match std::str::from_utf8(bytes) {
                Ok(text) => Value::from(text.to_owned()),
                Err(_) => Value::from(bytes.to_vec()),
            });
        }
        let entries = tree_entries(data, oid)?;
        if self.is_marker(&entries, depth)? {
            // Null and every empty dynamic container share this encoding.
            return Ok(Value::NULL);
        }
        if ordinal::validate_sequence(entries.iter().map(|(name, _, _)| name)).is_ok() {
            let mut array = VArray::new();
            for (_, child, kind) in entries {
                array.push(self.read_node(&child, &Node::Dynamic, depth + 1, Some(kind))?);
            }
            return Ok(array.into());
        }
        let mut object = VObject::new();
        for (encoded, child, kind) in entries {
            let name = decode_name(&encoded, oid)?;
            let value = self.read_node(&child, &Node::Dynamic, depth + 1, Some(kind))?;
            object.insert(name, value);
        }
        Ok(object.into())
    }
}

fn tree_entries(data: &Data<'_>, oid: &ObjectId) -> Result<Entries, SchemaReadError> {
    Ok(tree_entries_from_data(data, oid)?)
}

fn decode_name(name: &str, oid: &ObjectId) -> Result<String, SchemaReadError> {
    crate::name::unescape(name)
        .map_err(|error| invalid_tree(oid, format!("invalid name {name:?}: {error}")))
}

fn require_blob(data: &Data<'_>, oid: &ObjectId) -> Result<(), SchemaReadError> {
    if data.kind != Kind::Blob {
        return Err(DeserializeError::NotABlob(*oid).into());
    }
    Ok(())
}

fn text_bytes<'a>(data: &Data<'a>, oid: &ObjectId) -> Result<&'a [u8], SchemaReadError> {
    require_blob(data, oid)?;
    data.data
        .strip_suffix(b"\n")
        .ok_or_else(|| DeserializeError::MissingLeafNewline(*oid).into())
}

fn text<'a>(data: &Data<'a>, oid: &ObjectId) -> Result<&'a str, SchemaReadError> {
    std::str::from_utf8(text_bytes(data, oid)?)
        .map_err(|_| DeserializeError::NonUtf8Blob(*oid).into())
}

fn canonical_integer<T: std::str::FromStr + std::fmt::Display>(
    text: &str,
    schema: &'static str,
) -> Result<T, SchemaReadError> {
    let value: T = text.parse().map_err(|_| invalid_scalar(schema, text))?;
    if value.to_string() != text {
        return Err(invalid_scalar(schema, text));
    }
    Ok(value)
}

fn signed<T>(
    data: &Data<'_>,
    oid: &ObjectId,
    schema: &'static str,
) -> Result<Value, SchemaReadError>
where
    T: std::str::FromStr + std::fmt::Display + Into<i128>,
{
    let value: T = canonical_integer(text(data, oid)?, schema)?;
    Ok(VNumber::from_i128(value.into()).into())
}

fn unsigned<T>(
    data: &Data<'_>,
    oid: &ObjectId,
    schema: &'static str,
) -> Result<Value, SchemaReadError>
where
    T: std::str::FromStr + std::fmt::Display + Into<u128>,
{
    let value: T = canonical_integer(text(data, oid)?, schema)?;
    Ok(VNumber::from_u128(value.into()).into())
}

fn invalid_scalar(schema: &'static str, text: &str) -> SchemaReadError {
    SchemaReadError::InvalidScalar {
        schema,
        text: text.to_owned(),
    }
}

fn check_length(expected: usize, found: usize) -> Result<(), SchemaReadError> {
    if expected != found {
        return Err(SchemaReadError::ArrayLenMismatch { expected, found });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ObjectStore;
    use crate::error::SerializeError;
    use crate::schema::write::serialize_value_with_schema;
    use gix_object::{Kind, Write};

    fn schema(root: Node) -> Schema {
        Schema {
            kind: "test".into(),
            root,
            defs: BTreeMap::new(),
        }
    }

    #[test]
    fn cbor_roots_are_verbatim_bytes_and_roundtrip() -> anyhow::Result<()> {
        let doc = schema(Node::Cbor);
        let payloads: &[&[u8]] = &[
            &[0x00],
            &[0x0a],
            &[0xf6],
            &[0x42, 0x00, 0xff],
            &[0x62, b'h', b'i'],
            &[0xa2, 0x01, 0xf5, 0x61, b'a', 0x82, 0xf6, 0x0a],
            &[0xf9, 0x7e, 0x00],
            &[0xd8, 0x20, 0x61, b'a'],
        ];
        for hash in [gix_hash::Kind::Sha1, gix_hash::Kind::Sha256] {
            let store = ObjectStore::with_hash_kind(hash);
            let output = ObjectStore::with_hash_kind(hash);
            for payload in payloads {
                let oid = store
                    .write_buf(Kind::Blob, payload)
                    .map_err(SerializeError::Backend)?;
                let value = deserialize_value_with_schema(&oid, &doc, &store)?;
                assert_eq!(
                    value.as_bytes().map(|bytes| bytes.as_slice()),
                    Some(*payload)
                );
                validate_with_schema(&oid, &doc, &store)?;
                let reencoded = serialize_value_with_schema(&value, &doc, &output)?;
                assert_eq!(reencoded, oid, "CBOR {payload:02x?}");
                assert_eq!(output.get_blob(&reencoded)?.as_deref(), Some(*payload));
            }
        }
        Ok(())
    }

    #[test]
    fn cbor_reads_reject_invalid_and_trailing_bytes() -> anyhow::Result<()> {
        let doc = schema(Node::Cbor);
        let store = ObjectStore::default();
        let invalid: &[(&[u8], &str)] = &[
            (&[], "truncated"),
            (&[0x00, 0x00], "trailing bytes"),
            (&[0x00, 0x0a], "trailing bytes"),
            (&[0x18, 0x17], "non-minimal"),
            (&[0x5f, 0xff], "indefinite"),
            (&[0x62, b'a'], "truncated"),
            (&[0x61, 0xff], "UTF-8"),
            (&[0xa2, 0x01, 0x00, 0x01, 0x01], "duplicate"),
            (&[0xa2, 0x02, 0x00, 0x01, 0x01], "out-of-order"),
            (&[0xfa, 0x3f, 0xc0, 0x00, 0x00], "non-canonical"),
        ];
        for (payload, reason) in invalid {
            let oid = store
                .write_buf(Kind::Blob, payload)
                .map_err(SerializeError::Backend)?;
            let error = deserialize_value_with_schema(&oid, &doc, &store).unwrap_err();
            assert!(matches!(
                &error,
                SchemaReadError::Deserialize(DeserializeError::Reflect(message))
                    if message.contains(&oid.to_string()) && message.contains(reason)
            ));
            assert!(validate_with_schema(&oid, &doc, &store).is_err());
        }
        let mut too_deep = vec![0x81; MAX_VALUE_DEPTH + 1];
        too_deep.push(0xf6);
        let oid = store
            .write_buf(Kind::Blob, &too_deep)
            .map_err(SerializeError::Backend)?;
        assert!(deserialize_value_with_schema(&oid, &doc, &store).is_err());
        Ok(())
    }

    #[test]
    fn cbor_reads_reject_tree_objects_and_mismatched_entry_kinds() -> anyhow::Result<()> {
        let store = ObjectStore::default();
        let doc = schema(Node::Cbor);
        let tree = crate::marker::write_marker_tree(&store)?;
        let error = deserialize_value_with_schema(&tree, &doc, &store).unwrap_err();
        assert!(matches!(
            error,
            SchemaReadError::Deserialize(DeserializeError::NotABlob(oid)) if oid == tree
        ));

        let blob = store
            .write_buf(Kind::Blob, &[0x00])
            .map_err(SerializeError::Backend)?;
        let parent = store
            .write(&gix_object::Tree {
                entries: vec![crate::TreeEntry {
                    mode: crate::EntryMode::from(EntryKind::Tree),
                    filename: "payload".into(),
                    oid: blob,
                }],
            })
            .map_err(SerializeError::Backend)?;
        let doc = schema(Node::Struct(BTreeMap::from([(
            "payload".into(),
            Node::Cbor,
        )])));
        let error = deserialize_value_with_schema(&parent, &doc, &store).unwrap_err();
        assert!(matches!(
            error,
            SchemaReadError::Deserialize(DeserializeError::NotATree(oid)) if oid == blob
        ));
        Ok(())
    }
}
