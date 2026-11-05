//! Update information from the `.upd_info` ELF section of an AppImage.
//!
//! [AppImage update information](https://github.com/AppImage/AppImageSpec/blob/master/draft.md#update-information)

use std::fmt;
use std::str::{FromStr, Split};

const ZSYNC: &str = "zsync";
const GITHUB_RELEASES: &str = "gh-releases-zsync";
const PLING: &str = "pling-v1-zsync";

/// A pipe-delimited update record. Parsing checks the transport name and field
/// count. Field contents, including empty fields, are kept as supplied.
/// Formatting reproduces the input after removal of outer whitespace and NULs.
///
/// ```
/// use appimage::UpdateInformation;
///
/// let line = "gh-releases-zsync|probono|AppImages|latest|Subsurface-*.AppImage.zsync";
/// let information: UpdateInformation = line.parse()?;
/// let UpdateInformation::GitHubReleases { repository, .. } = &information else {
///     panic!("expected a GitHub release");
/// };
/// assert_eq!(repository, "AppImages");
/// assert_eq!(information.to_string(), line);
/// # Ok::<(), appimage::update::BlankError>(())
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum UpdateInformation {
    /// A direct link to a zsync control file.
    Zsync {
        /// The control file's URL.
        url: String,
    },
    /// A zsync control file attached to a GitHub release.
    GitHubReleases {
        /// Repository owner.
        username: String,
        /// Repository name.
        repository: String,
        /// A tag or a release selector such as `latest-pre`.
        release: String,
        /// The zsync asset's filename or wildcard pattern.
        filename: String,
    },
    /// A Pling product and an AppImage filename pattern.
    Pling {
        /// The Pling product ID.
        product_id: String,
        /// The pattern used to select the AppImage.
        filename: String,
    },
    /// An unrecognized transport or a record with the wrong field count.
    Other {
        /// The original leading token.
        transport: String,
        /// All fields after the transport, including empty ones.
        fields: Vec<String>,
    },
}

impl UpdateInformation {
    /// The leading token of the record.
    pub fn transport(&self) -> &str {
        match self {
            Self::Zsync { .. } => ZSYNC,
            Self::GitHubReleases { .. } => GITHUB_RELEASES,
            Self::Pling { .. } => PLING,
            Self::Other { transport, .. } => transport,
        }
    }

    fn known_transport(transport: &str, fields: Split<'_, char>) -> Option<Self> {
        Some(match transport {
            ZSYNC => {
                let [url] = owned_fields(fields)?;
                Self::Zsync { url }
            }
            GITHUB_RELEASES => {
                let [username, repository, release, filename] = owned_fields(fields)?;
                Self::GitHubReleases {
                    username,
                    repository,
                    release,
                    filename,
                }
            }
            PLING => {
                let [product_id, filename] = owned_fields(fields)?;
                Self::Pling {
                    product_id,
                    filename,
                }
            }
            _ => return None,
        })
    }
}

/// Own the fields only after their number matches the caller's array pattern.
fn owned_fields<const COUNT: usize>(mut fields: Split<'_, char>) -> Option<[String; COUNT]> {
    let mut values = [""; COUNT];
    for value in &mut values {
        *value = fields.next()?;
    }
    fields.next().is_none().then(|| values.map(String::from))
}

/// The update record contained only whitespace or NULs, or was empty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlankError;

impl fmt::Display for BlankError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("update information is blank")
    }
}

impl std::error::Error for BlankError {}

impl FromStr for UpdateInformation {
    type Err = BlankError;

    /// # Errors
    ///
    /// Returns [`BlankError`] if the input is empty or consists entirely of padding.
    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let line =
            line.trim_matches(|character: char| character == '\0' || character.is_whitespace());
        if line.is_empty() {
            return Err(BlankError);
        }

        let mut fields = line.split('|');
        let transport = fields.next().unwrap_or_default();
        let known = Self::known_transport(transport, fields.clone());
        Ok(known.unwrap_or_else(|| Self::Other {
            transport: transport.into(),
            fields: fields.map(String::from).collect(),
        }))
    }
}

impl fmt::Display for UpdateInformation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.transport())?;
        match self {
            Self::Zsync { url } => write_fields(formatter, [url]),
            Self::GitHubReleases {
                username,
                repository,
                release,
                filename,
            } => write_fields(formatter, [username, repository, release, filename]),
            Self::Pling {
                product_id,
                filename,
            } => write_fields(formatter, [product_id, filename]),
            Self::Other { fields, .. } => write_fields(formatter, fields),
        }
    }
}

fn write_fields<'field>(
    formatter: &mut fmt::Formatter<'_>,
    fields: impl IntoIterator<Item = &'field String>,
) -> fmt::Result {
    for field in fields {
        formatter.write_str("|")?;
        formatter.write_str(field)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BlankError, UpdateInformation};
    use UpdateInformation::{GitHubReleases, Other, Pling, Zsync};

    fn round_trip(line: &str) -> UpdateInformation {
        let information: UpdateInformation = line.parse().unwrap();
        assert_eq!(information.to_string(), line);
        information
    }

    #[test]
    fn recognized_fields_have_their_named_values() {
        assert_eq!(round_trip("zsync|"), Zsync { url: String::new() });
        assert_eq!(
            round_trip("gh-releases-zsync|owner|project|latest-pre|*.zsync"),
            GitHubReleases {
                username: "owner".into(),
                repository: "project".into(),
                release: "latest-pre".into(),
                filename: "*.zsync".into(),
            }
        );
        assert_eq!(
            round_trip("pling-v1-zsync|123|*-x86_64.AppImage"),
            Pling {
                product_id: "123".into(),
                filename: "*-x86_64.AppImage".into()
            }
        );
    }

    #[test]
    fn other_records_retain_every_field() {
        for line in [
            "bare",
            "|",
            "future||α|",
            "zsync|file|",
            "gh-releases-zsync|owner",
            "pling-v1-zsync|123",
            "pling-zsync|123|file",
            "bintray-zsync|123|file",
            "bintray-zsync|owner|repository|package|app.zsync",
        ] {
            let Other { transport, fields } = round_trip(line) else {
                panic!("expected Other for {line:?}");
            };
            let mut expected = line.split('|');
            assert_eq!(Some(transport.as_str()), expected.next());
            assert!(expected.eq(fields), "{line:?}");
        }
    }

    #[test]
    fn only_the_record_edges_are_trimmed() {
        for line in ["", " \t\r\n", "\0\u{2003}\0"] {
            assert_eq!(line.parse::<UpdateInformation>(), Err(BlankError));
        }
        let padded = "\u{2003}\0 zsync| a\0\nb \0\t";
        assert_eq!(
            padded.parse(),
            Ok(Zsync {
                url: " a\0\nb".into()
            })
        );
    }
}
