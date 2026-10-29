//! Environment-driven FFmpeg/FFprobe input options shared by every spawn site:
//! decode thread budget, FTP credentials, and URL redaction for logs.
//!
//! Env contract (set by the batch worker / deployment):
//! - `DBSRT_FFMPEG_THREADS`: decode threads per process (default `DEFAULT_FFMPEG_DECODE_THREADS`).
//! - `DBSRT_FTP_USER` / `DBSRT_FTP_PASSWORD`: credentials for `ftp://` inputs without userinfo,
//!   passed as `-ftp-user` / `-ftp-password` input options. Their values are never logged.
//! - `DBSRT_FTP_PASSWORD_FILE`: file holding the password (no trailing newline), passed as
//!   `-/ftp-password <file>` (FFmpeg >= 7.0) so the secret never reaches the ffmpeg argv.
//!   Wins over `DBSRT_FTP_PASSWORD`.

use std::path::Path;
use std::process::Command;

/// Default decoding threads allocated per FFmpeg process to enforce resource predictability
/// and prevent host process/thread starvation on shared cloud runtimes.
pub const DEFAULT_FFMPEG_DECODE_THREADS: &str = "2";

/// Env var overriding the FFmpeg/FFprobe decode thread count (`0` lets FFmpeg decide).
pub const FFMPEG_THREADS_ENV: &str = "DBSRT_FFMPEG_THREADS";
/// Env var holding the FTP user for `ftp://` inputs without userinfo.
pub const FTP_USER_ENV: &str = "DBSRT_FTP_USER";
/// Env var holding the FTP password for `ftp://` inputs without userinfo.
pub const FTP_PASSWORD_ENV: &str = "DBSRT_FTP_PASSWORD";
/// Env var holding the path of a file whose content is the FTP password.
pub const FTP_PASSWORD_FILE_ENV: &str = "DBSRT_FTP_PASSWORD_FILE";

const FTP_SCHEME_PREFIX: &str = "ftp://";
const SCHEME_SEPARATOR: &str = "://";
const REDACTED_SECRET: &str = "***";

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
/// A non-empty `password_file` wins over `password` and is read by FFmpeg itself.
pub fn ftp_credential_args(
    input: &str,
    user: Option<&str>,
    password: Option<&str>,
    password_file: Option<&str>,
) -> Vec<String> {
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
    match (password_file.filter(|f| !f.is_empty()), password) {
        (Some(file), _) => args.extend(["-/ftp-password".to_string(), file.to_string()]),
        (None, Some(password)) => {
            args.extend(["-ftp-password".to_string(), password.to_string()]);
        }
        (None, None) => {}
    }
    args
}

/// Appends FTP credential input options from `DBSRT_FTP_USER` / `DBSRT_FTP_PASSWORD[_FILE]`
/// when `input` is an `ftp://` URL without userinfo. Must be called before the input argument.
pub fn apply_ftp_credentials(cmd: &mut Command, input: &Path) {
    let user = std::env::var(FTP_USER_ENV).ok();
    let password = std::env::var(FTP_PASSWORD_ENV).ok();
    let password_file = std::env::var(FTP_PASSWORD_FILE_ENV).ok();
    cmd.args(ftp_credential_args(
        &input.to_string_lossy(),
        user.as_deref(),
        password.as_deref(),
        password_file.as_deref(),
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

/// Joins a process command line for display, masking the `-ftp-password` value and URL
/// userinfo passwords (process command lines are shown in the TUI).
pub fn redact_command_line(args: &[String]) -> String {
    let mut previous_was_password_flag = false;
    let masked: Vec<String> = args
        .iter()
        .map(|arg| {
            let shown = if previous_was_password_flag {
                REDACTED_SECRET.to_string()
            } else {
                redact_url_userinfo(arg)
            };
            previous_was_password_flag = arg == "-ftp-password";
            shown
        })
        .collect();
    masked.join(" ")
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
            ftp_credential_args(url, Some("u"), Some("p"), None),
            ["-ftp-user", "u", "-ftp-password", "p"]
        );
        assert_eq!(
            ftp_credential_args(url, Some("u"), None, None),
            ["-ftp-user", "u"]
        );
        assert!(ftp_credential_args(url, None, Some("p"), None).is_empty());
        assert!(ftp_credential_args(url, Some(""), Some("p"), None).is_empty());
        // Userinfo in the URL wins; other schemes and local paths get nothing.
        assert!(ftp_credential_args("ftp://a:b@nas/x.mkv", Some("u"), Some("p"), None).is_empty());
        assert!(ftp_credential_args("http://nas/x.mkv", Some("u"), Some("p"), None).is_empty());
        assert!(ftp_credential_args("/data/x.mkv", Some("u"), Some("p"), None).is_empty());
        // A password file wins and keeps the secret out of the ffmpeg argv.
        assert_eq!(
            ftp_credential_args(url, Some("u"), Some("p"), Some("/run/pw")),
            ["-ftp-user", "u", "-/ftp-password", "/run/pw"]
        );
        assert_eq!(
            ftp_credential_args(url, Some("u"), Some("p"), Some("")),
            ["-ftp-user", "u", "-ftp-password", "p"]
        );
        // An '@' in the path is not userinfo.
        assert_eq!(
            ftp_credential_args("FTP://nas/dir@2024/x.mkv", Some("u"), None, None),
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
