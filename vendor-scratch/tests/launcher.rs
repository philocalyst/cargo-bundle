//! The launcher, end to end: pack an AppDir behind the freshly built
//! `appimage-runtime` and execute the result.

#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use appimage::{AppDir, Packer, Runtime};
use tempfile::TempDir;

fn appimage_dir(temp: &TempDir, script: &str) -> std::path::PathBuf {
    let appdir = temp.path().join("Example App.AppDir");
    fs::create_dir_all(&appdir).expect("create AppDir");
    let app_run = appdir.join("AppRun");
    fs::write(&app_run, script).expect("write AppRun");
    let mut permissions = fs::metadata(&app_run).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&app_run, permissions).expect("make AppRun executable");
    appdir
}

fn pack(appdir: &std::path::Path, output: &std::path::Path) {
    let runtime =
        Runtime::try_from(fs::read(env!("CARGO_BIN_EXE_appimage-runtime")).expect("read runtime"))
            .expect("valid runtime");
    let appdir = AppDir::open(appdir).expect("open AppDir");
    Packer::new(&appdir, &runtime)
        .write_to(output)
        .expect("pack");
}

#[test]
fn launcher_preserves_args_environment_exit_code_and_cleans_extraction() {
    let temp = TempDir::new().unwrap();
    let appdir = appimage_dir(
        &temp,
        "#!/bin/sh\nprintf '%s\\n' \"$APPIMAGE\" \"$APPDIR\" \"$OWD\" \"$RUNTIME_TEST_VALUE\" \"$1\" \"$2\" > \"$RUNTIME_TEST_OUTPUT\"\nexit \"$RUNTIME_TEST_EXIT\"\n",
    );
    let image_path = temp.path().join("Example Image.AppImage");
    pack(&appdir, &image_path);
    let working_dir = temp.path().join("caller working directory");
    fs::create_dir(&working_dir).unwrap();
    let result_path = temp.path().join("runtime output.txt");

    let status = Command::new(&image_path)
        .args(["first argument has spaces", "second"])
        .current_dir(&working_dir)
        .env("RUNTIME_TEST_VALUE", "environment value has spaces")
        .env("RUNTIME_TEST_EXIT", "23")
        .env("RUNTIME_TEST_OUTPUT", &result_path)
        .status()
        .expect("execute AppImage");
    assert_eq!(status.code(), Some(23));

    let values: Vec<_> = fs::read_to_string(&result_path)
        .expect("AppRun wrote its observations")
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(values[0], image_path.to_string_lossy());
    assert_eq!(values[2], working_dir.to_string_lossy());
    assert_eq!(values[3], "environment value has spaces");
    assert_eq!(values[4], "first argument has spaces");
    assert_eq!(values[5], "second");
    let appdir = std::path::Path::new(&values[1]);
    assert!(appdir.is_absolute());
    assert_ne!(appdir, image_path.parent().unwrap());
    assert!(
        !appdir.exists(),
        "temporary extraction is removed after AppRun exits"
    );
}

#[test]
fn launcher_forwards_termination_signals_to_apprun() {
    let temp = TempDir::new().unwrap();
    let appdir = appimage_dir(
        &temp,
        "#!/bin/sh\ntrap 'echo delivered > \"$RUNTIME_TEST_OUTPUT\"; exit 42' TERM\necho ready > \"$RUNTIME_TEST_OUTPUT\"\nwhile :; do sleep 0.1; done\n",
    );
    let image_path = temp.path().join("Signal Image.AppImage");
    pack(&appdir, &image_path);
    let result_path = temp.path().join("signal.txt");
    let mut child = Command::new(&image_path)
        .env("RUNTIME_TEST_OUTPUT", &result_path)
        .spawn()
        .expect("start AppImage");

    wait_for_file_contents(&result_path, "ready", &mut child);
    // SAFETY: `child.id()` is the live PID returned by `Command::spawn`.
    let result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    assert_eq!(result, 0, "send SIGTERM to runtime process");
    let status = wait_for_exit(&mut child);
    assert_eq!(status.code(), Some(42));
    assert_eq!(fs::read_to_string(result_path).unwrap().trim(), "delivered");
}

#[cfg(target_os = "linux")]
#[test]
fn launcher_gives_apprun_foreground_terminal_access() {
    use std::ffi::CString;
    use std::os::fd::RawFd;

    let temp = TempDir::new().unwrap();
    let appdir = appimage_dir(
        &temp,
        "#!/bin/sh\necho READY\nread answer\necho GOT:$answer\n",
    );
    let image_path = CString::new(
        temp.path()
            .join("Terminal.AppImage")
            .as_os_str()
            .as_encoded_bytes(),
    )
    .unwrap();
    pack(&appdir, std::path::Path::new(image_path.to_str().unwrap()));

    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        0
    );
    let pid = unsafe { libc::fork() };
    assert!(pid >= 0);
    if pid == 0 {
        unsafe {
            libc::close(master);
            if libc::setsid() < 0 || libc::ioctl(slave, libc::TIOCSCTTY as _, 0) < 0 {
                libc::_exit(120);
            }
            for fd in [libc::STDIN_FILENO, libc::STDOUT_FILENO, libc::STDERR_FILENO] {
                if libc::dup2(slave, fd) < 0 {
                    libc::_exit(121);
                }
            }
            libc::close(slave);
            libc::execl(
                image_path.as_ptr(),
                image_path.as_ptr(),
                std::ptr::null::<libc::c_char>(),
            );
            libc::_exit(122);
        }
    }
    unsafe {
        libc::close(slave);
    }
    let mut output = Vec::new();
    let mut sent = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let mut pollfd = libc::pollfd {
            fd: master,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 100) };
        if ready > 0 {
            let mut buf = [0u8; 256];
            let count = unsafe { libc::read(master, buf.as_mut_ptr().cast(), buf.len()) };
            if count > 0 {
                output.extend_from_slice(&buf[..count as usize]);
            }
        }
        if !sent && output.windows(5).any(|w| w == b"READY") {
            assert_eq!(
                unsafe { libc::write(master, b"terminal input\n".as_ptr().cast(), 15) },
                15
            );
            sent = true;
        }
        if output.windows(18).any(|w| w == b"GOT:terminal input") {
            break;
        }
    }
    let received = output.windows(18).any(|w| w == b"GOT:terminal input");
    let mut status = 0;
    unsafe {
        if !received {
            libc::kill(pid, libc::SIGKILL);
        }
        libc::waitpid(pid, &mut status, 0);
        libc::close(master);
    }
    assert!(
        sent,
        "AppRun never reached its terminal read: {}",
        String::from_utf8_lossy(&output)
    );
    assert!(
        received,
        "terminal input was not received: {}",
        String::from_utf8_lossy(&output)
    );
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
}

#[cfg(target_os = "linux")]
#[test]
fn launcher_restores_terminal_across_foreground_job_stop_and_resume() {
    run_job_control_pty(false);
}

#[cfg(target_os = "linux")]
#[test]
fn launcher_does_not_steal_terminal_on_background_resume() {
    run_job_control_pty(true);
}

#[cfg(target_os = "linux")]
fn run_job_control_pty(background_resume: bool) {
    use std::ffi::CString;
    use std::os::fd::RawFd;

    let temp = TempDir::new().unwrap();
    let pidfile = temp.path().join("apprun.pid");
    let appdir = appimage_dir(
        &temp,
        "#!/bin/sh\ntrap 'echo RESUMED; exit 0' CONT\necho $$ > \"$RUNTIME_TEST_PIDFILE\"\necho READY\nwhile :; do :; done\n",
    );
    let output_path = temp.path().join("Job Control.AppImage");
    pack(&appdir, &output_path);
    let image_path = CString::new(output_path.as_os_str().as_encoded_bytes()).unwrap();
    let pidfile_c = CString::new(pidfile.as_os_str().as_encoded_bytes()).unwrap();

    let mut master: RawFd = -1;
    let mut slave: RawFd = -1;
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        },
        0
    );
    let harness = unsafe { libc::fork() };
    assert!(harness >= 0);
    if harness == 0 {
        unsafe {
            libc::close(master);
            if libc::setsid() < 0 || libc::ioctl(slave, libc::TIOCSCTTY as _, 0) < 0 {
                libc::_exit(120);
            }
            let runtime = libc::fork();
            if runtime < 0 {
                libc::_exit(121);
            }
            if runtime == 0 {
                libc::setpgid(0, 0);
                libc::dup2(slave, libc::STDIN_FILENO);
                libc::dup2(slave, libc::STDOUT_FILENO);
                libc::dup2(slave, libc::STDERR_FILENO);
                libc::setenv(c"RUNTIME_TEST_PIDFILE".as_ptr(), pidfile_c.as_ptr(), 1);
                libc::execl(
                    image_path.as_ptr(),
                    image_path.as_ptr(),
                    std::ptr::null::<libc::c_char>(),
                );
                libc::_exit(122);
            }
            libc::setpgid(runtime, runtime);
            if libc::tcsetpgrp(slave, runtime) < 0 {
                libc::_exit(123);
            }
            let mut status = 0;
            loop {
                let result = libc::waitpid(runtime, &mut status, libc::WUNTRACED);
                if result < 0 {
                    libc::_exit(124);
                }
                if libc::WIFSTOPPED(status) {
                    let mut set: libc::sigset_t = std::mem::zeroed();
                    libc::sigemptyset(&mut set);
                    libc::sigaddset(&mut set, libc::SIGTTOU);
                    libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
                    let foreground_group = if background_resume {
                        libc::getpgrp()
                    } else {
                        runtime
                    };
                    if libc::tcsetpgrp(slave, foreground_group) < 0 {
                        libc::_exit(125);
                    }
                    libc::sigprocmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
                    libc::kill(runtime, libc::SIGCONT);
                    continue;
                }
                break;
            }
            let mut set: libc::sigset_t = std::mem::zeroed();
            let foreground_group = libc::tcgetpgrp(slave);
            let expected_group = if background_resume {
                libc::getpgrp()
            } else {
                runtime
            };
            if foreground_group != expected_group {
                libc::_exit(127);
            }
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGTTOU);
            libc::sigprocmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
            libc::tcsetpgrp(slave, libc::getpgrp());
            if libc::WIFEXITED(status) {
                libc::_exit(libc::WEXITSTATUS(status));
            }
            libc::_exit(126);
        }
    }
    unsafe {
        libc::close(slave);
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut output = Vec::new();
    let mut sent_stop = false;
    while Instant::now() < deadline {
        let mut pollfd = libc::pollfd {
            fd: master,
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut pollfd, 1, 50) } > 0 {
            let mut buf = [0u8; 256];
            let count = unsafe { libc::read(master, buf.as_mut_ptr().cast(), buf.len()) };
            if count > 0 {
                output.extend_from_slice(&buf[..count as usize]);
            }
        }
        if !sent_stop && output.windows(5).any(|w| w == b"READY") {
            let app_pid = fs::read_to_string(&pidfile)
                .unwrap()
                .trim()
                .parse::<libc::pid_t>()
                .unwrap();
            assert_eq!(unsafe { libc::kill(-app_pid, libc::SIGSTOP) }, 0);
            sent_stop = true;
        }
        if output.windows(7).any(|w| w == b"RESUMED") {
            break;
        }
    }
    let resumed = output.windows(7).any(|w| w == b"RESUMED");
    let mut status = 0;
    unsafe {
        if !resumed {
            libc::kill(harness, libc::SIGKILL);
        }
        libc::waitpid(harness, &mut status, 0);
        libc::close(master);
    }
    assert!(sent_stop, "AppRun did not reach its ready point");
    assert!(
        resumed,
        "AppRun did not resume: {}",
        String::from_utf8_lossy(&output)
    );
    assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0);
}

fn wait_for_file_contents(path: &std::path::Path, expected: &str, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if fs::read_to_string(path).is_ok_and(|contents| contents.trim() == expected) {
            return;
        }
        if let Some(status) = child.try_wait().expect("poll AppImage") {
            panic!("AppImage exited before readiness marker: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for AppRun marker"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_for_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("poll AppImage exit") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out waiting for AppImage to exit after SIGTERM");
        }
        thread::sleep(Duration::from_millis(20));
    }
}
