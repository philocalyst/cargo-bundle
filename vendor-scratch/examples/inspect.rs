//! Print what an AppImage says about itself.
//!
//!     cargo run --example inspect -- Some.AppImage

use appimage::{AppImage, section};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: inspect <AppImage>")?;
    let image = AppImage::open(path)?;

    println!("payload starts at byte {}", image.offset);
    match image.update_information()? {
        Some(info) => println!("updates: {info}"),
        None => println!("updates: none"),
    }
    println!("signed: {}", image.section(section::SIGNATURE)?.is_some());
    Ok(())
}
