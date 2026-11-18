//! The codes printed on parking QR stickers, and whose stickers they are.
//!
//! LTA's QR code parking system puts a sticker at every bicycle parking zone.
//! Its code is three letters and four letters or digits (`ABC-1234`), and a
//! ride on any operator's bike can end there. HelloRide prints stickers of
//! its own for zones only its bikes may use: four letters and three digits
//! (`GRAB-001`). With the dash in place the two shapes can't be confused, so a
//! code's shape alone says whose sticker it is, and nobody has to vote on it.
//!
//! Without the dash they can: `GRAB001` reads as `GRA-B001` too. That is why
//! the page asks which sticker it is before it places the dash, and why the
//! API takes codes only with their dash.
//!
//! Anywheel may have stickers of its own as well, but nobody has seen one
//! yet. Supporting them is one more row in [`FORMATS`], once someone has.

use serde::Serialize;

/// The shared operator id for LTA's stickers, which every operator honours.
pub const LTA: &str = "lta";

/// Operators someone can be riding with, for the map's filter. LTA's stickers
/// show for all of them; an operator's own stickers only for its riders.
pub const RIDING: &[&str] = &["helloride", "anywheel"];

/// One kind of sticker. The page builds its sticker switch, the code field's
/// mask and its error message from these, through `GET /api/formats`, so the
/// two can't disagree about what a code looks like.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Format {
    /// Whose sticker: [`LTA`], or the operator that alone honours it.
    pub operator: &'static str,
    /// The sticker switch's label.
    pub label: &'static str,
    /// Who can end a ride there, in a few words.
    pub scope: &'static str,
    /// Letters before the dash.
    pub head: usize,
    /// Characters after it.
    pub tail: usize,
    /// Whether the characters after the dash are digits only, rather than
    /// letters or digits.
    pub tail_digits_only: bool,
    pub example: &'static str,
    /// The format in words, for the code field's error message.
    pub hint: &'static str,
}

pub const FORMATS: &[Format] = &[
    Format {
        operator: LTA,
        label: "LTA",
        scope: "Any operator",
        head: 3,
        tail: 4,
        tail_digits_only: false,
        example: "ABC-1234",
        hint: "3 letters + 4 letters or digits",
    },
    Format {
        operator: "helloride",
        label: "HelloRide",
        scope: "HelloRide only",
        head: 4,
        tail: 3,
        tail_digits_only: true,
        example: "GRAB-001",
        hint: "4 letters + 3 digits",
    },
];

impl Format {
    fn fits(&self, code: &str) -> bool {
        let Some((head, tail)) = code.split_once('-') else {
            return false;
        };
        head.len() == self.head
            && tail.len() == self.tail
            && head.bytes().all(|b| b.is_ascii_uppercase())
            && tail.bytes().all(|b| b.is_ascii_digit() || (!self.tail_digits_only && b.is_ascii_uppercase()))
    }
}

/// The code in its stored form (trimmed, upper case) and its sticker's
/// format, or None when it isn't any sticker's code.
pub fn parse(input: &str) -> Option<(String, &'static Format)> {
    let code = input.trim().to_ascii_uppercase();
    let format = FORMATS.iter().find(|format| format.fits(&code))?;
    Some((code, format))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operator(code: &str) -> Option<&'static str> {
        parse(code).map(|(_, format)| format.operator)
    }

    #[test]
    fn a_code_says_whose_sticker_it_is() {
        assert_eq!(operator("ABC-1234"), Some(LTA));
        assert_eq!(operator("TPY-012A"), Some(LTA));
        assert_eq!(operator("NOV-00CE"), Some(LTA));
        assert_eq!(operator("GRAB-001"), Some("helloride"));
    }

    #[test]
    fn codes_are_stored_upper_case_and_trimmed() {
        assert_eq!(parse(" grab-001 ").map(|(code, _)| code).as_deref(), Some("GRAB-001"));
        assert_eq!(parse("abc-12cd").map(|(code, _)| code).as_deref(), Some("ABC-12CD"));
    }

    #[test]
    fn the_dash_decides_and_must_be_there() {
        // The same seven characters, read both ways.
        assert_eq!(operator("GRA-B001"), Some(LTA));
        assert_eq!(operator("GRAB-001"), Some("helloride"));
        assert_eq!(operator("GRAB001"), None);
        assert_eq!(operator("ABC1234"), None);
    }

    #[test]
    fn anything_else_is_no_code() {
        for bad in [
            "", "-", "AB-1234", "ABCD-1234", "ABC-123", "ABC-12345", "A1C-1234", "ABC-12-4",
            "GRAB-00A", "GRA1-001", "GRAB-0001", "ÀBC-1234", "ABC 1234",
        ] {
            assert_eq!(operator(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn examples_fit_their_own_format_and_no_other() {
        for format in FORMATS {
            let (_, found) = parse(format.example).unwrap();
            assert_eq!(found.operator, format.operator);
        }
    }

    #[test]
    fn riders_choose_among_operators_not_lta() {
        assert!(!RIDING.contains(&LTA));
        for format in FORMATS.iter().filter(|format| format.operator != LTA) {
            assert!(RIDING.contains(&format.operator), "{}", format.operator);
        }
    }
}
