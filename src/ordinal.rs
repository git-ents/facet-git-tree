//! Canonical positional names and sequence validation.

/// Maximum number of entries in any encoded tree.
pub(crate) const MAX_TREE_ENTRIES: usize = 0x1000;
/// Number of lowercase hexadecimal bytes in a positional name.
pub(crate) const ORDINAL_WIDTH: usize = 3;

/// An invalid ordinal or ordinal sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum OrdinalError {
    #[error("ordinal {index} is outside the range 0..4096")]
    OutOfRange { index: usize },
    #[error("ordinal has {found} bytes instead of 3")]
    InvalidWidth { found: usize },
    #[error("ordinal byte 0x{byte:02x} at position {position} is not lowercase hexadecimal")]
    InvalidDigit { position: usize, byte: u8 },
    #[error("ordinal sequence contains duplicate index {index}")]
    Duplicate { index: usize },
    #[error("ordinal sequence is missing index {expected} before index {found}")]
    Gap { expected: usize, found: usize },
    #[error("tree has more than 4096 entries")]
    TooManyEntries,
}

/// Encode an index as three lowercase hexadecimal bytes.
pub(crate) fn encode(index: usize) -> Result<String, OrdinalError> {
    if index >= MAX_TREE_ENTRIES {
        return Err(OrdinalError::OutOfRange { index });
    }
    Ok(format!("{index:03x}"))
}

/// Decode one ordinal from untrusted tree-entry bytes.
pub(crate) fn decode(name: impl AsRef<[u8]>) -> Result<usize, OrdinalError> {
    let name = name.as_ref();
    if name.len() != ORDINAL_WIDTH {
        return Err(OrdinalError::InvalidWidth { found: name.len() });
    }
    let mut index = 0usize;
    for (position, &byte) in name.iter().enumerate() {
        let digit = match byte {
            b'0'..=b'9' => usize::from(byte - b'0'),
            b'a'..=b'f' => usize::from(byte - b'a' + 10),
            _ => return Err(OrdinalError::InvalidDigit { position, byte }),
        };
        index = (index << 4) | digit;
    }
    Ok(index)
}

/// Validate ordered entry names as the contiguous sequence `000`, `001`, ... .
///
/// This checks the entry cap, each name, duplicates, gaps, and ordering without
/// sorting untrusted entries. It returns the count; callers must separately
/// enforce a schema's expected length and the representation of an empty tree.
pub(crate) fn validate_sequence<I, N>(names: I) -> Result<usize, OrdinalError>
where
    I: IntoIterator<Item = N>,
    N: AsRef<[u8]>,
{
    let mut expected = 0usize;
    for name in names {
        if expected == MAX_TREE_ENTRIES {
            return Err(OrdinalError::TooManyEntries);
        }
        let found = decode(name)?;
        if found < expected {
            return Err(OrdinalError::Duplicate { index: found });
        }
        if found > expected {
            return Err(OrdinalError::Gap { expected, found });
        }
        expected += 1;
    }
    Ok(expected)
}

#[cfg(test)]
mod tests {
    use super::{MAX_TREE_ENTRIES, ORDINAL_WIDTH, OrdinalError, decode, encode, validate_sequence};
    use proptest::prelude::*;

    #[test]
    fn boundary_and_hex_examples() {
        for (index, name) in [
            (0, "000"),
            (9, "009"),
            (10, "00a"),
            (15, "00f"),
            (16, "010"),
            (255, "0ff"),
            (256, "100"),
            (4094, "ffe"),
            (4095, "fff"),
        ] {
            assert_eq!(encode(index).as_deref(), Ok(name));
            assert_eq!(decode(name), Ok(index));
        }
        assert_eq!(
            encode(MAX_TREE_ENTRIES),
            Err(OrdinalError::OutOfRange {
                index: MAX_TREE_ENTRIES
            })
        );
        assert_eq!(
            encode(usize::MAX),
            Err(OrdinalError::OutOfRange { index: usize::MAX })
        );
    }

    #[test]
    fn every_ordinal_round_trips_in_byte_order() {
        let names: Vec<_> = (0..MAX_TREE_ENTRIES)
            .map(|index| {
                let name = encode(index).expect("in range");
                assert_eq!(name.len(), ORDINAL_WIDTH);
                assert_eq!(decode(&name), Ok(index));
                name
            })
            .collect();
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(validate_sequence(&names), Ok(MAX_TREE_ENTRIES));
    }

    #[test]
    fn invalid_widths_and_digits_are_rejected() {
        for name in ["", "0", "00", "0000", "1000", "0x0", "é0"] {
            let error = decode(name).expect_err("not a three-digit lowercase hex ordinal");
            assert!(
                matches!(
                    error,
                    OrdinalError::InvalidWidth { .. } | OrdinalError::InvalidDigit { .. }
                ),
                "{name:?}: {error:?}"
            );
        }
        for (name, position, byte) in [
            ("A00", 0, b'A'),
            ("0B0", 1, b'B'),
            ("00F", 2, b'F'),
            ("00g", 2, b'g'),
            ("0-0", 1, b'-'),
            (" 00", 0, b' '),
            ("+00", 0, b'+'),
        ] {
            assert_eq!(
                decode(name),
                Err(OrdinalError::InvalidDigit { position, byte }),
                "{name:?}"
            );
        }
        assert_eq!(
            decode([b'0', 0xff, b'0']),
            Err(OrdinalError::InvalidDigit {
                position: 1,
                byte: 0xff
            })
        );
    }

    #[test]
    fn sequences_reject_gaps_duplicates_and_out_of_order_entries() {
        assert_eq!(validate_sequence(["000", "001", "002"]), Ok(3));
        assert_eq!(validate_sequence(std::iter::empty::<&str>()), Ok(0));
        assert_eq!(
            validate_sequence(["001"]),
            Err(OrdinalError::Gap {
                expected: 0,
                found: 1
            })
        );
        assert_eq!(
            validate_sequence(["000", "002"]),
            Err(OrdinalError::Gap {
                expected: 1,
                found: 2
            })
        );
        assert_eq!(
            validate_sequence(["000", "001", "001"]),
            Err(OrdinalError::Duplicate { index: 1 })
        );
        assert_eq!(
            validate_sequence(["000", "002", "001"]),
            Err(OrdinalError::Gap {
                expected: 1,
                found: 2
            })
        );
        assert_eq!(
            validate_sequence(["000", "00A"]),
            Err(OrdinalError::InvalidDigit {
                position: 2,
                byte: b'A'
            })
        );
    }

    #[test]
    fn cap_is_enforced_even_for_a_stream_of_entries() {
        let names = (0..MAX_TREE_ENTRIES).map(|index| encode(index).expect("in range"));
        assert_eq!(validate_sequence(names), Ok(MAX_TREE_ENTRIES));
        let too_many = (0..MAX_TREE_ENTRIES)
            .map(|index| encode(index).expect("in range"))
            .chain(std::iter::once("000".to_owned()));
        assert_eq!(
            validate_sequence(too_many),
            Err(OrdinalError::TooManyEntries)
        );
    }

    proptest! {
        #[test]
        fn ordinal_round_trips_and_is_disjoint_from_structural_names(index in 0usize..MAX_TREE_ENTRIES) {
            let name = encode(index).expect("in range");
            prop_assert_eq!(name.len(), ORDINAL_WIDTH);
            for structural in [".schema", ".value", "none", "some"] {
                prop_assert_ne!(name.as_str(), structural);
            }
            prop_assert_eq!(decode(&name), Ok(index));
        }

        #[test]
        fn encoding_is_injective_and_preserves_order(a in 0usize..MAX_TREE_ENTRIES, b in 0usize..MAX_TREE_ENTRIES) {
            let encoded_a = encode(a).expect("in range");
            let encoded_b = encode(b).expect("in range");
            prop_assert_eq!(encoded_a.cmp(&encoded_b), a.cmp(&b));
        }

        #[test]
        fn accepted_arbitrary_three_bytes_reencode_exactly(bytes in any::<[u8; 3]>()) {
            if let Ok(index) = decode(bytes) {
                let name = encode(index).expect("decoded index fits");
                prop_assert_eq!(name.as_bytes(), bytes.as_slice());
            }
        }

        #[test]
        fn contiguous_sequences_are_accepted(len in 0usize..=MAX_TREE_ENTRIES) {
            let names = (0..len).map(|index| encode(index).expect("in range"));
            prop_assert_eq!(validate_sequence(names), Ok(len));
        }

        #[test]
        fn an_interior_gap_is_rejected(len in 2usize..100, omitted in 0usize..100) {
            let omitted = omitted % (len - 1);
            let names = (0..len)
                .filter(|&index| index != omitted)
                .map(|index| encode(index).expect("in range"));
            prop_assert_eq!(
                validate_sequence(names),
                Err(OrdinalError::Gap { expected: omitted, found: omitted + 1 })
            );
        }

        #[test]
        fn a_repeated_ordinal_is_rejected(len in 1usize..100, repeated in 0usize..100) {
            let repeated = repeated % len;
            let mut names: Vec<_> = (0..len).map(|index| encode(index).expect("in range")).collect();
            names.insert(repeated, encode(repeated).expect("in range"));
            prop_assert_eq!(validate_sequence(names), Err(OrdinalError::Duplicate { index: repeated }));
        }
    }
}
