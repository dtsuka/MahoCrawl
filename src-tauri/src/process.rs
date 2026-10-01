use serde::Serialize;
use std::fs;
use std::io::{BufReader, ErrorKind, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{
    build_arguments, CaptureViewport, CrawlConfiguration, CrawlError, CrawlStatus, OutputPlan,
    RunPhase, MAX_LOG_LENGTH, MAX_REPORT_LENGTH,
};
use crate::report::{filter_non_page_screenshots, list_capture_items};

#[derive(Debug, Default)]
pub(crate) struct RuntimeState {
    pub(crate) status: CrawlStatus,
    pub(crate) log: String,
    pub(crate) cancel_requested: bool,
    pub(crate) start_in_progress: bool,
    pub(crate) child: Option<Child>,
    pub(crate) allowed_open_roots: Vec<PathBuf>,
    pub(crate) allowed_open_exact_paths: Vec<PathBuf>,
    pub(crate) history_browse_roots: Vec<PathBuf>,
}

pub(crate) type SharedState = Arc<Mutex<RuntimeState>>;

pub struct AppState {
    pub(crate) runtime: SharedState,
}

pub(crate) fn binary_name() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "siteone-crawler-aarch64-apple-darwin"
    } else if cfg!(target_arch = "x86_64") {
        "siteone-crawler-x86_64-apple-darwin"
    } else {
        "siteone-crawler"
    }
}

pub(crate) fn locate_binary(app: &AppHandle) -> Result<PathBuf, CrawlError> {
    let mut candidates = Vec::new();
    #[cfg(debug_assertions)]
    if let Some(value) = std::env::var_os("SITEONE_CRAWLER_PATH") {
        candidates.push(PathBuf::from(value));
    }
    if let Ok(resource) = app.path().resource_dir() {
        candidates.push(resource.join(binary_name()));
        candidates.push(resource.join("binaries").join(binary_name()));
        candidates.push(resource.join("siteone-crawler"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(binary_name()));
            candidates.push(parent.join("binaries").join(binary_name()));
            candidates.push(parent.join("siteone-crawler"));
        }
    }
    // 開発時のみソースツリー内のバイナリを探す。リリースビルドにビルド環境の絶対パスを埋め込まない。
    #[cfg(debug_assertions)]
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(binary_name()),
    );
    #[cfg(debug_assertions)]
    candidates.push(PathBuf::from("/opt/homebrew/bin/siteone-crawler"));
    #[cfg(debug_assertions)]
    candidates.push(PathBuf::from("/usr/local/bin/siteone-crawler"));
    candidates
        .into_iter()
        .find(|path| is_executable(path))
        .ok_or(CrawlError::BinaryNotFound)
}

pub(crate) fn is_executable(path: &Path) -> bool {
    path.is_file()
        && (cfg!(not(unix)) || {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(path)
                .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

pub(crate) fn timezone_or_utc(result: Result<String, ()>) -> String {
    result.unwrap_or_else(|_| "UTC".into())
}

pub(crate) fn emit<T: Serialize>(app: &AppHandle, event: &str, payload: &T) {
    let _ = app.emit(event, payload);
}

pub(crate) fn emit_status(app: &AppHandle, state: &SharedState) {
    if let Ok(guard) = state.lock() {
        emit(app, "crawl://status", &guard.status);
    }
}

pub(crate) fn trim_log_to_limit(log: &mut String) {
    if log.len() <= MAX_LOG_LENGTH {
        return;
    }
    let remove = log.len() - MAX_LOG_LENGTH;
    let mut boundary = remove;
    while !log.is_char_boundary(boundary) {
        boundary += 1;
    }
    log.drain(..boundary);
}

pub(crate) fn append_log(app: &AppHandle, state: &SharedState, text: &str) {
    if let Ok(mut guard) = state.lock() {
        guard.log.push_str(text);
        trim_log_to_limit(&mut guard.log);
    }
    emit(app, "crawl://output", &serde_json::json!({ "text": text }));
}

#[cfg(unix)]
pub(crate) fn signal_process_group(pid: u32, signal: i32) {
    // A negative PID targets the process group created for this SiteOne run.
    unsafe {
        let _ = libc::kill(-(pid as i32), signal);
    }
}

#[cfg(not(unix))]
pub(crate) fn signal_process_group(_pid: u32, _signal: i32) {}

pub(crate) fn was_cancel_requested(state: &SharedState) -> bool {
    state
        .lock()
        .map(|guard| guard.cancel_requested)
        .unwrap_or(true)
}

pub(crate) fn reserve_start(state: &SharedState) -> Result<(), String> {
    let mut guard = state
        .lock()
        .map_err(|_| "状態を更新できません".to_string())?;
    if guard.start_in_progress || guard.status.phase.is_active() {
        return Err(CrawlError::AlreadyRunning.to_string());
    }
    guard.start_in_progress = true;
    Ok(())
}

pub(crate) fn clear_start_reservation(state: &SharedState) {
    if let Ok(mut guard) = state.lock() {
        guard.start_in_progress = false;
    }
}

pub(crate) fn child_exit_code(child: &mut Child) -> Result<Option<i32>, CrawlError> {
    child
        .try_wait()
        .map(|status| status.map(|status| status.code().unwrap_or(1)))
        .map_err(|error| CrawlError::Io(error.to_string()))
}

pub(crate) fn terminate_child(child: &mut Child) {
    signal_process_group(child.id(), libc::SIGINT);
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    // Kill the entire group even if its leader already exited.
    signal_process_group(child.id(), libc::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
}

pub(crate) fn shutdown_runtime(state: &SharedState) {
    let child = {
        let mut guard = state.lock().unwrap_or_else(|error| error.into_inner());
        guard.cancel_requested = true;
        guard.child.take()
    };
    if let Some(mut child) = child {
        terminate_child(&mut child);
    }
}

pub(crate) fn write_child_stdin(child: &mut Child, payload: &str) -> Result<(), CrawlError> {
    let result = child
        .stdin
        .take()
        .ok_or_else(|| std::io::Error::new(ErrorKind::BrokenPipe, "stdin is unavailable"))
        .and_then(|mut stdin| stdin.write_all(payload.as_bytes()));
    if let Err(error) = result {
        terminate_child(child);
        let mut stderr = String::new();
        if let Some(stream) = child.stderr.take() {
            let _ = stream.take(MAX_REPORT_LENGTH).read_to_string(&mut stderr);
        }
        return Err(CrawlError::Io(format!("{error}: {stderr}")));
    }
    Ok(())
}

pub(crate) fn run_child(
    app: &AppHandle,
    state: &SharedState,
    binary: &Path,
    arguments: &[String],
    cwd: &Path,
    stdin_payload: Option<&str>,
) -> Result<i32, CrawlError> {
    let mut command = Command::new(binary);
    command
        .args(arguments)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");
    if stdin_payload.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|error| CrawlError::Io(error.to_string()))?;
    if let Some(payload) = stdin_payload {
        write_child_stdin(&mut child, payload)?;
    }
    let child_pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    match state.lock() {
        Ok(mut guard) if !guard.cancel_requested => guard.child = Some(child),
        Ok(_) => {
            terminate_child(&mut child);
            return Err(CrawlError::Io("クロールは終了中です。".into()));
        }
        Err(error) => {
            terminate_child(&mut child);
            return Err(CrawlError::Io(error.to_string()));
        }
    }

    let (sender, receiver) = mpsc::channel::<String>();
    fn forward_output<R: Read + Send + 'static>(stream: R, sender: mpsc::Sender<String>) {
        thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => {
                        let _ =
                            sender.send(String::from_utf8_lossy(&buffer[..length]).into_owned());
                    }
                }
            }
        });
    }
    if let Some(stream) = stdout {
        forward_output(stream, sender.clone());
    }
    if let Some(stream) = stderr {
        forward_output(stream, sender.clone());
    }
    drop(sender);

    let mut cancel_sent_at: Option<Instant> = None;
    loop {
        if was_cancel_requested(state) {
            if cancel_sent_at.is_none() {
                signal_process_group(child_pid, libc::SIGINT);
                cancel_sent_at = Some(Instant::now());
            } else if cancel_sent_at
                .is_some_and(|started| started.elapsed() >= Duration::from_millis(1500))
            {
                signal_process_group(child_pid, libc::SIGKILL);
            }
        }
        while let Ok(text) = receiver.try_recv() {
            append_log(app, state, &text);
        }
        let finished = match state.lock() {
            Ok(mut guard) => match guard.child.as_mut() {
                Some(child) => child_exit_code(child),
                None => return Err(CrawlError::Io("クロールプロセスが見つかりません。".into())),
            },
            Err(error) => {
                let message = error.to_string();
                drop(error);
                shutdown_runtime(state);
                return Err(CrawlError::Io(message));
            }
        };
        let finished = match finished {
            Ok(value) => value,
            Err(error) => {
                shutdown_runtime(state);
                return Err(error);
            }
        };
        if let Some(exit_code) = finished {
            // Reader threads may still be draining a final stderr chunk. Wait
            // until both streams close so no tail is lost in the UI log.
            loop {
                match receiver.recv_timeout(Duration::from_millis(50)) {
                    Ok(text) => append_log(app, state, &text),
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                }
            }
            if let Ok(mut guard) = state.lock() {
                if let Some(mut child) = guard.child.take() {
                    let _ = child.wait();
                }
            }
            return Ok(exit_code);
        }
        thread::sleep(Duration::from_millis(40));
    }
}

pub(crate) fn run_queue(
    app: AppHandle,
    state: SharedState,
    configuration: CrawlConfiguration,
    root: PathBuf,
    run_captures: Vec<CaptureViewport>,
    plans: Vec<OutputPlan>,
) {
    let enabled = run_captures;
    let binary = match locate_binary(&app) {
        Ok(path) => path,
        Err(error) => {
            if let Ok(mut guard) = state.lock() {
                guard.status.phase = RunPhase::Failed;
                guard.status.message = Some(error.to_string());
            }
            append_log(&app, &state, &format!("\n実行エラー: {}\n", error));
            emit_status(&app, &state);
            return;
        }
    };
    append_log(&app, &state, &format!("Engine: {}\n", binary.display()));
    for (index, plan) in plans.iter().enumerate() {
        if was_cancel_requested(&state) {
            break;
        }
        let viewport = enabled.get(index);
        let size_root = PathBuf::from(&plan.size_root);
        let output_directory = if viewport.is_some() {
            &plan.captures
        } else {
            &plan.size_root
        };
        if let Err(error) = fs::create_dir_all(output_directory) {
            append_log(
                &app,
                &state,
                &format!("\n保存先を作成できません: {}\n", error),
            );
            if let Ok(mut guard) = state.lock() {
                guard.status.phase = RunPhase::Failed;
                guard.status.message = Some(error.to_string());
            }
            emit_status(&app, &state);
            return;
        }
        if let Err(error) = fs::create_dir_all(&plan.http_cache_dir) {
            append_log(
                &app,
                &state,
                &format!("\nHTTP cacheを作成できません: {}\n", error),
            );
            if let Ok(mut guard) = state.lock() {
                guard.status.phase = RunPhase::Failed;
                guard.status.message = Some(error.to_string());
            }
            emit_status(&app, &state);
            return;
        }
        if let Ok(mut guard) = state.lock() {
            guard.status.size_index = if viewport.is_some() { index + 1 } else { 0 };
            guard.status.current_size_id = viewport.map(|size| size.id.clone());
            guard.status.current_size_label = viewport.map(|size| size.label.clone());
            guard.status.current_plan = Some(plan.clone());
            guard.status.message = Some(viewport.map_or_else(
                || "メタ情報を取得中".to_string(),
                |size| format!("{} を撮影中", size.label),
            ));
        }
        emit_status(&app, &state);
        let step_label = viewport.map_or_else(
            || "\nメタ情報のみ取得します（キャプチャなし）。\n".to_string(),
            |viewport| {
                format!(
                    "\n[サイズ {}/{}] {} ({}x{})\n",
                    index + 1,
                    enabled.len(),
                    viewport.label,
                    viewport.width,
                    viewport.height
                )
            },
        );
        append_log(&app, &state, &step_label);
        let arguments = match build_arguments(
            &configuration,
            viewport,
            plan,
            &timezone_or_utc(iana_time_zone::get_timezone().map_err(|_| ())),
        ) {
            Ok(arguments) => arguments,
            Err(error) => {
                append_log(&app, &state, &format!("実行引数エラー: {}\n", error));
                if let Ok(mut guard) = state.lock() {
                    guard.status.phase = RunPhase::Failed;
                    guard.status.message = Some(error.to_string());
                }
                emit_status(&app, &state);
                return;
            }
        };
        let stdin_payload = configuration.http_auth_value().ok().flatten();
        let child_result = run_child(
            &app,
            &state,
            &binary,
            &arguments,
            &size_root,
            stdin_payload.as_deref(),
        );
        if viewport.is_some() {
            match filter_non_page_screenshots(
                Path::new(&plan.json_report),
                Path::new(&plan.captures),
            ) {
                Ok(removed) if removed > 0 => append_log(
                    &app,
                    &state,
                    &format!(
                        "ページ以外のスクリーンショットを{}件除外しました。\n",
                        removed
                    ),
                ),
                Ok(_) => {}
                Err(error) => append_log(
                    &app,
                    &state,
                    &format!("スクリーンショットの整理をスキップしました: {}\n", error),
                ),
            }
        }
        // The cache is private to this size and disposable. Removing it after
        // the child exits keeps the run tree report/gallery-oriented while the
        // hidden-directory guard below protects interrupted runs.
        let _ = fs::remove_dir_all(&plan.http_cache_dir);
        match child_result {
            Ok(code) if code != 0 && !was_cancel_requested(&state) => {
                append_log(
                    &app,
                    &state,
                    &format!("SiteOne Crawler が終了コード {} で停止しました。\n", code),
                );
                if let Ok(mut guard) = state.lock() {
                    guard.status.phase = RunPhase::Failed;
                    guard.status.message = Some(format!("終了コード {}", code));
                }
                emit_status(&app, &state);
                return;
            }
            Err(error) if !was_cancel_requested(&state) => {
                append_log(&app, &state, &format!("実行エラー: {}\n", error));
                if let Ok(mut guard) = state.lock() {
                    guard.status.phase = RunPhase::Failed;
                    guard.status.message = Some(error.to_string());
                }
                emit_status(&app, &state);
                return;
            }
            _ => {}
        }
        let _ =
            list_capture_items(&root, &enabled).map(|items| emit(&app, "crawl://captures", &items));
        if state
            .lock()
            .map(|guard| guard.status.phase == RunPhase::Failed)
            .unwrap_or(true)
        {
            emit_status(&app, &state);
            return;
        }
    }

    if let Ok(mut guard) = state.lock() {
        if guard.cancel_requested {
            guard.status.phase = RunPhase::Cancelled;
            guard.status.message = Some("クロールを中止しました".to_string());
        } else {
            guard.status.phase = RunPhase::Succeeded;
            guard.status.message = Some(if enabled.is_empty() {
                "メタ情報の取得が完了しました".to_string()
            } else {
                "すべてのサイズが完了しました".to_string()
            });
        }
        guard.status.current_size_id = None;
        guard.status.current_size_label = None;
    }
    if was_cancel_requested(&state) {
        append_log(&app, &state, "\nクロールを中止しました。\n");
    } else {
        append_log(&app, &state, "\nクロールが完了しました。\n");
    }
    emit_status(&app, &state);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timezone_resolution_uses_system_result_or_utc() {
        assert_eq!(timezone_or_utc(Ok("Europe/London".into())), "Europe/London");
        assert_eq!(timezone_or_utc(Err(())), "UTC");
    }
    #[test]
    fn shutdown_reaps_child_even_when_runtime_is_poisoned() {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "trap '' INT; while :; do sleep 1; done"]);
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        let child = command.spawn().unwrap();
        let pid = child.id();
        state.lock().unwrap().child = Some(child);
        let other = Arc::clone(&state);
        let _ = thread::spawn(move || {
            let _guard = other.lock().unwrap();
            panic!("poison");
        })
        .join();
        shutdown_runtime(&state);
        let guard = state.lock().unwrap_err().into_inner();
        assert!(guard.cancel_requested);
        assert!(guard.child.is_none());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }
    #[test]
    fn failed_stdin_write_reaps_child_and_preserves_stderr() {
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "echo rejected >&2; exec 0<&-; sleep 10"])
            .stdin(Stdio::piped())
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        let mut child = command.spawn().unwrap();
        let error = write_child_stdin(&mut child, &"x".repeat(1024 * 1024)).unwrap_err();
        assert!(error.to_string().contains("rejected"));
        assert!(child.try_wait().unwrap().is_some());
    }
    #[test]
    fn log_limit_preserves_utf8_tail() {
        let mut log = format!("あ{}", "x".repeat(MAX_LOG_LENGTH - 1));
        trim_log_to_limit(&mut log);
        assert_eq!(log, "x".repeat(MAX_LOG_LENGTH - 1));
    }
    #[test]
    fn child_exit_code_keeps_running_separate_from_success_and_failure() {
        for expected in [0, 7] {
            // The pipe keeps the process running without depending on a sleep.
            let mut child = Command::new("/bin/sh")
                .args(["-c", &format!("read input; exit {}", expected)])
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            assert_eq!(child_exit_code(&mut child).unwrap(), None);
            drop(child.stdin.take());
            child.wait().unwrap();
            assert_eq!(child_exit_code(&mut child).unwrap(), Some(expected));
        }
    }
    #[test]
    fn trims_log_at_a_utf8_boundary_with_japanese_and_ascii() {
        let mut log = "あ".repeat(MAX_LOG_LENGTH / "あ".len());
        log.push_str("ASCII");

        trim_log_to_limit(&mut log);

        assert!(log.len() <= MAX_LOG_LENGTH);
        assert!(log.starts_with('あ'));
        assert!(log.ends_with("ASCII"));
    }
    #[test]
    fn start_reservation_rejects_duplicate_starts_and_can_be_released() {
        let state = Arc::new(Mutex::new(RuntimeState::default()));
        reserve_start(&state).unwrap();
        assert_eq!(
            reserve_start(&state).unwrap_err(),
            CrawlError::AlreadyRunning.to_string()
        );
        clear_start_reservation(&state);

        {
            let mut guard = state.lock().unwrap();
            assert!(!guard.start_in_progress);
            guard.status.phase = RunPhase::Running;
        }
        assert_eq!(
            reserve_start(&state).unwrap_err(),
            CrawlError::AlreadyRunning.to_string()
        );
    }
}
