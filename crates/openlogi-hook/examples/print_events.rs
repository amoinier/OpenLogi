//! Manual smoke-test for the OS-level mouse hook.
//!
//! Prints every mouse event except pointer motion to stdout — including the
//! source device the backend attributed it to — and passes all events through
//! unchanged. Press Ctrl-C to stop.
//!
//! # Linux permissions
//!
//! Requires read access to `/dev/input/eventN` and write access to
//! `/dev/uinput`. Add your user to the `input` group and apply a udev rule:
//!
//! ```sh
//! sudo usermod -aG input $USER
//! echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' \
//!     | sudo tee /etc/udev/rules.d/99-uinput.rules
//! sudo udevadm trigger /dev/uinput
//! # log out and back in, then:
//! cargo run --example print_events -p openlogi-hook
//! ```
//!
//! # macOS permissions
//!
//! The terminal running the example needs Accessibility (System Settings →
//! Privacy & Security → Accessibility). The first run asks for it.

// A crate-level `#![cfg(...)]` would leave an empty crate with no `main` on
// other targets (E0601), breaking `cargo build --all-targets` there — so gate
// the body on `main` instead and provide a trivial fallback.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn main() {
    use openlogi_hook::{EventDisposition, Hook, HookEvent, MouseEvent};

    let hook = match Hook::start(|event| {
        if !matches!(event, HookEvent::Mouse(MouseEvent::Moved { .. })) {
            println!("{event:?}");
        }
        EventDisposition::PassThrough
    }) {
        Ok(h) => h,
        Err(e) => {
            // macOS: list the terminal under Accessibility; no-op elsewhere.
            Hook::prompt_accessibility();
            eprintln!("error: failed to start hook: {e}");
            std::process::exit(1);
        }
    };

    println!("Hook running — click buttons or scroll. Press Ctrl-C to stop.");
    wait_for_ctrl_c();
    hook.stop();
    println!("Hook stopped.");
}

/// Block until Ctrl-C.
#[cfg(target_os = "linux")]
fn wait_for_ctrl_c() {
    let (tx, rx) = std::sync::mpsc::channel();
    #[expect(
        clippy::expect_used,
        reason = "example binary; aborting on a failed handler install is fine"
    )]
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })
    .expect("failed to set Ctrl-C handler");
    rx.recv().ok();
}

/// Block until Ctrl-C. The default SIGINT disposition ends the process, and
/// macOS destroys the process-owned event tap with it.
#[cfg(target_os = "macos")]
fn wait_for_ctrl_c() {
    loop {
        std::thread::park();
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn main() {
    eprintln!("print_events runs on Linux and macOS only (no-op on this platform).");
}
