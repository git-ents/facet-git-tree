//! Canonical floating-point text used by every codec path.

/// Format an `f32` using the frozen wire spelling.
pub(crate) fn format_f32(value: f32) -> String {
    if value.is_nan() {
        "nan".to_owned()
    } else if value == 0.0 {
        "0.0".to_owned()
    } else if value == f32::INFINITY {
        "inf".to_owned()
    } else if value == f32::NEG_INFINITY {
        "-inf".to_owned()
    } else {
        ryu::Buffer::new().format_finite(value).to_owned()
    }
}

/// Format an `f64` using the frozen wire spelling.
pub(crate) fn format_f64(value: f64) -> String {
    if value.is_nan() {
        "nan".to_owned()
    } else if value == 0.0 {
        "0.0".to_owned()
    } else if value == f64::INFINITY {
        "inf".to_owned()
    } else if value == f64::NEG_INFINITY {
        "-inf".to_owned()
    } else {
        ryu::Buffer::new().format_finite(value).to_owned()
    }
}

/// Parse only canonical `f32` text, yielding the canonical quiet NaN.
pub(crate) fn parse_f32(text: &str) -> Result<f32, ()> {
    match text {
        "nan" => Ok(f32::from_bits(0x7fc0_0000)),
        "inf" => Ok(f32::INFINITY),
        "-inf" => Ok(f32::NEG_INFINITY),
        _ => {
            let value = text.parse::<f32>().map_err(|_| ())?;
            if !value.is_finite()
                || (value == 0.0 && value.is_sign_negative())
                || format_f32(value) != text
            {
                return Err(());
            }
            Ok(value)
        }
    }
}

/// Parse only canonical `f64` text, yielding the canonical quiet NaN.
pub(crate) fn parse_f64(text: &str) -> Result<f64, ()> {
    match text {
        "nan" => Ok(f64::from_bits(0x7ff8_0000_0000_0000)),
        "inf" => Ok(f64::INFINITY),
        "-inf" => Ok(f64::NEG_INFINITY),
        _ => {
            let value = text.parse::<f64>().map_err(|_| ())?;
            if !value.is_finite()
                || (value == 0.0 && value.is_sign_negative())
                || format_f64(value) != text
            {
                return Err(());
            }
            Ok(value)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_f32, format_f64, parse_f32, parse_f64};
    use proptest::prelude::*;

    #[test]
    fn canonical_special_values_and_ryu_ties() {
        assert_eq!(format_f32(f32::from_bits(0x4895_1f84)), "305404.12");
        assert_eq!(format_f32(-0.0), "0.0");
        assert_eq!(format_f64(-0.0), "0.0");
        assert_eq!(format_f64(f64::NAN), "nan");
        assert_eq!(format_f64(f64::INFINITY), "inf");
        assert_eq!(format_f64(f64::NEG_INFINITY), "-inf");
        assert_eq!(parse_f32("nan").unwrap().to_bits(), 0x7fc0_0000);
        assert_eq!(parse_f64("nan").unwrap().to_bits(), 0x7ff8_0000_0000_0000);
        for text in [
            "-0.0", "-0e0", "0", "1", "+1.0", "01.0", "NaN", "+inf", "infinity", "1e999",
        ] {
            assert!(parse_f32(text).is_err(), "{text}");
            assert!(parse_f64(text).is_err(), "{text}");
        }
        assert!(parse_f32("1e39").is_err());
    }

    proptest! {
        #[test]
        fn f32_canonical_text_roundtrips(bits in any::<u32>()) {
            let value = f32::from_bits(bits);
            let text = format_f32(value);
            let decoded = parse_f32(&text).unwrap();
            prop_assert_eq!(format_f32(decoded), text);
            if value.is_finite() && value != 0.0 {
                prop_assert_eq!(decoded.to_bits(), bits);
            }
        }

        #[test]
        fn f64_canonical_text_roundtrips(bits in any::<u64>()) {
            let value = f64::from_bits(bits);
            let text = format_f64(value);
            let decoded = parse_f64(&text).unwrap();
            prop_assert_eq!(format_f64(decoded), text);
            if value.is_finite() && value != 0.0 {
                prop_assert_eq!(decoded.to_bits(), bits);
            }
        }
    }
}
