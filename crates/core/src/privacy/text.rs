use super::mapping::PrivacyMapping;

pub fn anonymize(input: &str, mapping: &mut PrivacyMapping) -> String {
    let mut known = mapping.known_anonymous_tokens();

    transform_chunks(input, |chunk| {
        if known.contains(chunk) {
            return chunk.to_owned();
        }

        let replacement = mapping
            .existing_alias_for_original(chunk)
            .unwrap_or_else(|| mapping.string_value(chunk));

        known.insert(replacement.clone());

        replacement
    })
}

pub fn deanonymize(input: &str, mapping: &PrivacyMapping) -> String {
    transform_chunks(input, |chunk| {
        mapping
            .original_for_alias(chunk)
            .unwrap_or(chunk)
            .to_owned()
    })
}

fn transform_chunks<F>(input: &str, mut transform: F) -> String
where
    F: FnMut(&str) -> String,
{
    let mut result = String::with_capacity(input.len());

    let mut start = None;

    for (index, character) in input.char_indices() {
        if character.is_whitespace() {
            if let Some(chunk_start) = start.take() {
                result.push_str(&transform(&input[chunk_start..index]));
            }

            result.push(character);
        } else if start.is_none() {
            start = Some(index);
        }
    }

    if let Some(chunk_start) = start {
        result.push_str(&transform(&input[chunk_start..]));
    }

    result
}

// Unlike `anonymize`, this mode only replaces originals already recorded in
// the scope's mapping. It does not create new aliases or recognize new secrets.
// A character trie keeps matching independent of the number of known entries.
#[derive(Default)]
struct MatchNode {
    edges: std::collections::BTreeMap<char, usize>,
    replacement: Option<String>,
}

fn known_replacements(mapping: &PrivacyMapping, reverse: bool) -> Vec<(String, String)> {
    let originals = mapping
        .identifiers
        .keys()
        .chain(mapping.strings.keys())
        .chain(mapping.integers.keys())
        .chain(mapping.floats.keys())
        .chain(mapping.bitstrings.keys())
        .chain(mapping.redactions.keys());

    let mut entries = std::collections::BTreeMap::<String, String>::new();
    for original in originals {
        // Extremely short matches are too ambiguous in natural-language errors.
        if original.chars().count() < 3 {
            continue;
        }
        if let Some(alias) = mapping.existing_alias_for_original(original) {
            let (source, target) = if reverse {
                (alias, original.clone())
            } else {
                (original.clone(), alias)
            };
            entries.entry(source).or_insert(target);
        }
    }

    if !reverse {
        // Do not anonymize a pseudonym for the second time.
        for alias in mapping.known_anonymous_tokens() {
            entries.insert(alias.clone(), alias);
        }
    }

    entries.into_iter().collect()
}

fn word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

fn boundary_ok(input: &str, start: usize, end: usize) -> bool {
    let first = input[start..end].chars().next().unwrap_or(' ');
    let last = input[start..end].chars().next_back().unwrap_or(' ');
    let before = input[..start].chars().next_back();
    let after = input[end..].chars().next();

    (!word_character(first) || before.is_none_or(|value| !word_character(value)))
        && (!word_character(last) || after.is_none_or(|value| !word_character(value)))
}

fn replace_known(input: &str, replacements: Vec<(String, String)>) -> String {
    let mut nodes = vec![MatchNode::default()];
    for (source, replacement) in replacements {
        if source.is_empty() {
            continue;
        }
        let mut node = 0;
        for character in source.chars() {
            let next = match nodes[node].edges.get(&character) {
                Some(&next) => next,
                None => {
                    let next = nodes.len();
                    nodes.push(MatchNode::default());
                    nodes[node].edges.insert(character, next);
                    next
                }
            };
            node = next;
        }
        nodes[node].replacement = Some(replacement);
    }

    let mut output = String::with_capacity(input.len());
    let mut start = 0;
    while start < input.len() {
        let mut node = 0;
        let mut best = None;
        for (offset, character) in input[start..].char_indices() {
            let Some(&next) = nodes[node].edges.get(&character) else {
                break;
            };
            node = next;
            if let Some(replacement) = &nodes[node].replacement {
                let end = start + offset + character.len_utf8();
                if boundary_ok(input, start, end) {
                    best = Some((end, replacement));
                }
            }
        }
        if let Some((end, replacement)) = best {
            output.push_str(replacement);
            start = end;
        } else {
            let character = input[start..].chars().next().expect("valid UTF-8");
            output.push(character);
            start += character.len_utf8();
        }
    }
    output
}

pub fn anonymize_known(input: &str, mapping: &PrivacyMapping) -> String {
    replace_known(input, known_replacements(mapping, false))
}

pub fn deanonymize_known(input: &str, mapping: &PrivacyMapping) -> String {
    replace_known(input, known_replacements(mapping, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_text_preserves_error_message_and_round_trips() {
        let mut mapping = PrivacyMapping::default();
        let table = mapping.identifier("secret_orders", "table");
        let email = mapping.string_value("admin@secret.internal");
        let source = "ERROR: relation secret_orders does not exist\nDETAIL: owner admin@secret.internal, unknown_token";
        let anonymized = anonymize_known(source, &mapping);
        assert_eq!(
            anonymized,
            format!("ERROR: relation {table} does not exist\nDETAIL: owner {email}, unknown_token"),
        );
        assert_eq!(deanonymize_known(&anonymized, &mapping), source);
        assert!(mapping.strings.len() == 1);
    }

    #[test]
    fn known_text_does_not_replace_identifier_substrings_or_unknowns() {
        let mut mapping = PrivacyMapping::default();
        let alias = mapping.identifier("orders", "table");
        assert_eq!(
            anonymize_known("orders orders_backup myorders and new_secret", &mapping),
            format!("{alias} orders_backup myorders and new_secret"),
        );
    }

    #[test]
    fn known_text_handles_unicode_and_existing_alias() {
        let mut mapping = PrivacyMapping::default();
        let alias = mapping.identifier("таблица", "table");
        let source = format!("Ошибка: таблица; уже скрыто: {alias}. 😀");
        assert_eq!(
            anonymize_known(&source, &mapping),
            format!("Ошибка: {alias}; уже скрыто: {alias}. 😀"),
        );
    }

    #[test]
    fn known_text_skips_one_character_mapping() {
        let mut mapping = PrivacyMapping::default();
        mapping.string_value("x");
        assert_eq!(
            anonymize_known("syntax error at x", &mapping),
            "syntax error at x"
        );
    }

    #[test]
    fn text_round_trip_keeps_whitespace() {
        let mut mapping = PrivacyMapping::default();

        let source = "hello  world\n\tsecret";

        let anonymous = anonymize(source, &mut mapping);

        let restored = deanonymize(&anonymous, &mapping);

        assert_eq!(restored, source,);

        assert_eq!(
            anonymous
                .chars()
                .filter(|value| { value.is_whitespace() },)
                .collect::<String>(),
            source
                .chars()
                .filter(|value| { value.is_whitespace() },)
                .collect::<String>(),
        );
    }
    #[test]
    fn text_reuses_existing_mapping_across_categories() {
        let mut mapping = PrivacyMapping::default();
        let table_alias = mapping.identifier("orders", "table");
        let column_alias = mapping.identifier("customer_id", "column");
        let number_alias = mapping.integer_value("42").to_string();

        let source = "orders\ncustomer_id\n42";
        let anonymous = anonymize(source, &mut mapping);

        assert_eq!(
            anonymous,
            format!("{table_alias}\n{column_alias}\n{number_alias}"),
        );
        assert_eq!(deanonymize(&anonymous, &mapping), source);
        assert!(mapping.strings.is_empty());
    }

    #[test]
    fn text_deanonymize_uses_existing_non_string_alias() {
        let mut mapping = PrivacyMapping::default();
        let alias = mapping.identifier("private_schema", "schema");

        assert_eq!(deanonymize(&alias, &mapping), "private_schema");
    }
}
