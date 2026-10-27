//! Evaluator for parameter expansion AST and template streaming.

use std::io::{BufRead, Write};

use crate::ModeConfig;
use crate::arith::eval_arithmetic;
use crate::ast::*;
use crate::env::VariableTable;
use crate::error::XesubstError;
use crate::glob::{Pattern, PatternAtom, parse_pattern_from_str};
use crate::parser::{TemplateElement, parse_template};
use crate::span::Span;

/// Expands a template string against an environment.
pub fn expand_str(
    input: &str,
    env: &mut dyn VariableTable,
    config: &ModeConfig,
) -> Result<String, XesubstError> {
    let elements = parse_template(input, config)?;
    let mut out = String::new();

    for elem in elements {
        match elem {
            TemplateElement::Literal(s) => out.push_str(&s),
            TemplateElement::SimpleVar(spanned_name) => {
                let name = &spanned_name.node;
                if let Some(ref allowed) = config.shell_format_vars
                    && !allowed.contains(name)
                {
                    out.push('$');
                    out.push_str(name);
                    continue;
                }
                if let Some(val) = env.get(name) {
                    out.push_str(val);
                } else if config.nounset {
                    return Err(XesubstError::Expansion {
                        span: spanned_name.span,
                        message: format!("{name}: unbound variable"),
                    });
                }
            }
            TemplateElement::Arith(spanned_expr) => {
                let val = eval_arithmetic(&spanned_expr.node, env, spanned_expr.span)?;
                out.push_str(&val.to_string());
            }
            TemplateElement::Braced(spanned_param) => {
                let name = &spanned_param.node.name;
                if let Some(ref allowed) = config.shell_format_vars
                    && !allowed.contains(name)
                    && config.compat
                {
                    out.push_str("${");
                    out.push_str(name);
                    out.push('}');
                    continue;
                }
                let val = eval_param_exp(&spanned_param.node, env, config, spanned_param.span)?;
                out.push_str(&val);
            }
        }
    }

    Ok(out)
}

/// Streams expansion line-by-line or chunk-by-chunk to avoid loading large files into memory.
pub fn expand_stream<R: BufRead, W: Write>(
    mut reader: R,
    writer: &mut W,
    env: &mut dyn VariableTable,
    config: &ModeConfig,
) -> Result<(), XesubstError> {
    let mut line = String::new();
    let mut line_num = 1;

    while reader.read_line(&mut line)? > 0 {
        // Detect CRLF
        if line.ends_with("\r\n") {
            if config.strict {
                let span = Span::new(line_num, line.len().saturating_sub(1), 0, 2);
                return Err(XesubstError::CrlfStrict { span });
            }
            // Strip trailing \r before \n
            line.pop(); // pop \n
            line.pop(); // pop \r
            line.push('\n');
            eprintln!(
                "{}:{}: warning: carriage return (CRLF) detected and stripped",
                config.filename, line_num
            );
        }

        let expanded = expand_str(&line, env, config)?;
        writer.write_all(expanded.as_bytes())?;
        line.clear();
        line_num += 1;
    }

    Ok(())
}

/// Evaluates a parameter expansion.
pub fn eval_param_exp(
    param: &ParamExp,
    env: &mut dyn VariableTable,
    config: &ModeConfig,
    span: Span,
) -> Result<String, XesubstError> {
    // Resolve indirect expansion name if requested: ${!v...}
    let actual_name = if param.indirect {
        match env.get(&param.name) {
            Some(indirect_var) if !indirect_var.is_empty() => indirect_var.to_string(),
            _ => {
                if config.nounset {
                    return Err(XesubstError::Expansion {
                        span,
                        message: format!("{}: unbound variable", param.name),
                    });
                }
                String::new()
            }
        }
    } else {
        param.name.clone()
    };

    match &param.op {
        ParamOp::Value => {
            if let Some(val) = env.get(&actual_name) {
                Ok(val.to_string())
            } else if config.nounset {
                Err(XesubstError::Expansion {
                    span,
                    message: format!("{actual_name}: unbound variable"),
                })
            } else {
                Ok(String::new())
            }
        }
        ParamOp::Length => {
            let val = env.get(&actual_name);
            if val.is_none() && config.nounset {
                return Err(XesubstError::Expansion {
                    span,
                    message: format!("{actual_name}: unbound variable"),
                });
            }
            let s = val.unwrap_or("");
            Ok(s.chars().count().to_string())
        }
        ParamOp::PrefixNames { .. } => {
            let names = env.names_with_prefix(&param.name);
            Ok(names.join(" "))
        }
        ParamOp::Default { colon, kind, word } => {
            let val_opt = env.get(&actual_name);
            let is_empty_or_unset = match val_opt {
                None => true,
                Some(s) => *colon && s.is_empty(),
            };

            match kind {
                DefaultKind::UseDefault => {
                    if is_empty_or_unset {
                        eval_word(word, env, config)
                    } else {
                        Ok(val_opt.unwrap().to_string())
                    }
                }
                DefaultKind::AssignDefault => {
                    if is_empty_or_unset {
                        let evaluated = eval_word(word, env, config)?;
                        env.set(&actual_name, &evaluated);
                        Ok(evaluated)
                    } else {
                        Ok(val_opt.unwrap().to_string())
                    }
                }
                DefaultKind::ErrorIfUnset => {
                    if is_empty_or_unset {
                        let msg = eval_word(word, env, config)?;
                        let final_msg = if msg.is_empty() {
                            "parameter null or not set".to_string()
                        } else {
                            msg
                        };
                        Err(XesubstError::Expansion {
                            span,
                            message: format!("{actual_name}: {final_msg}"),
                        })
                    } else {
                        Ok(val_opt.unwrap().to_string())
                    }
                }
                DefaultKind::UseAlternate => {
                    if is_empty_or_unset {
                        Ok(String::new())
                    } else {
                        eval_word(word, env, config)
                    }
                }
            }
        }
        ParamOp::Substring { offset, length } => {
            let val_opt = env.get(&actual_name);
            if val_opt.is_none() && config.nounset {
                return Err(XesubstError::Expansion {
                    span,
                    message: format!("{actual_name}: unbound variable"),
                });
            }
            let val = val_opt.unwrap_or("");
            let chars: Vec<char> = val.chars().collect();
            let total_len = chars.len() as i64;

            let offset_val = eval_arithmetic(offset, env, span)?;
            let start_idx = if offset_val < 0 {
                total_len + offset_val
            } else {
                offset_val
            };

            if start_idx < 0 || start_idx >= total_len {
                return Ok(String::new());
            }

            let start_usize = start_idx as usize;

            if let Some(len_str) = length {
                let len_val = eval_arithmetic(len_str, env, span)?;
                if len_val < 0 {
                    let end_idx = total_len + len_val;
                    if end_idx < start_idx {
                        return Err(XesubstError::Expansion {
                            span,
                            message: format!("{len_val}: substring expression < 0"),
                        });
                    }
                    Ok(chars[start_usize..end_idx as usize].iter().collect())
                } else {
                    let end_usize = (start_usize + len_val as usize).min(chars.len());
                    Ok(chars[start_usize..end_usize].iter().collect())
                }
            } else {
                Ok(chars[start_usize..].iter().collect())
            }
        }
        ParamOp::Strip {
            side,
            greedy,
            pattern,
        } => {
            if env.get(&actual_name).is_none() && config.nounset {
                return Err(XesubstError::Expansion {
                    span,
                    message: format!("{actual_name}: unbound variable"),
                });
            }
            let val = env.get(&actual_name).unwrap_or("").to_string();
            let pat = eval_word_to_pattern(pattern, env, config)?;
            let result = match side {
                StripSide::Prefix => pat.strip_prefix(&val, *greedy),
                StripSide::Suffix => pat.strip_suffix(&val, *greedy),
            };
            Ok(result.to_string())
        }
        ParamOp::Replace {
            mode,
            pattern,
            replacement,
        } => {
            if env.get(&actual_name).is_none() && config.nounset {
                return Err(XesubstError::Expansion {
                    span,
                    message: format!("{actual_name}: unbound variable"),
                });
            }
            let val = env.get(&actual_name).unwrap_or("").to_string();
            let pat = eval_word_to_pattern(pattern, env, config)?;
            let rep_str = if let Some(rep_word) = replacement {
                eval_word(rep_word, env, config)?
            } else {
                String::new()
            };
            Ok(pat.replace(&val, &rep_str, *mode))
        }
        ParamOp::CaseMod {
            upper,
            all,
            pattern,
        } => {
            if env.get(&actual_name).is_none() && config.nounset {
                return Err(XesubstError::Expansion {
                    span,
                    message: format!("{actual_name}: unbound variable"),
                });
            }
            let val = env.get(&actual_name).unwrap_or("").to_string();
            let pat = if let Some(pat_word) = pattern {
                eval_word_to_pattern(pat_word, env, config)?
            } else {
                Pattern::new(vec![PatternAtom::AnyChar])
            };

            let mut out = String::new();
            for (idx, c) in val.chars().enumerate() {
                if (*all || idx == 0) && pat.matches_all(&[c]) {
                    if *upper {
                        out.extend(c.to_uppercase());
                    } else {
                        out.extend(c.to_lowercase());
                    }
                } else {
                    out.push(c);
                }
            }
            Ok(out)
        }
        ParamOp::Transform { op } => {
            let val_opt = env.get(&actual_name);
            if val_opt.is_none() && config.nounset {
                return Err(XesubstError::Expansion {
                    span,
                    message: format!("{actual_name}: unbound variable"),
                });
            }
            let val = val_opt.unwrap_or("");
            match op {
                TransformOp::Quote => {
                    // Bash @Q: 'escaped' where ' is '\''
                    let escaped = val.replace('\'', "'\\''");
                    Ok(format!("'{escaped}'"))
                }
                TransformOp::Escape => {
                    // Bash @E: expand backslash escape sequences
                    Ok(expand_backslash_escapes(val))
                }
                TransformOp::Assignment => {
                    let escaped = val.replace('\'', "'\\''");
                    Ok(format!("{actual_name}='{escaped}'"))
                }
                TransformOp::UpperAll => Ok(val.to_uppercase()),
                TransformOp::UpperFirst => {
                    let mut chars = val.chars();
                    if let Some(first) = chars.next() {
                        let mut out = String::new();
                        out.extend(first.to_uppercase());
                        out.extend(chars);
                        Ok(out)
                    } else {
                        Ok(String::new())
                    }
                }
                TransformOp::LowerAll => Ok(val.to_lowercase()),
            }
        }
    }
}

/// Evaluates a Word to a string.
pub fn eval_word(
    word: &Word,
    env: &mut dyn VariableTable,
    config: &ModeConfig,
) -> Result<String, XesubstError> {
    let mut out = String::new();
    for part in &word.parts {
        match &part.node {
            WordPart::Literal(s) => out.push_str(s),
            WordPart::Escaped(c) => out.push(*c),
            WordPart::SingleQuoted(s) => out.push_str(s),
            WordPart::DoubleQuoted(subparts) => {
                let subword = Word::new(subparts.clone());
                let s = eval_word(&subword, env, config)?;
                out.push_str(&s);
            }
            WordPart::SimpleVar(name) => {
                if let Some(val) = env.get(name) {
                    out.push_str(val);
                } else if config.nounset {
                    return Err(XesubstError::Expansion {
                        span: part.span,
                        message: format!("{name}: unbound variable"),
                    });
                }
            }
            WordPart::Braced(param) => {
                let s = eval_param_exp(param, env, config, part.span)?;
                out.push_str(&s);
            }
            WordPart::Arith(expr) => {
                let val = eval_arithmetic(expr, env, part.span)?;
                out.push_str(&val.to_string());
            }
        }
    }
    Ok(out)
}

/// Evaluates a Word to a compiled Pattern, honoring literalness of quotes/escapes.
pub fn eval_word_to_pattern(
    word: &Word,
    env: &mut dyn VariableTable,
    config: &ModeConfig,
) -> Result<Pattern, XesubstError> {
    let mut atoms = Vec::new();
    for part in &word.parts {
        match &part.node {
            WordPart::Literal(s) => {
                let parsed = parse_pattern_from_str(s);
                atoms.extend(parsed.atoms);
            }
            WordPart::Escaped(c) => {
                atoms.push(PatternAtom::Literal(*c));
            }
            WordPart::SingleQuoted(s) => {
                for c in s.chars() {
                    atoms.push(PatternAtom::Literal(c));
                }
            }
            WordPart::DoubleQuoted(subparts) => {
                let subword = Word::new(subparts.clone());
                let text = eval_word(&subword, env, config)?;
                for c in text.chars() {
                    atoms.push(PatternAtom::Literal(c));
                }
            }
            WordPart::SimpleVar(name) => {
                let val = env.get(name).unwrap_or("");
                let parsed = parse_pattern_from_str(val);
                atoms.extend(parsed.atoms);
            }
            WordPart::Braced(param) => {
                let text = eval_param_exp(param, env, config, part.span)?;
                let parsed = parse_pattern_from_str(&text);
                atoms.extend(parsed.atoms);
            }
            WordPart::Arith(expr) => {
                let val = eval_arithmetic(expr, env, part.span)?;
                for c in val.to_string().chars() {
                    atoms.push(PatternAtom::Literal(c));
                }
            }
        }
    }
    Ok(Pattern::new(atoms))
}

/// Expands backslash escapes for Bash @E operator.
fn expand_backslash_escapes(s: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            i += 1;
            match chars[i] {
                'a' => {
                    out.push('\x07');
                    i += 1;
                }
                'b' => {
                    out.push('\x08');
                    i += 1;
                }
                'e' | 'E' => {
                    out.push('\x1B');
                    i += 1;
                }
                'f' => {
                    out.push('\x0C');
                    i += 1;
                }
                'n' => {
                    out.push('\n');
                    i += 1;
                }
                'r' => {
                    out.push('\r');
                    i += 1;
                }
                't' => {
                    out.push('\t');
                    i += 1;
                }
                'v' => {
                    out.push('\x0B');
                    i += 1;
                }
                '\\' => {
                    out.push('\\');
                    i += 1;
                }
                '\'' => {
                    out.push('\'');
                    i += 1;
                }
                '"' => {
                    out.push('"');
                    i += 1;
                }
                'x' => {
                    // \xHH
                    i += 1;
                    let mut hex = String::new();
                    while i < chars.len() && hex.len() < 2 && chars[i].is_ascii_hexdigit() {
                        hex.push(chars[i]);
                        i += 1;
                    }
                    if let Ok(b) = u8::from_str_radix(&hex, 16) {
                        out.push(b as char);
                    }
                }
                '0'..='7' => {
                    // \0NNN octal
                    let mut oct = String::new();
                    while i < chars.len() && oct.len() < 3 && ('0'..='7').contains(&chars[i]) {
                        oct.push(chars[i]);
                        i += 1;
                    }
                    if let Ok(b) = u8::from_str_radix(&oct, 8) {
                        out.push(b as char);
                    }
                }
                c => {
                    out.push('\\');
                    out.push(c);
                    i += 1;
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }

    out
}
