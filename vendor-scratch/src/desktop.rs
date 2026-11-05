//! Desktop entries that cannot be made to say anything but what they are given.
//!
//! ```
//! use appimage::{AppId, DesktopCommand, DesktopEntry, FileArgument};
//!
//! let id = AppId::new("org.example.Viewer")?;
//! let mut open = DesktopCommand::new("viewer");
//! open.files = Some(FileArgument::Files);
//! let mut entry = DesktopEntry::new("Viewer", open);
//! entry.categories = &["Graphics", "Viewer"];
//!
//! let text = entry.render(&id)?;
//! assert!(text.contains("Exec=\"viewer\"\\s%F\n"));
//! assert!(text.contains("Categories=Graphics;Viewer;\n"));
//! # Ok::<(), appimage::Error>(())
//! ```

use std::fmt;

use crate::{AppDirError, Result};

/// File stem shared by an application's desktop file and icon.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppId(String);

impl AppId {
    /// Accept ASCII letters, digits, '.', '_' and '-', starting with a letter or digit.
    ///
    /// # Errors
    ///
    /// [`AppDirError::InvalidId`] for anything else, so an ID can never be a
    /// path or smuggle a second line into the desktop file.
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(AppDirError::InvalidId(value).into());
        }
        Ok(Self(value))
    }

    /// The ID as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl AsRef<str> for AppId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Files or URLs supplied by a desktop launcher, appended after literal arguments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileArgument {
    /// A single file (`%f`).
    File,
    /// Any number of files (`%F`).
    Files,
    /// A single URL (`%u`).
    Url,
    /// Any number of URLs (`%U`).
    Urls,
}

impl FileArgument {
    fn code(self) -> &'static str {
        match self {
            Self::File => "%f",
            Self::Files => "%F",
            Self::Url => "%u",
            Self::Urls => "%U",
        }
    }
}

/// A desktop command with literal arguments, without shell syntax.
#[derive(Clone, Copy, Debug)]
pub struct DesktopCommand<'a> {
    /// An executable name or absolute path, without shell syntax.
    pub executable: &'a str,
    /// Literal arguments. Percent signs are escaped, not expanded as field codes.
    pub arguments: &'a [&'a str],
    /// An optional file or URL argument supplied by the desktop launcher.
    pub files: Option<FileArgument>,
}

impl<'a> DesktopCommand<'a> {
    /// A command with no arguments.
    #[must_use]
    pub fn new(executable: &'a str) -> Self {
        Self {
            executable,
            arguments: &[],
            files: None,
        }
    }

    fn render(&self) -> Result<String> {
        if self.executable.is_empty() || self.executable.contains('=') {
            return Err(field_error(
                "Exec",
                "executable must be nonempty and contain no '='",
            ));
        }
        let mut command = quote_argument(self.executable)?;
        for argument in self.arguments {
            command.push(' ');
            command.push_str(&quote_argument(argument)?);
        }
        if let Some(files) = self.files {
            command.push(' ');
            command.push_str(files.code());
        }
        Ok(escape_value(&command))
    }
}

/// Desktop metadata, validated when rendered or written.
/// The application ID supplies the icon name and desktop filename.
#[derive(Clone, Copy, Debug)]
pub struct DesktopEntry<'a> {
    /// The name shown in menus. Must not be blank.
    pub name: &'a str,
    /// What running the entry runs.
    pub command: DesktopCommand<'a>,
    /// A tooltip.
    pub comment: Option<&'a str>,
    /// A generic name for the kind of application, like "Web Browser".
    pub generic_name: Option<&'a str>,
    /// Whether to run in a terminal.
    pub terminal: bool,
    /// Menu categories. ASCII only.
    pub categories: &'a [&'a str],
    /// Search keywords.
    pub keywords: &'a [&'a str],
    /// MIME types the application opens. ASCII only.
    pub mime_types: &'a [&'a str],
}

impl<'a> DesktopEntry<'a> {
    /// Required metadata with optional fields left empty.
    #[must_use]
    pub fn new(name: &'a str, command: DesktopCommand<'a>) -> Self {
        Self {
            name,
            command,
            comment: None,
            generic_name: None,
            terminal: false,
            categories: &[],
            keywords: &[],
            mime_types: &[],
        }
    }

    /// Serialize the entry, rejecting invalid values before anything is written.
    ///
    /// # Errors
    ///
    /// [`AppDirError::DesktopField`], naming the key that is wrong.
    pub fn render(&self, id: &AppId) -> Result<String> {
        if self.name.trim().is_empty() {
            return Err(field_error("Name", "must not be empty"));
        }
        let mut output = String::from("[Desktop Entry]\nType=Application\nVersion=1.0\n");
        append_value(&mut output, "Name", self.name)?;
        if let Some(value) = self.generic_name {
            append_value(&mut output, "GenericName", value)?;
        }
        if let Some(value) = self.comment {
            append_value(&mut output, "Comment", value)?;
        }
        output.push_str("Exec=");
        output.push_str(&self.command.render()?);
        output.push('\n');
        append_value(&mut output, "Icon", id.as_str())?;
        output.push_str(if self.terminal {
            "Terminal=true\n"
        } else {
            "Terminal=false\n"
        });
        for (key, values, ascii) in [
            ("Categories", self.categories, true),
            ("Keywords", self.keywords, false),
            ("MimeType", self.mime_types, true),
        ] {
            if values.is_empty() {
                continue;
            }
            output.push_str(key);
            output.push('=');
            for value in values {
                validate_value(key, value)?;
                if value.is_empty() || (ascii && !value.is_ascii()) {
                    return Err(field_error(
                        key,
                        "list items must be nonempty; Categories and MimeType must be ASCII",
                    ));
                }
                output.push_str(&escape_value(value).replace(';', "\\;"));
                output.push(';');
            }
            output.push('\n');
        }
        Ok(output)
    }
}

fn field_error(field: &'static str, reason: &'static str) -> crate::Error {
    AppDirError::DesktopField { field, reason }.into()
}

fn validate_value(field: &'static str, value: &str) -> Result<()> {
    if value
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\t' | '\r'))
    {
        return Err(field_error(field, "unsupported control character"));
    }
    Ok(())
}

fn append_value(output: &mut String, key: &'static str, value: &str) -> Result<()> {
    validate_value(key, value)?;
    output.push_str(key);
    output.push('=');
    output.push_str(&escape_value(value));
    output.push('\n');
    Ok(())
}

fn escape_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\t' => escaped.push_str("\\t"),
            '\r' => escaped.push_str("\\r"),
            ' ' => escaped.push_str("\\s"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn quote_argument(value: &str) -> Result<String> {
    if !value.is_ascii() || value.chars().any(char::is_control) {
        return Err(field_error(
            "Exec",
            "arguments must be ASCII without control characters",
        ));
    }
    let mut quoted = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' | '`' | '$' | '\\' => quoted.push('\\'),
            '%' => quoted.push('%'),
            _ => {}
        }
        quoted.push(ch);
    }
    quoted.push('"');
    Ok(quoted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_cannot_be_paths_or_hidden_files() {
        for id in [
            "",
            ".",
            "..",
            "../app",
            "/app",
            "app/icon",
            "app\nType=Link",
            ".hidden",
            "a\\b",
        ] {
            assert!(matches!(
                AppId::new(id),
                Err(crate::Error::AppDir(AppDirError::InvalidId(_)))
            ));
        }
        assert_eq!(
            AppId::new("org.example.My-App_2").unwrap().as_str(),
            "org.example.My-App_2"
        );
    }

    #[test]
    fn metadata_cannot_inject_keys_or_list_items() {
        let id = AppId::new("app").unwrap();
        let mut entry = DesktopEntry::new(" App\nType=Link ", DesktopCommand::new("app"));
        entry.keywords = &["one;two", "café"];
        let text = entry.render(&id).unwrap();
        assert!(text.contains("Name=\\sApp\\nType=Link\\s\n"));
        assert!(text.contains("Keywords=one\\;two;café;\n"));
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("Type="))
                .count(),
            1
        );
    }

    #[test]
    fn command_quotes_literals_and_keeps_field_codes_separate() {
        let command = DesktopCommand {
            executable: "my app",
            arguments: &["", "a\"b", "$HOME", "`id`", "a\\b", "%U", "one;two"],
            files: Some(FileArgument::Urls),
        };
        assert_eq!(
            command.render().unwrap(),
            r#""my\sapp"\s""\s"a\\"b"\s"\\$HOME"\s"\\`id\\`"\s"a\\\\b"\s"%%U"\s"one;two"\s%U"#
        );
    }

    #[test]
    fn required_fields_and_control_characters_are_validated() {
        let id = AppId::new("app").unwrap();
        for (name, command) in [
            (" ", "app"),
            ("App", ""),
            ("App", "app=bad"),
            ("App\0", "app"),
            ("App", "app\nType=Link"),
            ("App", "café"),
        ] {
            assert!(matches!(
                DesktopEntry::new(name, DesktopCommand::new(command)).render(&id),
                Err(crate::Error::AppDir(AppDirError::DesktopField { .. }))
            ));
        }
    }
}
