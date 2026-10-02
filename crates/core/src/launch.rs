//! argv for F3/F4: a configured program (no shell) with the file appended.

use std::ffi::OsString;
use std::path::Path;

pub fn command(cmd: &[String], default: &[&str], file: &Path) -> Vec<OsString> {
    let mut argv: Vec<OsString> = if cmd.is_empty() {
        default.iter().map(OsString::from).collect()
    } else {
        cmd.iter().map(OsString::from).collect()
    };
    argv.push(file.into());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_command_uses_default() {
        let argv = command(&[], &["xdg-open"], Path::new("/a b/f.txt"));
        assert_eq!(argv, [OsString::from("xdg-open"), "/a b/f.txt".into()]);
    }

    #[test]
    fn configured_command_keeps_its_args_and_appends_file() {
        let cmd = vec!["foot".to_string(), "-e".into(), "less".into()];
        let argv = command(&cmd, &["xdg-open"], Path::new("/f"));
        assert_eq!(argv, ["foot", "-e", "less", "/f"].map(OsString::from));
    }
}
