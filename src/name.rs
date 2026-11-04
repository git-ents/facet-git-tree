//! Canonical user-name encoding for Git tree entries.

/// Maximum length of an encoded tree-entry name, in bytes.
pub(crate) const MAX_ENCODED_NAME_BYTES: usize = 255;

/// An invalid or unrepresentable user-name encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum NameError {
    #[error("encoded name exceeds 255 bytes")]
    TooLong,
    #[error("a structural name is not a user name")]
    Structural,
    #[error("malformed percent escape at encoded byte {offset}")]
    MalformedEscape { offset: usize },
    #[error("encoded user name is not canonical")]
    NonCanonical,
    #[error("encoded name or decoded escape is not valid UTF-8")]
    InvalidUtf8(#[from] std::str::Utf8Error),
}

fn is_safe_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

fn is_verbatim(bytes: &[u8]) -> bool {
    let Some((&first, _)) = bytes.split_first() else {
        return false;
    };
    !matches!(first, b'.' | b'%')
        && bytes != b"none"
        && bytes != b"some"
        && bytes.iter().copied().all(is_safe_byte)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

/// Encode an arbitrary UTF-8 user name, rejecting an encoded length above 255.
///
/// Escaped names have an initial `%` in addition to any `%XX` byte escapes.
/// The result never collides with a structural name or contains a slash or NUL.
pub(crate) fn escape(name: &str) -> Result<String, NameError> {
    if name.len() > MAX_ENCODED_NAME_BYTES {
        return Err(NameError::TooLong);
    }
    if is_verbatim(name.as_bytes()) {
        return Ok(name.to_owned());
    }

    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(name.len() + 1);
    encoded.push('%');
    for byte in name.bytes() {
        let width = if is_safe_byte(byte) { 1 } else { 3 };
        if encoded.len() + width > MAX_ENCODED_NAME_BYTES {
            return Err(NameError::TooLong);
        }
        if is_safe_byte(byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    Ok(encoded)
}

/// Decode a canonical user name from untrusted tree-entry bytes.
///
/// Structural names are not user names and must be handled separately in the
/// context that expects them. Invalid UTF-8, non-canonical forms, malformed
/// escapes, and names longer than 255 encoded bytes are rejected.
pub(crate) fn unescape(name: impl AsRef<[u8]>) -> Result<String, NameError> {
    let encoded = name.as_ref();
    if encoded.len() > MAX_ENCODED_NAME_BYTES {
        return Err(NameError::TooLong);
    }
    if matches!(encoded, b".schema" | b".value" | b"none" | b"some") {
        return Err(NameError::Structural);
    }

    let Some(escaped) = encoded.strip_prefix(b"%") else {
        let decoded = std::str::from_utf8(encoded)?;
        return if is_verbatim(encoded) {
            Ok(decoded.to_owned())
        } else {
            Err(NameError::NonCanonical)
        };
    };

    let mut decoded = Vec::with_capacity(escaped.len());
    let mut bytes = escaped.iter().copied().enumerate();
    while let Some((position, byte)) = bytes.next() {
        if byte != b'%' {
            decoded.push(byte);
            continue;
        }
        let offset = position + 1;
        let Some((_, high)) = bytes.next() else {
            return Err(NameError::MalformedEscape { offset });
        };
        let Some((_, low)) = bytes.next() else {
            return Err(NameError::MalformedEscape { offset });
        };
        let (Some(high), Some(low)) = (hex_value(high), hex_value(low)) else {
            return Err(NameError::MalformedEscape { offset });
        };
        decoded.push((high << 4) | low);
    }

    let decoded = std::str::from_utf8(&decoded)?.to_owned();
    match escape(&decoded) {
        Ok(canonical) if canonical.as_bytes() == encoded => Ok(decoded),
        Ok(_) | Err(NameError::TooLong) => Err(NameError::NonCanonical),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_ENCODED_NAME_BYTES, NameError, escape, unescape};
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn small_string() -> impl Strategy<Value = String> {
        proptest::collection::vec(any::<char>(), 0..=20)
            .prop_map(|chars| chars.into_iter().collect())
    }

    #[test]
    fn examples_follow_the_whole_name_rule() {
        for (name, encoded) in [
            ("title", "title"),
            ("author.name", "author.name"),
            ("_", "_"),
            ("none", "%none"),
            ("some", "%some"),
            (".schema", "%.schema"),
            (".value", "%.value"),
            (".git", "%.git"),
            (".", "%."),
            ("..", "%.."),
            ("a/b", "%a%2Fb"),
            ("a b", "%a%20b"),
            ("naïve", "%na%C3%AFve"),
            ("", "%"),
            ("%", "%%25"),
            ("a%b", "%a%25b"),
            ("\0", "%%00"),
            ("\n", "%%0A"),
            ("\u{10ffff}", "%%F4%8F%BF%BF"),
        ] {
            assert_eq!(escape(name).as_deref(), Ok(encoded), "{name:?}");
            assert_eq!(unescape(encoded).as_deref(), Ok(name), "{encoded:?}");
        }
    }

    #[test]
    fn all_ascii_bytes_round_trip_to_distinct_names() {
        let mut names = HashSet::new();
        for byte in 0..=127u8 {
            let name = char::from(byte).to_string();
            let encoded = escape(&name).expect("ASCII name fits");
            assert!(names.insert(encoded.clone()), "collision for {name:?}");
            assert_eq!(unescape(&encoded).as_deref(), Ok(name.as_str()));
        }
    }

    #[test]
    fn structural_names_are_not_decoded_as_user_names() {
        for name in [".schema", ".value", "none", "some"] {
            assert_eq!(unescape(name), Err(NameError::Structural));
            let encoded = escape(name).expect("short name");
            assert_ne!(encoded, name);
            assert_eq!(unescape(encoded).as_deref(), Ok(name));
        }
    }

    #[test]
    fn noncanonical_names_are_rejected() {
        for encoded in [
            "", ".", "..", ".hidden", "a/b", "a b", "a%2Fb", "%title", "%74itle", "%%74itle",
            "%%2f", "%%41", "%%2E", "%a%2fb", "naïve", "%naïve", "%a b",
        ] {
            assert_eq!(
                unescape(encoded),
                Err(NameError::NonCanonical),
                "{encoded:?}"
            );
        }
    }

    #[test]
    fn malformed_escape_reports_its_encoded_offset() {
        for (encoded, offset) in [
            ("%%", 1),
            ("%%A", 1),
            ("%%GG", 1),
            ("%%0g", 1),
            ("%safe%", 5),
            ("%a%2", 2),
            ("%%%2F", 1),
        ] {
            assert_eq!(
                unescape(encoded),
                Err(NameError::MalformedEscape { offset }),
                "{encoded:?}"
            );
        }
    }

    #[test]
    fn invalid_utf8_is_rejected_before_and_after_percent_decoding() {
        let invalid: &[&[u8]] = &[
            b"\xff",
            b"%\xff",
            b"%%FF",
            b"%%C3",
            b"%%C0%80",
            b"%%ED%A0%80",
            b"%%F4%90%80%80",
        ];
        for encoded in invalid {
            assert!(
                matches!(unescape(encoded), Err(NameError::InvalidUtf8(_))),
                "{encoded:?}"
            );
        }
    }

    #[test]
    fn limit_counts_encoded_bytes_including_prefix_and_expansion() {
        let verbatim = "a".repeat(MAX_ENCODED_NAME_BYTES);
        assert_eq!(escape(&verbatim).as_deref(), Ok(verbatim.as_str()));
        assert_eq!(unescape(&verbatim).as_deref(), Ok(verbatim.as_str()));
        assert_eq!(escape(&format!("{verbatim}a")), Err(NameError::TooLong));
        assert_eq!(unescape(format!("{verbatim}a")), Err(NameError::TooLong));

        let dotted = format!(".{}", "a".repeat(253));
        let encoded = escape(&dotted).expect("255 encoded bytes");
        assert_eq!(encoded.len(), MAX_ENCODED_NAME_BYTES);
        assert_eq!(unescape(&encoded).as_deref(), Ok(dotted.as_str()));
        assert_eq!(escape(&format!("{dotted}a")), Err(NameError::TooLong));

        let unicode = format!("{}ab", "é".repeat(42));
        let encoded = escape(&unicode).expect("255 encoded bytes");
        assert_eq!(encoded.len(), MAX_ENCODED_NAME_BYTES);
        assert_eq!(unescape(&encoded).as_deref(), Ok(unicode.as_str()));
        assert_eq!(escape(&format!("{unicode}c")), Err(NameError::TooLong));

        // This raw name fits the limit, but its canonical form would not.
        let noncanonical = format!("%{}", "é".repeat(127));
        assert_eq!(noncanonical.len(), MAX_ENCODED_NAME_BYTES);
        assert_eq!(unescape(noncanonical), Err(NameError::NonCanonical));
    }

    proptest! {
        #[test]
        fn round_trip_is_legal_and_disjoint_from_structural_names(name in small_string()) {
            let encoded = escape(&name).expect("20 Unicode scalars always fit");
            prop_assert!(encoded.len() <= MAX_ENCODED_NAME_BYTES);
            prop_assert!(!encoded.is_empty());
            prop_assert!(!encoded.starts_with('.'));
            prop_assert!(encoded.is_ascii());
            prop_assert!(!encoded.as_bytes().contains(&b'/'));
            prop_assert!(!encoded.as_bytes().contains(&0));
            for structural in [".schema", ".value", "none", "some"] {
                prop_assert_ne!(encoded.as_str(), structural);
            }
            prop_assert_eq!(unescape(&encoded).expect("canonical name"), name);
        }

        #[test]
        fn distinct_user_names_have_distinct_encodings(a in small_string(), b in small_string()) {
            let same = a == b;
            let encoded_a = escape(&a).expect("short name");
            let encoded_b = escape(&b).expect("short name");
            prop_assert_eq!(encoded_a == encoded_b, same);
        }

        #[test]
        fn accepted_arbitrary_bytes_reencode_exactly(bytes in proptest::collection::vec(any::<u8>(), 0..=280)) {
            if let Ok(decoded) = unescape(&bytes) {
                let encoded = escape(&decoded).expect("accepted name is representable");
                prop_assert_eq!(encoded.as_bytes(), bytes.as_slice());
            }
        }

        #[test]
        fn unnecessary_escape_prefix_is_rejected(name in "[A-Za-z0-9_-][A-Za-z0-9._-]{0,25}") {
            prop_assume!(name != "none" && name != "some");
            prop_assert_eq!(unescape(format!("%{name}")), Err(NameError::NonCanonical));
        }
    }
}
