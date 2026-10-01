//! OS ごとに異なる処理（ホームディレクトリ、同梱 sidecar の名前、子プロセスの起動・停止）をまとめる。
//! macOS と Windows の差分はこのモジュールに閉じ込め、他のモジュールからは OS を意識せずに呼び出す。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 環境変数の値からホームディレクトリを決める。Windows では HOME が無く USERPROFILE を使う。
pub(crate) fn home_dir_from(
    home: Option<OsString>,
    user_profile: Option<OsString>,
) -> Option<PathBuf> {
    [home, user_profile]
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// 現在のユーザーのホームディレクトリ。取得できない場合は一時ディレクトリを返す。
pub(crate) fn home_dir() -> PathBuf {
    home_dir_from(std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
        .unwrap_or_else(std::env::temp_dir)
}

/// Tauri externalBin の命名規則に従った sidecar のファイル名を OS / CPU から決める。
pub(crate) fn sidecar_binary_name_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("siteone-crawler-aarch64-apple-darwin"),
        ("macos", "x86_64") => Some("siteone-crawler-x86_64-apple-darwin"),
        ("windows", "x86_64") => Some("siteone-crawler-x86_64-pc-windows-msvc.exe"),
        _ => None,
    }
}

/// 実行中のアプリに対応する sidecar のファイル名。
pub(crate) fn sidecar_binary_name() -> &'static str {
    sidecar_binary_name_for(std::env::consts::OS, std::env::consts::ARCH)
        .unwrap_or("siteone-crawler")
}

/// Tauri が bundle 時に target 名を取り除いて配置する sidecar のファイル名。
pub(crate) fn installed_sidecar_name() -> String {
    format!("siteone-crawler{}", std::env::consts::EXE_SUFFIX)
}

/// 実行可能なファイルかどうか。Unix では実行権限も確認する。
pub(crate) fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// パスを正規化する。Windows では `\\?\` 付きの verbatim パスにならないようにする。
pub(crate) fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Windows でコンソール画面を表示せずにプロセスを起動するフラグ。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// GUI から起動する補助コマンドの設定。Windows ではコンソール画面を表示しない。
pub(crate) fn configure_background_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// クロール用の子プロセスの設定。子孫プロセスもまとめて停止できるようにする。
pub(crate) fn configure_child_command(command: &mut Command) {
    configure_background_command(command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // 子プロセスを新しいプロセスグループのリーダーにし、Chrome などの子孫ごと停止できるようにする。
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
}

/// 子プロセスとその子孫に、可能であれば穏やかな停止を要求する。
/// Windows の GUI アプリからはコンソールの Ctrl+C を送れないため何もせず、強制終了に任せる。
pub(crate) fn interrupt_process_tree(pid: u32) {
    #[cfg(unix)]
    signal_process_group(pid, libc::SIGINT);
    #[cfg(not(unix))]
    let _ = pid;
}

/// 子プロセスとその子孫を強制終了する。
pub(crate) fn kill_process_tree(pid: u32) {
    #[cfg(unix)]
    signal_process_group(pid, libc::SIGKILL);
    #[cfg(windows)]
    {
        // /T で子孫プロセス（Chrome など）も含めて終了する。
        let mut command = Command::new("taskkill");
        command.args(["/PID", &pid.to_string(), "/T", "/F"]);
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        configure_background_command(&mut command);
        let _ = command.status();
    }
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) {
    // 負の PID は、このクロール用に作ったプロセスグループ全体を指す。
    unsafe {
        let _ = libc::kill(-(pid as i32), signal);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn home_dir_prefers_home_and_falls_back_to_user_profile() {
        assert_eq!(
            home_dir_from(Some("/Users/maho".into()), Some("C:\\Users\\maho".into())),
            Some(PathBuf::from("/Users/maho"))
        );
        assert_eq!(
            home_dir_from(None, Some("C:\\Users\\maho".into())),
            Some(PathBuf::from("C:\\Users\\maho"))
        );
        assert_eq!(
            home_dir_from(Some("".into()), Some("C:\\Users\\maho".into())),
            Some(PathBuf::from("C:\\Users\\maho"))
        );
        assert_eq!(home_dir_from(None, None), None);
    }

    #[test]
    fn home_dir_is_never_empty() {
        assert!(!home_dir().as_os_str().is_empty());
    }

    #[test]
    fn sidecar_names_follow_tauri_target_triples() {
        assert_eq!(
            sidecar_binary_name_for("macos", "aarch64"),
            Some("siteone-crawler-aarch64-apple-darwin")
        );
        assert_eq!(
            sidecar_binary_name_for("macos", "x86_64"),
            Some("siteone-crawler-x86_64-apple-darwin")
        );
        assert_eq!(
            sidecar_binary_name_for("windows", "x86_64"),
            Some("siteone-crawler-x86_64-pc-windows-msvc.exe")
        );
        assert_eq!(sidecar_binary_name_for("linux", "x86_64"), None);
    }

    #[test]
    fn current_sidecar_names_match_the_running_platform() {
        assert_eq!(
            Some(sidecar_binary_name()),
            sidecar_binary_name_for(std::env::consts::OS, std::env::consts::ARCH)
        );
        assert_eq!(
            installed_sidecar_name(),
            format!("siteone-crawler{}", std::env::consts::EXE_SUFFIX)
        );
    }

    #[test]
    fn canonicalize_returns_a_plain_absolute_path() {
        let path = canonicalize(&std::env::temp_dir()).unwrap();
        assert!(path.is_absolute());
        assert!(!path.to_string_lossy().starts_with(r"\\?\"));
    }

    #[test]
    fn is_executable_rejects_missing_paths_and_directories() {
        assert!(!is_executable(Path::new("definitely-missing-maho-binary")));
        assert!(!is_executable(&std::env::temp_dir()));
    }

    /// 子プロセスが孫プロセスを起動し、さらに待機し続けるコマンド。
    fn long_running_tree() -> Command {
        #[cfg(unix)]
        {
            let mut command = Command::new("/bin/sh");
            command.args([
                "-c",
                "trap '' INT; sleep 30 & echo ready; while :; do sleep 1; done",
            ]);
            command
        }
        #[cfg(windows)]
        {
            let mut command = Command::new("cmd");
            command.args([
                "/C",
                "ping -n 30 127.0.0.1 >NUL & ping -n 30 127.0.0.1 >NUL",
            ]);
            command
        }
    }

    #[test]
    fn kill_process_tree_stops_a_child_that_ignores_interrupts() {
        let mut command = long_running_tree();
        command.stdout(Stdio::piped()).stderr(Stdio::null());
        configure_child_command(&mut command);
        let mut child = command.spawn().unwrap();
        // シグナルの扱いを設定し終えるまで待ってから停止を要求する。
        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        assert_eq!(ready.trim(), "ready");
        interrupt_process_tree(child.id());
        thread::sleep(Duration::from_millis(100));
        assert!(child.try_wait().unwrap().is_none());

        kill_process_tree(child.id());
        let deadline = Instant::now() + Duration::from_secs(3);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "子プロセスが終了しません");
            thread::sleep(Duration::from_millis(20));
        }
    }
}
