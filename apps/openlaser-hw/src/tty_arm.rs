//! The real terminal, and the only file that may build a TTY confirmation (SPEC §2.3 rule 11):
//! `HumanConfirm::from_tty` and `TtyWitness::new` are clippy disallowed-methods everywhere else, and are called
//! only after `IsTerminal` holds on stdin and stdout.
//!
//! WP0 stub: `confirm_arm` refuses (returns None).
#![allow(clippy::disallowed_methods)]

use std::io::{BufRead, IsTerminal, Write};

use openlaser_arm::HumanConfirm;
use openlaser_types::MonoTime;

use crate::Terminal;

/// stdin/stdout of a real terminal.
#[derive(Debug, Default)]
pub struct TtyTerminal;

impl Terminal for TtyTerminal {
    fn is_tty(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_owned()),
        }
    }

    fn write(&mut self, text: &str) {
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn confirm_arm(&mut self, challenge: u64, at: MonoTime) -> Option<HumanConfirm> {
        let _ = (challenge, at);
        None
    }
}
