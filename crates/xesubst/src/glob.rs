//! Custom Bash glob pattern engine.
//!
//! Implements Bash globbing without external crates:
//! - `*` (zero or more chars)
//! - `?` (any single char)
//! - `[...]`, `[!...]`, `[^...]` character classes, ranges, and POSIX character classes.
//! - Literal quoted/escaped chars.
//! - Shortest and longest match semantics for prefix/suffix stripping and replacement.

/// An atom in a compiled glob pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternAtom {
    Literal(char),
    AnyChar,
    AnyString,
    CharClass {
        negated: bool,
        items: Vec<ClassItem>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClassItem {
    Single(char),
    Range(char, char),
    Posix(PosixClass),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosixClass {
    Alnum,
    Alpha,
    Ascii,
    Blank,
    Cntrl,
    Digit,
    Graph,
    Lower,
    Print,
    Punct,
    Space,
    Upper,
    Word,
    Xdigit,
}

impl PosixClass {
    pub fn matches(&self, c: char) -> bool {
        match self {
            Self::Alnum => c.is_alphanumeric(),
            Self::Alpha => c.is_alphabetic(),
            Self::Ascii => c.is_ascii(),
            Self::Blank => c == ' ' || c == '\t',
            Self::Cntrl => c.is_control(),
            Self::Digit => c.is_ascii_digit(),
            Self::Graph => c.is_ascii_graphic(),
            Self::Lower => c.is_lowercase(),
            Self::Print => !c.is_control(),
            Self::Punct => c.is_ascii_punctuation(),
            Self::Space => c.is_whitespace(),
            Self::Upper => c.is_uppercase(),
            Self::Word => c.is_alphanumeric() || c == '_',
            Self::Xdigit => c.is_ascii_hexdigit(),
        }
    }
}

/// A compiled Bash pattern.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Pattern {
    pub atoms: Vec<PatternAtom>,
}

impl Pattern {
    /// Compiles a pattern from a slice of atoms.
    pub fn new(atoms: Vec<PatternAtom>) -> Self {
        Self { atoms }
    }

    /// Checks if the pattern is empty.
    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }

    /// Returns whether the pattern matches the full slice of characters.
    pub fn matches_all(&self, s: &[char]) -> bool {
        match_atoms(&self.atoms, s)
    }

    /// Finds the length of the longest prefix of `s` that matches this pattern.
    pub fn longest_prefix_match(&self, s: &[char]) -> Option<usize> {
        for len in (0..=s.len()).rev() {
            if self.matches_all(&s[..len]) {
                return Some(len);
            }
        }
        None
    }

    /// Finds the length of the shortest prefix of `s` that matches this pattern.
    pub fn shortest_prefix_match(&self, s: &[char]) -> Option<usize> {
        for len in 0..=s.len() {
            if self.matches_all(&s[..len]) {
                return Some(len);
            }
        }
        None
    }

    /// Finds the length of the longest suffix of `s` that matches this pattern.
    pub fn longest_suffix_match(&self, s: &[char]) -> Option<usize> {
        for len in (0..=s.len()).rev() {
            if self.matches_all(&s[s.len() - len..]) {
                return Some(len);
            }
        }
        None
    }

    /// Finds the length of the shortest suffix of `s` that matches this pattern.
    pub fn shortest_suffix_match(&self, s: &[char]) -> Option<usize> {
        for len in 0..=s.len() {
            if self.matches_all(&s[s.len() - len..]) {
                return Some(len);
            }
        }
        None
    }

    /// Strips prefix from string: shortest (`greedy=false`) or longest (`greedy=true`).
    pub fn strip_prefix<'a>(&self, text: &'a str, greedy: bool) -> &'a str {
        if self.is_empty() {
            return text;
        }
        let chars: Vec<char> = text.chars().collect();
        let match_len = if greedy {
            self.longest_prefix_match(&chars)
        } else {
            self.shortest_prefix_match(&chars)
        };

        if let Some(char_count) = match_len {
            let byte_idx = text
                .char_indices()
                .nth(char_count)
                .map(|(idx, _)| idx)
                .unwrap_or(text.len());
            &text[byte_idx..]
        } else {
            text
        }
    }

    /// Strips suffix from string: shortest (`greedy=false`) or longest (`greedy=true`).
    pub fn strip_suffix<'a>(&self, text: &'a str, greedy: bool) -> &'a str {
        if self.is_empty() {
            return text;
        }
        let chars: Vec<char> = text.chars().collect();
        let match_len = if greedy {
            self.longest_suffix_match(&chars)
        } else {
            self.shortest_suffix_match(&chars)
        };

        if let Some(char_count) = match_len {
            let keep_count = chars.len() - char_count;
            let byte_idx = text
                .char_indices()
                .nth(keep_count)
                .map(|(idx, _)| idx)
                .unwrap_or(text.len());
            &text[..byte_idx]
        } else {
            text
        }
    }

    /// Replaces occurrences in `text` based on mode.
    pub fn replace(&self, text: &str, replacement: &str, mode: crate::ast::ReplaceMode) -> String {
        if self.is_empty() {
            return text.to_string();
        }

        let chars: Vec<char> = text.chars().collect();
        match mode {
            crate::ast::ReplaceMode::Prefix => {
                // Anchored at beginning, longest match
                if let Some(len) = self.longest_prefix_match(&chars) {
                    let mut res = replacement.to_string();
                    let byte_idx = text
                        .char_indices()
                        .nth(len)
                        .map(|(i, _)| i)
                        .unwrap_or(text.len());
                    res.push_str(&text[byte_idx..]);
                    res
                } else {
                    text.to_string()
                }
            }
            crate::ast::ReplaceMode::Suffix => {
                // Anchored at end, longest match
                if let Some(len) = self.longest_suffix_match(&chars) {
                    let keep_count = chars.len() - len;
                    let byte_idx = text
                        .char_indices()
                        .nth(keep_count)
                        .map(|(i, _)| i)
                        .unwrap_or(text.len());
                    let mut res = text[..byte_idx].to_string();
                    res.push_str(replacement);
                    res
                } else {
                    text.to_string()
                }
            }
            crate::ast::ReplaceMode::First => {
                // First match in string (longest at that start index)
                for start in 0..chars.len() {
                    if let Some(len) = self.longest_prefix_match(&chars[start..]) {
                        let mut res = String::new();
                        let start_byte = text
                            .char_indices()
                            .nth(start)
                            .map(|(i, _)| i)
                            .unwrap_or(text.len());
                        let end_byte = text
                            .char_indices()
                            .nth(start + len)
                            .map(|(i, _)| i)
                            .unwrap_or(text.len());

                        res.push_str(&text[..start_byte]);
                        res.push_str(replacement);
                        res.push_str(&text[end_byte..]);
                        return res;
                    }
                }
                text.to_string()
            }
            crate::ast::ReplaceMode::All => {
                let mut res = String::new();
                let mut idx = 0;
                while idx < chars.len() {
                    if let Some(len) = self.longest_prefix_match(&chars[idx..]) {
                        res.push_str(replacement);
                        idx += if len > 0 { len } else { 1 };
                    } else {
                        res.push(chars[idx]);
                        idx += 1;
                    }
                }
                res
            }
        }
    }
}

fn match_atoms(atoms: &[PatternAtom], s: &[char]) -> bool {
    if atoms.is_empty() {
        return s.is_empty();
    }

    match &atoms[0] {
        PatternAtom::Literal(c) => {
            if !s.is_empty() && s[0] == *c {
                match_atoms(&atoms[1..], &s[1..])
            } else {
                false
            }
        }
        PatternAtom::AnyChar => {
            if !s.is_empty() {
                match_atoms(&atoms[1..], &s[1..])
            } else {
                false
            }
        }
        PatternAtom::CharClass { negated, items } => {
            if s.is_empty() {
                return false;
            }
            let c = s[0];
            let mut matches_any = false;
            for item in items {
                match item {
                    ClassItem::Single(ch) => {
                        if *ch == c {
                            matches_any = true;
                            break;
                        }
                    }
                    ClassItem::Range(start, end) => {
                        if c >= *start && c <= *end {
                            matches_any = true;
                            break;
                        }
                    }
                    ClassItem::Posix(posix) => {
                        if posix.matches(c) {
                            matches_any = true;
                            break;
                        }
                    }
                }
            }
            if matches_any != *negated {
                match_atoms(&atoms[1..], &s[1..])
            } else {
                false
            }
        }
        PatternAtom::AnyString => {
            // Collapse consecutive *
            let mut next_atom_idx = 1;
            while next_atom_idx < atoms.len()
                && matches!(atoms[next_atom_idx], PatternAtom::AnyString)
            {
                next_atom_idx += 1;
            }
            if next_atom_idx >= atoms.len() {
                return true;
            }

            // Try matching remaining pattern from any position in s
            for skip in 0..=s.len() {
                if match_atoms(&atoms[next_atom_idx..], &s[skip..]) {
                    return true;
                }
            }
            false
        }
    }
}

/// Parses an unquoted pattern string into PatternAtoms.
pub fn parse_pattern_from_str(s: &str) -> Pattern {
    let mut atoms = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        match chars[i] {
            '*' => {
                atoms.push(PatternAtom::AnyString);
                i += 1;
            }
            '?' => {
                atoms.push(PatternAtom::AnyChar);
                i += 1;
            }
            '\\' => {
                if i + 1 < chars.len() {
                    atoms.push(PatternAtom::Literal(chars[i + 1]));
                    i += 2;
                } else {
                    atoms.push(PatternAtom::Literal('\\'));
                    i += 1;
                }
            }
            '[' => {
                if let Some((class_atom, next_idx)) = parse_char_class(&chars, i) {
                    atoms.push(class_atom);
                    i = next_idx;
                } else {
                    atoms.push(PatternAtom::Literal('['));
                    i += 1;
                }
            }
            c => {
                atoms.push(PatternAtom::Literal(c));
                i += 1;
            }
        }
    }

    Pattern::new(atoms)
}

fn parse_char_class(chars: &[char], start: usize) -> Option<(PatternAtom, usize)> {
    if start >= chars.len() || chars[start] != '[' {
        return None;
    }
    let mut idx = start + 1;
    if idx >= chars.len() {
        return None;
    }

    let negated = if chars[idx] == '!' || chars[idx] == '^' {
        idx += 1;
        true
    } else {
        false
    };

    if idx >= chars.len() {
        return None;
    }

    let mut items = Vec::new();

    // If ']' or '-' is first character, it is treated as a literal
    if idx < chars.len() && chars[idx] == ']' {
        items.push(ClassItem::Single(']'));
        idx += 1;
    }

    while idx < chars.len() && chars[idx] != ']' {
        // Check for POSIX class [:class:]
        if chars[idx] == '[' && idx + 1 < chars.len() && chars[idx + 1] == ':' {
            let posix_start = idx + 2;
            let mut end = posix_start;
            while end + 1 < chars.len() && !(chars[end] == ':' && chars[end + 1] == ']') {
                end += 1;
            }
            if end + 1 < chars.len() && chars[end] == ':' && chars[end + 1] == ']' {
                let name: String = chars[posix_start..end].iter().collect();
                if let Some(posix) = match_posix_name(&name) {
                    items.push(ClassItem::Posix(posix));
                    idx = end + 2;
                    continue;
                }
            }
        }

        // Check for range a-b
        if idx + 2 < chars.len() && chars[idx + 1] == '-' && chars[idx + 2] != ']' {
            items.push(ClassItem::Range(chars[idx], chars[idx + 2]));
            idx += 3;
            continue;
        }

        items.push(ClassItem::Single(chars[idx]));
        idx += 1;
    }

    if idx < chars.len() && chars[idx] == ']' {
        Some((PatternAtom::CharClass { negated, items }, idx + 1))
    } else {
        None
    }
}

fn match_posix_name(name: &str) -> Option<PosixClass> {
    match name {
        "alnum" => Some(PosixClass::Alnum),
        "alpha" => Some(PosixClass::Alpha),
        "ascii" => Some(PosixClass::Ascii),
        "blank" => Some(PosixClass::Blank),
        "cntrl" => Some(PosixClass::Cntrl),
        "digit" => Some(PosixClass::Digit),
        "graph" => Some(PosixClass::Graph),
        "lower" => Some(PosixClass::Lower),
        "print" => Some(PosixClass::Print),
        "punct" => Some(PosixClass::Punct),
        "space" => Some(PosixClass::Space),
        "upper" => Some(PosixClass::Upper),
        "word" => Some(PosixClass::Word),
        "xdigit" => Some(PosixClass::Xdigit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prefix_suffix_strip() {
        let pat1 = parse_pattern_from_str("b*a");
        assert_eq!(pat1.strip_prefix("banana", false), "nana");
        assert_eq!(pat1.strip_prefix("banana", true), "");

        let pat2 = parse_pattern_from_str("a*a");
        assert_eq!(pat2.strip_suffix("banana", false), "ban");
        assert_eq!(pat2.strip_suffix("banana", true), "b");
    }

    #[test]
    fn test_pattern_replace() {
        let pat = parse_pattern_from_str("an");
        assert_eq!(
            pat.replace("banana", "X", crate::ast::ReplaceMode::First),
            "bXana"
        );
        assert_eq!(
            pat.replace("banana", "X", crate::ast::ReplaceMode::All),
            "bXXa"
        );

        let pat_prefix = parse_pattern_from_str("b*a");
        assert_eq!(
            pat_prefix.replace("banana", "X", crate::ast::ReplaceMode::Prefix),
            "X"
        );

        let pat_suffix = parse_pattern_from_str("a*a");
        assert_eq!(
            pat_suffix.replace("banana", "X", crate::ast::ReplaceMode::Suffix),
            "bX"
        );
    }

    #[test]
    fn test_posix_and_classes() {
        let pat = parse_pattern_from_str("[[:alpha:]]*");
        assert!(pat.matches_all(&['h', 'e', 'l', 'l', 'o']));

        let pat_neg = parse_pattern_from_str("[!0-9]*");
        assert!(pat_neg.matches_all(&['a', 'b', 'c']));
        assert!(!pat_neg.matches_all(&['1', 'a']));
    }
}
