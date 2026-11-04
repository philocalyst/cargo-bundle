use std::path::PathBuf;

pub struct ResourcePaths<'a> {
    pattern_iter: std::slice::Iter<'a, String>,
    glob_iter: Option<glob::Paths>,
    walk_iter: Option<walkdir::IntoIter>,
    allow_walk: bool,
}

impl<'a> ResourcePaths<'a> {
    pub(super) fn new(patterns: &'a [String], allow_walk: bool) -> ResourcePaths<'a> {
        ResourcePaths {
            pattern_iter: patterns.iter(),
            glob_iter: None,
            walk_iter: None,
            allow_walk,
        }
    }
}

impl Iterator for ResourcePaths<'_> {
    type Item = crate::Result<PathBuf>;

    fn next(&mut self) -> Option<crate::Result<PathBuf>> {
        loop {
            if let Some(ref mut walk_entries) = self.walk_iter
                && let Some(entry) = walk_entries.next()
            {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => return Some(Err(anyhow::Error::from(error))),
                };
                let path = entry.path();
                if path.is_dir() {
                    continue;
                }
                return Some(Ok(path.to_path_buf()));
            }
            self.walk_iter = None;
            if let Some(ref mut glob_paths) = self.glob_iter
                && let Some(glob_result) = glob_paths.next()
            {
                let path = match glob_result {
                    Ok(path) => path,
                    Err(error) => return Some(Err(anyhow::Error::from(error))),
                };
                if path.is_dir() {
                    if self.allow_walk {
                        let walk = walkdir::WalkDir::new(path);
                        self.walk_iter = Some(walk.into_iter());
                        continue;
                    } else {
                        return Some(Err(anyhow::anyhow!("{path:?} is a directory")));
                    }
                }
                return Some(Ok(path));
            }
            self.glob_iter = None;
            if let Some(pattern) = self.pattern_iter.next() {
                let glob = match glob::glob(pattern) {
                    Ok(glob) => glob,
                    Err(error) => return Some(Err(anyhow::Error::from(error))),
                };
                self.glob_iter = Some(glob);
                continue;
            }
            return None;
        }
    }
}
