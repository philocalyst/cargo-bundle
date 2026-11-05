//! Extract an AppImage, or just the paths matching a glob.
//!
//!     cargo run --example unpack -- Some.AppImage out
//!     cargo run --example unpack -- Some.AppImage out 'usr/share/*.desktop'

use appimage::AppImage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(image), Some(out)) = (args.next(), args.next()) else {
        return Err("usage: unpack <AppImage> <dir> [glob]".into());
    };

    let image = AppImage::open(image)?;
    let mut unpack = image.unpack();
    if let Some(glob) = args.next() {
        unpack = unpack.matching(glob);
    }
    let dir = unpack.to(out)?;
    println!("unpacked to {}", dir.display());
    Ok(())
}
