//! Keep the computer from idle-sleeping while sharing (issue #8), so the phone
//! can still reach it. Idle sleep only: closing a laptop lid still sleeps it.
//!
//! macOS: a `caffeinate -i` child, tied to our pid so it can't outlive us.
//! Windows: `SetThreadExecutionState` held by a dedicated thread — the request
//! lasts until that thread clears it or exits.

use std::sync::Mutex;

#[derive(Default)]
pub struct KeepAwake {
    inner: Mutex<Option<Hold>>,
}

impl KeepAwake {
    /// Turn the hold on or off; idempotent.
    pub fn set(&self, on: bool) {
        let mut inner = self.inner.lock().unwrap();
        match (on, inner.is_some()) {
            (true, false) => *inner = Hold::start(),
            (false, true) => {
                if let Some(h) = inner.take() {
                    h.stop();
                }
            }
            _ => {}
        }
    }
}

#[cfg(target_os = "macos")]
struct Hold(std::process::Child);

#[cfg(target_os = "macos")]
impl Hold {
    fn start() -> Option<Self> {
        std::process::Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &std::process::id().to_string()])
            .spawn()
            .ok()
            .map(Hold)
    }

    fn stop(mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(windows)]
struct Hold(std::sync::mpsc::Sender<()>);

#[cfg(windows)]
impl Hold {
    fn start() -> Option<Self> {
        use windows_sys::Win32::System::Power::{
            SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED,
        };
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            // Block until stopped (or the sender is dropped), then release.
            let _ = rx.recv();
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        });
        Some(Hold(tx))
    }

    fn stop(self) {
        let _ = self.0.send(());
    }
}

#[cfg(all(not(windows), not(target_os = "macos")))]
struct Hold;

#[cfg(all(not(windows), not(target_os = "macos")))]
impl Hold {
    fn start() -> Option<Self> {
        None
    }

    fn stop(self) {}
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn hold_starts_and_releases() {
        let k = KeepAwake::default();
        k.set(true);
        k.set(true); // idempotent: still one hold
        let pid = k.inner.lock().unwrap().as_ref().map(|h| h.0.id());
        assert!(pid.is_some(), "caffeinate should be running");
        k.set(false);
        assert!(k.inner.lock().unwrap().is_none());
    }
}
