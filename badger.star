# Navigation badges, not cached build/version status. Live status is in Readme.md.
globals(
    badges_dir = ".badges",
    base_url = "https://raw.githubusercontent.com/philocalyst/cargo-bundle/master",
    corner_size = 2.0,
)

badge(
    primary = color("darkslategray"),
    secondary = color("steelblue"),
    producer = simple("crates.io", "cargo-bundle"),
    id = "crate",
    link = "https://crates.io/crates/cargo-bundle",
)

badge(
    primary = color("darkslategray"),
    secondary = color("seagreen"),
    producer = simple("guide", "mdBook"),
    id = "guide",
    link = "https://github.com/philocalyst/cargo-bundle/blob/master/docs/src/index.md",
)
