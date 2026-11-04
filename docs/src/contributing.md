# Contributing

Check the implementation before documenting a setting or output. Packaging
behavior can be incomplete or platform-dependent; describe what the code does
today, and label stubs and experimental formats clearly.

## Build and check the guide

The book source is in `docs/src/`; add new chapters to `SUMMARY.md`. Book
settings, including the edit links and site URL, are in `docs/book.toml`.
Install the mdBook version used by CI:

```sh
cargo install mdbook --version 0.5.4 --locked
```

From the repository root, build or preview the book:

```sh
mdbook build docs
mdbook serve docs
```

The source checker uses Python 3.11 or newer for `tomllib`; CI uses Python
3.12. Run it before and after a build; the second command also checks links
and anchors in the rendered HTML:

```sh
python3 scripts/check-docs.py
python3 scripts/check-docs.py --html docs/book
```

It checks local Markdown links, summary coverage, documented settings,
formats and CLI options, plus links and anchors in the built pages. To check
the first-bundle example, build the crate and run:

```sh
cargo build
python3 scripts/smoke-docs.py
```

The smoke check packages the guide's example on the host's native format and
inspects the output; on macOS it also runs the bundled executable. CI runs it
on Linux, macOS and Windows.

## Check Rust changes

These are the checks in `.github/workflows/ci.yml`:

```sh
cargo fmt --package cargo-bundle -- --check
cargo clippy --all-targets -- -D warnings
cargo build --all-features --verbose --all-targets
cargo test --all-features --verbose --all-targets
```

The build and tests run on Linux, macOS and Windows; Clippy and formatting
run on Linux. Check
platform-specific packaging changes on the platform they affect when possible.

The portable DMG tests verify UDIF checksums, HFS+ consistency, application
contents and Finder aliases through fstool. The AppImage source checkout also
supplies embedded-launcher execution tests and provenance checks.

## Write documentation

Lead with a working example when one helps explain a task. For style, see
[Jiff's documentation](https://docs.rs/jiff/latest/jiff/), the
[Wikipedia Manual of Style](https://en.wikipedia.org/wiki/Wikipedia:Manual_of_Style),
and Gwern's [Manual of Style](https://gwern.net/style-guide). Use them as
references, not templates. A link to the format specification or the platform's
own documentation lets readers check a requirement when it changes. The
[Diátaxis distinction between tutorials and reference](https://diataxis.fr/start-here/)
helps keep the first-bundle walkthrough focused while the manifest reference
records defaults and exceptions.

For each change:

- Give the reader a task, the command or configuration needed, and a way to
  check the result. State which host and files an example requires.
- Keep defaults and field semantics in the reference. Link to them from recipes
  rather than repeating the same explanation in several chapters.
- Verify claims against the backend that writes the output. A field being
  accepted by the parser does not mean every format uses it.
- Cut sentences that neither explain behavior nor help the reader choose,
  configure, build, or inspect a package.

## Update README badges

`badger run --readme Readme.md` reads the definitions in `badger.star` and
updates the SVGs and README badge block. Include the generated `.badges/`
files with `Readme.md` in the commit. The two GitHub workflow badges use live
status from GitHub.

## Docs workflow and Pages

The Docs workflow runs on manual dispatch and on pushes or pull requests that
change one of these paths:

| Paths |
| --- |
| `docs/**`, `upstream/**`, `src/**`, `.github/**` |
| `Readme.md`, `CONTRIBUTING.md`, `Cargo.toml`, `badger.star` |
| `scripts/check-docs.py`, `scripts/smoke-docs.py` |

Each run uploads the `cargo-bundle-guide` artifact for review. A push or
manual dispatch on the repository's default branch also deploys to GitHub
Pages; pull requests do not deploy. Pages must be enabled with **Build and
deployment** set to **GitHub Actions**.

When publishing a fork, start with these settings in `docs/book.toml`:

| Setting | Use in a fork |
| --- | --- |
| `git-repository-url` | The fork's repository URL |
| `edit-url-template` | An edit URL using its default branch; currently `master` |
| `site-url` | Its Pages base path; currently `/cargo-bundle/` |

The repository URLs in `Readme.md`, `badger.star`, and `Cargo.toml` need a
separate review. The Docs workflow gets the default branch from GitHub; any
branch names added to other workflows need to match the fork.

## AppImage release source

Dependencies come from crates.io, including `fstool` 0.5.2, `rust-apk` 1.0.0,
`alias_record` 0.3.0 and `ds_parser` 0.4.0. The sole local path is `appimage-rs`
on `recovered/embedded-runtime` at `d29cee02dad244f23843a046c13150b9ecc80772`,
restored with its embedded static Rust launchers.
Keep its checkout beside cargo-bundle until it is published. Packaging requires
no runtime fetch, mount helpers or external extraction tools.

CI uses `APPIMAGE_RS_URL` and `APPIMAGE_RS_REF` (a full 40-character commit SHA)
to check out that source. Publish the AppImage source commit at the configured
URL before enabling CI. The other crates require no sibling checkouts or CI
source variables. The recovered manifest is still version 0.1.0, which already exists on crates.io;
choose a new release version and rebuild the embedded launchers before publication.
The published 0.2.0 has a different API and does not embed these launchers.
After publishing the recovered implementation, update its dependency version and remove the local `path`
from Cargo.toml and the remaining sibling checkout action.

Before publishing cargo-bundle, run `cargo package` against the published
dependency graph. Cargo removes local paths from the published manifest.
