//! Fixed-point verification for the unbound schema-of-schemas.
//!
//! This checks the codec paths exercised by `S0`, not every rule in the wire
//! format, and it does not inspect a repository or a `.schema` envelope. Wire
//! changes outside `S0`, such as binary leaves, may not affect this check.

use gix_hash::Kind as HashKind;
use gix_object::{Find, Write};

use crate::hash::{HashFormat, Sha1, Sha256, Tagged};
use crate::{
    DeserializeError, ObjectId, ObjectStore, Schema, SchemaError, SerializeError, deserialize,
    schema_of, serialize_into,
};

/// SHA-1 Git tree id of the canonical, unbound `schema_of::<Schema>()`.
pub const S0_SHA1: ObjectId = ObjectId::Sha1([
    0xb8, 0xb6, 0x13, 0x90, 0xb2, 0x3a, 0x8d, 0xdc, 0xd7, 0x2a, 0xb6, 0x17, 0x1c, 0x67, 0x6e, 0x89,
    0xfb, 0xf6, 0x7f, 0xad,
]);

/// SHA-256 Git tree id of the canonical, unbound `schema_of::<Schema>()`.
pub const S0_SHA256: ObjectId = ObjectId::Sha256([
    0xe4, 0x90, 0x63, 0x0b, 0x57, 0x48, 0x6d, 0x68, 0xa1, 0x9d, 0x90, 0xbc, 0xd9, 0xa9, 0xde, 0xf2,
    0x7b, 0xa9, 0xf5, 0x87, 0x60, 0xcc, 0xcf, 0x06, 0xe7, 0xeb, 0x08, 0x30, 0x39, 0xfd, 0xc5, 0xe8,
]);

/// The two checked Git object formats and their pinned `S0` ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoctorReport {
    /// The verified SHA-1 id.
    pub sha1: ObjectId,
    /// The verified SHA-256 id.
    pub sha256: ObjectId,
}

/// A failure to generate, encode, decode, or verify `S0`.
#[derive(Debug, thiserror::Error)]
pub enum DoctorError {
    /// The compiled-in schema vocabulary could not be described.
    #[error("cannot derive the schema of Schema")]
    Schema(#[source] SchemaError),
    /// The selected hash format has no pinned `S0` id.
    #[error("unsupported S0 hash format {hash_kind:?}")]
    UnsupportedHash {
        /// The requested Git object hash format.
        hash_kind: HashKind,
    },
    /// The reference id uses a different format than the selected store.
    #[error("S0 pin format {expected:?} does not match selected hash format {hash_kind:?}")]
    HashFormat {
        /// The selected store hash format.
        hash_kind: HashKind,
        /// The format of the supplied id.
        expected: HashKind,
    },
    /// Generating either the initial or round-trip object failed.
    #[error("cannot {stage} S0 with {hash_kind:?}")]
    Encode {
        /// The selected Git object hash format.
        hash_kind: HashKind,
        /// Whether this is the initial encode or the round-trip encode.
        stage: EncodeStage,
        /// The underlying codec error.
        #[source]
        source: SerializeError,
    },
    /// The initial unbound tree did not match the pinned Git object id.
    #[error("S0 {hash_kind:?} id mismatch: expected {expected}, got {actual}")]
    DigestMismatch {
        /// The selected Git object hash format.
        hash_kind: HashKind,
        /// The expected id.
        expected: ObjectId,
        /// The generated id.
        actual: ObjectId,
    },
    /// The generated tree was not readable by the typed bare decoder.
    #[error("cannot decode S0 {oid} with {hash_kind:?}")]
    Decode {
        /// The selected Git object hash format.
        hash_kind: HashKind,
        /// The id of the encoded schema.
        oid: ObjectId,
        /// The underlying codec error.
        #[source]
        source: DeserializeError,
    },
    /// The bare decoder returned a different value than the generated schema.
    #[error("decoded S0 {oid} differs from the generated schema ({hash_kind:?})")]
    ValueMismatch {
        /// The selected Git object hash format.
        hash_kind: HashKind,
        /// The id of the encoded schema.
        oid: ObjectId,
    },
    /// Encoding the decoded value produced a different tree id.
    #[error("S0 {hash_kind:?} re-encode changed its id from {original} to {actual}")]
    RoundTripMismatch {
        /// The selected Git object hash format.
        hash_kind: HashKind,
        /// The original pinned id.
        original: ObjectId,
        /// The id obtained after decoding and re-encoding.
        actual: ObjectId,
    },
}

/// The codec step that failed while encoding `S0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodeStage {
    /// Encoding `schema_of::<Schema>()` into the fresh store.
    Initial,
    /// Re-encoding the schema returned by the bare decoder.
    RoundTrip,
}

impl std::fmt::Display for EncodeStage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Initial => formatter.write_str("encode"),
            Self::RoundTrip => formatter.write_str("re-encode"),
        }
    }
}

/// Verify the pinned `S0` fixed point independently in both supported formats.
///
/// The check stops at the first error, beginning with SHA-1. A passing result
/// does not establish correctness of codec paths that `S0` does not exercise.
pub fn check() -> Result<DoctorReport, DoctorError> {
    let schema = schema_of::<Schema>().map_err(DoctorError::Schema)?;
    let sha1 = verify(&schema, HashKind::Sha1, S0_SHA1)?;
    let sha256 = verify(&schema, HashKind::Sha256, S0_SHA256)?;
    Ok(DoctorReport { sha1, sha256 })
}

/// Verify `S0` in one format against an explicitly supplied reference id.
///
/// Use [`check`] for the built-in pins. This variant is useful when diagnosing
/// a differing expected digest; it still generates `S0` from the compiled-in
/// schema and uses a fresh store in the requested hash format.
pub fn check_against(hash_kind: HashKind, expected: ObjectId) -> Result<ObjectId, DoctorError> {
    if !matches!(hash_kind, HashKind::Sha1 | HashKind::Sha256) {
        return Err(DoctorError::UnsupportedHash { hash_kind });
    }
    if expected.kind() != hash_kind {
        return Err(DoctorError::HashFormat {
            hash_kind,
            expected: expected.kind(),
        });
    }
    let schema = schema_of::<Schema>().map_err(DoctorError::Schema)?;
    verify(&schema, hash_kind, expected)
}

fn verify(
    schema: &Schema,
    hash_kind: HashKind,
    expected: ObjectId,
) -> Result<ObjectId, DoctorError> {
    match hash_kind {
        HashKind::Sha1 => {
            verify_in::<Sha1, _>(schema, hash_kind, expected, &ObjectStore::<Sha1>::new())
        }
        HashKind::Sha256 => {
            verify_in::<Sha256, _>(schema, hash_kind, expected, &ObjectStore::<Sha256>::new())
        }
        other => Err(DoctorError::UnsupportedHash { hash_kind: other }),
    }
}

fn verify_in<F: HashFormat, S>(
    schema: &Schema,
    hash_kind: HashKind,
    expected: ObjectId,
    store: &S,
) -> Result<ObjectId, DoctorError>
where
    S: Find + Write + Tagged<F> + ?Sized,
{
    let initial = serialize_into(schema, store).map_err(|source| DoctorError::Encode {
        hash_kind,
        stage: EncodeStage::Initial,
        source,
    })?;
    if initial.to_raw() != expected {
        return Err(DoctorError::DigestMismatch {
            hash_kind,
            expected,
            actual: initial.to_raw(),
        });
    }
    let decoded: Schema = deserialize(&initial, store).map_err(|source| DoctorError::Decode {
        hash_kind,
        oid: initial.to_raw(),
        source,
    })?;
    if &decoded != schema {
        return Err(DoctorError::ValueMismatch {
            hash_kind,
            oid: initial.to_raw(),
        });
    }
    let reencoded = serialize_into(&decoded, store).map_err(|source| DoctorError::Encode {
        hash_kind,
        stage: EncodeStage::RoundTrip,
        source,
    })?;
    if reencoded.to_raw() != initial.to_raw() {
        return Err(DoctorError::RoundTripMismatch {
            hash_kind,
            original: initial.to_raw(),
            actual: reencoded.to_raw(),
        });
    }
    Ok(initial.to_raw())
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Read;

    use gix_object::{Data, Kind};

    use super::*;

    #[derive(Clone, Copy)]
    enum Fault {
        RejectWrites,
        MissingRoot,
        MalformedRoot,
        WrongSchema,
        RejectReencode,
        AlterReencodedRoot,
    }

    struct FaultyStore<F: HashFormat> {
        inner: ObjectStore<F>,
        hash_kind: HashKind,
        fault: Fault,
        root_read: Cell<bool>,
    }

    impl<F: HashFormat> Find for FaultyStore<F> {
        fn try_find<'a>(
            &self,
            id: &gix_hash::oid,
            buffer: &'a mut Vec<u8>,
        ) -> Result<Option<Data<'a>>, gix_object::find::Error> {
            let root = match self.hash_kind {
                HashKind::Sha1 => S0_SHA1,
                HashKind::Sha256 => S0_SHA256,
                _ => return Ok(None),
            };
            if id == root.as_ref() {
                self.root_read.set(true);
                match self.fault {
                    Fault::MissingRoot => return Ok(None),
                    Fault::MalformedRoot => {
                        buffer.clear();
                        buffer.extend_from_slice(b"not a tree");
                        return Ok(Some(Data {
                            kind: Kind::Tree,
                            data: buffer.as_slice(),
                            object_hash: self.hash_kind,
                        }));
                    }
                    _ => {}
                }
            }
            let data = self.inner.try_find(id, buffer)?;
            Ok(data.map(|data| {
                if matches!(self.fault, Fault::WrongSchema)
                    && data.kind == Kind::Blob
                    && data.data == b"Schema\n"
                {
                    Data {
                        kind: data.kind,
                        // A same-length but wrong schema name, so the pinned
                        // digest must change.
                        data: b"Zchema\n",
                        object_hash: data.object_hash,
                    }
                } else {
                    data
                }
            }))
        }
    }

    impl<F: HashFormat> Write for FaultyStore<F> {
        fn write_stream(
            &self,
            kind: Kind,
            size: u64,
            from: &mut dyn Read,
        ) -> Result<ObjectId, gix_object::write::Error> {
            if matches!(self.fault, Fault::RejectWrites)
                || matches!(self.fault, Fault::RejectReencode) && self.root_read.get()
            {
                return Err("deliberate write failure".into());
            }
            let oid = Write::write_stream(&self.inner, kind, size, from)?;
            let expected = match self.hash_kind {
                HashKind::Sha1 => S0_SHA1,
                HashKind::Sha256 => S0_SHA256,
                _ => return Ok(oid),
            };
            if matches!(self.fault, Fault::AlterReencodedRoot)
                && self.root_read.get()
                && oid == expected
            {
                return Ok(ObjectId::empty_tree(self.hash_kind));
            }
            Ok(oid)
        }

        fn write_buf_with_known_id(
            &self,
            kind: Kind,
            from: &[u8],
            id: ObjectId,
        ) -> Result<ObjectId, gix_object::write::Error> {
            self.inner.write_buf_with_known_id(kind, from, id)
        }

        fn write_stream_with_known_id(
            &self,
            kind: Kind,
            size: u64,
            from: &mut dyn Read,
            id: ObjectId,
        ) -> Result<ObjectId, gix_object::write::Error> {
            self.inner.write_stream_with_known_id(kind, size, from, id)
        }
    }

    impl<F: HashFormat> Tagged<F> for FaultyStore<F> {}

    fn check_fault<F: HashFormat>(
        hash_kind: HashKind,
        expected: ObjectId,
        fault: Fault,
    ) -> DoctorError {
        let store = FaultyStore::<F> {
            inner: ObjectStore::<F>::new(),
            hash_kind,
            fault,
            root_read: Cell::new(false),
        };
        let schema = schema_of::<Schema>().expect("derive S0");
        verify_in(&schema, hash_kind, expected, &store).expect_err("fault must be detected")
    }

    #[test]
    fn changed_schema_is_rejected_by_the_frozen_digest() {
        fn check<F: HashFormat>(hash_kind: HashKind, expected: ObjectId) {
            let original = schema_of::<Schema>().expect("derive S0");
            // A one-character change to the opaque kind label must change the id.
            let schema = Schema::new(
                format!("{}!", original.kind()),
                original.root().clone(),
                original.defs().clone(),
            )
            .expect("valid changed schema");
            let store = ObjectStore::<F>::new();
            let changed = serialize_into(&schema, &store).expect("encode changed schema");
            assert_ne!(changed.to_raw(), expected);
            assert!(matches!(
                verify_in(&schema, hash_kind, expected, &store),
                Err(DoctorError::DigestMismatch { hash_kind: found, expected: pin, actual })
                    if found == hash_kind && pin == expected && actual == changed.to_raw()
            ));
        }
        check::<Sha1>(HashKind::Sha1, S0_SHA1);
        check::<Sha256>(HashKind::Sha256, S0_SHA256);
    }

    #[test]
    fn reports_backend_and_fixed_point_failures_in_both_formats() {
        fn check<F: HashFormat>(hash_kind: HashKind, expected: ObjectId) {
            assert!(matches!(
                check_fault::<F>(hash_kind, expected, Fault::RejectWrites),
                DoctorError::Encode {
                    hash_kind: found,
                    stage: EncodeStage::Initial,
                    source: SerializeError::Backend(_),
                } if found == hash_kind
            ));
            assert!(matches!(
                check_fault::<F>(hash_kind, expected, Fault::MissingRoot),
                DoctorError::Decode {
                    hash_kind: found,
                    oid,
                    source: DeserializeError::NotFound(missing),
                } if found == hash_kind && oid == expected && missing == expected
            ));
            assert!(matches!(
                check_fault::<F>(hash_kind, expected, Fault::MalformedRoot),
                DoctorError::Decode {
                    hash_kind: found,
                    oid,
                    source: DeserializeError::Reflect(_),
                } if found == hash_kind && oid == expected
            ));
            assert!(matches!(
                check_fault::<F>(hash_kind, expected, Fault::WrongSchema),
                DoctorError::ValueMismatch { hash_kind: found, oid }
                    if found == hash_kind && oid == expected
            ));
            assert!(matches!(
                check_fault::<F>(hash_kind, expected, Fault::RejectReencode),
                DoctorError::Encode {
                    hash_kind: found,
                    stage: EncodeStage::RoundTrip,
                    source: SerializeError::Backend(_),
                } if found == hash_kind
            ));
            assert!(matches!(
                check_fault::<F>(hash_kind, expected, Fault::AlterReencodedRoot),
                DoctorError::RoundTripMismatch { hash_kind: found, original, actual }
                    if found == hash_kind && original == expected && actual == ObjectId::empty_tree(hash_kind)
            ));
        }
        check::<Sha1>(HashKind::Sha1, S0_SHA1);
        check::<Sha256>(HashKind::Sha256, S0_SHA256);
    }
}
