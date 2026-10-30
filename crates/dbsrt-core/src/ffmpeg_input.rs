//! Environment-driven FFmpeg/FFprobe input options shared by every spawn site:
//! decode thread budget, FTP credentials, and URL redaction for logs.
//!
//! Env contract (set by the batch worker / deployment):
//! - `DBSRT_FFMPEG_THREADS`: decode threads per process (default `DEFAULT_FFMPEG_DECODE_THREADS`).
//! - `DBSRT_FTP_USER` / `DBSRT_FTP_PASSWORD`: credentials for `ftp://` inputs without userinfo,
//!   passed as `-ftp-user` / `-ftp-password` input options. Their values are never logged.
//!   FFmpeg (checked up to 8.1) rejects `-/ftp-password <file>`: protocol options cannot be
//!   read from a file, so the password stays in the ffmpeg argv.

use std::path::Path;
use std::process::Command;

/// Default decoding threads per FFmpeg process: `0` lets FFmpeg use every core, as before the
/// env existed. Shared cloud runtimes (o2switch) cap it through `DBSRT_FFMPEG_THREADS`.
pub const DEFAULT_FFMPEG_DECODE_THREADS: &str = "0";

/// Env var overriding the FFmpeg/FFprobe decode thread count (`0` lets FFmpeg decide).
pub const FFMPEG_THREADS_ENV: &str = "DBSRT_FFMPEG_THREADS";
/// Env var holding the FTP user for `ftp://` inputs without userinfo.
pub const FTP_USER_ENV: &str = "DBSRT_FTP_USER";
/// Env var holding the FTP password for `ftp://` inputs without userinfo.
pub const FTP_PASSWORD_ENV: &str = "DBSRT_FTP_PASSWORD";

const FTP_SCHEME_PREFIX: &str = "ftp://";
const SCHEME_SEPARATOR: &str = "://";
const REDACTED_SECRET: &str = "***";
const FTP_PASSWORD_FLAG: &str = "-ftp-password";

/// Returns the thread count from a raw env value: a non-negative integer, else the default.
pub fn parse_ffmpeg_threads(raw: Option<&str>) -> String {
    raw.map(str::trim)
        .filter(|v| v.parse::<u16>().is_ok())
        .map_or_else(|| DEFAULT_FFMPEG_DECODE_THREADS.to_string(), str::to_string)
}

/// Decode thread count for `-threads` / `-filter_threads`, honoring `DBSRT_FFMPEG_THREADS`.
pub fn ffmpeg_decode_threads() -> String {
    parse_ffmpeg_threads(std::env::var(FFMPEG_THREADS_ENV).ok().as_deref())
}

/// Builds `-ftp-user` / `-ftp-password` input options for an `ftp://` input that carries no
/// userinfo. Returns no options for other inputs, URLs with userinfo, or an empty user.
pub fn ftp_credential_args(input: &str, user: Option<&str>, password: Option<&str>) -> Vec<String> {
    let Some(rest) = input
        .get(..FTP_SCHEME_PREFIX.len())
        .filter(|scheme| scheme.eq_ignore_ascii_case(FTP_SCHEME_PREFIX))
        .map(|_| &input[FTP_SCHEME_PREFIX.len()..])
    else {
        return Vec::new();
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let user = user.filter(|u| !u.is_empty());
    let (false, Some(user)) = (authority.contains('@'), user) else {
        return Vec::new();
    };
    let mut args = vec!["-ftp-user".to_string(), user.to_string()];
    if let Some(password) = password {
        args.extend(["-ftp-password".to_string(), password.to_string()]);
    }
    args
}

/// Appends FTP credential input options from `DBSRT_FTP_USER` / `DBSRT_FTP_PASSWORD`
/// when `input` is an `ftp://` URL without userinfo. Must be called before the input argument.
pub fn apply_ftp_credentials(cmd: &mut Command, input: &Path) {
    let user = std::env::var(FTP_USER_ENV).ok();
    let password = std::env::var(FTP_PASSWORD_ENV).ok();
    cmd.args(ftp_credential_args(
        &input.to_string_lossy(),
        user.as_deref(),
        password.as_deref(),
    ));
}

/// Masks the password of every `scheme://user:password@host` occurrence as `user:***@host`.
pub fn redact_url_userinfo(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find(SCHEME_SEPARATOR) {
        let authority_start = pos + SCHEME_SEPARATOR.len();
        out.push_str(&rest[..authority_start]);
        rest = &rest[authority_start..];
        let authority_end = rest
            .find(|c: char| c == '/' || c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        push_redacted_authority(&rest[..authority_end], &mut out);
        rest = &rest[authority_end..];
    }
    out.push_str(rest);
    out
}

/// Appends a URL authority (`[user[:password]@]host[:port]`) with the password masked.
fn push_redacted_authority(authority: &str, out: &mut String) {
    let Some(at) = authority.rfind('@') else {
        out.push_str(authority);
        return;
    };
    let userinfo = &authority[..at];
    match userinfo.split_once(':') {
        Some((user, _password)) => {
            out.push_str(user);
            out.push(':');
            out.push_str(REDACTED_SECRET);
        }
        None => out.push_str(userinfo),
    }
    out.push_str(&authority[at..]);
}

/// Path/URL rendering safe for logs and error messages (userinfo password masked).
pub fn display_input(input: &Path) -> String {
    redact_url_userinfo(&input.to_string_lossy())
}

/// Byte length of the shell word starting `text`: ends at the first whitespace outside quotes
/// and not backslash-escaped. An unterminated quote runs to the end, so the rest gets masked.
fn shell_word_len(text: &str) -> usize {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in text.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            // Backslash is literal inside single quotes
            (Some('\''), _) => {}
            (_, '\\') => escaped = true,
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, c) if c.is_whitespace() => return i,
            _ => {}
        }
    }
    text.len()
}

/// Masks URL userinfo passwords and the shell word following `-ftp-password` in free text
/// (a shell wrapper carries a whole ffmpeg command line in one argument). Quoted or escaped
/// passwords with spaces are masked whole.
pub fn redact_secrets(text: &str) -> String {
    let text = redact_url_userinfo(text);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(pos) = rest.find(FTP_PASSWORD_FLAG) {
        let flag_end = pos + FTP_PASSWORD_FLAG.len();
        let after = &rest[flag_end..];
        let value_start = after.len() - after.trim_start().len();
        out.push_str(&rest[..flag_end + value_start]);
        rest = &after[value_start..];
        // A flag glued to other text (`-ftp-passwordX`) or ending the text has no value here
        if value_start == 0 || rest.is_empty() {
            continue;
        }
        let value_end = shell_word_len(rest);
        out.push_str(REDACTED_SECRET);
        rest = &rest[value_end..];
    }
    out.push_str(rest);
    out
}

/// Masks a process command line argument by argument: the argument after `-ftp-password`,
/// then [`redact_secrets`] on every other argument.
pub fn redact_args(args: &[String]) -> Vec<String> {
    let mut previous_was_password_flag = false;
    args.iter()
        .map(|arg| {
            let shown = if previous_was_password_flag {
                REDACTED_SECRET.to_string()
            } else {
                redact_secrets(arg)
            };
            previous_was_password_flag = arg == FTP_PASSWORD_FLAG;
            shown
        })
        .collect()
}

/// Joins a process command line for display with its secrets masked (see [`redact_args`]).
pub fn redact_command_line(args: &[String]) -> String {
    redact_args(args).join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_command_line() {
        let args: Vec<String> = [
            "ffmpeg",
            "-ftp-user",
            "u",
            "-ftp-password",
            "p",
            "-i",
            "ftp://a:b@h/x",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            redact_command_line(&args),
            "ffmpeg -ftp-user u -ftp-password *** -i ftp://a:***@h/x"
        );
    }

    #[test]
    fn test_redact_secrets_in_free_text() {
        assert_eq!(
            redact_secrets("sh -c ffmpeg -ftp-password  s3cret -i ftp://u:p2@h/x.mkv"),
            "sh -c ffmpeg -ftp-password  *** -i ftp://u:***@h/x.mkv"
        );
        assert_eq!(
            redact_secrets("a -ftp-password x b -ftp-password y"),
            "a -ftp-password *** b -ftp-password ***"
        );
        assert_eq!(redact_secrets("ends -ftp-password"), "ends -ftp-password");
        assert_eq!(redact_secrets("-ftp-passwordX keep"), "-ftp-passwordX keep");
        assert_eq!(redact_secrets("/local/a.mkv"), "/local/a.mkv");
        // Idempotent: masking an already masked line changes nothing
        let once = redact_secrets("-ftp-password s -i ftp://u:p@h/x");
        assert_eq!(redact_secrets(&once), once);
    }

    #[test]
    fn test_redact_secrets_masks_quoted_and_escaped_passwords_whole() {
        let cases = [
            (
                "ffmpeg -ftp-password 'p a s s' -i x",
                "ffmpeg -ftp-password *** -i x",
            ),
            (
                "ffmpeg -ftp-password \"p a s s\" -i x",
                "ffmpeg -ftp-password *** -i x",
            ),
            (
                "ffmpeg -ftp-password p\\ a\\ s\\ s -i x",
                "ffmpeg -ftp-password *** -i x",
            ),
            (
                "ffmpeg -ftp-password pre\"mid dle\"post -i x",
                "ffmpeg -ftp-password *** -i x",
            ),
            // Escaped quote inside double quotes does not close them
            (
                "ffmpeg -ftp-password \"a \\\" b\" -i x",
                "ffmpeg -ftp-password *** -i x",
            ),
            // Backslash is literal inside single quotes: the quote closes at the next '
            (
                "ffmpeg -ftp-password 'a\\' -i x",
                "ffmpeg -ftp-password *** -i x",
            ),
            // Unterminated quote: fail safe, mask everything after the flag
            (
                "ffmpeg -ftp-password 'p a s s -i x",
                "ffmpeg -ftp-password ***",
            ),
            (
                "ffmpeg -ftp-password\t'p a'\t-i x",
                "ffmpeg -ftp-password\t***\t-i x",
            ),
        ];
        for (input, expected) in cases {
            let masked = redact_secrets(input);
            assert_eq!(masked, expected, "input: {input}");
            assert_eq!(redact_secrets(&masked), masked, "idempotent: {input}");
            for leaked in ["p a s s", "mid dle", "a \\\" b"] {
                assert!(!masked.contains(leaked), "{leaked} leaked from {input}");
            }
        }
    }

    #[test]
    fn test_redact_args_masks_password_argument_with_spaces() {
        let args: Vec<String> = ["ffmpeg", "-ftp-password", "p a s s", "-i", "x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            redact_args(&args),
            ["ffmpeg", "-ftp-password", "***", "-i", "x"]
        );
    }

    #[test]
    fn test_redact_args_masks_split_and_embedded_secrets() {
        let args: Vec<String> = [
            "bash",
            "-c",
            "ffmpeg -ftp-password s1 -i x",
            "-ftp-password",
            "s2",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let masked = redact_args(&args);
        assert_eq!(masked[2], "ffmpeg -ftp-password *** -i x");
        assert_eq!(masked[4], "***");
        assert!(!masked.join(" ").contains("s1") && !masked.join(" ").contains("s2"));
    }

    #[test]
    fn test_parse_ffmpeg_threads() {
        assert_eq!(parse_ffmpeg_threads(None), DEFAULT_FFMPEG_DECODE_THREADS);
        assert_eq!(parse_ffmpeg_threads(Some("4")), "4");
        assert_eq!(parse_ffmpeg_threads(Some(" 0 ")), "0");
        assert_eq!(
            parse_ffmpeg_threads(Some("")),
            DEFAULT_FFMPEG_DECODE_THREADS
        );
        assert_eq!(
            parse_ffmpeg_threads(Some("-1")),
            DEFAULT_FFMPEG_DECODE_THREADS
        );
        assert_eq!(
            parse_ffmpeg_threads(Some("auto")),
            DEFAULT_FFMPEG_DECODE_THREADS
        );
    }

    #[test]
    fn test_ftp_credential_args() {
        let url = "ftp://nas.example:21/media/movie.mkv";
        assert_eq!(
            ftp_credential_args(url, Some("u"), Some("p")),
            ["-ftp-user", "u", "-ftp-password", "p"]
        );
        assert_eq!(
            ftp_credential_args(url, Some("u"), None),
            ["-ftp-user", "u"]
        );
        assert!(ftp_credential_args(url, None, Some("p")).is_empty());
        assert!(ftp_credential_args(url, Some(""), Some("p")).is_empty());
        // Userinfo in the URL wins; other schemes and local paths get nothing.
        assert!(ftp_credential_args("ftp://a:b@nas/x.mkv", Some("u"), Some("p")).is_empty());
        assert!(ftp_credential_args("http://nas/x.mkv", Some("u"), Some("p")).is_empty());
        assert!(ftp_credential_args("/data/x.mkv", Some("u"), Some("p")).is_empty());
        // An '@' in the path is not userinfo.
        assert_eq!(
            ftp_credential_args("FTP://nas/dir@2024/x.mkv", Some("u"), None),
            ["-ftp-user", "u"]
        );
    }

    #[test]
    fn test_redact_url_userinfo() {
        assert_eq!(
            redact_url_userinfo("open ftp://user:s3cret@nas:21/a.mkv failed"),
            "open ftp://user:***@nas:21/a.mkv failed"
        );
        assert_eq!(
            redact_url_userinfo("\"ftp://u:p@h/x\" and http://h2/y"),
            "\"ftp://u:***@h/x\" and http://h2/y"
        );
        assert_eq!(redact_url_userinfo("ftp://user@nas/a"), "ftp://user@nas/a");
        assert_eq!(redact_url_userinfo("/local/file.mkv"), "/local/file.mkv");
        assert_eq!(
            redact_url_userinfo("ftp://nas/dir@2024/x"),
            "ftp://nas/dir@2024/x"
        );
    }
}
