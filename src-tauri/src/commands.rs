use base64::Engine;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::thread;
use std::time::SystemTime;
use tauri::AppHandle;

use crate::config::{
    config_path, read_configuration, write_configuration, CaptureImage, CaptureItem,
    CrawlConfiguration, CrawlError, CrawlStatus, OutputPlan, RunPhase, ScanRunSummary, SeoPageItem,
    StartResponse,
};
use crate::paths::{
    discover_scan_run, expand_path, is_open_path_allowed, is_path_allowed, make_output_plans,
    register_output_paths, validate_external_url,
};
use crate::process::{
    clear_start_reservation, emit_status, locate_binary, reserve_start, run_queue,
    signal_process_group, AppState,
};
use crate::report::{
    clean_ansi, encode_thumbnail, image_mime_type, list_capture_items, list_seo_page_items,
};

#[tauri::command]
pub(crate) fn load_configuration(app: AppHandle) -> Result<Option<CrawlConfiguration>, String> {
    let path = config_path(&app).map_err(|error| error.to_string())?;
    read_configuration(&path).map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn save_configuration(
    app: AppHandle,
    configuration: CrawlConfiguration,
) -> Result<(), String> {
    configuration
        .validate()
        .map_err(|error| error.to_string())?;
    let path = config_path(&app).map_err(|error| error.to_string())?;
    write_configuration(&path, &configuration)
}

#[tauri::command]
pub(crate) fn validate_configuration(configuration: CrawlConfiguration) -> Result<(), String> {
    configuration.validate().map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn get_status(state: tauri::State<'_, AppState>) -> Result<CrawlStatus, String> {
    state
        .runtime
        .lock()
        .map(|guard| guard.status.clone())
        .map_err(|_| "状態を読み込めません".to_string())
}

#[tauri::command]
pub(crate) fn get_log(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state
        .runtime
        .lock()
        .map(|guard| guard.log.clone())
        .map_err(|_| "ログを読み込めません".to_string())
}

#[tauri::command(async)]
pub(crate) async fn get_engine_version(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let binary = locate_binary(&app).map_err(|error| error.to_string())?;
        let output = Command::new(binary)
            .arg("--version")
            .output()
            .map_err(|error| error.to_string())?;
        let text = clean_ansi(&String::from_utf8_lossy(&output.stdout));
        Ok(text
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("SiteOne Crawler 2.5.1")
            .trim()
            .to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn start_crawl(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    configuration: CrawlConfiguration,
) -> Result<StartResponse, String> {
    configuration
        .validate()
        .map_err(|error| error.to_string())?;
    let enabled_count = configuration.enabled_captures().len();
    let run_captures = configuration.enabled_captures();
    reserve_start(&state.runtime)?;
    let setup = (|| -> Result<(PathBuf, PathBuf, Vec<OutputPlan>, String), String> {
        let requested_base = expand_path(&configuration.output_root);
        fs::create_dir_all(&requested_base).map_err(|error| error.to_string())?;
        // Resolve aliases such as ~/Pictures -> /Volumes/... before passing paths
        // to the sidecar. This keeps reports and child-process working directories
        // on the actual mounted volume, which is also the path macOS protects.
        let base = fs::canonicalize(&requested_base).unwrap_or(requested_base);
        let planning_configuration = CrawlConfiguration {
            output_root: base.to_string_lossy().into_owned(),
            ..configuration.clone()
        };
        let (root, plans) = make_output_plans(&planning_configuration, SystemTime::now())
            .map_err(|error| error.to_string())?;
        let root = PathBuf::from(root);
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let run_id = root
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("run")
            .to_string();
        Ok((base, root, plans, run_id))
    })();
    let (base, root, plans, run_id) = match setup {
        Ok(values) => values,
        Err(error) => {
            clear_start_reservation(&state.runtime);
            return Err(error);
        }
    };
    {
        let mut guard = state
            .runtime
            .lock()
            .map_err(|_| "状態を更新できません".to_string())?;
        guard.start_in_progress = false;
        guard.cancel_requested = false;
        guard.log.clear();
        guard.status = CrawlStatus {
            phase: RunPhase::Running,
            run_id: Some(run_id.clone()),
            size_index: 0,
            size_total: enabled_count,
            current_size_id: None,
            current_size_label: None,
            current_plan: None,
            plans: plans.clone(),
            run_captures: run_captures.clone(),
            message: Some("準備中".to_string()),
        };
    }
    emit_status(&app, &state.runtime);
    if let Ok(mut guard) = state.runtime.lock() {
        register_output_paths(&mut guard, &base, &root);
    }
    let thread_state = Arc::clone(&state.runtime);
    let thread_app = app.clone();
    let response_root = root.to_string_lossy().to_string();
    let thread_captures = run_captures.clone();
    let thread_plans = plans.clone();
    thread::spawn(move || {
        run_queue(
            thread_app,
            thread_state,
            configuration,
            root,
            thread_captures,
            thread_plans,
        )
    });
    Ok(StartResponse {
        run_id,
        root: response_root,
        total_sizes: enabled_count,
    })
}

#[tauri::command]
pub(crate) fn stop_crawl(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut guard = state
        .runtime
        .lock()
        .map_err(|_| "状態を更新できません".to_string())?;
    if !guard.status.phase.is_active() {
        return Err(CrawlError::NotRunning.to_string());
    }
    guard.cancel_requested = true;
    guard.status.phase = RunPhase::Cancelling;
    guard.status.message = Some("停止処理中".to_string());
    if let Some(child) = guard.child.as_mut() {
        signal_process_group(child.id(), libc::SIGINT);
    }
    drop(guard);
    emit_status(&app, &state.runtime);
    Ok(())
}

#[tauri::command(async)]
pub(crate) async fn list_captures(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<Vec<CaptureItem>, String> {
    let state = Arc::clone(&state.runtime);
    tauri::async_runtime::spawn_blocking(move || {
        let root = expand_path(root.trim());
        let (allowed, configured) = state
            .lock()
            .map(|guard| {
                (
                    guard.allowed_open_roots.clone(),
                    guard.status.run_captures.clone(),
                )
            })
            .map_err(|_| "パスの許可情報を読み込めません。".to_string())?;
        if !is_path_allowed(&root, &allowed) {
            return Err("実行で生成した保存先以外は列挙できません。".to_string());
        }
        list_capture_items(&root, &configured).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command(async)]
pub(crate) async fn list_seo_pages(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<Vec<SeoPageItem>, String> {
    let state = Arc::clone(&state.runtime);
    tauri::async_runtime::spawn_blocking(move || {
        let root = expand_path(root.trim());
        let (allowed, configured) = state
            .lock()
            .map(|guard| {
                (
                    guard.allowed_open_roots.clone(),
                    guard.status.run_captures.clone(),
                )
            })
            .map_err(|_| "パスの許可情報を読み込めません。".to_string())?;
        if !is_path_allowed(&root, &allowed) {
            return Err("実行で生成した保存先以外はSEOレポートを読み込めません。".to_string());
        }
        list_seo_page_items(&root, &configured, &allowed).map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command(async)]
pub(crate) async fn list_scan_runs(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<Vec<ScanRunSummary>, String> {
    let state = Arc::clone(&state.runtime);
    tauri::async_runtime::spawn_blocking(move || {
        let requested = expand_path(root.trim());
        if !requested.exists() {
            return Ok(Vec::new());
        }
        let root = fs::canonicalize(&requested).map_err(|error| error.to_string())?;
        if !root.is_dir() {
            return Err("スキャンの保存先フォルダを指定してください。".to_string());
        }
        {
            let mut guard = state
                .lock()
                .map_err(|_| "履歴フォルダの許可情報を更新できません。".to_string())?;
            if !guard
                .history_browse_roots
                .iter()
                .any(|known| known == &root)
            {
                guard.history_browse_roots.push(root.clone());
            }
        }

        if let Some((summary, _)) = discover_scan_run(&root).map_err(|error| error.to_string())? {
            return Ok(vec![summary]);
        }

        let mut runs = fs::read_dir(&root)
            .map_err(|error| error.to_string())?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter_map(|path| {
                discover_scan_run(&path)
                    .ok()
                    .flatten()
                    .map(|(summary, _)| summary)
            })
            .collect::<Vec<_>>();
        runs.sort_by(|left, right| {
            right
                .modified_at
                .cmp(&left.modified_at)
                .then_with(|| right.run_id.cmp(&left.run_id))
        });
        runs.truncate(500);
        Ok(runs)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command(async)]
pub(crate) async fn load_scan_run(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<CrawlStatus, String> {
    let state = Arc::clone(&state.runtime);
    tauri::async_runtime::spawn_blocking(move || {
        let path = expand_path(path.trim());
        let browse_roots = state
            .lock()
            .map(|guard| {
                if guard.status.phase.is_active() || guard.start_in_progress {
                    return Err("クロール実行中は過去のスキャンを開けません。".to_string());
                }
                Ok(guard.history_browse_roots.clone())
            })
            .map_err(|_| "履歴フォルダの許可情報を読み込めません。".to_string())??;
        if !is_path_allowed(&path, &browse_roots) {
            return Err("一覧に表示されたスキャン以外は開けません。".to_string());
        }
        let root = fs::canonicalize(&path).map_err(|error| error.to_string())?;
        let (_, status) = discover_scan_run(&root)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "MahoCrawlのスキャン結果が見つかりません。".to_string())?;
        let mut guard = state
            .lock()
            .map_err(|_| "過去のスキャンを読み込めません。".to_string())?;
        if guard.status.phase.is_active() || guard.start_in_progress {
            return Err("クロール実行中は過去のスキャンを開けません。".to_string());
        }
        guard.status = status.clone();
        guard.log.clear();
        if !guard.allowed_open_roots.iter().any(|known| known == &root) {
            guard.allowed_open_roots.push(root);
        }
        Ok(status)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command(async)]
pub(crate) async fn read_capture(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<CaptureImage, String> {
    let state = Arc::clone(&state.runtime);
    tauri::async_runtime::spawn_blocking(move || {
        let path = expand_path(path.trim());
        let allowed = state
            .lock()
            .map_err(|_| "パスの許可情報を読み込めません。".to_string())?
            .allowed_open_roots
            .clone();
        if !is_path_allowed(&path, &allowed) {
            return Err("実行で生成した保存先以外は読み込めません。".to_string());
        }
        let mime_type =
            image_mime_type(&path).ok_or_else(|| "対応していない画像形式です。".to_string())?;
        let metadata = fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.len() > 24 * 1024 * 1024 {
            return Err("画像が大きすぎるためプレビューできません。".to_string());
        }
        let bytes = fs::read(&path).map_err(|error| error.to_string())?;
        Ok(CaptureImage {
            mime_type: mime_type.to_string(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command(async)]
pub(crate) async fn read_capture_thumbnail(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<CaptureImage, String> {
    let state = Arc::clone(&state.runtime);
    tauri::async_runtime::spawn_blocking(move || {
        let path = expand_path(path.trim());
        let allowed = state
            .lock()
            .map_err(|_| "パスの許可情報を読み込めません。".to_string())?
            .allowed_open_roots
            .clone();
        if !is_path_allowed(&path, &allowed) {
            return Err("実行で生成した保存先以外は読み込めません。".to_string());
        }
        if image_mime_type(&path).is_none() {
            return Err("対応していない画像形式です。".to_string());
        }
        encode_thumbnail(&path)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) fn open_path(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    let path = expand_path(path.trim());
    if !path.exists() {
        return Err("指定したパスが見つかりません。".to_string());
    }
    let allowed = state
        .runtime
        .lock()
        .map(|guard| is_open_path_allowed(&path, &guard))
        .map_err(|_| "パスの許可情報を読み込めません。".to_string())?;
    if !allowed {
        return Err("実行で生成した保存先以外は開けません。".to_string());
    }
    Command::new("open")
        .arg(path)
        .status()
        .map_err(|error| error.to_string())
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err("パスを開けませんでした。".to_string())
            }
        })
}

#[tauri::command]
pub(crate) fn reveal_path(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    let path = expand_path(path.trim());
    let parent = path
        .parent()
        .ok_or_else(|| "親フォルダが見つかりません。".to_string())?
        .to_path_buf();
    let allowed = state
        .runtime
        .lock()
        .map_err(|_| "パスの許可情報を読み込めません。".to_string())?
        .allowed_open_roots
        .clone();
    if !is_path_allowed(&path, &allowed) {
        return Err("実行で生成した保存先以外は表示できません。".to_string());
    }
    Command::new("open")
        .arg("-R")
        .arg(path)
        .status()
        .map_err(|error| error.to_string())
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "フォルダを表示できませんでした: {}",
                    parent.display()
                ))
            }
        })
}

#[tauri::command]
pub(crate) fn open_url(url: String) -> Result<(), String> {
    let validated = validate_external_url(&url)?;
    let status = Command::new("open")
        .arg(validated)
        .status()
        .map_err(|error| format!("サイトを開けませんでした: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("サイトを開けませんでした。".to_string())
    }
}

#[tauri::command]
pub(crate) fn select_output_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let selected = app
        .dialog()
        .file()
        .set_title("保存先を選択")
        .blocking_pick_folder();
    Ok(selected
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub(crate) fn select_scan_folder(app: AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let selected = app
        .dialog()
        .file()
        .set_title("スキャン結果のフォルダを選択")
        .blocking_pick_folder();
    Ok(selected
        .and_then(|path| path.into_path().ok())
        .map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub(crate) fn clear_log(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .runtime
        .lock()
        .map(|mut guard| guard.log.clear())
        .map_err(|_| "ログを消去できません".to_string())
}

#[cfg(test)]
mod tests {

    #[test]
    fn entrypoint_delegates_to_focused_modules() {
        let entrypoint = include_str!("lib.rs");
        assert!(
            entrypoint.lines().count() < 200,
            "entrypoint still contains all implementation"
        );
        for name in ["config", "paths", "process", "report", "commands"] {
            assert!(entrypoint.contains(&format!("mod {name};")));
        }
    }
    #[test]
    fn desktop_command_contract_uses_background_dispatch_and_minimal_permissions() {
        let source = include_str!("commands.rs");
        for name in [
            "read_capture",
            "read_capture_thumbnail",
            "list_seo_pages",
            "list_scan_runs",
            "load_scan_run",
            "list_captures",
            "get_engine_version",
        ] {
            assert!(
                source.contains(&format!(
                    "#[tauri::command(async)]\npub(crate) async fn {name}("
                )),
                "{name} must dispatch off the UI thread"
            );
        }
        let capabilities: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        assert!(!capabilities["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "dialog:allow-open"));
        assert!(!source.contains(&["fn app_", "metadata("].concat()));
    }
}
