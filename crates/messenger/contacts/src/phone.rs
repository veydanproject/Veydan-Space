// SPDX-FileCopyrightText: 2026 Veydan Project
// SPDX-License-Identifier: LicenseRef-PolyForm-Perimeter-1.0.1

//! Phone numbers in one form: `+` and 7 to 15 digits.
//!
//! The number is private: it never goes into kind 0, only into my own
//! notes and into my own contact card when I choose so. The input may carry
//! spaces, dashes, dots and parentheses; they are dropped, and so is a
//! trunk zero written `(0)` after the country code (`+44 (0) 20…`). Nothing
//! else is rewritten: a leading 8 stays an 8, since only the user knows the
//! country.

/// Digits of a number, without the `+`.
pub const MIN_DIGITS: usize = 7;
pub const MAX_DIGITS: usize = 15;
/// Longer input is not a phone number.
const MAX_INPUT_BYTES: usize = 64;

/// `"+79991234567"` from `"+7 (999) 123-45-67"`; error `"phone_invalid"`.
pub fn normalize_phone(input: &str) -> Result<String, &'static str> {
    let s = input.trim();
    if s.is_empty() || s.len() > MAX_INPUT_BYTES {
        return Err("phone_invalid");
    }
    let s = drop_trunk_zero(s)?;
    let mut digits = String::with_capacity(MAX_DIGITS);
    for (i, c) in s.char_indices() {
        match c {
            '0'..='9' => digits.push(c),
            '+' if i == 0 => {}
            ' ' | '-' | '.' | '(' | ')' => {}
            _ => return Err("phone_invalid"),
        }
    }
    if !valid_digits(&digits) {
        return Err("phone_invalid");
    }
    Ok(format!("+{digits}"))
}

/// `"+44 (0) 20…"` without its `(0)`: the trunk zero that is left out when
/// calling from abroad. It is taken once, after `+` and a country code;
/// anywhere else it is refused, since the user must say what is meant. A
/// bare 0 stays (Italy dials it).
fn drop_trunk_zero(s: &str) -> Result<String, &'static str> {
    let Some(at) = s.find("(0)") else { return Ok(s.to_string()) };
    let before = &s[..at];
    let after = &s[at + 3..];
    if !s.starts_with('+') || !before.bytes().any(|b| b.is_ascii_digit()) || after.contains("(0)") {
        return Err("phone_invalid");
    }
    Ok(format!("{before} {after}"))
}

/// A stored or received number: exactly the form `normalize_phone` gives.
pub fn is_phone(s: &str) -> bool {
    s.strip_prefix('+').is_some_and(valid_digits)
}

/// 7 to 15 ASCII digits; no country code starts with 0.
pub(crate) fn valid_digits(d: &str) -> bool {
    (MIN_DIGITS..=MAX_DIGITS).contains(&d.len())
        && d.bytes().all(|b| b.is_ascii_digit())
        && !d.starts_with('0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_international() {
        assert_eq!(normalize_phone("+79991234567").unwrap(), "+79991234567");
        assert_eq!(normalize_phone("  +1 555 123 4567  ").unwrap(), "+15551234567");
    }

    #[test]
    fn separators_are_dropped() {
        assert_eq!(normalize_phone("+7 (999) 123-45-67").unwrap(), "+79991234567");
        assert_eq!(normalize_phone("+49.30.1234567").unwrap(), "+49301234567");
    }

    #[test]
    fn trunk_zero_in_parentheses_is_dropped() {
        assert_eq!(normalize_phone("+44 (0) 20-7946 0958").unwrap(), "+442079460958");
        assert_eq!(normalize_phone("+49 (0)30 1234567").unwrap(), "+49301234567");
        assert_eq!(normalize_phone("+41(0)44 668 18 00").unwrap(), "+41446681800");
        // A bare 0 stays: Italy keeps it.
        assert_eq!(normalize_phone("+39 06 1234 5678").unwrap(), "+390612345678");
        // A 0 among other digits in parentheses is a digit.
        assert_eq!(normalize_phone("+7 (095) 123-45-67").unwrap(), "+70951234567");
        // Without the country code before it, or twice, it is not clear what is meant.
        for bad in ["(0) 30 1234567", "8 (0) 999 123 45 67", "+49 (0) 30 (0) 1234567", "+(0)49 30 1234567"] {
            assert_eq!(normalize_phone(bad), Err("phone_invalid"), "{bad:?}");
        }
    }

    #[test]
    fn plus_is_optional_and_leading_eight_stays() {
        assert_eq!(normalize_phone("89991234567").unwrap(), "+89991234567");
        assert_eq!(normalize_phone("8 (999) 123-45-67").unwrap(), "+89991234567");
    }

    #[test]
    fn length_bounds() {
        assert_eq!(normalize_phone("+1234567").unwrap(), "+1234567");
        assert_eq!(normalize_phone("+123456789012345").unwrap(), "+123456789012345");
        assert_eq!(normalize_phone("+123456"), Err("phone_invalid"));
        assert_eq!(normalize_phone("+1234567890123456"), Err("phone_invalid"));
        assert_eq!(normalize_phone(""), Err("phone_invalid"));
        assert_eq!(normalize_phone("+"), Err("phone_invalid"));
        assert_eq!(normalize_phone("   "), Err("phone_invalid"));
        let long = format!("+1{}", " ".repeat(80));
        assert_eq!(normalize_phone(&format!("{long}234567")), Err("phone_invalid"));
    }

    #[test]
    fn refuses_other_characters() {
        for bad in [
            "+7 999 123 45 6a",
            "++79991234567",
            "7+9991234567",
            "+7/999/1234567",
            "+7_999_1234567",
            "tel:+79991234567",
            "+7 999 123 45 67 ext 1",
            "+7\u{00a0}9991234567",
            "+7\t9991234567",
            "+7\n9991234567",
            "+٧٩٩٩١٢٣٤٥٦٧",
            "+７９９９１２３４５６７",
            "+7\u{200e}9991234567",
            "+7,999,1234567",
            "+7#9991234567",
        ] {
            assert_eq!(normalize_phone(bad), Err("phone_invalid"), "{bad:?}");
        }
    }

    #[test]
    fn no_leading_zero() {
        assert_eq!(normalize_phone("+0123456789"), Err("phone_invalid"));
        assert_eq!(normalize_phone("0049301234567"), Err("phone_invalid"));
    }

    #[test]
    fn is_phone_accepts_only_the_normal_form() {
        assert!(is_phone("+79991234567"));
        assert!(!is_phone("79991234567"));
        assert!(!is_phone("+7 999 123 45 67"));
        assert!(!is_phone("+0123456789"));
        assert!(!is_phone("+123"));
        assert!(!is_phone(""));
    }

    #[test]
    fn idempotent() {
        for s in ["+7 (999) 123-45-67", "89991234567", "+1.555.123.4567"] {
            let once = normalize_phone(s).unwrap();
            assert_eq!(normalize_phone(&once).unwrap(), once);
            assert!(is_phone(&once));
        }
    }
}
