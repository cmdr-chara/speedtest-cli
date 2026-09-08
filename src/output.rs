//! Line-oriented output: no panics on closed pipes and no terminal control injection.
use std::{
    fmt,
    io::{self, Write},
};

/// Keep printable Unicode and layout whitespace, but remove terminal/bidi controls.
/// JSON serializers already escape controls; this also protects human-readable data.
pub fn safe_text(text: &str) -> String {
    text.chars()
        .filter(|c| {
            (!c.is_control() || matches!(c, '\n' | '\t'))
                && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

pub fn line(arguments: fmt::Arguments<'_>) -> io::Result<()> {
    writeln!(io::stdout().lock(), "{}", safe_text(&arguments.to_string()))
}

pub fn diagnostic(arguments: fmt::Arguments<'_>) -> io::Result<()> {
    writeln!(io::stderr().lock(), "{}", safe_text(&arguments.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_escape_c1_bidi_and_carriage_return_controls() {
        let output = safe_text("server\x1b[2J\r\u{009b}\u{202e}name\nUnicode: café\t✓");
        assert_eq!(output, "server[2Jname\nUnicode: café\t✓");
    }
}
