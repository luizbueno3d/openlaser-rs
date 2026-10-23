//! The real terminal, and the only file that may build a TTY confirmation (SPEC §2.3 rule 11):
//! `HumanConfirm::from_tty` and `TtyWitness::new` are clippy disallowed-methods everywhere else, and are called
//! only after `IsTerminal` holds on stdin and stdout, around a line read from that terminal in this file.
//!
//! Also the crossterm key thread (SPEC §5.4): while a machine frames or marks, the terminal is in raw mode and a
//! thread reads single keys; Esc, Space and Ctrl-C trip every latch right there, Enter goes to the main loop. In raw
//! mode Ctrl-C is a key, not SIGINT, so this thread is what stops on it.
#![allow(clippy::disallowed_methods)]

use std::io::{BufRead, IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::thread::JoinHandle;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use openlaser_arm::{HumanConfirm, TtyWitness};
use openlaser_types::MonoTime;

use crate::{KeyPoll, KeySession, StopFn, StopKey, Terminal};

/// How long the key thread blocks in one `poll` before it looks at its stop flag.
const KEY_POLL: Duration = Duration::from_millis(10);

/// The exact line the operator types for `challenge` (6 digits, as printed).
pub fn arm_line(challenge: u64) -> String {
    format!("ARM {challenge:06}")
}

/// The typed line confirms `challenge` (surrounding whitespace ignored; nothing else is).
pub fn arm_line_matches(typed: &str, challenge: u64) -> bool {
    typed.trim() == arm_line(challenge)
}

/// What one key means while a machine is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Stop(StopKey),
    Enter,
}

/// Esc, Space and Ctrl-C stop; Enter accepts; everything else (and key releases/repeats) is ignored.
pub fn classify(k: &KeyEvent) -> Option<KeyAction> {
    if k.kind == KeyEventKind::Release {
        return None;
    }
    match k.code {
        KeyCode::Esc => Some(KeyAction::Stop(StopKey::Esc)),
        KeyCode::Char(' ') => Some(KeyAction::Stop(StopKey::Space)),
        KeyCode::Char('c' | 'C') if k.modifiers.contains(KeyModifiers::CONTROL) => Some(KeyAction::Stop(StopKey::CtrlC)),
        KeyCode::Enter if k.kind == KeyEventKind::Press => Some(KeyAction::Enter),
        _ => None,
    }
}

/// stdin/stdout of a real terminal.
#[derive(Debug, Default)]
pub struct TtyTerminal {
    /// Raw mode is on (a key session is live): lines need `\r\n`.
    raw: Arc<AtomicBool>,
}

impl Terminal for TtyTerminal {
    fn is_tty(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
    }

    fn read_line(&mut self) -> Option<String> {
        if self.raw.load(Ordering::SeqCst) {
            return None;
        }
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_owned()),
        }
    }

    fn write(&mut self, text: &str) {
        let mut out = std::io::stdout().lock();
        let text = if self.raw.load(Ordering::SeqCst) { text.replace('\n', "\r\n") } else { text.to_owned() };
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    fn confirm_arm(&mut self, challenge: u64, at: MonoTime) -> Option<HumanConfirm> {
        // A line can only come from a person at this terminal: both ends are TTYs and no key session is reading.
        if !self.is_tty() || self.raw.load(Ordering::SeqCst) {
            return None;
        }
        self.write(&format!("Type  {}  and press Enter to start the laser (anything else cancels): ", arm_line(challenge)));
        let line = self.read_line()?;
        if !self.is_tty() || !arm_line_matches(&line, challenge) {
            return None;
        }
        Some(HumanConfirm::from_tty(TtyWitness::new(), line.trim(), challenge, at))
    }

    fn keys(&mut self, stop: StopFn) -> Result<Box<dyn KeySession>, String> {
        if !self.is_tty() {
            return Err("stdin and stdout are not a terminal".to_owned());
        }
        crossterm::terminal::enable_raw_mode().map_err(|e| format!("cannot switch the terminal to raw mode: {e}"))?;
        self.raw.store(true, Ordering::SeqCst);
        let done = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let d = Arc::clone(&done);
        let spawned = std::thread::Builder::new().name("openlaser-hw-keys".to_owned()).spawn(move || {
            while !d.load(Ordering::SeqCst) {
                match crossterm::event::poll(KEY_POLL) {
                    Ok(false) => {}
                    Ok(true) => match crossterm::event::read() {
                        Ok(Event::Key(k)) => match classify(&k) {
                            Some(KeyAction::Stop(s)) => stop(s),
                            Some(KeyAction::Enter) => {
                                let _ = tx.send(KeyPoll::Enter);
                            }
                            None => {}
                        },
                        Ok(_) => {}
                        Err(_) => {
                            stop(StopKey::InputLost);
                            let _ = tx.send(KeyPoll::Closed);
                            return;
                        }
                    },
                    Err(_) => {
                        stop(StopKey::InputLost);
                        let _ = tx.send(KeyPoll::Closed);
                        return;
                    }
                }
            }
        });
        match spawned {
            Ok(handle) => Ok(Box::new(RawKeys { done, handle: Some(handle), rx, raw: Arc::clone(&self.raw) })),
            Err(e) => {
                let _ = crossterm::terminal::disable_raw_mode();
                self.raw.store(false, Ordering::SeqCst);
                Err(format!("cannot start the key thread: {e}"))
            }
        }
    }
}

/// A live key session; dropping it stops the thread and restores the terminal.
struct RawKeys {
    done: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    rx: Receiver<KeyPoll>,
    raw: Arc<AtomicBool>,
}

impl KeySession for RawKeys {
    fn poll(&mut self) -> KeyPoll {
        match self.rx.try_recv() {
            Ok(k) => k,
            Err(TryRecvError::Empty) => KeyPoll::Nothing,
            // The thread ended without saying why: treat it as a lost input (the flow stops the machine).
            Err(TryRecvError::Disconnected) => KeyPoll::Closed,
        }
    }
}

impl Drop for RawKeys {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let _ = crossterm::terminal::disable_raw_mode();
        self.raw.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEventState;

    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> KeyEvent {
        KeyEvent { code, modifiers, kind, state: KeyEventState::NONE }
    }

    #[test]
    fn cli_space_and_esc_stop() {
        let press = KeyEventKind::Press;
        assert_eq!(classify(&key(KeyCode::Esc, KeyModifiers::NONE, press)), Some(KeyAction::Stop(StopKey::Esc)));
        assert_eq!(classify(&key(KeyCode::Char(' '), KeyModifiers::NONE, press)), Some(KeyAction::Stop(StopKey::Space)));
        assert_eq!(classify(&key(KeyCode::Char('c'), KeyModifiers::CONTROL, press)), Some(KeyAction::Stop(StopKey::CtrlC)));
        // A held key repeats: still a stop. A release is not a second key.
        assert_eq!(classify(&key(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Repeat)), Some(KeyAction::Stop(StopKey::Esc)));
        assert_eq!(classify(&key(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release)), None);
        assert_eq!(classify(&key(KeyCode::Enter, KeyModifiers::NONE, press)), Some(KeyAction::Enter));
        assert_eq!(classify(&key(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Repeat)), None, "a held Enter is one acceptance");
        assert_eq!(classify(&key(KeyCode::Char('c'), KeyModifiers::NONE, press)), None);
        assert_eq!(classify(&key(KeyCode::Char('a'), KeyModifiers::NONE, press)), None);
    }

    #[test]
    fn the_arm_line_is_exact() {
        assert!(arm_line_matches("ARM 123456", 123_456));
        assert!(arm_line_matches("  ARM 004201 \r", 4201));
        assert!(!arm_line_matches("ARM 4201", 4201), "printed as 6 digits, typed as 6 digits");
        assert!(!arm_line_matches("arm 123456", 123_456));
        assert!(!arm_line_matches("ARM 123457", 123_456));
        assert!(!arm_line_matches("ARM  123456", 123_456));
        assert!(!arm_line_matches("ARM", 123_456));
        assert!(!arm_line_matches("ARM 123456 now", 123_456));
    }

    #[test]
    fn a_pipe_is_not_a_terminal() {
        // Under `cargo test` stdout is captured, so this terminal is never a TTY: no witness can be built here.
        let mut t = TtyTerminal::default();
        if !t.is_tty() {
            assert!(t.confirm_arm(123_456, MonoTime::default()).is_none());
            assert!(t.keys(Arc::new(|_| {})).is_err());
        }
    }
}
