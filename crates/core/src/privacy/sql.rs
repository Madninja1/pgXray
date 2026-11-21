use super::mapping::PrivacyMapping;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TokenKind {
    Word,
    QuotedIdentifier,
    String,
    DollarString,
    Integer,
    Float,
    Parameter,
    Comment,
    Other,
}

#[derive(Clone, Debug)]
struct Token<'a> {
    kind: TokenKind,
    raw: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WordContext {
    Sql,
    Plpgsql,
}

pub fn anonymize(
    input: &str,
    mapping: &mut PrivacyMapping,
) -> Result<(String, Vec<String>), String> {
    let parsed = pg_query::parse(input)
        .map_err(|error| format!("PostgreSQL parser отклонил SQL: {error}"))?;

    let classes = identifier_classes(&parsed, input);

    for (original, kind) in &classes {
        mapping.identifier(original, kind);
    }

    let (output, warnings) = anonymize_internal(input, mapping, &classes, WordContext::Sql)?;

    pg_query::parse(&output).map_err(|error| {
        format!("После анонимизации SQL перестал разбираться PostgreSQL parser: {error}")
    })?;

    Ok((output, warnings))
}

pub fn deanonymize(input: &str, mapping: &PrivacyMapping) -> Result<String, String> {
    let tokens = tokenize(input)?;

    let mut output = String::with_capacity(input.len());

    let mut cursor = 0;

    for (token_index, token) in tokens.iter().enumerate() {
        let start = token.raw.as_ptr() as usize - input.as_ptr() as usize;

        let end = start + token.raw.len();

        output.push_str(&input[cursor..start]);

        let replacement = if token.kind == TokenKind::DollarString {
            deanonymize_dollar_string(
                token.raw,
                mapping,
                dollar_string_language(&tokens, token_index).as_deref(),
                is_format_dollar_string(&tokens, token_index),
            )?
        } else if token.kind == TokenKind::String && is_regclass_cast(&tokens, token_index) {
            deanonymize_regclass_string(token.raw, mapping)
        } else {
            deanonymize_token(token, mapping)
        };

        output.push_str(&replacement);

        cursor = end;
    }

    output.push_str(&input[cursor..]);

    Ok(output)
}

pub fn anonymize_fragment(input: &str, mapping: &mut PrivacyMapping) -> Result<String, String> {
    let (output, _) = anonymize_internal(input, mapping, &BTreeMap::new(), WordContext::Sql)?;

    Ok(output)
}

pub fn deanonymize_fragment(input: &str, mapping: &PrivacyMapping) -> Result<String, String> {
    deanonymize(input, mapping)
}

fn anonymize_internal(
    input: &str,
    mapping: &mut PrivacyMapping,
    classes: &BTreeMap<String, String>,
    word_context: WordContext,
) -> Result<(String, Vec<String>), String> {
    let tokens = tokenize(input)?;
    let postgres_keyword_spans = postgres_keyword_spans(input)?;

    let mut output = String::with_capacity(input.len());

    let mut cursor = 0;

    let mut comments_removed = 0_u64;
    let mut warnings = Vec::new();

    for (token_index, token) in tokens.iter().enumerate() {
        let start = token.raw.as_ptr() as usize - input.as_ptr() as usize;

        let end = start + token.raw.len();

        output.push_str(&input[cursor..start]);

        let replacement = match token.kind {
            TokenKind::Word => anonymize_word(
                token.raw,
                mapping,
                classes,
                word_context,
                postgres_keyword_spans.contains(&(start, end)),
                is_sql_builtin_at(&tokens, token_index, word_context),
                is_user_schema_function_call(&tokens, token_index),
            ),

            TokenKind::QuotedIdentifier => anonymize_quoted_identifier(token.raw, mapping, classes),

            TokenKind::String => anonymize_string(token.raw, mapping),

            TokenKind::DollarString => {
                let (replacement, nested_warnings) = anonymize_dollar_string(
                    token.raw,
                    mapping,
                    dollar_string_language(&tokens, token_index).as_deref(),
                )?;

                warnings.extend(nested_warnings);

                replacement
            }

            TokenKind::Integer => {
                if is_type_modifier_number(&tokens, token_index)
                    || is_percentile_direct_argument(&tokens, token_index)
                    || is_recursive_step_one(&tokens, token_index)
                    || mapping.reverse_integer(token.raw).is_some()
                {
                    token.raw.to_owned()
                } else {
                    mapping.integer_value(token.raw).to_string()
                }
            }

            TokenKind::Float => {
                if is_percentile_direct_argument(&tokens, token_index)
                    || mapping.reverse_float(token.raw).is_some()
                {
                    token.raw.to_owned()
                } else {
                    mapping.float_value(token.raw)
                }
            }

            TokenKind::Parameter | TokenKind::Other => token.raw.to_owned(),

            TokenKind::Comment => {
                comments_removed = comments_removed.saturating_add(1);

                preserve_comment_layout(token.raw)
            }
        };

        output.push_str(&replacement);

        cursor = end;
    }

    output.push_str(&input[cursor..]);

    if comments_removed > 0 {
        warnings.push(
            format!(
                "Удалено SQL comments: {comments_removed}. Comments намеренно не восстанавливаются при deanonymize."
            ),
        );
    }

    Ok((output, warnings))
}

fn identifier_classes(parsed: &pg_query::ParseResult, input: &str) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();

    for table in parsed.tables() {
        register_qualified(&mut result, &table, "table");
    }

    for function in parsed.functions() {
        register_qualified(&mut result, &function, "function");
    }

    for cte in &parsed.cte_names {
        register_class(&mut result, normalize_identifier(cte), "cte");
    }

    for alias in parsed.aliases.keys() {
        register_class(&mut result, normalize_identifier(alias), "alias");
    }

    for (relation, column) in &parsed.filter_columns {
        register_class(&mut result, normalize_identifier(column), "column");

        if let Some(relation) = relation {
            register_class(&mut result, normalize_identifier(relation), "alias");
        }
    }

    register_create_targets(input, &mut result);

    result
}

fn register_create_targets(input: &str, target: &mut BTreeMap<String, String>) {
    let Ok(tokens) = tokenize(input) else {
        return;
    };

    let tokens = tokens
        .into_iter()
        .filter(|token| token.kind != TokenKind::Comment)
        .collect::<Vec<_>>();

    let mut index = 0;

    while index < tokens.len() {
        if !token_word_eq(&tokens[index], "create") {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;

        if tokens
            .get(cursor)
            .is_some_and(|token| token_word_eq(token, "or"))
            && tokens
                .get(cursor + 1)
                .is_some_and(|token| token_word_eq(token, "replace"))
        {
            cursor += 2;
        }

        while tokens.get(cursor).is_some_and(|token| {
            token_word_in(token, &["temp", "temporary", "recursive", "materialized"])
        }) {
            cursor += 1;
        }

        let kind = match tokens.get(cursor) {
            Some(token) if token_word_eq(token, "function") => "function",
            Some(token) if token_word_eq(token, "procedure") => "procedure",
            Some(token) if token_word_eq(token, "view") => "view",
            _ => {
                index += 1;
                continue;
            }
        };

        register_qualified_tokens(target, &tokens, cursor + 1, kind);

        index = cursor + 1;
    }
}

fn register_qualified_tokens(
    target: &mut BTreeMap<String, String>,
    tokens: &[Token<'_>],
    start: usize,
    final_kind: &str,
) {
    let mut parts = Vec::new();
    let mut cursor = start;

    while let Some(token) = tokens.get(cursor) {
        let Some(identifier) = token_identifier(token) else {
            break;
        };

        parts.push(identifier);
        cursor += 1;

        if tokens
            .get(cursor)
            .is_some_and(|token| token.kind == TokenKind::Other && token.raw == ".")
        {
            cursor += 1;
            continue;
        }

        break;
    }

    for (index, part) in parts.iter().enumerate() {
        let kind = if index + 1 == parts.len() {
            final_kind
        } else {
            "schema"
        };

        register_class(target, part.clone(), kind);
    }
}

fn token_identifier(token: &Token<'_>) -> Option<String> {
    match token.kind {
        TokenKind::Word => Some(normalize_identifier(token.raw)),
        TokenKind::QuotedIdentifier => {
            Some(normalize_identifier(&decode_quoted_identifier(token.raw)))
        }
        _ => None,
    }
}

fn token_word_eq(token: &Token<'_>, expected: &str) -> bool {
    token.kind == TokenKind::Word && normalize_identifier(token.raw) == expected
}

fn token_word_in(token: &Token<'_>, expected: &[&str]) -> bool {
    expected.iter().any(|value| token_word_eq(token, value))
}

fn register_class(target: &mut BTreeMap<String, String>, original: String, kind: &str) {
    match target.get(&original) {
        Some(current) if kind_priority(current) >= kind_priority(kind) => {}
        _ => {
            target.insert(original, kind.to_owned());
        }
    }
}

fn kind_priority(kind: &str) -> u8 {
    match kind {
        "database" => 100,
        "schema" => 95,
        "view" => 92,
        "table" => 90,
        "index" => 85,
        "procedure" => 82,
        "function" => 80,
        "constraint" => 75,
        "column" => 70,
        "cte" => 65,
        "column_alias" => 60,
        "alias" => 55,
        _ => 10,
    }
}

fn register_qualified(target: &mut BTreeMap<String, String>, value: &str, final_kind: &str) {
    let parts = value.split('.').collect::<Vec<_>>();

    for (index, part) in parts.iter().enumerate() {
        let kind = if index + 1 == parts.len() {
            final_kind
        } else {
            "schema"
        };

        register_class(target, normalize_identifier(part), kind);
    }
}

// Preserve PostgreSQL syntax only when the neighboring tokens identify a built-in
// function, type or PL/pgSQL exception condition. Do not globally whitelist these
// words: a user can also use them as ordinary column or relation identifiers.
fn is_sql_builtin_at(tokens: &[Token<'_>], index: usize, context: WordContext) -> bool {
    let token = &tokens[index];
    if token.kind != TokenKind::Word {
        return false;
    }

    let name = normalize_identifier(token.raw);
    let previous = previous_significant_token(tokens, index).map(|i| &tokens[i]);
    let next = next_significant_token(tokens, index + 1, tokens.len()).map(|i| &tokens[i]);

    let is_qualified = previous.is_some_and(|token| token.raw == ".");
    // A routine's return type is a type reference, not a column identifier.
    // Only known PostgreSQL types are exempt; user-defined return types must
    // still be pseudonymized. Do not preserve every word after RETURNS.
    if matches!(name.as_str(), "json" | "jsonb" | "uuid" | "timestamptz")
        && previous.is_some_and(|token| token_word_eq(token, "returns"))
    {
        return true;
    }
    // A call explicitly qualified with pg_catalog is resolved as a system
    // function. Do not grant this exception to other schemas.
    if is_qualified && next.is_some_and(|token| token.raw == "(") {
        if let Some(dot_index) = previous_significant_token(tokens, index) {
            if let Some(schema_index) = previous_significant_token(tokens, dot_index) {
                if token_word_eq(&tokens[schema_index], "pg_catalog") {
                    return true;
                }
            }
        }
    }
    if !is_qualified
        && next.is_some_and(|token| token.raw == "(")
        && matches!(
            name.as_str(),
            "percentile_cont"
                | "percentile_disc"
                | "jsonb_array_elements"
                | "jsonb_array_elements_text"
                | "jsonb_each"
                | "jsonb_each_text"
                | "jsonb_object_keys"
                | "json_array_elements"
                | "json_array_elements_text"
                | "json_each"
                | "json_each_text"
                | "to_char"
                | "to_date"
                | "to_timestamp"
                | "format"
        )
    {
        return true;
    }

    if matches!(name.as_str(), "uuid" | "jsonb" | "timestamptz") {
        // Cast syntax (e.g. value::uuid).
        let after_cast = previous.is_some_and(|token| token.raw == ":")
            && previous_significant_token(tokens, index.saturating_sub(1))
                .is_some_and(|i| tokens[i].raw == ":");
        // Typed string constants (e.g. TIMESTAMPTZ '...').
        let before_literal = next.is_some_and(|token| token.kind == TokenKind::String);
        // CAST(value AS type) -- only preserve AS inside a CAST expression.
        let inside_cast = previous.is_some_and(|token| token_word_eq(token, "as"))
            && is_in_cast_expression(tokens, index);
        if after_cast || before_literal || inside_cast {
            return true;
        }
    }

    // SQLSTATE condition names in EXCEPTION WHEN, not user-defined identifiers.
    if context == WordContext::Plpgsql
        && previous.is_some_and(|token| token_word_eq(token, "when"))
        && matches!(
            name.as_str(),
            "undefined_table"
                | "unique_violation"
                | "foreign_key_violation"
                | "check_violation"
                | "not_null_violation"
                | "division_by_zero"
                | "invalid_text_representation"
                | "numeric_value_out_of_range"
                | "invalid_parameter_value"
                | "raise_exception"
                | "no_data_found"
                | "too_many_rows"
        )
    {
        return true;
    }

    false
}

fn is_in_cast_expression(tokens: &[Token<'_>], index: usize) -> bool {
    let mut depth = 0usize;
    for i in (0..index).rev() {
        match tokens[i].raw {
            ")" => depth += 1,
            "(" if depth > 0 => depth -= 1,
            "(" => {
                return previous_significant_token(tokens, i)
                    .is_some_and(|j| token_word_eq(&tokens[j], "cast"));
            }
            _ => {}
        }
    }
    false
}

// numeric(precision, scale) contains type modifiers, not sensitive data values.
fn is_type_modifier_number(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(previous) = previous_significant_token(tokens, index) else {
        return false;
    };
    let opening = if tokens[previous].raw == "(" {
        previous
    } else if tokens[previous].raw == "," {
        let Some(opening) = previous_significant_token(tokens, previous) else {
            return false;
        };
        if tokens[opening].kind != TokenKind::Integer {
            return false;
        }
        let Some(opening) = previous_significant_token(tokens, opening) else {
            return false;
        };
        opening
    } else {
        return false;
    };
    if tokens[opening].raw != "(" {
        return false;
    }
    previous_significant_token(tokens, opening)
        .is_some_and(|i| token_word_in(&tokens[i], &["numeric", "decimal"]))
}

// Preserve direct percentile fractions: replacing 0.95 with 900000.xxx
// makes percentile_cont/percentile_disc invalid at execution time.
// Do not preserve arbitrary numeric parameters in other function calls.
fn is_percentile_direct_argument(tokens: &[Token<'_>], index: usize) -> bool {
    if !matches!(tokens[index].kind, TokenKind::Integer | TokenKind::Float) {
        return false;
    }
    let Some(open_index) = previous_significant_token(tokens, index) else {
        return false;
    };
    if tokens[open_index].raw != "(" {
        return false;
    }
    let Some(function_index) = previous_significant_token(tokens, open_index) else {
        return false;
    };
    if !token_word_in(
        &tokens[function_index],
        &["percentile_cont", "percentile_disc"],
    ) {
        return false;
    }
    let Some(close_index) = next_significant_token(tokens, index + 1, tokens.len()) else {
        return false;
    };
    if tokens[close_index].raw != ")" {
        return false;
    }
    if let Some(before_function) = previous_significant_token(tokens, function_index) {
        if tokens[before_function].raw == "." {
            let Some(schema_index) = previous_significant_token(tokens, before_function) else {
                return false;
            };
            return token_word_eq(&tokens[schema_index], "pg_catalog");
        }
    }
    true
}

// Preserve + 1 inside a recursive CTE to keep the common recursion step.
// Other numeric literals (WHERE, LIMIT, OFFSET) remain anonymized.
fn is_recursive_step_one(tokens: &[Token<'_>], index: usize) -> bool {
    if tokens[index].raw != "1"
        || !previous_significant_token(tokens, index).is_some_and(|i| tokens[i].raw == "+")
    {
        return false;
    }
    tokens
        .windows(2)
        .any(|pair| token_word_eq(&pair[0], "with") && token_word_eq(&pair[1], "recursive"))
}

// A globally safe function name is *not* safe when it denotes a routine in
// an application schema. SQL name resolution for schema-qualified calls is
// explicit, so this rule does not require a function-name whitelist.
fn is_user_schema_function_call(tokens: &[Token<'_>], index: usize) -> bool {
    let Some(dot) = previous_significant_token(tokens, index) else {
        return false;
    };
    if tokens[dot].raw != "." {
        return false;
    }
    let Some(schema) = previous_significant_token(tokens, dot) else {
        return false;
    };
    if token_identifier(&tokens[schema]).is_none_or(|name| name == "pg_catalog") {
        return false;
    }
    next_significant_token(tokens, index + 1, tokens.len())
        .is_some_and(|next| tokens[next].raw == "(")
}

fn anonymize_word(
    raw: &str,
    mapping: &mut PrivacyMapping,
    classes: &BTreeMap<String, String>,
    word_context: WordContext,
    is_postgres_keyword: bool,
    is_sql_builtin: bool,
    is_user_function: bool,
) -> String {
    let normalized = normalize_identifier(raw);

    if is_user_function {
        if mapping.reverse_identifier(raw).is_some() {
            return raw.to_owned();
        }
        return mapping.identifier(&normalized, "function");
    }

    if is_postgres_keyword
        || is_sql_builtin
        || is_safe_system_word(&normalized)
        || (word_context == WordContext::Plpgsql && is_safe_plpgsql_word(&normalized))
    {
        return raw.to_owned();
    }

    if mapping.reverse_identifier(raw).is_some() {
        return raw.to_owned();
    }

    let kind = classes
        .get(&normalized)
        .map(String::as_str)
        .unwrap_or("identifier");

    mapping.identifier(&normalized, kind)
}

fn anonymize_quoted_identifier(
    raw: &str,
    mapping: &mut PrivacyMapping,
    classes: &BTreeMap<String, String>,
) -> String {
    let decoded = decode_quoted_identifier(raw);

    if is_safe_schema(&decoded) {
        return raw.to_owned();
    }

    if mapping.reverse_identifier(&decoded).is_some() {
        return raw.to_owned();
    }

    let kind = classes
        .get(&normalize_identifier(&decoded))
        .map(String::as_str)
        .unwrap_or("identifier");

    quote_identifier(&mapping.identifier(&decoded, kind))
}

fn anonymize_string(raw: &str, mapping: &mut PrivacyMapping) -> String {
    let (prefix, literal) = split_string_prefix(raw);

    let inner = decode_single_quoted(literal);

    if mapping.reverse_string(&inner).is_some() {
        return raw.to_owned();
    }

    let replacement = mapping.string_value(&inner);

    format!("{prefix}'{}'", replacement.replace('\'', "''",),)
}

fn anonymize_dollar_string(
    raw: &str,
    mapping: &mut PrivacyMapping,
    routine_language: Option<&str>,
) -> Result<(String, Vec<String>), String> {
    let Some((tag, body)) = split_dollar_string(raw) else {
        return Ok((raw.to_owned(), Vec::new()));
    };

    match routine_language {
        Some("plpgsql") => {
            let (replacement, warnings) =
                anonymize_internal(body, mapping, &BTreeMap::new(), WordContext::Plpgsql)?;

            Ok((format!("{tag}{replacement}{tag}"), warnings))
        }

        Some("sql") => {
            let Ok(parsed) = pg_query::parse(body) else {
                let opaque = mapping.string_value(body);

                return Ok((
                    format!("{tag}{opaque}{tag}"),
                    vec![
                        crate::i18n::text("LANGUAGE sql body не удалось разобрать отдельно от CREATE FUNCTION; body анонимизирован как opaque sensitive value.", "Could not parse the LANGUAGE sql body separately from CREATE FUNCTION; the body was anonymized as an opaque sensitive value.").into(),
                    ],
                ));
            };

            let classes = identifier_classes(&parsed, body);
            let mut candidate_mapping = mapping.clone();

            for (original, kind) in &classes {
                candidate_mapping.identifier(original, kind);
            }

            let (replacement, warnings) =
                anonymize_internal(body, &mut candidate_mapping, &classes, WordContext::Sql)?;

            if pg_query::parse(&replacement).is_err() {
                let opaque = mapping.string_value(body);

                return Ok((
                    format!("{tag}{opaque}{tag}"),
                    vec![
                        crate::i18n::text("LANGUAGE sql body после структурированной анонимизации не разбирается PostgreSQL parser; body анонимизирован как opaque sensitive value.", "The PostgreSQL parser cannot parse the LANGUAGE sql body after structured anonymization; the body was anonymized as an opaque sensitive value.").into(),
                    ],
                ));
            }

            *mapping = candidate_mapping;

            Ok((format!("{tag}{replacement}{tag}"), warnings))
        }

        _ => {
            if mapping.reverse_string(body).is_some() {
                return Ok((raw.to_owned(), Vec::new()));
            }

            let replacement = mapping.string_value(body);

            Ok((format!("{tag}{replacement}{tag}"), Vec::new()))
        }
    }
}

fn deanonymize_dollar_string(
    raw: &str,
    mapping: &PrivacyMapping,
    routine_language: Option<&str>,
    is_format_template: bool,
) -> Result<String, String> {
    let Some((tag, body)) = split_dollar_string(raw) else {
        return Ok(raw.to_owned());
    };

    if matches!(routine_language, Some("plpgsql" | "sql")) || is_format_template {
        let restored = deanonymize(body, mapping)?;

        return Ok(format!("{tag}{restored}{tag}"));
    }

    Ok(match mapping.reverse_string(body) {
        Some(original) => format!("{tag}{original}{tag}"),
        None => raw.to_owned(),
    })
}

fn is_format_dollar_string(tokens: &[Token<'_>], body_index: usize) -> bool {
    if !tokens
        .get(body_index)
        .is_some_and(|token| token.kind == TokenKind::DollarString)
    {
        return false;
    }

    let Some(open_paren) = previous_significant_token(tokens, body_index) else {
        return false;
    };

    if tokens[open_paren].kind != TokenKind::Other || tokens[open_paren].raw != "(" {
        return false;
    }

    let Some(function_index) = previous_significant_token(tokens, open_paren) else {
        return false;
    };

    token_word_eq(&tokens[function_index], "format")
}

fn dollar_string_language(tokens: &[Token<'_>], body_index: usize) -> Option<String> {
    if tokens.get(body_index)?.kind != TokenKind::DollarString {
        return None;
    }

    let (statement_start, statement_end) = statement_bounds(tokens, body_index);

    if let Some(language) = do_block_language(tokens, statement_start, statement_end) {
        return Some(language);
    }

    let previous = previous_significant_token(tokens, body_index)?;

    if !token_word_eq(&tokens[previous], "as") {
        return None;
    }

    if !is_routine_definition(&tokens[statement_start..statement_end]) {
        return None;
    }

    let mut index = statement_start;

    while index < statement_end {
        if token_word_eq(&tokens[index], "language") {
            let language_index = next_significant_token(tokens, index + 1, statement_end)?;

            return token_identifier(&tokens[language_index]);
        }

        index += 1;
    }

    None
}

fn do_block_language(
    tokens: &[Token<'_>],
    statement_start: usize,
    statement_end: usize,
) -> Option<String> {
    let first = next_significant_token(tokens, statement_start, statement_end)?;

    if !token_word_eq(&tokens[first], "do") {
        return None;
    }

    let mut index = first + 1;

    while index < statement_end {
        if token_word_eq(&tokens[index], "language") {
            let language_index = next_significant_token(tokens, index + 1, statement_end)?;

            return token_identifier(&tokens[language_index]);
        }

        index += 1;
    }

    // PostgreSQL uses plpgsql for DO blocks when LANGUAGE is omitted.
    Some("plpgsql".to_owned())
}

fn previous_significant_token(tokens: &[Token<'_>], before: usize) -> Option<usize> {
    (0..before)
        .rev()
        .find(|index| tokens[*index].kind != TokenKind::Comment)
}

fn next_significant_token(tokens: &[Token<'_>], start: usize, end: usize) -> Option<usize> {
    (start..end).find(|index| tokens[*index].kind != TokenKind::Comment)
}

fn statement_bounds(tokens: &[Token<'_>], index: usize) -> (usize, usize) {
    let start = (0..index)
        .rev()
        .find(|cursor| tokens[*cursor].kind == TokenKind::Other && tokens[*cursor].raw == ";")
        .map(|cursor| cursor + 1)
        .unwrap_or(0);

    let end = (index + 1..tokens.len())
        .find(|cursor| tokens[*cursor].kind == TokenKind::Other && tokens[*cursor].raw == ";")
        .unwrap_or(tokens.len());

    (start, end)
}

fn is_routine_definition(tokens: &[Token<'_>]) -> bool {
    let mut saw_create = false;

    for token in tokens {
        if token.kind == TokenKind::Comment {
            continue;
        }

        if !saw_create {
            saw_create = token_word_eq(token, "create");
            continue;
        }

        if token_word_in(token, &["function", "procedure"]) {
            return true;
        }
    }

    false
}

fn is_regclass_cast(tokens: &[Token<'_>], string_index: usize) -> bool {
    let end = tokens.len();

    let Some(first_colon) = next_significant_token(tokens, string_index + 1, end) else {
        return false;
    };

    if tokens[first_colon].kind != TokenKind::Other || tokens[first_colon].raw != ":" {
        return false;
    }

    let Some(second_colon) = next_significant_token(tokens, first_colon + 1, end) else {
        return false;
    };

    if tokens[second_colon].kind != TokenKind::Other || tokens[second_colon].raw != ":" {
        return false;
    }

    let Some(type_index) = next_significant_token(tokens, second_colon + 1, end) else {
        return false;
    };

    token_word_eq(&tokens[type_index], "regclass")
}

fn deanonymize_regclass_string(raw: &str, mapping: &PrivacyMapping) -> String {
    let (prefix, literal) = split_string_prefix(raw);
    let inner = decode_single_quoted(literal);

    if let Some(original) = mapping
        .reverse_string(&inner)
        .or_else(|| mapping.reverse_identifier(&inner))
    {
        return format!("{prefix}'{}'", original.replace('\'', "''"));
    }

    let Some((schema_alias, relation_alias)) = inner.split_once('.') else {
        return raw.to_owned();
    };

    if relation_alias.contains('.') {
        return raw.to_owned();
    }

    let schema = mapping
        .reverse_identifier(schema_alias)
        .unwrap_or(schema_alias);
    let relation = mapping
        .reverse_identifier(relation_alias)
        .unwrap_or(relation_alias);

    if schema == schema_alias && relation == relation_alias {
        return raw.to_owned();
    }

    let restored = format!("{schema}.{relation}");

    format!("{prefix}'{}'", restored.replace('\'', "''"))
}

fn deanonymize_token(token: &Token<'_>, mapping: &PrivacyMapping) -> String {
    match token.kind {
        TokenKind::Word => mapping
            .reverse_identifier(token.raw)
            .unwrap_or(token.raw)
            .to_owned(),

        TokenKind::QuotedIdentifier => {
            let decoded = decode_quoted_identifier(token.raw);

            match mapping.reverse_identifier(&decoded) {
                Some(original) => quote_identifier(original),

                None => token.raw.to_owned(),
            }
        }

        TokenKind::String => {
            let (prefix, literal) = split_string_prefix(token.raw);

            let inner = decode_single_quoted(literal);

            match mapping
                .reverse_string(&inner)
                .or_else(|| mapping.reverse_identifier(&inner))
            {
                Some(original) => {
                    format!("{prefix}'{}'", original.replace('\'', "''",),)
                }

                None => token.raw.to_owned(),
            }
        }

        TokenKind::DollarString => {
            let Some((tag, body)) = split_dollar_string(token.raw) else {
                return token.raw.to_owned();
            };

            match mapping.reverse_string(body) {
                Some(original) => {
                    format!("{tag}{original}{tag}")
                }

                None => token.raw.to_owned(),
            }
        }

        TokenKind::Integer => mapping
            .reverse_integer(token.raw)
            .unwrap_or(token.raw)
            .to_owned(),

        TokenKind::Float => mapping
            .reverse_float(token.raw)
            .unwrap_or(token.raw)
            .to_owned(),

        TokenKind::Parameter | TokenKind::Comment | TokenKind::Other => token.raw.to_owned(),
    }
}

fn postgres_keyword_spans(input: &str) -> Result<BTreeSet<(usize, usize)>, String> {
    let scanned = pg_query::scan(input)
        .map_err(|error| format!("PostgreSQL scanner отклонил SQL: {error}"))?;

    scanned
        .tokens
        .into_iter()
        .filter(|token| token.keyword_kind != pg_query::protobuf::KeywordKind::NoKeyword as i32)
        .map(|token| {
            let start = usize::try_from(token.start).map_err(|_| {
                format!(
                    "PostgreSQL scanner вернул некорректную start position: {}",
                    token.start
                )
            })?;

            let end = usize::try_from(token.end).map_err(|_| {
                format!(
                    "PostgreSQL scanner вернул некорректную end position: {}",
                    token.end
                )
            })?;

            Ok((start, end))
        })
        .collect()
}

fn tokenize(input: &str) -> Result<Vec<Token<'_>>, String> {
    let mut tokens = Vec::new();

    let mut index = 0;

    while index < input.len() {
        let (character, width) = next_char(input, index).ok_or(crate::i18n::text(
            "Некорректная UTF-8 позиция SQL tokenizer",
            "Invalid UTF-8 position in the SQL tokenizer",
        ))?;

        if character.is_whitespace() {
            index += width;
            continue;
        }

        let start = index;

        if input[start..].starts_with("--") {
            index = scan_line_comment(input, start);

            tokens.push(Token {
                kind: TokenKind::Comment,
                raw: &input[start..index],
            });

            continue;
        }

        if input[start..].starts_with("/*") {
            index = scan_block_comment(input, start)?;

            tokens.push(Token {
                kind: TokenKind::Comment,
                raw: &input[start..index],
            });

            continue;
        }

        if character == '"' {
            index = scan_quoted_identifier(input, start)?;

            tokens.push(Token {
                kind: TokenKind::QuotedIdentifier,
                raw: &input[start..index],
            });

            continue;
        }

        if character == '\'' {
            index = scan_single_quoted(input, start)?;

            tokens.push(Token {
                kind: TokenKind::String,
                raw: &input[start..index],
            });

            continue;
        }

        if matches!(character, 'e' | 'E') {
            let next = start + width;

            if input[next..].starts_with('\'') {
                index = scan_single_quoted(input, next)?;

                tokens.push(Token {
                    kind: TokenKind::String,
                    raw: &input[start..index],
                });

                continue;
            }
        }

        if character == '$' {
            if let Some(end) = scan_parameter(input, start) {
                index = end;

                tokens.push(Token {
                    kind: TokenKind::Parameter,
                    raw: &input[start..index],
                });

                continue;
            }

            if let Some(end) = scan_dollar_quoted(input, start) {
                index = end?;

                tokens.push(Token {
                    kind: TokenKind::DollarString,
                    raw: &input[start..index],
                });

                continue;
            }
        }

        if character.is_ascii_digit()
            || (character == '.'
                && next_char(input, start + width).is_some_and(|(value, _)| value.is_ascii_digit()))
        {
            let (end, is_float) = scan_number(input, start);

            index = end;

            tokens.push(Token {
                kind: if is_float {
                    TokenKind::Float
                } else {
                    TokenKind::Integer
                },
                raw: &input[start..index],
            });

            continue;
        }

        if is_identifier_start(character) {
            index = scan_word(input, start);

            tokens.push(Token {
                kind: TokenKind::Word,
                raw: &input[start..index],
            });

            continue;
        }

        index += width;

        tokens.push(Token {
            kind: TokenKind::Other,
            raw: &input[start..index],
        });
    }

    Ok(tokens)
}

fn scan_line_comment(input: &str, start: usize) -> usize {
    input[start..]
        .find('\n')
        .map(|offset| start + offset)
        .unwrap_or(input.len())
}

fn scan_block_comment(input: &str, start: usize) -> Result<usize, String> {
    let mut index = start + 2;

    let mut depth = 1_u32;

    while index < input.len() {
        if input[index..].starts_with("/*") {
            depth = depth.saturating_add(1);

            index += 2;
            continue;
        }

        if input[index..].starts_with("*/") {
            depth = depth.saturating_sub(1);

            index += 2;

            if depth == 0 {
                return Ok(index);
            }

            continue;
        }

        let Some((_, width)) = next_char(input, index) else {
            break;
        };

        index += width;
    }

    Err("Незакрытый block comment в SQL".into())
}

fn scan_quoted_identifier(input: &str, start: usize) -> Result<usize, String> {
    let mut index = start + 1;

    while index < input.len() {
        if input[index..].starts_with("\"\"") {
            index += 2;
            continue;
        }

        if input[index..].starts_with('"') {
            return Ok(index + 1);
        }

        let Some((_, width)) = next_char(input, index) else {
            break;
        };

        index += width;
    }

    Err("Незакрытый quoted identifier".into())
}

fn scan_single_quoted(input: &str, quote_start: usize) -> Result<usize, String> {
    let mut index = quote_start + 1;

    while index < input.len() {
        if input[index..].starts_with("''") {
            index += 2;
            continue;
        }

        if input[index..].starts_with('\'') {
            return Ok(index + 1);
        }

        let Some((_, width)) = next_char(input, index) else {
            break;
        };

        index += width;
    }

    Err("Незакрытый string literal".into())
}

fn scan_parameter(input: &str, start: usize) -> Option<usize> {
    let mut index = start + 1;

    let mut found = false;

    while let Some((character, width)) = next_char(input, index) {
        if !character.is_ascii_digit() {
            break;
        }

        found = true;

        index += width;
    }

    found.then_some(index)
}

fn scan_dollar_quoted(input: &str, start: usize) -> Option<Result<usize, String>> {
    let rest = &input[start + 1..];

    let mut tag_end = None;

    for (offset, character) in rest.char_indices() {
        if character == '$' {
            tag_end = Some(start + 1 + offset + 1);

            break;
        }

        if !(character.is_ascii_alphanumeric() || character == '_') {
            return None;
        }
    }

    let tag_end = tag_end?;

    let tag = &input[start..tag_end];

    let body_start = tag_end;

    match input[body_start..].find(tag) {
        Some(offset) => Some(Ok(body_start + offset + tag.len())),

        None => Some(Err("Незакрытый dollar-quoted string".into())),
    }
}

fn scan_number(input: &str, start: usize) -> (usize, bool) {
    let mut index = start;

    let mut is_float = false;

    while let Some((character, width)) = next_char(input, index) {
        if character.is_ascii_digit() {
            index += width;

            continue;
        }

        break;
    }

    if next_char(input, index).is_some_and(|(character, _)| character == '.') {
        is_float = true;

        index += 1;

        while let Some((character, width)) = next_char(input, index) {
            if !character.is_ascii_digit() {
                break;
            }

            index += width;
        }
    }

    if next_char(input, index).is_some_and(|(character, _)| matches!(character, 'e' | 'E')) {
        is_float = true;

        index += 1;

        if next_char(input, index).is_some_and(|(character, _)| matches!(character, '+' | '-')) {
            index += 1;
        }

        while let Some((character, width)) = next_char(input, index) {
            if !character.is_ascii_digit() {
                break;
            }

            index += width;
        }
    }

    (index, is_float)
}

fn scan_word(input: &str, start: usize) -> usize {
    let mut index = start;

    while let Some((character, width)) = next_char(input, index) {
        if !is_identifier_continue(character) {
            break;
        }

        index += width;
    }

    index
}

fn next_char(input: &str, index: usize) -> Option<(char, usize)> {
    if index >= input.len() {
        return None;
    }

    let character = input[index..].chars().next()?;

    Some((character, character.len_utf8()))
}

fn is_identifier_start(value: char) -> bool {
    value == '_' || value.is_alphabetic() || !value.is_ascii()
}

fn is_identifier_continue(value: char) -> bool {
    is_identifier_start(value) || value.is_ascii_digit() || value == '$'
}

fn normalize_identifier(value: &str) -> String {
    value.trim_matches('"').to_ascii_lowercase()
}

fn decode_quoted_identifier(raw: &str) -> String {
    raw.trim_matches('"').replace("\"\"", "\"")
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\"",),)
}

fn split_string_prefix(raw: &str) -> (&str, &str) {
    if raw.len() >= 2 && matches!(raw.as_bytes()[0], b'e' | b'E') && raw.as_bytes()[1] == b'\'' {
        (&raw[..1], &raw[1..])
    } else {
        ("", raw)
    }
}

fn decode_single_quoted(literal: &str) -> String {
    literal
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .unwrap_or(literal)
        .replace("''", "'")
}

fn split_dollar_string(raw: &str) -> Option<(&str, &str)> {
    if !raw.starts_with('$') {
        return None;
    }

    let tag_end = raw[1..].find('$')? + 2;

    let tag = &raw[..tag_end];

    let body = raw.get(tag_end..raw.len().checked_sub(tag.len())?)?;

    Some((tag, body))
}

fn preserve_comment_layout(raw: &str) -> String {
    raw.chars()
        .map(|character| {
            if character == '\n' || character == '\r' {
                character
            } else {
                ' '
            }
        })
        .collect()
}

fn is_safe_schema(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "pg_catalog" | "information_schema" | "public"
    )
}

fn is_safe_system_word(value: &str) -> bool {
    if is_safe_schema(value) {
        return true;
    }

    matches!(
        value,
        "sql"
            | "plpgsql"
            | "date"
            | "now"
            | "date_trunc"
            | "coalesce"
            | "greatest"
            | "least"
            | "count"
            | "sum"
            | "avg"
            | "min"
            | "max"
            | "lower"
            | "upper"
            | "substring"
            | "trim"
            | "btrim"
            | "ltrim"
            | "rtrim"
            | "extract"
            | "round"
            | "floor"
            | "ceil"
            | "ceiling"
            | "abs"
            | "mod"
            | "array_agg"
            | "json_agg"
            | "jsonb_agg"
            | "json_build_object"
            | "jsonb_build_object"
            | "row_number"
            | "rank"
            | "dense_rank"
            | "lag"
            | "lead"
    )
}

fn is_safe_plpgsql_word(value: &str) -> bool {
    matches!(
        value,
        "alias"
            | "assert"
            | "backward"
            | "close"
            | "column_name"
            | "constant"
            | "constraint_name"
            | "continue"
            | "cursor"
            | "debug"
            | "declare"
            | "detail"
            | "diagnostics"
            | "elsif"
            | "errcode"
            | "exception"
            | "execute"
            | "exit"
            | "foreach"
            | "forward"
            | "found"
            | "get"
            | "hint"
            | "info"
            | "last"
            | "log"
            | "loop"
            | "message"
            | "message_text"
            | "move"
            | "new"
            | "next"
            | "no"
            | "notice"
            | "old"
            | "open"
            | "others"
            | "perform"
            | "pg_context"
            | "pg_datatype_name"
            | "pg_exception_context"
            | "pg_exception_detail"
            | "pg_exception_hint"
            | "pg_routine_oid"
            | "print_strict_params"
            | "variable_conflict"
            | "use_variable"
            | "use_column"
            | "error"
            | "prior"
            | "query"
            | "raise"
            | "record"
            | "refcursor"
            | "relative"
            | "return"
            | "returned_sqlstate"
            | "reverse"
            | "row_count"
            | "rowtype"
            | "schema_name"
            | "scroll"
            | "slice"
            | "sqlerrm"
            | "sqlstate"
            | "stacked"
            | "table_name"
            | "type"
            | "warning"
            | "while"
            | "tg_argv"
            | "tg_event"
            | "tg_level"
            | "tg_name"
            | "tg_nargs"
            | "tg_op"
            | "tg_relid"
            | "tg_table_name"
            | "tg_table_schema"
            | "tg_tag"
            | "tg_when"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_typed_timestamptz_literals() {
        let source =
            "SELECT * FROM app.orders WHERE created_at >= TIMESTAMPTZ '2026-01-01 00:00:00+03';";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains("TIMESTAMPTZ 'value_"));
        assert!(!anonymous.contains("2026-01-01"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn keeps_builtin_aggregate_and_json_table_functions() {
        let source = "SELECT percentile_cont(0.95) WITHIN GROUP (ORDER BY duration_ms) FROM app.events; SELECT x.value FROM app.events e CROSS JOIN LATERAL jsonb_array_elements(e.payload) AS x(value);";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains("percentile_cont("));
        assert!(anonymous.contains("jsonb_array_elements("));
        assert!(!anonymous.contains("duration_ms"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn keeps_cast_types_and_numeric_modifiers() {
        let source = "SELECT id::uuid, payload::jsonb, (price)::numeric(12,2), CAST(created_at AS date) FROM app.orders;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains("::uuid"));
        assert!(anonymous.contains("::jsonb"));
        assert!(anonymous.contains("::numeric(12,2)"));
        assert!(anonymous.contains(" AS date)"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn retains_recursive_additive_step_but_anonymizes_filter_value() {
        let source = "WITH RECURSIVE t(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM t WHERE n < 8) SELECT n FROM t;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains(" + 1 FROM "));
        assert!(!anonymous.contains(" < 8"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn retains_plpgsql_exception_conditions() {
        let source = "CREATE PROCEDURE app.handle_error() LANGUAGE plpgsql AS $body$ BEGIN BEGIN RAISE NOTICE 'secret'; EXCEPTION WHEN undefined_table THEN RAISE WARNING 'hidden'; END; END; $body$;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains("WHEN undefined_table THEN"));
        assert!(!anonymous.contains("'secret'"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn does_not_expose_builtin_names_when_used_as_columns() {
        let source = "SELECT uuid, jsonb, percentile_cont FROM private_table;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(!anonymous.contains("SELECT uuid, jsonb, percentile_cont"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn preserves_ordered_set_percentile_fraction_but_not_other_floats() {
        let source = "SELECT percentile_cont(0.95) WITHIN GROUP (ORDER BY duration_ms), percentile_disc(0.5) WITHIN GROUP (ORDER BY duration_ms) FROM app.events WHERE score > 99.95;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains("percentile_cont(0.95)"));
        assert!(anonymous.contains("percentile_disc(0.5)"));
        assert!(!anonymous.contains("99.95"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn preserves_pg_catalog_qualified_builtin_without_broad_whitelist() {
        let source = "SELECT pg_catalog.jsonb_typeof(payload) FROM app.events;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains("pg_catalog.jsonb_typeof("));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn still_anonymizes_user_schema_qualified_function() {
        let source = "SELECT private_api.jsonb_typeof(payload) FROM app.events;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(!anonymous.contains("private_api.jsonb_typeof("));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn user_schema_qualified_builtins_are_pseudonymized() {
        let source = "SELECT private_api.lower(email), private_api.upper(email), pg_catalog.lower(email) FROM app.users;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(!anonymous.contains("private_api"));
        assert!(anonymous.contains("pg_catalog.lower("));
        assert!(!anonymous.contains("schema_001.lower("));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn keeps_jsonb_return_type_in_plpgsql_function() {
        let source = "CREATE FUNCTION private_api.report() RETURNS jsonb LANGUAGE plpgsql AS $$ BEGIN RETURN '{}'::jsonb; END; $$;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(anonymous.contains("RETURNS jsonb"));
        assert!(!anonymous.contains("private_api"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn user_defined_return_type_is_not_exempted() {
        let source = "CREATE FUNCTION private_api.report() RETURNS private_api.secret_report LANGUAGE plpgsql AS $$ BEGIN RETURN NULL; END; $$;";
        let mut mapping = PrivacyMapping::default();
        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();
        assert!(!anonymous.contains("secret_report"));
        assert_eq!(deanonymize(&anonymous, &mapping).unwrap(), source);
    }

    #[test]
    fn tokenizer_keeps_parameter() {
        let tokens = tokenize("select * from t where id = $1").unwrap();

        assert!(tokens
            .iter()
            .any(|token| { token.kind == TokenKind::Parameter && token.raw == "$1" }),);
    }

    #[test]
    fn fragment_round_trip() {
        let mut mapping = PrivacyMapping::default();

        let source = "orders.customer_id = 'secret'";

        let anonymous = anonymize_fragment(source, &mut mapping).unwrap();

        let restored = deanonymize_fragment(&anonymous, &mapping).unwrap();

        assert!(restored.contains("orders",),);

        assert!(restored.contains("customer_id",),);

        assert!(restored.contains("secret",),);
    }

    #[test]
    fn deanonymizes_identifier_aliases_inside_string_literals() {
        let mut mapping = PrivacyMapping::default();

        let table_alias = mapping.identifier("orders", "table");
        let column_alias = mapping.identifier("customer_id", "column");

        let source = format!(
            "SELECT * FROM pg_stats WHERE tablename = '{table_alias}' AND attname IN ('{column_alias}')"
        );

        let restored = deanonymize(&source, &mapping).unwrap();

        assert_eq!(
            restored,
            "SELECT * FROM pg_stats WHERE tablename = 'orders' AND attname IN ('customer_id')"
        );
    }

    #[test]
    fn deanonymizes_qualified_regclass_inside_string_literal() {
        let mut mapping = PrivacyMapping::default();

        let schema_alias = mapping.identifier("private_schema", "schema");
        let table_alias = mapping.identifier("orders", "table");

        let source = format!("SELECT '{schema_alias}.{table_alias}'::regclass;");

        let restored = deanonymize(&source, &mapping).unwrap();

        assert_eq!(restored, "SELECT 'private_schema.orders'::regclass;");
    }

    #[test]
    fn keeps_qualified_identifier_string_opaque_without_regclass_cast() {
        let mut mapping = PrivacyMapping::default();

        let schema_alias = mapping.identifier("private_schema", "schema");
        let table_alias = mapping.identifier("orders", "table");

        let source = format!("SELECT '{schema_alias}.{table_alias}';");

        let restored = deanonymize(&source, &mapping).unwrap();

        assert_eq!(restored, source);
    }

    #[test]
    fn anonymizes_create_or_replace_procedure() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"CREATE OR REPLACE PROCEDURE app.refresh_orders(IN p_id bigint)
LANGUAGE plpgsql
AS $body$
BEGIN
    NULL;
END;
$body$;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("CREATE OR REPLACE PROCEDURE"));
        assert!(anonymous.contains("procedure_001"));
        assert!(!anonymous.contains("refresh_orders"));
        pg_query::parse(&anonymous).unwrap();

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn anonymizes_create_or_replace_function() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"CREATE OR REPLACE FUNCTION app.calculate_total(
    IN p_id bigint,
    OUT p_total bigint
)
RETURNS bigint
LANGUAGE sql
AS $body$
SELECT p_id
$body$;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("CREATE OR REPLACE FUNCTION"));
        assert!(anonymous.contains("function_001"));
        assert!(!anonymous.contains("calculate_total"));
        pg_query::parse(&anonymous).unwrap();

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn preserves_date_type_cast() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"SELECT created_at
FROM app.orders
WHERE created_at >= NOW()::date;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("NOW()::date"));
        assert!(!anonymous.contains("created_at"));
        pg_query::parse(&anonymous).unwrap();

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn anonymizes_create_or_replace_view() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"CREATE OR REPLACE VIEW reporting.active_orders AS
SELECT id
FROM app.orders
WHERE deleted_at IS NULL;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("CREATE OR REPLACE VIEW"));
        assert!(anonymous.contains("view_001"));
        assert!(!anonymous.contains("active_orders"));
        pg_query::parse(&anonymous).unwrap();

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn preserves_postgresql_keywords_inside_view_definition() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"CREATE OR REPLACE VIEW reporting.order_activity AS
SELECT
    created_at AT TIME ZONE 'UTC' AS local_created_at,
    created_at::timestamp WITHOUT TIME ZONE AS naive_created_at,
    sum(amount) OVER (
        ORDER BY created_at
        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
    ) AS running_total
FROM app.orders;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains(" AT TIME ZONE "));
        assert!(anonymous.contains(" WITHOUT TIME ZONE "));
        assert!(anonymous.contains(" UNBOUNDED PRECEDING "));
        assert!(!anonymous.contains("order_activity"));
        assert!(!anonymous.contains("created_at"));
        assert!(!anonymous.contains("'UTC'"));
        pg_query::parse(&anonymous).unwrap();

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn anonymizes_plpgsql_procedure_body_with_procedure_tag() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"CREATE OR REPLACE PROCEDURE demo.process_sample_items()
LANGUAGE plpgsql
AS $procedure$
DECLARE
    v_batch_size integer;
    v_group_id integer;
    m_cursor CURSOR FOR
        SELECT batch_size
        FROM demo.sample_items
        WHERE group_id = v_group_id;
BEGIN
    v_batch_size := 42;
    RAISE NOTICE 'batch size %', v_batch_size;
END;
$procedure$;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("LANGUAGE plpgsql"));
        assert!(anonymous.contains("$procedure$\nDECLARE"));
        assert!(anonymous.contains("CURSOR FOR"));
        assert!(anonymous.contains("BEGIN"));
        assert!(anonymous.contains("RAISE NOTICE"));
        assert!(!anonymous.contains("v_batch_size"));
        assert!(!anonymous.contains("v_group_id"));
        assert!(!anonymous.contains("sample_items"));
        assert!(!anonymous.contains("'batch size %'"));

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn anonymizes_plpgsql_function_body_with_function_tag_and_trailing_language() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"CREATE OR REPLACE FUNCTION app.calculate_total(p_id bigint)
RETURNS bigint
AS $function$
DECLARE
    p_total bigint;
BEGIN
    SELECT amount INTO p_total
    FROM orders
    WHERE id = p_id;

    RETURN p_total;
END;
$function$
LANGUAGE plpgsql;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("$function$\nDECLARE"));
        assert!(anonymous.contains("LANGUAGE plpgsql"));
        assert!(anonymous.contains("RETURN"));
        assert!(!anonymous.contains("p_total"));
        assert!(!anonymous.contains("orders"));

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn anonymizes_sql_function_body_without_collapsing_dollar_string() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"CREATE OR REPLACE FUNCTION app.order_name(p_id bigint)
RETURNS text
LANGUAGE sql
AS $function$
SELECT name
FROM orders
WHERE id = p_id
$function$;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("LANGUAGE sql"));
        assert!(anonymous.contains("$function$\nSELECT"));
        assert!(anonymous.contains("FROM"));
        assert!(!anonymous.contains("orders"));

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn round_trips_default_plpgsql_do_block() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"DO $$
DECLARE
    p_limit integer;
    p_name text;
BEGIN
    p_limit := 10;
    p_name := 'private';

    RAISE NOTICE 'name=% limit=%', p_name, p_limit;
END;
$$;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("DO $$\nDECLARE"));
        assert!(!anonymous.contains("p_limit"));
        assert!(!anonymous.contains("p_name"));
        assert!(!anonymous.contains("'private'"));

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn round_trips_do_block_with_explicit_plpgsql_language() {
        let mut mapping = PrivacyMapping::default();

        let source = r#"DO $body$
BEGIN
    RAISE NOTICE 'private';
END;
$body$ LANGUAGE plpgsql;"#;

        let (anonymous, _) = anonymize(source, &mut mapping).unwrap();

        assert!(anonymous.contains("DO $body$\nBEGIN"));
        assert!(anonymous.contains("$body$ LANGUAGE plpgsql;"));
        assert!(!anonymous.contains("'private'"));

        let restored = deanonymize(&anonymous, &mapping).unwrap();
        assert_eq!(restored, source);
    }

    #[test]
    fn deanonymizes_identifiers_inside_format_dollar_string() {
        let mut mapping = PrivacyMapping::default();

        let table_alias = mapping.identifier("orders", "table");
        let source_column_alias = mapping.identifier("customer_id", "column");
        let target_column_alias = mapping.identifier("id", "column");

        let source = format!(
            r#"SELECT format(
    $q$
SELECT %L AS table_name
FROM public.%I t
WHERE t.{source_column_alias} IS NOT NULL
  AND EXISTS (
      SELECT 1
      FROM public.{table_alias} p
      WHERE p.{target_column_alias} = t.{source_column_alias}
  )
    $q$,
    c.relname,
    c.relname
);"#,
        );

        let restored = deanonymize(&source, &mapping).unwrap();

        assert!(restored.contains("t.customer_id"));
        assert!(restored.contains("FROM public.orders p"));
        assert!(restored.contains("p.id = t.customer_id"));
        assert!(restored.contains("%L AS table_name"));
        assert!(restored.contains("FROM public.%I t"));
    }

    #[test]
    fn keeps_plain_dollar_string_opaque() {
        let mut mapping = PrivacyMapping::default();
        let table_alias = mapping.identifier("orders", "table");
        let source = format!("SELECT $q${table_alias}$q$;");

        let restored = deanonymize(&source, &mapping).unwrap();

        assert_eq!(restored, source);
    }
}
