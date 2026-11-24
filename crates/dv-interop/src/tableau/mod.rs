//! Tableau workbooks (`.twb`, `.twbx`).

mod export;
mod import;

pub(crate) use export::export;
pub(crate) use import::import;

/// The text of every `[...]` group in `text`, in order (`[a].[b]` gives `a`, `b`).
/// A doubled `]]` inside a group is a literal `]`.
pub(crate) fn bracket_parts(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut parts = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '[' {
            i += 1;
            let mut part = String::new();
            while i < chars.len() {
                if chars[i] == ']' {
                    if chars.get(i + 1) == Some(&']') {
                        part.push(']');
                        i += 2;
                        continue;
                    }
                    break;
                }
                part.push(chars[i]);
                i += 1;
            }
            parts.push(part);
        }
        i += 1;
    }
    parts
}

pub(crate) fn strip_brackets(text: &str) -> String {
    let t = text.trim();
    t.strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .unwrap_or(t)
        .to_string()
}

#[cfg(test)]
mod tests;
