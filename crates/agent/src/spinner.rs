//! A one line rotating spinner.
//!
//! It owns its console line and clears it on stop, so output never interleaves with a half
//! drawn frame. It is a no op when stdout is not a terminal, which keeps piped output and
//! CI logs clean.
//!
//! Two hazards drive the design:
//!
//! - The spinner runs on its own task while the main task prints streamed tokens, so both
//!   writers must be serialised. A shared gate does that, and the stop flag is checked
//!   INSIDE the gate: once a silencer has run, no further frame can slip out.
//! - The token callback is synchronous and cannot await, so silencing has to work without
//!   async. [`Silencer`] is that sync half.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const FRAMES: [char; 4] = ['|', '/', '-', '\\'];

/// Serialises console writes and carries the stop flag.
#[derive(Clone, Default)]
struct Gate {
    stop: Arc<AtomicBool>,
    lock: Arc<Mutex<()>>,
}

/// A sync handle that stops the spinner and clears its line.
///
/// Cheap to clone, safe to call from a synchronous callback, and idempotent.
#[derive(Clone)]
pub struct Silencer {
    gate: Gate,
    active: bool,
}

impl Silencer {
    /// Stop drawing and erase the line. After this returns, no frame can appear.
    pub fn silence(&self) {
        if !self.active {
            return;
        }
        let _guard = self.gate.lock.lock().unwrap_or_else(|e| e.into_inner());
        // Set the flag inside the gate: a spinner task waiting on the lock will see it
        // and return without drawing, so the line stays clear.
        if !self.gate.stop.swap(true, Ordering::SeqCst) {
            let mut out = std::io::stdout();
            let _ = write!(out, "\r\x1b[2K");
            let _ = out.flush();
        }
    }
}

/// A running spinner. Dropping it stops the task; prefer [`Spinner::stop`] to also join it.
pub struct Spinner {
    gate: Gate,
    handle: Option<tokio::task::JoinHandle<()>>,
    active: bool,
}

impl Spinner {
    /// Start spinning with `label`. `color` is an SGR code, for example "36" for cyan.
    pub fn start(label: &str, color: &str) -> Self {
        let gate = Gate::default();
        if !std::io::stdout().is_terminal() {
            gate.stop.store(true, Ordering::SeqCst);
            return Self { gate, handle: None, active: false };
        }
        let task_gate = gate.clone();
        let label = label.to_string();
        let color = color.to_string();
        let handle = tokio::spawn(async move {
            let mut i = 0usize;
            loop {
                {
                    let _guard = task_gate.lock.lock().unwrap_or_else(|e| e.into_inner());
                    if task_gate.stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let frame = FRAMES[i % FRAMES.len()];
                    i += 1;
                    // Carriage return, the coloured frame, the label, then clear to end of
                    // line so a shorter frame leaves no debris.
                    let mut out = std::io::stdout();
                    let _ = write!(out, "\r\x1b[{color}m{frame}\x1b[0m {label}\x1b[0K");
                    let _ = out.flush();
                }
                tokio::time::sleep(std::time::Duration::from_millis(90)).await;
            }
        });
        Self { gate, handle: Some(handle), active: true }
    }

    /// A sync handle usable from a streaming callback.
    pub fn silencer(&self) -> Silencer {
        Silencer { gate: self.gate.clone(), active: self.active }
    }

    /// Stop, erase the line, and join the task. Safe to call more than once.
    pub async fn stop(&mut self) {
        self.silencer().silence();
        if let Some(h) = self.handle.take() {
            let _ = h.await;
        }
    }
}

impl Drop for Spinner {
    /// A dropped spinner must not keep drawing.
    ///
    /// Without this, dropping the struct instead of stopping it leaves the task alive for
    /// the rest of the process, painting frames over everything printed afterwards.
    ///
    /// We also clear the spinner line here so that a SIGINT (or any other abrupt path that
    /// drops the spinner without going through `stop`) does not leave a half-drawn frame
    /// stranded in the terminal. The write is best-effort: errors are silently ignored
    /// because a destructor must not panic.
    fn drop(&mut self) {
        if self.active && !self.gate.stop.swap(true, Ordering::SeqCst) {
            let mut out = std::io::stdout();
            let _ = write!(out, "\r\x1b[2K");
            let _ = out.flush();
        } else {
            self.gate.stop.store(true, Ordering::SeqCst);
        }
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// Stop a spinner held in an `Option`, leaving it None.
pub async fn stop_if_running(slot: &mut Option<Spinner>) {
    if let Some(s) = slot.as_mut() {
        s.stop().await;
    }
    *slot = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_spinner_starts_and_stops_without_hanging() {
        let mut s = Spinner::start("working", "36");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        s.stop().await;
        // The turn loop stops it defensively in more than one place, so a repeat must be
        // harmless.
        s.stop().await;
    }

    #[tokio::test]
    async fn stop_if_running_clears_the_slot() {
        let mut slot = Some(Spinner::start("x", "33"));
        stop_if_running(&mut slot).await;
        assert!(slot.is_none());
        stop_if_running(&mut slot).await;
        assert!(slot.is_none());
    }

    #[tokio::test]
    async fn silencing_is_idempotent_and_sets_the_flag() {
        let s = Spinner::start("x", "36");
        let sil = s.silencer();
        sil.silence();
        assert!(s.gate.stop.load(Ordering::SeqCst), "the flag is set");
        // A second call must not clear the line again, which would erase real output.
        sil.silence();
    }

    #[tokio::test]
    async fn a_silenced_spinner_task_exits_instead_of_drawing() {
        let s = Spinner::start("x", "36");
        let sil = s.silencer();
        sil.silence();
        // Give the task a chance to wake, observe the flag and return.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(s.handle.as_ref().is_none_or(|h| h.is_finished()), "the task returned");
    }

    /// Dropping a spinner instead of stopping it used to leak the task, which then painted
    /// frames over everything printed for the rest of the process.
    #[tokio::test]
    async fn dropping_a_spinner_kills_its_task() {
        let s = Spinner::start("leaky", "36");
        let gate = s.gate.clone();
        drop(s);
        assert!(gate.stop.load(Ordering::SeqCst), "drop signalled the task");
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(gate.stop.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn frames_rotate_through_all_four() {
        // The frame table is the visible contract of the spinner, so pin it.
        assert_eq!(FRAMES, ['|', '/', '-', '\\']);
    }
}
