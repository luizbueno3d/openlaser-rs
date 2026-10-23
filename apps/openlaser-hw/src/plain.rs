//! Plain text for the operator's terminal (SPEC §1.3 rule 3, §4.6 mechanism 4).
//!
//! The CLI prints strings that come from outside it (layer names and notices from the document and the engine,
//! driver notes and warnings, profile files, USB descriptors) right next to the decoded numbers the operator confirms
//! by typing `ARM <challenge>`. A terminal obeys the escape sequences in what it prints, so an agent-named layer that
//! carries `ESC[8m` could conceal the real power and leave forged numbers in its place. Every such string goes
//! through [`field`] before it is printed: control characters (C0 including ESC, CR, LF and TAB; DEL; C1 including
//! the one-byte CSI U+009B), bidirectional overrides and isolates and invisible characters are dropped, and the
//! length is bounded. [`block`] applies the same filter to a whole block the CLI writes, keeping the CLI's own `\n`:
//! `run_with` passes everything a command writes through it, and `TtyTerminal::write` does once more.

/// The longest layer name the summary shows.
pub const NAME_MAX_CHARS: usize = 32;
/// The longest single message from outside the CLI (a notice, a warning, a checklist item, a descriptor).
pub const MESSAGE_MAX_CHARS: usize = 300;

/// A character the operator would not see, or that changes how the rest of the line reads: every control
/// character, bidirectional formatting, zero-width and other invisible characters, and the Unicode tag block.
pub fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// One string from outside the CLI as one line of plain text: [`hidden`] characters dropped, at most `max`
/// characters (the last one `…` when it was cut).
pub fn field(text: &str, max: usize) -> String {
    let kept: Vec<char> = text.chars().filter(|c| !hidden(*c)).collect();
    if kept.len() <= max {
        return kept.into_iter().collect();
    }
    let mut out: String = kept.into_iter().take(max.saturating_sub(1)).collect();
    if max > 0 {
        out.push('…');
    }
    out
}

/// A block the CLI writes: [`hidden`] characters dropped except the CLI's own `\n` (its parts that came from outside
/// were bounded as fields already).
pub fn block(text: &str) -> String {
    text.chars().filter(|c| *c == '\n' || !hidden(*c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_drop_every_control_and_invisible_character() {
        // ESC sequences (7-bit and the one-byte C1 CSI), CR, DEL, bidi overrides and isolates, zero-width characters.
        let forged = "C00 5.00 %\u{1b}[8m\r\u{9b}2J\u{7f}\u{202E}%08\u{202C}\u{200B}\u{2066}x\u{2069}\u{FEFF}\u{E0041}\n\tend";
        assert_eq!(field(forged, 100), "C00 5.00 %[8m2J%08xend");
        assert_eq!(field("Visitenkarte, Ébauche, 名刺", 100), "Visitenkarte, Ébauche, 名刺");
    }

    #[test]
    fn fields_are_bounded() {
        assert_eq!(field(&"x".repeat(40), 32).chars().count(), 32);
        assert!(field(&"x".repeat(40), 32).ends_with('…'));
        assert_eq!(field(&"x".repeat(32), 32), "x".repeat(32));
        assert_eq!(field("abc", 0), "");
        // Dropped characters do not count.
        assert_eq!(field("\u{1b}\u{1b}\u{1b}abc", 3), "abc");
    }

    #[test]
    fn blocks_keep_only_the_cli_line_breaks() {
        assert_eq!(block("a\u{1b}[8mb\nc\r\n\u{9b}d\u{202E}\n"), "a[8mb\nc\nd\n");
    }
}
