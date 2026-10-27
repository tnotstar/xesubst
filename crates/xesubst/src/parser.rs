//! Parser for template streams, parameter expansions, and words.

use crate::ModeConfig;
use crate::ast::*;
use crate::error::XesubstError;
use crate::lexer::{SourceCursor, is_ident_continue, is_ident_start};
use crate::span::{Span, Spanned};

/// A top-level element in a parsed template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateElement {
    /// Literal text chunk.
    Literal(String),
    /// A simple variable reference: `$VAR`.
    SimpleVar(Spanned<String>),
    /// A braced parameter expansion: `${...}`.
    Braced(Spanned<ParamExp>),
    /// An arithmetic expansion: `$((...))`.
    Arith(Spanned<String>),
}

/// Parses an entire template string into a list of template elements.
pub fn parse_template(
    input: &str,
    config: &ModeConfig,
) -> Result<Vec<TemplateElement>, XesubstError> {
    let mut cursor = SourceCursor::new(input);
    let mut elements = Vec::new();
    let mut literal_buf = String::new();

    while !cursor.is_eof() {
        if cursor.peek() == Some('$') {
            let dollar_span = cursor.current_span(1);

            // Check for $(( ... )) arithmetic expansion
            if cursor.starts_with("$((") {
                if config.arith {
                    if !literal_buf.is_empty() {
                        elements.push(TemplateElement::Literal(std::mem::take(&mut literal_buf)));
                    }
                    let arith_elem = parse_arith_expansion(&mut cursor, config)?;
                    elements.push(arith_elem);
                    continue;
                } else if config.strict {
                    return Err(XesubstError::StrictViolation {
                        span: cursor.current_span(3),
                        message: "arithmetic expansion $((...)) requires --arith".to_string(),
                    });
                }
            }

            // Check for $( ... ) command substitution
            if cursor.starts_with("$(") && !cursor.starts_with("$((") {
                if config.strict {
                    return Err(XesubstError::StrictViolation {
                        span: cursor.current_span(2),
                        message: "command substitution $(...) is not allowed".to_string(),
                    });
                }
                // Leave literal $(
                literal_buf.push(cursor.next().unwrap());
                continue;
            }

            // Check for positional / special parameters like $1, $$, $?, $@
            if let Some(next_c) = cursor.peek_ahead(1)
                && (next_c.is_ascii_digit()
                    || next_c == '$'
                    || next_c == '?'
                    || next_c == '@'
                    || next_c == '*'
                    || next_c == '#'
                    || next_c == '!'
                    || next_c == '-')
            {
                if config.strict {
                    return Err(XesubstError::StrictViolation {
                        span: cursor.current_span(2),
                        message: format!("special/positional parameter '${next_c}' is not allowed"),
                    });
                }
                literal_buf.push(cursor.next().unwrap());
                literal_buf.push(cursor.next().unwrap());
                continue;
            }

            // Check for ${...}
            if cursor.starts_with("${") {
                if config.compat {
                    // In compat mode, only ${IDENT} is parsed as a variable. Anything else is literal.
                    let start_offset = cursor.current_offset();
                    let start_span = cursor.current_span(2);
                    cursor.next(); // consume $
                    cursor.next(); // consume {

                    let var_start = cursor.current_offset();
                    let mut is_valid = false;
                    if let Some(c) = cursor.peek()
                        && is_ident_start(c)
                    {
                        cursor.next();
                        while let Some(c2) = cursor.peek() {
                            if is_ident_continue(c2) {
                                cursor.next();
                            } else {
                                break;
                            }
                        }
                        if cursor.peek() == Some('}') {
                            cursor.next();
                            is_valid = true;
                        }
                    }

                    if is_valid {
                        if !literal_buf.is_empty() {
                            elements
                                .push(TemplateElement::Literal(std::mem::take(&mut literal_buf)));
                        }
                        let var_name = &cursor.input[var_start..cursor.current_offset() - 1];
                        elements.push(TemplateElement::Braced(Spanned::new(
                            ParamExp {
                                name: var_name.to_string(),
                                indirect: false,
                                op: ParamOp::Value,
                            },
                            start_span,
                        )));
                    } else {
                        // Rewind and treat as literal
                        // Reset cursor to start_offset
                        let chars_consumed = cursor.input[start_offset..cursor.current_offset()]
                            .chars()
                            .count();
                        cursor.char_idx -= chars_consumed;
                        // Just consume the '$'
                        literal_buf.push(cursor.next().unwrap());
                    }
                    continue;
                }

                // Standard Bash expansion mode
                if !literal_buf.is_empty() {
                    elements.push(TemplateElement::Literal(std::mem::take(&mut literal_buf)));
                }
                let braced = parse_braced_param(&mut cursor, config)?;
                elements.push(TemplateElement::Braced(braced));
                continue;
            }

            // Check for $IDENT
            if let Some(next_c) = cursor.peek_ahead(1)
                && is_ident_start(next_c)
            {
                if !literal_buf.is_empty() {
                    elements.push(TemplateElement::Literal(std::mem::take(&mut literal_buf)));
                }
                cursor.next(); // consume $
                let name_start = cursor.current_offset();
                while let Some(c) = cursor.peek() {
                    if is_ident_continue(c) {
                        cursor.next();
                    } else {
                        break;
                    }
                }
                let name = &cursor.input[name_start..cursor.current_offset()];
                elements.push(TemplateElement::SimpleVar(Spanned::new(
                    name.to_string(),
                    dollar_span,
                )));
                continue;
            }

            // Bare $ or unhandled character after $
            literal_buf.push(cursor.next().unwrap());
        } else {
            literal_buf.push(cursor.next().unwrap());
        }
    }

    if !literal_buf.is_empty() {
        elements.push(TemplateElement::Literal(literal_buf));
    }

    Ok(elements)
}

/// Parses arithmetic expansion `$(( ... ))`.
fn parse_arith_expansion(
    cursor: &mut SourceCursor<'_>,
    _config: &ModeConfig,
) -> Result<TemplateElement, XesubstError> {
    let start_span = cursor.current_span(3);
    cursor.advance_str("$((");

    let mut depth = 1;
    let mut expr = String::new();

    while !cursor.is_eof() {
        if cursor.starts_with("((") {
            depth += 1;
            expr.push_str("((");
            cursor.advance_str("((");
        } else if cursor.starts_with("))") {
            depth -= 1;
            if depth == 0 {
                cursor.advance_str("))");
                return Ok(TemplateElement::Arith(Spanned::new(expr, start_span)));
            }
            expr.push_str("))");
            cursor.advance_str("))");
        } else {
            expr.push(cursor.next().unwrap());
        }
    }

    Err(XesubstError::Syntax {
        span: start_span,
        message: "unterminated arithmetic expansion '$((...'".to_string(),
    })
}

/// Parses a braced parameter expansion `${...}`.
pub fn parse_braced_param(
    cursor: &mut SourceCursor<'_>,
    config: &ModeConfig,
) -> Result<Spanned<ParamExp>, XesubstError> {
    let start_span = cursor.current_span(2);
    cursor.advance_str("${");

    if cursor.is_eof() {
        return Err(XesubstError::Syntax {
            span: start_span,
            message: "unterminated '${'".to_string(),
        });
    }

    // Check for length operator: ${#VAR}
    if cursor.peek() == Some('#') {
        // Look ahead: must be followed by identifier and closing brace
        let saved_idx = cursor.char_idx;
        cursor.next(); // consume #
        let ident_start = cursor.current_offset();
        if let Some(c) = cursor.peek()
            && is_ident_start(c)
        {
            cursor.next();
            while let Some(c2) = cursor.peek() {
                if is_ident_continue(c2) {
                    cursor.next();
                } else {
                    break;
                }
            }
            let name = &cursor.input[ident_start..cursor.current_offset()];
            if cursor.peek() == Some('}') {
                cursor.next(); // consume }
                return Ok(Spanned::new(
                    ParamExp {
                        name: name.to_string(),
                        indirect: false,
                        op: ParamOp::Length,
                    },
                    start_span,
                ));
            }
        }
        // If not ${#VAR}, restore
        cursor.char_idx = saved_idx;
    }

    // Check for indirect or prefix listing: ${!prefix*} / ${!prefix@} / ${!VAR}
    let mut indirect = false;
    if cursor.peek() == Some('!') {
        cursor.next();
        let ident_start = cursor.current_offset();
        if let Some(c) = cursor.peek() {
            if is_ident_start(c) {
                cursor.next();
                while let Some(c2) = cursor.peek() {
                    if is_ident_continue(c2) {
                        cursor.next();
                    } else {
                        break;
                    }
                }
                let prefix_name = &cursor.input[ident_start..cursor.current_offset()];
                if cursor.peek() == Some('*') && cursor.peek_ahead(1) == Some('}') {
                    cursor.next(); // consume *
                    cursor.next(); // consume }
                    return Ok(Spanned::new(
                        ParamExp {
                            name: prefix_name.to_string(),
                            indirect: false,
                            op: ParamOp::PrefixNames { star: true },
                        },
                        start_span,
                    ));
                }
                if cursor.peek() == Some('@') && cursor.peek_ahead(1) == Some('}') {
                    cursor.next(); // consume @
                    cursor.next(); // consume }
                    return Ok(Spanned::new(
                        ParamExp {
                            name: prefix_name.to_string(),
                            indirect: false,
                            op: ParamOp::PrefixNames { star: false },
                        },
                        start_span,
                    ));
                }
                // Otherwise this is indirect expansion: ${!VAR...}
                indirect = true;
                // Rewind cursor to before the identifier so main parser reads it as variable name
                let chars_to_rewind = prefix_name.chars().count();
                cursor.char_idx -= chars_to_rewind;
            } else {
                return Err(XesubstError::Syntax {
                    span: cursor.current_span(1),
                    message: "expected variable name after '${!'".to_string(),
                });
            }
        } else {
            return Err(XesubstError::Syntax {
                span: start_span,
                message: "unterminated '${!'".to_string(),
            });
        }
    }

    // Parse variable identifier
    let ident_span = cursor.current_span(1);
    let ident_start = cursor.current_offset();
    if let Some(c) = cursor.peek() {
        if !is_ident_start(c) {
            if config.strict {
                return Err(XesubstError::StrictViolation {
                    span: ident_span,
                    message: format!(
                        "invalid variable name starting with '{c}' in parameter expansion"
                    ),
                });
            } else {
                return Err(XesubstError::Syntax {
                    span: ident_span,
                    message: format!("expected variable name, found '{c}'"),
                });
            }
        }
        cursor.next();
        while let Some(c2) = cursor.peek() {
            if is_ident_continue(c2) {
                cursor.next();
            } else {
                break;
            }
        }
    } else {
        return Err(XesubstError::Syntax {
            span: start_span,
            message: "unterminated '${'".to_string(),
        });
    }

    let name = cursor.input[ident_start..cursor.current_offset()].to_string();

    // Now inspect operator
    if cursor.peek() == Some('}') {
        cursor.next(); // consume }
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Value,
            },
            start_span,
        ));
    }

    // Substring or colon-default
    if cursor.peek() == Some(':') {
        let next_c = cursor.peek_ahead(1);
        if let Some(op_c @ ('-' | '=' | '?' | '+')) = next_c {
            // Colon default: :- := :? :+
            cursor.next(); // consume :
            cursor.next(); // consume op_c
            let kind = match op_c {
                '-' => DefaultKind::UseDefault,
                '=' => DefaultKind::AssignDefault,
                '?' => DefaultKind::ErrorIfUnset,
                '+' => DefaultKind::UseAlternate,
                _ => unreachable!(),
            };
            let word = parse_word(cursor, &['}'], config)?;
            expect_char(cursor, '}', start_span)?;
            return Ok(Spanned::new(
                ParamExp {
                    name,
                    indirect,
                    op: ParamOp::Default {
                        colon: true,
                        kind,
                        word,
                    },
                },
                start_span,
            ));
        } else {
            // Substring: ${v:offset} or ${v:offset:len}
            cursor.next(); // consume :
            let (offset_str, len_str) = parse_substring_parts(cursor, start_span)?;
            return Ok(Spanned::new(
                ParamExp {
                    name,
                    indirect,
                    op: ParamOp::Substring {
                        offset: offset_str,
                        length: len_str,
                    },
                },
                start_span,
            ));
        }
    }

    // Non-colon default: - = ? +
    if let Some(op_c @ ('-' | '=' | '?' | '+')) = cursor.peek() {
        cursor.next();
        let kind = match op_c {
            '-' => DefaultKind::UseDefault,
            '=' => DefaultKind::AssignDefault,
            '?' => DefaultKind::ErrorIfUnset,
            '+' => DefaultKind::UseAlternate,
            _ => unreachable!(),
        };
        let word = parse_word(cursor, &['}'], config)?;
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Default {
                    colon: false,
                    kind,
                    word,
                },
            },
            start_span,
        ));
    }

    // Strip prefix/suffix: ## # %% %
    if cursor.starts_with("##") {
        cursor.advance_str("##");
        let pattern = parse_word(cursor, &['}'], config)?;
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Strip {
                    side: StripSide::Prefix,
                    greedy: true,
                    pattern,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with("#") {
        cursor.advance_str("#");
        let pattern = parse_word(cursor, &['}'], config)?;
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Strip {
                    side: StripSide::Prefix,
                    greedy: false,
                    pattern,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with("%%") {
        cursor.advance_str("%%");
        let pattern = parse_word(cursor, &['}'], config)?;
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Strip {
                    side: StripSide::Suffix,
                    greedy: true,
                    pattern,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with("%") {
        cursor.advance_str("%");
        let pattern = parse_word(cursor, &['}'], config)?;
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Strip {
                    side: StripSide::Suffix,
                    greedy: false,
                    pattern,
                },
            },
            start_span,
        ));
    }

    // Pattern replace: // /# /% /
    if cursor.starts_with("//") {
        cursor.advance_str("//");
        let (pattern, replacement) = parse_replace_words(cursor, config, start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Replace {
                    mode: ReplaceMode::All,
                    pattern,
                    replacement,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with("/#") {
        cursor.advance_str("/#");
        let (pattern, replacement) = parse_replace_words(cursor, config, start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Replace {
                    mode: ReplaceMode::Prefix,
                    pattern,
                    replacement,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with("/%") {
        cursor.advance_str("/%");
        let (pattern, replacement) = parse_replace_words(cursor, config, start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Replace {
                    mode: ReplaceMode::Suffix,
                    pattern,
                    replacement,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with("/") {
        cursor.advance_str("/");
        let (pattern, replacement) = parse_replace_words(cursor, config, start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Replace {
                    mode: ReplaceMode::First,
                    pattern,
                    replacement,
                },
            },
            start_span,
        ));
    }

    // Case modification: ^^ ^ ,, ,
    if cursor.starts_with("^^") {
        cursor.advance_str("^^");
        let pattern = if cursor.peek() == Some('}') {
            None
        } else {
            Some(parse_word(cursor, &['}'], config)?)
        };
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::CaseMod {
                    upper: true,
                    all: true,
                    pattern,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with("^") {
        cursor.advance_str("^");
        let pattern = if cursor.peek() == Some('}') {
            None
        } else {
            Some(parse_word(cursor, &['}'], config)?)
        };
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::CaseMod {
                    upper: true,
                    all: false,
                    pattern,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with(",,") {
        cursor.advance_str(",,");
        let pattern = if cursor.peek() == Some('}') {
            None
        } else {
            Some(parse_word(cursor, &['}'], config)?)
        };
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::CaseMod {
                    upper: false,
                    all: true,
                    pattern,
                },
            },
            start_span,
        ));
    }
    if cursor.starts_with(",") {
        cursor.advance_str(",");
        let pattern = if cursor.peek() == Some('}') {
            None
        } else {
            Some(parse_word(cursor, &['}'], config)?)
        };
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::CaseMod {
                    upper: false,
                    all: false,
                    pattern,
                },
            },
            start_span,
        ));
    }

    // Transformations: @Q @E @A @U @u @L
    if cursor.peek() == Some('@') {
        let op_span = cursor.current_span(2);
        cursor.next(); // consume @
        let transform_c = cursor.next().unwrap_or('\0');
        let op = match transform_c {
            'Q' => TransformOp::Quote,
            'E' => TransformOp::Escape,
            'A' => TransformOp::Assignment,
            'U' => TransformOp::UpperAll,
            'u' => TransformOp::UpperFirst,
            'L' => TransformOp::LowerAll,
            'P' | 'K' | 'a' => {
                return Err(XesubstError::Expansion {
                    span: op_span,
                    message: format!("transformation operator '@{transform_c}' is not supported"),
                });
            }
            c => {
                return Err(XesubstError::Syntax {
                    span: op_span,
                    message: format!("unknown transformation operator '@{c}'"),
                });
            }
        };
        expect_char(cursor, '}', start_span)?;
        return Ok(Spanned::new(
            ParamExp {
                name,
                indirect,
                op: ParamOp::Transform { op },
            },
            start_span,
        ));
    }

    // Unexpected character inside ${...}
    let offending_span = cursor.current_span(1);
    let c = cursor.peek().unwrap_or('\0');
    Err(XesubstError::Syntax {
        span: offending_span,
        message: format!("unexpected character '{c}' in parameter expansion"),
    })
}

fn expect_char(
    cursor: &mut SourceCursor<'_>,
    expected: char,
    start_span: Span,
) -> Result<(), XesubstError> {
    if cursor.peek() == Some(expected) {
        cursor.next();
        Ok(())
    } else {
        Err(XesubstError::Syntax {
            span: start_span,
            message: format!("unterminated '${{' (expected '{expected}')"),
        })
    }
}

/// Parses substring offset and optional length.
fn parse_substring_parts(
    cursor: &mut SourceCursor<'_>,
    start_span: Span,
) -> Result<(String, Option<String>), XesubstError> {
    let mut offset = String::new();
    let length: Option<String>;
    let mut paren_depth: usize = 0;

    // Parse offset up to unquoted/unparenthesized ':' or '}'
    while !cursor.is_eof() {
        let c = cursor.peek().unwrap();
        if c == '(' {
            paren_depth += 1;
            offset.push(cursor.next().unwrap());
        } else if c == ')' {
            paren_depth = paren_depth.saturating_sub(1);
            offset.push(cursor.next().unwrap());
        } else if c == ':' && paren_depth == 0 {
            cursor.next(); // consume :
            break;
        } else if c == '}' && paren_depth == 0 {
            cursor.next(); // consume }
            return Ok((offset, None));
        } else {
            offset.push(cursor.next().unwrap());
        }
    }

    // Parse length up to unparenthesized '}'
    let mut len_str = String::new();
    while !cursor.is_eof() {
        let c = cursor.peek().unwrap();
        if c == '(' {
            paren_depth += 1;
            len_str.push(cursor.next().unwrap());
        } else if c == ')' {
            paren_depth = paren_depth.saturating_sub(1);
            len_str.push(cursor.next().unwrap());
        } else if c == '}' && paren_depth == 0 {
            cursor.next(); // consume }
            length = Some(len_str);
            return Ok((offset, length));
        } else {
            len_str.push(cursor.next().unwrap());
        }
    }

    Err(XesubstError::Syntax {
        span: start_span,
        message: "unterminated substring expansion".to_string(),
    })
}

/// Parses pattern and replacement words in `${v/pattern/replacement}`.
fn parse_replace_words(
    cursor: &mut SourceCursor<'_>,
    config: &ModeConfig,
    start_span: Span,
) -> Result<(Word, Option<Word>), XesubstError> {
    let pattern = parse_word(cursor, &['/', '}'], config)?;
    if cursor.peek() == Some('/') {
        cursor.next(); // consume /
        let replacement = parse_word(cursor, &['}'], config)?;
        expect_char(cursor, '}', start_span)?;
        Ok((pattern, Some(replacement)))
    } else if cursor.peek() == Some('}') {
        cursor.next(); // consume }
        Ok((pattern, None))
    } else {
        Err(XesubstError::Syntax {
            span: start_span,
            message: "unterminated replace expansion".to_string(),
        })
    }
}

/// Parses a word/pattern expression inside `${...}`.
pub fn parse_word(
    cursor: &mut SourceCursor<'_>,
    delimiters: &[char],
    config: &ModeConfig,
) -> Result<Word, XesubstError> {
    let mut parts = Vec::new();
    let mut lit_buf = String::new();
    let mut lit_start_span = cursor.current_span(1);

    while !cursor.is_eof() {
        let c = cursor.peek().unwrap();
        if delimiters.contains(&c) {
            break;
        }

        // Backslash escaping
        if c == '\\' {
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(
                    WordPart::Literal(std::mem::take(&mut lit_buf)),
                    lit_start_span,
                ));
            }
            let esc_span = cursor.current_span(2);
            cursor.next(); // consume \
            if let Some(esc_c) = cursor.next() {
                parts.push(Spanned::new(WordPart::Escaped(esc_c), esc_span));
            } else {
                parts.push(Spanned::new(WordPart::Literal("\\".to_string()), esc_span));
            }
            lit_start_span = cursor.current_span(1);
            continue;
        }

        // Single quotes
        if c == '\'' {
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(
                    WordPart::Literal(std::mem::take(&mut lit_buf)),
                    lit_start_span,
                ));
            }
            let sq_span = cursor.current_span(1);
            cursor.next(); // consume '
            let mut sq_content = String::new();
            let mut closed = false;
            for sc in cursor.by_ref() {
                if sc == '\'' {
                    closed = true;
                    break;
                }
                sq_content.push(sc);
            }
            if !closed {
                return Err(XesubstError::Syntax {
                    span: sq_span,
                    message: "unterminated single quote in parameter expansion".to_string(),
                });
            }
            parts.push(Spanned::new(WordPart::SingleQuoted(sq_content), sq_span));
            lit_start_span = cursor.current_span(1);
            continue;
        }

        // Double quotes
        if c == '"' {
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(
                    WordPart::Literal(std::mem::take(&mut lit_buf)),
                    lit_start_span,
                ));
            }
            let dq_span = cursor.current_span(1);
            cursor.next(); // consume "
            let dq_parts = parse_double_quoted(cursor, config, dq_span)?;
            parts.push(Spanned::new(WordPart::DoubleQuoted(dq_parts), dq_span));
            lit_start_span = cursor.current_span(1);
            continue;
        }

        // Variables: $VAR, ${...}, $((...))
        if c == '$' {
            if cursor.starts_with("${") {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let nested = parse_braced_param(cursor, config)?;
                parts.push(Spanned::new(
                    WordPart::Braced(Box::new(nested.node)),
                    nested.span,
                ));
                lit_start_span = cursor.current_span(1);
                continue;
            }

            if cursor.starts_with("$((") && config.arith {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let arith_elem = parse_arith_expansion(cursor, config)?;
                if let TemplateElement::Arith(spanned_arith) = arith_elem {
                    parts.push(Spanned::new(
                        WordPart::Arith(spanned_arith.node),
                        spanned_arith.span,
                    ));
                }
                lit_start_span = cursor.current_span(1);
                continue;
            }

            if let Some(next_c) = cursor.peek_ahead(1)
                && is_ident_start(next_c)
            {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let var_span = cursor.current_span(1);
                cursor.next(); // consume $
                let var_start = cursor.current_offset();
                while let Some(ic) = cursor.peek() {
                    if is_ident_continue(ic) {
                        cursor.next();
                    } else {
                        break;
                    }
                }
                let var_name = cursor.input[var_start..cursor.current_offset()].to_string();
                parts.push(Spanned::new(WordPart::SimpleVar(var_name), var_span));
                lit_start_span = cursor.current_span(1);
                continue;
            }
        }

        lit_buf.push(cursor.next().unwrap());
    }

    if !lit_buf.is_empty() {
        parts.push(Spanned::new(WordPart::Literal(lit_buf), lit_start_span));
    }

    Ok(Word::new(parts))
}

/// Parses contents inside double quotes `"..."`.
fn parse_double_quoted(
    cursor: &mut SourceCursor<'_>,
    config: &ModeConfig,
    open_span: Span,
) -> Result<Vec<Spanned<WordPart>>, XesubstError> {
    let mut parts = Vec::new();
    let mut lit_buf = String::new();
    let mut lit_start_span = cursor.current_span(1);

    while !cursor.is_eof() {
        let c = cursor.peek().unwrap();
        if c == '"' {
            cursor.next(); // consume closing "
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(WordPart::Literal(lit_buf), lit_start_span));
            }
            return Ok(parts);
        }

        // Inside double quotes in Bash: \$ \" \\ \` are escaped
        if c == '\\'
            && let Some(esc_c) = cursor.peek_ahead(1)
            && (esc_c == '$' || esc_c == '"' || esc_c == '\\' || esc_c == '`')
        {
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(
                    WordPart::Literal(std::mem::take(&mut lit_buf)),
                    lit_start_span,
                ));
            }
            let esc_span = cursor.current_span(2);
            cursor.next(); // consume \
            cursor.next(); // consume esc_c
            parts.push(Spanned::new(WordPart::Escaped(esc_c), esc_span));
            lit_start_span = cursor.current_span(1);
            continue;
        }

        // Parameter expansion inside double quotes
        if c == '$' {
            if cursor.starts_with("${") {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let nested = parse_braced_param(cursor, config)?;
                parts.push(Spanned::new(
                    WordPart::Braced(Box::new(nested.node)),
                    nested.span,
                ));
                lit_start_span = cursor.current_span(1);
                continue;
            }

            if let Some(next_c) = cursor.peek_ahead(1)
                && is_ident_start(next_c)
            {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let var_span = cursor.current_span(1);
                cursor.next(); // consume $
                let var_start = cursor.current_offset();
                while let Some(ic) = cursor.peek() {
                    if is_ident_continue(ic) {
                        cursor.next();
                    } else {
                        break;
                    }
                }
                let var_name = cursor.input[var_start..cursor.current_offset()].to_string();
                parts.push(Spanned::new(WordPart::SimpleVar(var_name), var_span));
                lit_start_span = cursor.current_span(1);
                continue;
            }
        }

        lit_buf.push(cursor.next().unwrap());
    }

    Err(XesubstError::Syntax {
        span: open_span,
        message: "unterminated double quote in parameter expansion".to_string(),
    })
}

/// Parses a value in a .env assignment, stopping at unquoted whitespace or comment `#`.
pub fn parse_dotenv_value(
    cursor: &mut SourceCursor<'_>,
    config: &ModeConfig,
) -> Result<Word, XesubstError> {
    let mut parts = Vec::new();
    let mut lit_buf = String::new();
    let mut lit_start_span = cursor.current_span(1);

    while !cursor.is_eof() {
        let c = cursor.peek().unwrap();
        if c.is_whitespace() || c == '#' {
            break;
        }

        // Backslash escaping
        if c == '\\' {
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(
                    WordPart::Literal(std::mem::take(&mut lit_buf)),
                    lit_start_span,
                ));
            }
            let esc_span = cursor.current_span(2);
            cursor.next(); // consume \
            if let Some(esc_c) = cursor.next() {
                parts.push(Spanned::new(WordPart::Escaped(esc_c), esc_span));
            } else {
                parts.push(Spanned::new(WordPart::Literal("\\".to_string()), esc_span));
            }
            lit_start_span = cursor.current_span(1);
            continue;
        }

        // Single quotes
        if c == '\'' {
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(
                    WordPart::Literal(std::mem::take(&mut lit_buf)),
                    lit_start_span,
                ));
            }
            let sq_span = cursor.current_span(1);
            cursor.next(); // consume '
            let mut sq_content = String::new();
            let mut closed = false;
            for sc in cursor.by_ref() {
                if sc == '\'' {
                    closed = true;
                    break;
                }
                sq_content.push(sc);
            }
            if !closed {
                return Err(XesubstError::Syntax {
                    span: sq_span,
                    message: "unterminated single quote in value".to_string(),
                });
            }
            parts.push(Spanned::new(WordPart::SingleQuoted(sq_content), sq_span));
            lit_start_span = cursor.current_span(1);
            continue;
        }

        // Double quotes
        if c == '"' {
            if !lit_buf.is_empty() {
                parts.push(Spanned::new(
                    WordPart::Literal(std::mem::take(&mut lit_buf)),
                    lit_start_span,
                ));
            }
            let dq_span = cursor.current_span(1);
            cursor.next(); // consume "
            let dq_parts = parse_double_quoted(cursor, config, dq_span)?;
            parts.push(Spanned::new(WordPart::DoubleQuoted(dq_parts), dq_span));
            lit_start_span = cursor.current_span(1);
            continue;
        }

        // Variables: $VAR, ${...}, $((...))
        if c == '$' {
            if cursor.starts_with("${") {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let nested = parse_braced_param(cursor, config)?;
                parts.push(Spanned::new(
                    WordPart::Braced(Box::new(nested.node)),
                    nested.span,
                ));
                lit_start_span = cursor.current_span(1);
                continue;
            }

            if cursor.starts_with("$((") && config.arith {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let arith_elem = parse_arith_expansion(cursor, config)?;
                if let TemplateElement::Arith(spanned_arith) = arith_elem {
                    parts.push(Spanned::new(
                        WordPart::Arith(spanned_arith.node),
                        spanned_arith.span,
                    ));
                }
                lit_start_span = cursor.current_span(1);
                continue;
            }

            if let Some(next_c) = cursor.peek_ahead(1)
                && is_ident_start(next_c)
            {
                if !lit_buf.is_empty() {
                    parts.push(Spanned::new(
                        WordPart::Literal(std::mem::take(&mut lit_buf)),
                        lit_start_span,
                    ));
                }
                let var_span = cursor.current_span(1);
                cursor.next(); // consume $
                let var_start = cursor.current_offset();
                while let Some(ic) = cursor.peek() {
                    if is_ident_continue(ic) {
                        cursor.next();
                    } else {
                        break;
                    }
                }
                let var_name = cursor.input[var_start..cursor.current_offset()].to_string();
                parts.push(Spanned::new(WordPart::SimpleVar(var_name), var_span));
                lit_start_span = cursor.current_span(1);
                continue;
            }
        }

        lit_buf.push(cursor.next().unwrap());
    }

    if !lit_buf.is_empty() {
        parts.push(Spanned::new(WordPart::Literal(lit_buf), lit_start_span));
    }

    Ok(Word::new(parts))
}

/// Extracts variable names referenced in a SHELL-FORMAT string.
pub fn extract_shell_format_variables(format: &str) -> Vec<String> {
    let mut vars = Vec::new();
    let mut cursor = SourceCursor::new(format);

    while !cursor.is_eof() {
        if cursor.peek() == Some('$') {
            cursor.next(); // consume $
            if cursor.peek() == Some('{') {
                cursor.next(); // consume {
                let start = cursor.current_offset();
                if let Some(c) = cursor.peek()
                    && is_ident_start(c)
                {
                    cursor.next();
                    while let Some(c2) = cursor.peek() {
                        if is_ident_continue(c2) {
                            cursor.next();
                        } else {
                            break;
                        }
                    }
                    let name = cursor.input[start..cursor.current_offset()].to_string();
                    if cursor.peek() == Some('}') {
                        cursor.next();
                        if !vars.contains(&name) {
                            vars.push(name);
                        }
                    }
                }
            } else if let Some(c) = cursor.peek()
                && is_ident_start(c)
            {
                let start = cursor.current_offset();
                cursor.next();
                while let Some(c2) = cursor.peek() {
                    if is_ident_continue(c2) {
                        cursor.next();
                    } else {
                        break;
                    }
                }
                let name = cursor.input[start..cursor.current_offset()].to_string();
                if !vars.contains(&name) {
                    vars.push(name);
                }
            }
        } else {
            cursor.next();
        }
    }

    vars
}
