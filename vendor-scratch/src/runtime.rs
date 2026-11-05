use std::ops::Deref;

use crate::elf;
use crate::error::{At, Error, Operation, Result};

/// `e_ident[8..11]`, the bytes that mark an ELF as a type-2 AppImage.
const TYPE_2_MARKER: [u8; 3] = *b"AI\x02";

/// An ELF64 header and nothing else, written out byte by byte at compile time.
const INERT: [u8; 64] = {
    let mut elf = [0; 64];
    elf[0] = 0x7f;
    elf[1] = b'E';
    elf[2] = b'L';
    elf[3] = b'F';
    elf[4] = 2; // ELFCLASS64
    elf[5] = 1; // ELFDATA2LSB
    elf[6] = 1; // EV_CURRENT
    elf[16] = 2; // e_type: ET_EXEC
    elf[18] = 0x3e; // e_machine: x86-64
    elf[20] = 1; // e_version
    elf[52] = 64; // e_ehsize
    elf
};

/// The ELF executable at the front of an AppImage: the part that runs when
/// someone executes the file.
///
/// An AppImage is just a runtime with a SquashFS appended, so *anything* that
/// is a Linux executable can be one. This crate does not bundle a launcher.
/// Instead, build one from this crate's own source (see [`launcher`]), or use
/// the program you are already running with [`Runtime::current_exe`].
///
/// A `Runtime` is always a well-formed ELF, cut to exactly its own length and
/// stamped with the type-2 marker, so [`Packer`](crate::Packer) can append a
/// payload to it without further checks.
///
/// [`launcher`]: crate::launcher
///
/// # Example
///
/// ```no_run
/// use appimage::Runtime;
///
/// // A static launcher built with
/// // `cargo build --profile runtime --features launcher --bin appimage-runtime`.
/// let runtime = Runtime::try_from(std::fs::read("target/runtime/appimage-runtime")?)?;
/// assert_eq!(&runtime[1..4], b"ELF");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Runtime(Vec<u8>);

impl Runtime {
    /// A runtime that is only an ELF header.
    ///
    /// The AppImages it makes can be opened, inspected, unpacked and mounted,
    /// but do nothing when executed. That is exactly what tools, tests and
    /// examples that only consume the payload need, and it works on any host.
    ///
    /// ```
    /// use appimage::Runtime;
    ///
    /// assert_eq!(Runtime::inert().len(), 64);
    /// ```
    #[must_use]
    pub fn inert() -> Self {
        let mut bytes = INERT;
        bytes[8..11].copy_from_slice(&TYPE_2_MARKER);
        Self(bytes.to_vec())
    }

    /// Use the running executable as the runtime.
    ///
    /// The resulting AppImage is a copy of this program that, when started,
    /// finds the payload appended to itself. For that to work the program must
    /// call [`launcher::init`](crate::launcher::init) first thing in `main`.
    ///
    /// The runtime is as big as the program is; for a small, static launcher
    /// build the `appimage-runtime` binary instead.
    pub fn current_exe() -> Result<Self> {
        let path = std::env::current_exe().map_err(Error::CurrentExecutable)?;
        std::fs::read(&path).at(Operation::Read, &path)?.try_into()
    }
}

impl Deref for Runtime {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Runtime {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Runtime> for Vec<u8> {
    fn from(runtime: Runtime) -> Self {
        runtime.0
    }
}

/// Validate the bytes as an ELF executable.
///
/// Anything after the end of the ELF is discarded, because that is where the
/// payload goes. This is what makes it safe to pass an executable that is
/// itself an AppImage.
///
/// # Errors
///
/// [`Error::InvalidRuntime`] if the bytes are not an ELF, or if the ELF refers
/// to regions beyond the end of the data.
impl TryFrom<Vec<u8>> for Runtime {
    type Error = Error;

    fn try_from(mut bytes: Vec<u8>) -> Result<Self> {
        let len = elf::elf_len(&bytes).map_err(Error::InvalidRuntime)?;
        bytes.truncate(len as usize);
        bytes[8..11].copy_from_slice(&TYPE_2_MARKER);
        Ok(Self(bytes))
    }
}
