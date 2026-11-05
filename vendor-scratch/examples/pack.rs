//! Pack an AppDir into an AppImage.
//!
//! The runtime is whatever executable you give it. To build one:
//!
//!     cargo build --profile runtime --features launcher --bin appimage-runtime
//!     cargo run --example pack -- Hello.AppDir Hello.AppImage target/runtime/appimage-runtime
//!
//! Leave the last argument off and the AppImage is packed behind a bare ELF
//! header: it can be inspected and unpacked, but does nothing when run.

use appimage::{AppDir, Packer, Runtime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(app_dir), Some(output)) = (args.next(), args.next()) else {
        return Err("usage: pack <AppDir> <output> [runtime]".into());
    };
    let runtime = match args.next() {
        Some(path) => Runtime::try_from(std::fs::read(path)?)?,
        None => Runtime::inert(),
    };

    let image = Packer::new(&AppDir::open(app_dir)?, &runtime).write_to(output)?;
    println!(
        "wrote {} (payload at byte {})",
        image.path.display(),
        image.offset
    );
    Ok(())
}
