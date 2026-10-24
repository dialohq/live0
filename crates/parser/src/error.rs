use std::fmt::{self, Write as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span(pub usize, pub usize);

#[derive(Debug, Clone, PartialEq)]
pub struct SyntaxError {
    pub message: String,
    pub span: Span,
}

impl SyntaxError {
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    pub fn format(&self, src: &str) -> String {
        let start = floor_char_boundary(src, self.span.0.min(src.len()));
        let end = floor_char_boundary(src, self.span.1.clamp(start, src.len()));

        let line_start = src[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = src[start..].find('\n').map_or(src.len(), |i| start + i);
        let line = src[line_start..line_end].trim_end_matches('\r');
        let line_no = src[..start].bytes().filter(|&b| b == b'\n').count() + 1;

        let prefix = &src[line_start..start];
        let col = prefix.chars().count() + 1;

        let pad: String = prefix
            .chars()
            .map(|c| if c == '\t' { '\t' } else { ' ' })
            .collect();

        let underline_end = end.min(line_start + line.len()).max(start);
        let width = src[start..underline_end].chars().count().max(1);
        let continues = end > underline_end && end > line_start + line.len();

        let gutter = line_no.to_string().len();
        let mut out = String::new();
        let _ = writeln!(out, "error: {}", self.message);
        let _ = writeln!(out, "{:w$}--> {}:{}", "", line_no, col, w = gutter);
        let _ = writeln!(out, "{:w$} |", "", w = gutter);
        let _ = writeln!(out, "{line_no} | {line}");
        let _ = write!(out, "{:w$} | {pad}{}", "", "^".repeat(width), w = gutter);
        if continues {
            out.push_str("...");
        }
        out
    }
}

impl fmt::Display for SyntaxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.span.0)
    }
}

impl std::error::Error for SyntaxError {}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
