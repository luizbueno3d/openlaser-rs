//! `openlaser-hw ports [--watch]` (H11, SPEC M3): the read-only input-port probe. It asks the board `ReadPort`
//! (0x0025) and prints every bit of the four reply words; `--watch` keeps asking and prints each bit that changes, so a
//! person at the machine can press and release the foot switch (or open the door) and see which input it is, and at
//! which level it rests. The map is not assumed: research names `foot_pin = 15`, the probe decides.
//!
//! Safety shape: ReadPort only, on a freshly opened transport (no init, no Reset, no `WritePort`, nothing that moves
//! or fires). The real board needs a `hw` build, `OPENLASER_HW_PRESENT=1` and a bench session, as `status` does.
//! `--watch` needs a TTY (it reads Esc, Space or Ctrl-C, which end it; so does the key reader closing).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::args::PortsOpts;
use crate::rig::{Purpose, Rig, RigSpec};
use crate::{Env, HwError, KeyPoll, StopFn, Terminal, require_tty};

/// Reply word 3 is the board status (READY 0x20, BUSY 0x04), not an input; it is shown, and its changes are listed
/// apart from the others.
const STATUS_WORD: usize = 3;

pub fn run(o: &PortsOpts, term: &mut dyn Terminal, env: &mut Env) -> Result<(), HwError> {
    if o.watch {
        require_tty(term, "ports --watch")?;
    }
    let mut rig = env.rigs.open(&RigSpec { purpose: Purpose::Ports, device: o.device.clone(), extent: None })?;
    let r = probe(rig.as_mut(), o.watch, term, env);
    rig.disconnect();
    r
}

/// `0000 1000 0000 0001`: bit 15 on the left.
pub fn bits(w: u16) -> String {
    let b = format!("{w:016b}");
    let (a, rest) = b.split_at(4);
    let (c, rest) = rest.split_at(4);
    let (d, e) = rest.split_at(4);
    format!("{a} {c} {d} {e}")
}

fn set_bits(w: u16) -> String {
    let on: Vec<String> = (0..16u8).filter(|b| w >> b & 1 == 1).map(|b| b.to_string()).collect();
    if on.is_empty() { "none".to_owned() } else { on.join(", ") }
}

fn word_name(i: usize) -> String {
    if i == STATUS_WORD { format!("word {i} (status)") } else { format!("word {i}") }
}

fn print_reply(term: &mut dyn Terminal, w: [u16; 4]) {
    term.write(&format!("ReadPort (0x0025) reply: {:04x} {:04x} {:04x} {:04x}   (bits 15..0, left to right)\n", w[0], w[1], w[2], w[3]));
    for (i, v) in w.iter().enumerate() {
        term.write(&format!("  {:<17} {}   set: {}\n", word_name(i), bits(*v), set_bits(*v)));
    }
}

/// One bit that changed while watching.
struct Seen {
    word: usize,
    bit: u8,
    /// Its level when the watch started (the switch at rest).
    rest: bool,
    changes: u32,
}

fn probe(rig: &mut dyn Rig, watch: bool, term: &mut dyn Terminal, env: &Env) -> Result<(), HwError> {
    let first = rig.read_port()?;
    print_reply(term, first);
    if !watch {
        return Ok(());
    }
    term.write("Watching the input port. Press and release the foot switch a few times (hold each press about a second).\nEsc, Space or Ctrl-C ends.\n");
    let ended = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&ended);
    let stop: StopFn = Arc::new(move |_| flag.store(true, Ordering::SeqCst));
    let mut keys = term.keys(stop).map_err(HwError::Failed)?;
    let start = rig.now();
    let mut last = first;
    let mut seen: Vec<Seen> = Vec::new();
    let mut reads: u64 = 1;
    let result = loop {
        // A stop key trips `ended` on the key thread; poll first, so one pressed during this poll counts too.
        let closed = keys.poll() == KeyPoll::Closed;
        if closed || ended.load(Ordering::SeqCst) {
            break Ok(());
        }
        let now = match rig.read_port() {
            Ok(w) => w,
            Err(e) => break Err(e),
        };
        reads += 1;
        let at = rig.now().saturating_since(start);
        for (i, (old, new)) in last.iter().zip(now.iter()).enumerate() {
            let diff = old ^ new;
            for bit in (0..16u8).filter(|b| diff >> b & 1 == 1) {
                let level = new >> bit & 1 == 1;
                term.write(&format!("  +{:>8.3} s  {} bit {bit:>2}: {} -> {}\n", at.as_secs_f64(), word_name(i), u8::from(!level), u8::from(level)));
                match seen.iter_mut().find(|s| s.word == i && s.bit == bit) {
                    Some(s) => s.changes += 1,
                    None => seen.push(Seen { word: i, bit, rest: !level, changes: 1 }),
                }
            }
        }
        last = now;
        env.clock.sleep(env.poll);
    };
    drop(keys);
    let took = rig.now().saturating_since(start);
    term.write(&format!("Stopped watching after {} reads in {:.1} s.\n", reads, took.as_secs_f64()));
    print_reply(term, last);
    summarize(term, &seen, env.poll);
    result
}

fn summarize(term: &mut dyn Terminal, seen: &[Seen], poll: Duration) {
    let inputs: Vec<&Seen> = seen.iter().filter(|s| s.word != STATUS_WORD).collect();
    if inputs.is_empty() {
        term.write("No input bit changed. Check that the foot switch is plugged into the board's input connector, and press it while watching.\n");
    } else {
        term.write("Input bits that changed:\n");
        for s in &inputs {
            let kind = if s.rest { "rests at 1, so pressing pulls it to 0 (active low)" } else { "rests at 0, so pressing sets it to 1 (active high)" };
            term.write(&format!("  word {} bit {:>2}: {} change(s); {kind}\n", s.word, s.bit, s.changes));
        }
        term.write(&format!(
            "Changes shorter than {} ms between two reads can be missed; report the word, bit and level to the developers.\n",
            poll.as_millis()
        ));
    }
    let status: Vec<&Seen> = seen.iter().filter(|s| s.word == STATUS_WORD).collect();
    for s in status {
        term.write(&format!("  (status word bit {} changed {} time(s): board state, not an input)\n", s.bit, s.changes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_are_grouped_high_bit_first() {
        assert_eq!(bits(0x8001), "1000 0000 0000 0001");
        assert_eq!(set_bits(0x8001), "0, 15");
        assert_eq!(set_bits(0), "none");
    }
}
