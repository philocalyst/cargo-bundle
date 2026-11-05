//! Running `AppRun` as a well-behaved child: its own process group, the
//! terminal while it runs, signals passed along, and job control mirrored.

use std::io::{self, IsTerminal};
use std::os::fd::RawFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus};
use std::thread::{self, JoinHandle};

use libc::{c_int, pid_t};
use signal_hook::consts::signal::{SIGCONT, SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGTSTP, SIGWINCH};
use signal_hook::iterator::Signals;

/// The signals the launcher catches and hands to the app instead of dying of.
const FORWARDED: [c_int; 7] = [SIGINT, SIGTERM, SIGHUP, SIGQUIT, SIGWINCH, SIGTSTP, SIGCONT];

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("cannot forward signals: {0}")]
    Signals(io::Error),
    #[error("cannot start {}: {source}", program.display())]
    Spawn { program: PathBuf, source: io::Error },
    #[error("cannot wait for the app: {0}")]
    Wait(io::Error),
    #[error("the signal forwarding thread panicked")]
    Forwarder,
}

/// Run `command` to completion and report how it ended.
pub(super) fn run(command: &mut Command) -> Result<ExitStatus, Error> {
    let tty = Tty::attach();

    // The app and everything it starts live in one group of their own, so a
    // signal forwarded to the group is not also delivered to us.
    command.process_group(0);
    if let Some(tty) = tty {
        // SAFETY: between fork and exec, this calls only `getpgrp`,
        // `sigprocmask` and `tcsetpgrp`, which are async-signal-safe, and
        // allocates nothing.
        unsafe { command.pre_exec(move || tty.give_to(libc::getpgrp())) };
    }

    // Registered before the spawn: signals that arrive while the app starts
    // are queued, not fatal to us while the payload still needs cleaning up.
    let signals = Signals::new(FORWARDED).map_err(Error::Signals)?;
    let handle = signals.handle();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(source) => {
            handle.close();
            if let Some(tty) = tty {
                tty.take_back();
            }
            return Err(Error::Spawn {
                program: command.get_program().into(),
                source,
            });
        }
    };
    let group = child.id() as pid_t;
    let lease = tty.map(|tty| Lease { tty, group });

    let forwarder = match spawn_forwarder(signals, group, tty) {
        Ok(thread) => thread,
        Err(error) => {
            handle.close();
            kill_group(group, SIGTERM);
            let _ = child.wait();
            return Err(Error::Signals(error));
        }
    };
    let status = wait(&child, lease.as_ref());
    // Closing wakes the forwarder; joining guarantees no signal is in flight
    // once the payload is deleted.
    handle.close();
    forwarder.join().map_err(|_| Error::Forwarder)?;
    status.map_err(Error::Wait)
}

/// The controlling terminal, when we are in its foreground and so may hand it
/// to the app.
#[derive(Clone, Copy)]
struct Tty {
    fd: RawFd,
    /// Our own process group, to hand the terminal back to.
    home: pid_t,
}

impl Tty {
    fn attach() -> Option<Self> {
        let fd = libc::STDIN_FILENO;
        // SAFETY: `getpgrp` cannot fail and has no preconditions.
        let home = unsafe { libc::getpgrp() };
        // `tcsetpgrp` only works inside the terminal's own session; from
        // anywhere else keep the plain, non-interactive behavior.
        let tty = Self { fd, home };
        (io::stdin().is_terminal() && tty.foreground() == Some(home)).then_some(tty)
    }

    /// The process group that owns the terminal right now.
    fn foreground(self) -> Option<pid_t> {
        // SAFETY: plain syscall on a descriptor; failure is reported by -1.
        let group = unsafe { libc::tcgetpgrp(self.fd) };
        (group > 0).then_some(group)
    }

    /// Make `group` the owner of the terminal.
    ///
    /// `tcsetpgrp` raises `SIGTTOU` in a background group, which would stop
    /// us, so that signal is blocked just for the call.
    fn give_to(self, group: pid_t) -> io::Result<()> {
        // SAFETY: `set` and `previous` are initialized by the libc calls that
        // fill them before they are read, and the mask is put back.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            let mut previous: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTTOU);
            if libc::sigprocmask(libc::SIG_BLOCK, &set, &mut previous) != 0 {
                return Err(io::Error::last_os_error());
            }
            let result = libc::tcsetpgrp(self.fd, group);
            let error = (result != 0).then(io::Error::last_os_error);
            libc::sigprocmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
            error.map_or(Ok(()), Err)
        }
    }

    fn take_back(self) {
        let _ = self.give_to(self.home);
    }
}

/// The app's hold on the terminal. It gives the terminal back when dropped, or
/// earlier by [`Lease::release`], but only if the app still has it.
struct Lease {
    tty: Tty,
    group: pid_t,
}

impl Lease {
    fn release(&self) {
        if self.tty.foreground() == Some(self.group) {
            self.tty.take_back();
        }
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.release();
    }
}

fn kill_group(group: pid_t, signal: c_int) {
    // SAFETY: a plain syscall. ESRCH just means the group has already exited.
    let _ = unsafe { libc::kill(-group, signal) };
}

fn spawn_forwarder(
    mut signals: Signals,
    group: pid_t,
    tty: Option<Tty>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("appimage-signals".into())
        .spawn(move || {
            for signal in signals.forever() {
                // Resumed from the background by a shell's `bg`, we must not take
                // the terminal; resumed by `fg`, the shell has already given it to
                // us and the app should have it.
                if let Some(tty) =
                    tty.filter(|tty| signal == SIGCONT && tty.foreground() == Some(tty.home))
                {
                    let _ = tty.give_to(group);
                }
                kill_group(group, signal);
            }
        })
}

enum Event {
    Finished(ExitStatus),
    Stopped,
    Continued,
}

fn next_event(pid: pid_t) -> io::Result<Event> {
    loop {
        let mut status = 0;
        // SAFETY: `status` is a valid out-pointer for the call's duration.
        let reaped = unsafe { libc::waitpid(pid, &mut status, libc::WUNTRACED | libc::WCONTINUED) };
        if reaped < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        return Ok(if libc::WIFSTOPPED(status) {
            Event::Stopped
        } else if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
            Event::Finished(ExitStatus::from_raw(status))
        } else {
            Event::Continued
        });
    }
}

/// Wait for the app to end, and while it lives, mirror its job control: when it
/// is stopped, return the terminal and stop ourselves too, so the shell sees
/// one stopped job.
fn wait(child: &Child, lease: Option<&Lease>) -> io::Result<ExitStatus> {
    loop {
        match next_event(child.id() as pid_t)? {
            Event::Finished(status) => {
                lease.inspect(|lease| lease.release());
                return Ok(status);
            }
            Event::Stopped => {
                lease.inspect(|lease| lease.release());
                // SIGSTOP, unlike SIGTSTP, is not one of the signals we have
                // caught. The shell resumes us with SIGCONT, which the
                // forwarder turns into the terminal handover and a SIGCONT to
                // the app's group.
                // SAFETY: a plain syscall.
                unsafe { libc::raise(libc::SIGSTOP) };
            }
            Event::Continued => {}
        }
    }
}

/// Signals are process-wide, so tests that catch or raise them take turns.
#[cfg(test)]
pub(super) static SIGNAL_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn termination_is_forwarded_to_the_whole_group() {
        let _turn = SIGNAL_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let signals = Signals::new(FORWARDED).unwrap();
        let handle = signals.handle();
        let mut child = Command::new("sh")
            .args(["-c", "sleep 10 & wait"])
            .process_group(0)
            .spawn()
            .unwrap();
        let forwarder = spawn_forwarder(signals, child.id() as pid_t, None).unwrap();

        signal_hook::low_level::raise(SIGTERM).unwrap();
        let status = child.wait().unwrap();
        handle.close();
        forwarder.join().unwrap();
        assert_eq!(status.signal(), Some(SIGTERM));
    }
}
