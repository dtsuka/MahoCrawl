use std::sync::{Arc, Mutex};

mod commands;
mod config;
mod paths;
mod platform;
mod process;
mod report;

pub use config::{
    build_arguments, BrowserWait, CaptureImage, CaptureItem, CaptureViewport, CrawlConfiguration,
    CrawlStatus, OutputPlan, RunPhase, ScanRunSummary, ScreenshotFormat, ScreenshotMode,
    SeoPageItem, StartResponse, ValidationError,
};
pub use paths::{make_output_plans, safe_size_slug};
pub use process::AppState;
use process::{shutdown_runtime, RuntimeState};

pub fn run() {
    let runtime = Arc::new(Mutex::new(RuntimeState::default()));
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            runtime: Arc::clone(&runtime),
        })
        .invoke_handler(tauri::generate_handler![
            commands::load_configuration,
            commands::save_configuration,
            commands::validate_configuration,
            commands::get_status,
            commands::get_log,
            commands::get_engine_version,
            commands::start_crawl,
            commands::stop_crawl,
            commands::list_captures,
            commands::list_seo_pages,
            commands::list_scan_runs,
            commands::load_scan_run,
            commands::read_capture,
            commands::read_capture_thumbnail,
            commands::open_path,
            commands::open_url,
            commands::reveal_path,
            commands::select_output_folder,
            commands::select_scan_folder,
            commands::clear_log,
        ])
        .build(tauri::generate_context!())
        .expect("error while building MahoCrawl")
        .run(move |_app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                shutdown_runtime(&runtime);
            }
        });
}
