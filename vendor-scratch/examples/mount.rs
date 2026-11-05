//! Mount an AppImage and wait. Linux only.
//!
//!     cargo run --example mount -- Some.AppImage

use std::io::Read;

use appimage::AppImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: mount <AppImage>")?;
    let mount = AppImage::open(path)?.mount()?;

    println!("mounted at {}; press enter to unmount", mount.display());
    std::io::stdin().read_exact(&mut [0])?;
    Ok(())
}
