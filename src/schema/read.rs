//! Schema-directed decoding of bare Git objects into [`facet_value::Value`].
//!
//! A schema supplies the type information absent from the bare tree. Dynamic
//! nodes remain heuristic; CBOR nodes are opaque, validated canonical bytes.

use std::collections::BTreeMap;

use facet_value::{VArray, VNumber, VObject, Value};
use gix_object::{Data, Find, Kind};

use crate::ObjectId;
use crate::de::{
    DecodeBudget, EncodedName, Entry, LeafKind, find_object_with_budget, sort_by_ordinal,
    tree_entries_from_data, validate_marker,
};
use crate::error::{DeserializeError, SchemaReadError};
use crate::ordinal;
use crate::schema::{CheckedSchema, Node, RefError, Schema, VariantKind, invalid_schema};

/// Deserialize the bare object at `root` according to `doc`.
///
/// The document reached here has already passed the full well-formedness walk
/// that produced the [`CheckedSchema`]; it is not re-checked. This does not
/// read a `.schema` envelope. A CBOR node reads as opaque canonical bytes and
/// rejects invalid or trailing data. An unwrapped `Value` cannot distinguish
/// `Some(null)` from `None`, so such an option is rejected rather than
/// returning a value that re-encodes wrongly.
pub fn deserialize_value_with_schema<F: Find + ?Sized>(
    root: &ObjectId,
    doc: &CheckedSchema<'_>,
    store: &F,
) -> Result<Value, SchemaReadError> {
    let schema = doc.schema();
    Reader {
        doc: schema,
        store,
        budget: DecodeBudget::new(),
    }
    .read_node(root, schema.root(), 0, None)
}

/// Check the bare object at `root` against the entire schema.
pub fn validate_with_schema<F: Find + ?Sized>(
    root: &ObjectId,
    doc: &CheckedSchema<'_>,
    store: &F,
) -> Result<(), SchemaReadError> {
    deserialize_value_with_schema(root, doc, store).map(|_| ())
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
        entry_kind: Option<LeafKind>,
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
        entry_kind: Option<LeafKind>,
    ) -> Result<Value, SchemaReadError> {
        // References have no encoded tree level, so a chain of aliases is
        // equivalent to its target.
        let schema = schema
            .resolve(self.doc.defs())
            .map_err(|error| match error {
                RefError::Dangling { name } => SchemaReadError::UnknownRef(name),
                RefError::Cycle { name } => {
                    invalid_schema(format!("unguarded reference cycle through {name:?}"))
                }
            })?;

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
                Ok(self.read_struct(entries, fields, oid, depth)?.into())
            }
            Node::Tuple(elems) => {
                let entries = self.sequence(&data, oid, depth)?;
                Ok(self.read_tuple(entries, elems, depth)?.into())
            }
            Node::List(elem) => {
                let entries = self.sequence(&data, oid, depth)?;
                let mut array = VArray::new();
                for entry in entries {
                    array.push(self.read_node(&entry.oid, elem, depth + 1, Some(entry.kind))?);
                }
                Ok(array.into())
            }
            Node::Array { elem, len } => {
                let entries = self.sequence(&data, oid, depth)?;
                check_length(*len, entries.len())?;
                let mut array = VArray::new();
                for entry in entries {
                    array.push(self.read_node(&entry.oid, elem, depth + 1, Some(entry.kind))?);
                }
                Ok(array.into())
            }
            Node::Map { value, .. } => {
                let entries = tree_entries(&data, oid)?;
                if self.is_marker(&entries, depth)? {
                    return Ok(VObject::new().into());
                }
                let mut object = VObject::new();
                for entry in entries {
                    let name = user_name(&entry.name, oid)?;
                    let value = self.read_node(&entry.oid, value, depth + 1, Some(entry.kind))?;
                    object.insert(name.to_owned(), value);
                }
                Ok(object.into())
            }
            Node::Optional(inner) => {
                let entries = tree_entries(&data, oid)?;
                if self.is_marker(&entries, depth)? {
                    return Ok(Value::NULL);
                }
                let [entry] = entries.as_slice() else {
                    return Err(DeserializeError::MalformedOption {
                        found: entries.len(),
                    }
                    .into());
                };
                if entry.name.encoded() != crate::marker::SOME_KEY {
                    return Err(DeserializeError::MislabeledOption {
                        name: entry.name.encoded().to_owned(),
                    }
                    .into());
                }
                let value = self.read_node(&entry.oid, inner, depth + 1, Some(entry.kind))?;
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

    fn is_marker(&mut self, entries: &[Entry], depth: usize) -> Result<bool, SchemaReadError> {
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
    ) -> Result<Vec<Entry>, SchemaReadError> {
        let entries = tree_entries(data, oid)?;
        if self.is_marker(&entries, depth)? {
            return Ok(Vec::new());
        }
        sort_by_ordinal(&entries)?;
        Ok(entries)
    }

    fn read_tuple(
        &mut self,
        entries: Vec<Entry>,
        elems: &[Node],
        depth: usize,
    ) -> Result<VArray, SchemaReadError> {
        check_length(elems.len(), entries.len())?;
        let mut array = VArray::new();
        for (entry, elem) in entries.into_iter().zip(elems) {
            array.push(self.read_node(&entry.oid, elem, depth + 1, Some(entry.kind))?);
        }
        Ok(array)
    }

    fn read_struct(
        &mut self,
        entries: Vec<Entry>,
        fields: &BTreeMap<String, Node>,
        oid: &ObjectId,
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
        for entry in entries {
            let name = user_name(&entry.name, oid)?;
            let field = fields
                .get(name)
                .ok_or_else(|| SchemaReadError::UnexpectedEntry {
                    entry: name.to_owned(),
                })?;
            let value = self.read_node(&entry.oid, field, depth + 1, Some(entry.kind))?;
            object.insert(name.to_owned(), value);
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
            (decode_blob_name(text(data, oid)?, oid)?, None)
        } else {
            let entries = tree_entries(data, oid)?;
            let [entry] = entries.as_slice() else {
                return Err(DeserializeError::MalformedEnum {
                    found: entries.len(),
                }
                .into());
            };
            (
                user_name(&entry.name, oid)?.to_owned(),
                Some((entry.oid, entry.kind)),
            )
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
                self.read_struct(entries, fields, &child, depth + 1)?.into()
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
        if ordinal::validate_sequence(entries.iter().map(|entry| entry.name.encoded().as_bytes()))
            .is_ok()
        {
            let mut array = VArray::new();
            for entry in entries {
                array.push(self.read_node(
                    &entry.oid,
                    &Node::Dynamic,
                    depth + 1,
                    Some(entry.kind),
                )?);
            }
            return Ok(array.into());
        }
        let mut object = VObject::new();
        for entry in entries {
            let name = user_name(&entry.name, oid)?;
            let value = self.read_node(&entry.oid, &Node::Dynamic, depth + 1, Some(entry.kind))?;
            object.insert(name.to_owned(), value);
        }
        Ok(object.into())
    }
}

fn tree_entries(data: &Data<'_>, oid: &ObjectId) -> Result<Vec<Entry>, SchemaReadError> {
    Ok(tree_entries_from_data(data, oid)?)
}

/// The entry's once-unescaped user name.
///
/// A structural marker name (`none`/`some`) is not a user name; reading one
/// where a user name belongs is refused. The diagnostic names the *containing*
/// tree `oid`, never a child object.
fn user_name<'n>(name: &'n EncodedName, oid: &ObjectId) -> Result<&'n str, SchemaReadError> {
    name.user().ok_or_else(|| {
        invalid_tree(
            oid,
            format!(
                "invalid name {:?}: a structural name is not a user name",
                name.encoded()
            ),
        )
    })
}

/// Validate and unescape an enum tag blob's user name. A blob tag is not a tree
/// entry name, so it is unescaped here, exactly once.
fn decode_blob_name(text: &str, oid: &ObjectId) -> Result<String, SchemaReadError> {
    crate::name::unescape(text)
        .map_err(|error| invalid_tree(oid, format!("invalid name {text:?}: {error}")))
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
    use crate::error::SerializeError;
    use crate::schema::write::serialize_value_with_schema;
    use crate::{EntryKind, ObjectStore};
    use gix_object::{Kind, Write};

    fn schema(root: Node) -> Schema {
        Schema::new("test".into(), root, BTreeMap::new()).expect("valid schema")
    }

    #[test]
    fn cbor_roots_are_verbatim_bytes_and_roundtrip() -> anyhow::Result<()> {
        let doc = schema(Node::Cbor);
        let doc = doc.checked()?;
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
        let doc = doc.checked()?;
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
        let mut too_deep = vec![0x81; crate::limits::MAX_VALUE_DEPTH + 1];
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
        let doc = doc.checked()?;
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
        let doc = doc.checked()?;
        let error = deserialize_value_with_schema(&parent, &doc, &store).unwrap_err();
        assert!(matches!(
            error,
            SchemaReadError::Deserialize(DeserializeError::NotATree(oid)) if oid == blob
        ));
        Ok(())
    }
}
