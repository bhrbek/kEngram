//! Stable thought citations carried by content-bearing MCP responses.
//!
//! The grammar is intentionally small and byte-oriented:
//! `[kg:xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx]`.  Only canonical lowercase
//! UUIDs are directives.  `[[kg:uuid]]` stores one literal `[kg:uuid]`.

use std::collections::HashSet;

use kengram_core::ThoughtId;

pub const MAX_CITATION_OCCURRENCES: usize = 32;
pub const MAX_DISTINCT_CITATIONS: usize = 16;

const TOKEN_PREFIX: &[u8] = b"[kg:";
const UUID_LEN: usize = 36;
const TOKEN_LEN: usize = TOKEN_PREFIX.len() + UUID_LEN + 1;
const ESCAPED_TOKEN_LEN: usize = TOKEN_LEN + 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCitationContent {
    pub stripped_content: String,
    pub ordered_origin_ids: Vec<ThoughtId>,
    pub distinct_origin_ids: Vec<ThoughtId>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CitationError {
    #[error("too many citation occurrences: {got} (maximum {max}); escaped literals do not count")]
    TooManyOccurrences { got: usize, max: usize },

    #[error("too many distinct citation UUIDs: {got} (maximum {max})")]
    TooManyDistinct { got: usize, max: usize },
}

/// Format the one stable, full-UUID citation token.
pub fn format_citation(thought_id: ThoughtId) -> String {
    format!("[kg:{thought_id}]")
}

/// Parse citation directives and materialize the exact stored-content bytes.
///
/// The scan is linear in input bytes. Malformed and non-canonical near-tokens
/// remain byte-identical. The only whitespace changes are the frozen
/// single-U+0020 separator rules adjacent to a removed directive.
pub fn prepare_content(raw: &str) -> Result<PreparedCitationContent, CitationError> {
    let bytes = raw.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut ordered_origin_ids = Vec::new();
    let mut distinct_origin_ids = Vec::new();
    let mut seen = HashSet::new();
    let mut i = 0;

    while i < bytes.len() {
        // The exact escape form keeps the citation literal while dropping
        // only the outer bracket pair.
        if i + ESCAPED_TOKEN_LEN <= bytes.len()
            && bytes[i] == b'['
            && let Some((_, token_end)) = canonical_token_at(bytes, i + 1)
            && token_end + 1 < bytes.len()
            && bytes[token_end + 1] == b']'
        {
            output.extend_from_slice(&bytes[i + 1..=token_end]);
            i = token_end + 2;
            continue;
        }

        if let Some((thought_id, token_end)) = canonical_token_at(bytes, i) {
            ordered_origin_ids.push(thought_id);
            if ordered_origin_ids.len() > MAX_CITATION_OCCURRENCES {
                return Err(CitationError::TooManyOccurrences {
                    got: ordered_origin_ids.len(),
                    max: MAX_CITATION_OCCURRENCES,
                });
            }
            if seen.insert(thought_id) {
                distinct_origin_ids.push(thought_id);
                if distinct_origin_ids.len() > MAX_DISTINCT_CITATIONS {
                    return Err(CitationError::TooManyDistinct {
                        got: distinct_origin_ids.len(),
                        max: MAX_DISTINCT_CITATIONS,
                    });
                }
            }

            let mut next = token_end + 1;
            let left_is_exactly_one_space = output.last() == Some(&b' ')
                && (output.len() == 1 || output[output.len() - 2] != b' ');
            let right_is_exactly_one_space =
                bytes.get(next) == Some(&b' ') && bytes.get(next + 1) != Some(&b' ');

            if next == bytes.len() {
                // Token at EOF: remove one immediately preceding separator,
                // but preserve runs of two or more spaces.
                if left_is_exactly_one_space {
                    output.pop();
                }
            } else if output.is_empty() && right_is_exactly_one_space {
                // Token at byte zero: remove one immediately following
                // separator, but preserve runs of two or more spaces.
                next += 1;
            } else if left_is_exactly_one_space && right_is_exactly_one_space {
                // Interior token: deletion would create exactly two spaces;
                // keep the left separator and delete the right one.
                next += 1;
            }

            i = next;
            continue;
        }

        output.push(bytes[i]);
        i += 1;
    }

    Ok(PreparedCitationContent {
        // Removing ASCII grammar bytes from valid UTF-8 cannot split a
        // multi-byte scalar; exact escape replacement is ASCII-only too.
        stripped_content: String::from_utf8(output)
            .expect("citation stripping preserves UTF-8 boundaries"),
        ordered_origin_ids,
        distinct_origin_ids,
    })
}

/// Return `(id, inclusive_closing_bracket_index)` for an exact token.
fn canonical_token_at(bytes: &[u8], start: usize) -> Option<(ThoughtId, usize)> {
    let end_exclusive = start.checked_add(TOKEN_LEN)?;
    if end_exclusive > bytes.len() || &bytes[start..start + TOKEN_PREFIX.len()] != TOKEN_PREFIX {
        return None;
    }
    let uuid_start = start + TOKEN_PREFIX.len();
    let uuid_end = uuid_start + UUID_LEN;
    if bytes[uuid_end] != b']' {
        return None;
    }
    let uuid_text = std::str::from_utf8(&bytes[uuid_start..uuid_end]).ok()?;
    let parsed = uuid::Uuid::parse_str(uuid_text).ok()?;
    if parsed.hyphenated().to_string() != uuid_text {
        return None;
    }
    Some((ThoughtId::from(parsed), uuid_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "10000000-0000-4000-8000-000000000001";
    const B: &str = "20000000-0000-4000-8000-000000000002";
    const LOWER_HEX: &str = "abcdefab-cdef-4abc-8def-abcdefabcdef";

    #[test]
    fn citation_codec_full_uuid_caps_escape_and_round_trip() {
        let a = A.parse::<ThoughtId>().unwrap();
        let b = B.parse::<ThoughtId>().unwrap();
        assert_eq!(format_citation(a), format!("[kg:{A}]"));

        let prepared = prepare_content(&format!(
            "[kg:{A}] alpha [kg:{A}] middle [kg:{B}] [[kg:{A}]] [kg:BAD] [kg:{B}]"
        ))
        .unwrap();
        assert_eq!(
            prepared.stripped_content,
            format!("alpha middle [kg:{A}] [kg:BAD]")
        );
        assert_eq!(prepared.ordered_origin_ids, vec![a, a, b, b]);
        assert_eq!(prepared.distinct_origin_ids, vec![a, b]);

        // Only exact canonical lowercase full UUIDs are directives.
        for malformed in [
            format!("[KG:{A}]"),
            format!("[kg:{}]", LOWER_HEX.to_ascii_uppercase()),
            "[kg:10000000-0000-4000-8000-0000000000]".to_string(),
            format!("[kg:{A}x]"),
        ] {
            let literal = prepare_content(&malformed).unwrap();
            assert_eq!(literal.stripped_content, malformed);
            assert!(literal.distinct_origin_ids.is_empty());
        }

        // Frozen single-space separator behavior; tabs/newlines/runs survive.
        assert_eq!(
            prepare_content(&format!("left [kg:{A}] right"))
                .unwrap()
                .stripped_content,
            "left right"
        );
        assert_eq!(
            prepare_content(&format!("left  [kg:{A}]  right"))
                .unwrap()
                .stripped_content,
            "left    right"
        );
        assert_eq!(
            prepare_content(&format!("left\t[kg:{A}]\nright"))
                .unwrap()
                .stripped_content,
            "left\t\nright"
        );
        assert_eq!(
            prepare_content(&format!("[kg:{A}] start"))
                .unwrap()
                .stripped_content,
            "start"
        );
        assert_eq!(
            prepare_content(&format!("end [kg:{A}]"))
                .unwrap()
                .stripped_content,
            "end"
        );

        let at_occurrence_cap = std::iter::repeat_n(format!("[kg:{A}]"), 32)
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(
            prepare_content(&at_occurrence_cap)
                .unwrap()
                .ordered_origin_ids
                .len(),
            32
        );
        let over_occurrence_cap = format!("{at_occurrence_cap}[kg:{A}]");
        assert_eq!(
            prepare_content(&over_occurrence_cap).unwrap_err(),
            CitationError::TooManyOccurrences { got: 33, max: 32 }
        );

        let distinct = (0_u128..17)
            .map(|offset| {
                let id =
                    uuid::Uuid::from_u128(0x30000000_0000_4000_8000_000000000000_u128 + offset);
                format!("[kg:{id}]")
            })
            .collect::<String>();
        assert_eq!(
            prepare_content(&distinct).unwrap_err(),
            CitationError::TooManyDistinct { got: 17, max: 16 }
        );
    }
}
