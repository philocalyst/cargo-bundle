// A Windows application bundle copies the compiled executable and embeds
// version metadata and a multi-resolution icon directly into the PE binary
// using the Windows resource section.
//
// Resource embedding requires the `winres-edit` crate which only compiles on
// Windows, so the embedding step is unconditionally skipped on other platforms.
// Producing the output executable (a plain copy) works everywhere.

