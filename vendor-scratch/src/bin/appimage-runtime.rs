//! A standalone AppImage runtime. See [`appimage::launcher`].

fn main() -> std::process::ExitCode {
    appimage::launcher::main()
}
