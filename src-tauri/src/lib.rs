use base64::Engine;
use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, ImageReader};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufReader, Read, Write};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};
use thiserror::Error;
use url::Url;

const MIN_DIMENSION: u32 = 320;
const MAX_DIMENSION: u32 = 8192;
const MAX_LOG_LENGTH: usize = 400_000;
const CONFIG_FILENAME: &str = "configuration.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureViewport {
    pub id: String,
    pub label: String,
    pub width: u32,
    pub height: u32,
    pub enabled: bool,
}

impl CaptureViewport {
    pub fn desktop() -> Self {
        Self::new("desktop", "Desktop", 1440, 900)
    }

    pub fn tablet() -> Self {
        Self::new("tablet", "Tablet", 768, 1024)
    }

    pub fn mobile() -> Self {
        Self::new("mobile", "Mobile", 390, 844)
    }

    pub fn new(id: impl Into<String>, label: impl Into<String>, width: u32, height: u32) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            width,
            height,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrawlConfiguration {
    #[serde(default = "default_target_url")]
    pub target_url: String,
    #[serde(default)]
    pub single_page: bool,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default)]
    pub screenshot_mode: ScreenshotMode,
    #[serde(default)]
    pub screenshot_format: ScreenshotFormat,
    #[serde(default = "default_true")]
    pub hide_cookie_banner: bool,
    #[serde(default = "default_max_depth")]
    pub max_depth: u32,
    #[serde(default = "default_workers")]
    pub workers: u32,
    #[serde(default = "default_browser_workers")]
    pub browser_workers: u32,
    #[serde(default = "default_request_rate")]
    pub max_requests_per_second: u32,
    #[serde(default)]
    pub browser_wait: BrowserWait,
    #[serde(default = "default_timeout")]
    pub browser_timeout: u32,
    #[serde(default)]
    pub browser_path: String,
    #[serde(default)]
    pub auto_download_browser: bool,
    #[serde(default = "default_output_root")]
    pub output_root: String,
    #[serde(default = "default_viewports")]
    pub captures: Vec<CaptureViewport>,
}

impl Default for CrawlConfiguration {
    fn default() -> Self {
        Self {
            target_url: default_target_url(),
            single_page: false,
            user_agent: String::new(),
            screenshot_mode: ScreenshotMode::FullPage,
            screenshot_format: ScreenshotFormat::Png,
            hide_cookie_banner: true,
            max_depth: default_max_depth(),
            workers: default_workers(),
            browser_workers: default_browser_workers(),
            max_requests_per_second: default_request_rate(),
            browser_wait: BrowserWait::NetworkIdle,
            browser_timeout: default_timeout(),
            browser_path: String::new(),
            auto_download_browser: false,
            output_root: default_output_root(),
            captures: default_viewports(),
        }
    }
}

impl CrawlConfiguration {
    pub fn validate(&self) -> Result<(), ValidationError> {
        let trimmed = self.target_url.trim();
        let url = Url::parse(trimmed).map_err(|_| ValidationError::InvalidUrl)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(ValidationError::InvalidUrl);
        }
        if !(0..=20).contains(&self.max_depth) {
            return Err(ValidationError::InvalidDepth);
        }
        if !(1..=16).contains(&self.workers) || !(1..=8).contains(&self.browser_workers) {
            return Err(ValidationError::InvalidWorkerCount);
        }
        if !(1..=100).contains(&self.max_requests_per_second) {
            return Err(ValidationError::InvalidRequestRate);
        }
        if !(5..=300).contains(&self.browser_timeout) {
            return Err(ValidationError::InvalidBrowserTimeout);
        }
        if self.output_root.trim().is_empty() {
            return Err(ValidationError::MissingOutputFolder);
        }
        let root = expand_path(self.output_root.trim());
        if !root.is_absolute() {
            return Err(ValidationError::OutputFolderMustBeAbsolute);
        }
        if self.captures.is_empty() {
            return Err(ValidationError::MissingCaptureSize);
        }

        let mut ids = HashSet::new();
        let mut dimensions = HashSet::new();
        let mut enabled_count = 0;
        for viewport in &self.captures {
            if viewport.id.trim().is_empty() || viewport.id.len() > 80 {
                return Err(ValidationError::InvalidCaptureId);
            }
            if !ids.insert(viewport.id.trim().to_string()) {
                return Err(ValidationError::DuplicateCaptureId);
            }
            let label = viewport.label.trim();
            if label.is_empty() || label.len() > 80 {
                return Err(ValidationError::InvalidCaptureLabel);
            }
            if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&viewport.width)
                || !(MIN_DIMENSION..=MAX_DIMENSION).contains(&viewport.height)
            {
                return Err(ValidationError::InvalidViewport);
            }
            if !dimensions.insert((viewport.width, viewport.height)) {
                return Err(ValidationError::DuplicateViewport);
            }
            if viewport.enabled {
                enabled_count += 1;
            }
        }
        if enabled_count == 0 {
            return Err(ValidationError::NoEnabledCaptureSize);
        }
        Ok(())
    }

    pub fn enabled_captures(&self) -> Vec<CaptureViewport> {
        self.captures
            .iter()
            .filter(|item| item.enabled)
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ScreenshotMode {
    Viewport,
    #[default]
    FullPage,
}

impl ScreenshotMode {
    fn as_cli(&self) -> &'static str {
        match self {
            Self::Viewport => "viewport",
            Self::FullPage => "full-page",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScreenshotFormat {
    #[default]
    Png,
    Jpg,
    Webp,
}

impl ScreenshotFormat {
    fn as_cli(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BrowserWait {
    #[default]
    NetworkIdle,
    Load,
    DomContentLoaded,
}

impl BrowserWait {
    fn as_cli(&self) -> &'static str {
        match self {
            Self::NetworkIdle => "networkidle",
            Self::Load => "load",
            Self::DomContentLoaded => "domcontentloaded",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutputPlan {
    pub root: String,
    pub size_root: String,
    pub captures: String,
    pub http_cache_dir: String,
    pub html_report: String,
    pub json_report: String,
    pub text_report: String,
    pub size_slug: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrawlStatus {
    pub phase: RunPhase,
    pub run_id: Option<String>,
    pub size_index: usize,
    pub size_total: usize,
    pub current_size_id: Option<String>,
    pub current_size_label: Option<String>,
    pub current_plan: Option<OutputPlan>,
    pub plans: Vec<OutputPlan>,
    pub run_captures: Vec<CaptureViewport>,
    pub message: Option<String>,
}

impl Default for CrawlStatus {
    fn default() -> Self {
        Self {
            phase: RunPhase::Idle,
            run_id: None,
            size_index: 0,
            size_total: 0,
            current_size_id: None,
            current_size_label: None,
            current_plan: None,
            plans: Vec::new(),
            run_captures: Vec::new(),
            message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RunPhase {
    Idle,
    Running,
    Cancelling,
    Succeeded,
    Cancelled,
    Failed,
}

impl RunPhase {
    fn is_active(&self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureItem {
    pub path: String,
    pub filename: String,
    pub bytes: u64,
    pub modified_at: u64,
    pub size_id: Option<String>,
    pub size_label: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CaptureImage {
    pub mime_type: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StartResponse {
    pub run_id: String,
    pub root: String,
    pub total_sizes: usize,
}

#[derive(Debug, Error, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ValidationError {
    #[error("http または https で始まる有効なURLを入力してください。")]
    InvalidUrl,
    #[error("画面サイズは幅・高さとも320〜8192pxで指定してください。")]
    InvalidViewport,
    #[error("クロール深度は0〜20で指定してください。")]
    InvalidDepth,
    #[error("同時処理数が範囲外です。")]
    InvalidWorkerCount,
    #[error("1秒あたりの最大リクエスト数は1〜100で指定してください。")]
    InvalidRequestRate,
    #[error("ブラウザのタイムアウトは5〜300秒で指定してください。")]
    InvalidBrowserTimeout,
    #[error("保存先フォルダを指定してください。")]
    MissingOutputFolder,
    #[error("保存先は絶対パスで指定してください。")]
    OutputFolderMustBeAbsolute,
    #[error("キャプチャサイズを1件以上登録してください。")]
    MissingCaptureSize,
    #[error("有効なキャプチャサイズを1件以上残してください。")]
    NoEnabledCaptureSize,
    #[error("キャプチャサイズのIDが不正です。")]
    InvalidCaptureId,
    #[error("キャプチャサイズ名は1〜80文字で指定してください。")]
    InvalidCaptureLabel,
    #[error("キャプチャサイズIDが重複しています。")]
    DuplicateCaptureId,
    #[error("キャプチャサイズの幅・高さが重複しています。")]
    DuplicateViewport,
}

#[derive(Debug, Error)]
enum CrawlError {
    #[error(transparent)]
    Validation(#[from] ValidationError),
    #[error("クロールはすでに実行中です。")]
    AlreadyRunning,
    #[error("実行中のクロールがありません。")]
    NotRunning,
    #[error("SiteOne Crawler 実行ファイルが見つかりません。")]
    BinaryNotFound,
    #[error("保存先を準備できません: {0}")]
    Io(String),
    #[error("設定ファイルを読み込めません: {0}")]
    Configuration(String),
}

#[derive(Debug)]
struct RuntimeState {
    status: CrawlStatus,
    log: String,
    cancel_requested: bool,
    child: Option<Child>,
    allowed_open_roots: Vec<PathBuf>,
    run_captures: Vec<CaptureViewport>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            status: CrawlStatus::default(),
            log: String::new(),
            cancel_requested: false,
            child: None,
            allowed_open_roots: Vec::new(),
            run_captures: Vec::new(),
        }
    }
}

type SharedState = Arc<Mutex<RuntimeState>>;

pub struct AppState {
    runtime: SharedState,
}

fn default_target_url() -> String {
    "https://example.com".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_depth() -> u32 {
    2
}

fn default_workers() -> u32 {
    3
}

fn default_browser_workers() -> u32 {
    2
}

fn default_request_rate() -> u32 {
    5
}

fn default_timeout() -> u32 {
    30
}

fn default_output_root() -> String {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Pictures/MahoCrawl"))
        .unwrap_or_else(|| PathBuf::from("/tmp/MahoCrawl"))
        .to_string_lossy()
        .to_string()
}

fn default_viewports() -> Vec<CaptureViewport> {
    vec![
        CaptureViewport::desktop(),
        CaptureViewport::tablet(),
        CaptureViewport::mobile(),
    ]
}

fn expand_path(value: &str) -> PathBuf {
    if value == "~" {
        return dirs_home();
    }
    if let Some(rest) = value.strip_prefix("~/") {
        return dirs_home().join(rest);
    }
    PathBuf::from(value)
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn sanitize_component(value: &str, fallback: &str) -> String {
    let mut result = String::new();
    let mut previous_dash = false;
    for ch in value.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || matches!(lower, '-' | '_' | '.') {
            result.push(lower);
            previous_dash = false;
        } else if !previous_dash {
            result.push('-');
            previous_dash = true;
        }
    }
    let trimmed = result.trim_matches(['-', '.']);
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.chars().take(80).collect()
    }
}

pub fn safe_size_slug(viewport: &CaptureViewport) -> String {
    format!(
        "{}-{}x{}",
        sanitize_component(&viewport.label, "size"),
        viewport.width,
        viewport.height
    )
}

fn output_plan_for(root: &Path, viewport: &CaptureViewport) -> OutputPlan {
    let size_slug = safe_size_slug(viewport);
    let size_root = root.join(&size_slug);
    OutputPlan {
        root: root.to_string_lossy().to_string(),
        size_root: size_root.to_string_lossy().to_string(),
        captures: size_root.join("screenshots").to_string_lossy().to_string(),
        http_cache_dir: size_root
            .join(".siteone-http-cache")
            .to_string_lossy()
            .to_string(),
        html_report: size_root.join("report.html").to_string_lossy().to_string(),
        json_report: size_root.join("report.json").to_string_lossy().to_string(),
        text_report: size_root.join("report.txt").to_string_lossy().to_string(),
        size_slug,
    }
}

fn output_plans_for_root(root: &Path, captures: &[CaptureViewport]) -> Vec<OutputPlan> {
    captures
        .iter()
        .map(|viewport| output_plan_for(root, viewport))
        .collect()
}

pub fn make_output_plans(
    configuration: &CrawlConfiguration,
    now: SystemTime,
) -> Result<(String, Vec<OutputPlan>), ValidationError> {
    configuration.validate()?;
    let url =
        Url::parse(configuration.target_url.trim()).map_err(|_| ValidationError::InvalidUrl)?;
    let host = sanitize_component(url.host_str().unwrap_or("website"), "website");
    let timestamp = now
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let run_id = format!("{}-{}", host, timestamp);
    let root = expand_path(&configuration.output_root).join(&run_id);
    let mut plans = Vec::new();
    plans.extend(output_plans_for_root(
        &root,
        &configuration.enabled_captures(),
    ));
    Ok((root.to_string_lossy().to_string(), plans))
}

pub fn build_arguments(
    configuration: &CrawlConfiguration,
    viewport: &CaptureViewport,
    plan: &OutputPlan,
    timezone: &str,
) -> Result<Vec<String>, ValidationError> {
    configuration.validate()?;
    let url =
        Url::parse(configuration.target_url.trim()).map_err(|_| ValidationError::InvalidUrl)?;
    let mut arguments = vec![
        format!("--url={}", url.as_str()),
        "--browser".to_string(),
        "--screenshots".to_string(),
        format!("--screenshots-dir={}", plan.captures),
        format!(
            "--screenshot-mode={}",
            configuration.screenshot_mode.as_cli()
        ),
        format!(
            "--screenshot-viewport={}x{}",
            viewport.width, viewport.height
        ),
        format!(
            "--screenshot-format={}",
            configuration.screenshot_format.as_cli()
        ),
        format!("--browser-wait={}", configuration.browser_wait.as_cli()),
        format!("--browser-timeout={}", configuration.browser_timeout),
        format!("--workers={}", configuration.workers),
        format!("--browser-workers={}", configuration.browser_workers),
        format!(
            "--max-reqs-per-sec={}",
            configuration.max_requests_per_second
        ),
        format!("--http-cache-dir={}", plan.http_cache_dir),
        format!("--output-html-report={}", plan.html_report),
        format!("--output-json-file={}", plan.json_report),
        format!("--output-text-file={}", plan.text_report),
        format!("--timezone={}", timezone),
        "--no-color".to_string(),
        "--hide-progress-bar".to_string(),
    ];
    if configuration.single_page {
        arguments.push("--single-page".to_string());
    } else if configuration.max_depth > 0 {
        arguments.push(format!("--max-depth={}", configuration.max_depth));
    }
    let user_agent = configuration.user_agent.trim();
    if !user_agent.is_empty() {
        let exact = if user_agent.ends_with('!') {
            user_agent.to_string()
        } else {
            format!("{}!", user_agent)
        };
        arguments.push(format!("--user-agent={}", exact));
    }
    let browser_path = configuration.browser_path.trim();
    if !browser_path.is_empty() {
        arguments.push(format!(
            "--browser-path={}",
            expand_path(browser_path).display()
        ));
    }
    if configuration.auto_download_browser {
        arguments.push("--browser-auto-download".to_string());
    }
    // SiteOne Crawler 2.5.1 has no cookie-banner CLI flag. The value is
    // persisted and surfaced in the UI; keeping it out of argv avoids passing
    // an unsupported option that would make the crawl fail before starting.
    Ok(arguments)
}

fn binary_name() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "siteone-crawler-aarch64-apple-darwin"
    } else if cfg!(target_arch = "x86_64") {
        "siteone-crawler-x86_64-apple-darwin"
    } else {
        "siteone-crawler"
    }
}

fn locate_binary(app: &AppHandle) -> Result<PathBuf, CrawlError> {
    let mut candidates = Vec::new();
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
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join(binary_name()),
    );
    candidates.push(PathBuf::from("/opt/homebrew/bin/siteone-crawler"));
    candidates.push(PathBuf::from("/usr/local/bin/siteone-crawler"));
    candidates
        .into_iter()
        .find(|path| is_executable(path))
        .ok_or(CrawlError::BinaryNotFound)
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
        && (cfg!(not(unix)) || {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(path)
                .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

fn canonical_path(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn is_path_allowed(path: &Path, allowed_roots: &[PathBuf]) -> bool {
    let Some(canonical_target) = canonical_path(path) else {
        return false;
    };
    allowed_roots
        .iter()
        .filter_map(|root| canonical_path(root))
        .any(|root| canonical_target.starts_with(root))
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

fn clean_ansi(value: &str) -> String {
    let mut output = String::new();
    let mut escape = false;
    for ch in value.chars() {
        if escape {
            if ch.is_ascii_alphabetic() {
                escape = false;
            }
            continue;
        }
        if ch == '\u{1b}' {
            escape = true;
        } else {
            output.push(ch);
        }
    }
    output
}

fn encode_thumbnail(path: &Path) -> Result<CaptureImage, String> {
    let image = ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .decode()
        .map_err(|error| error.to_string())?;
    let thumbnail = image.thumbnail(480, 320);
    let mut bytes = Vec::new();
    let mut encoder = JpegEncoder::new_with_quality(&mut bytes, 78);
    encoder
        .encode_image(&thumbnail)
        .map_err(|error| error.to_string())?;
    Ok(CaptureImage {
        mime_type: "image/jpeg".to_string(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(bytes),
    })
}

fn config_path(app: &AppHandle) -> Result<PathBuf, CrawlError> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(CONFIG_FILENAME))
        .map_err(|error| CrawlError::Configuration(error.to_string()))
}

fn emit<T: Serialize>(app: &AppHandle, event: &str, payload: &T) {
    let _ = app.emit(event, payload);
}

fn emit_status(app: &AppHandle, state: &SharedState) {
    if let Ok(guard) = state.lock() {
        emit(app, "crawl://status", &guard.status);
    }
}

fn append_log(app: &AppHandle, state: &SharedState, text: &str) {
    if let Ok(mut guard) = state.lock() {
        guard.log.push_str(text);
        if guard.log.len() > MAX_LOG_LENGTH {
            let remove = guard.log.len() - MAX_LOG_LENGTH;
            guard.log.drain(..remove);
        }
    }
    emit(app, "crawl://output", &serde_json::json!({ "text": text }));
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) {
    // A negative PID targets the process group created for this SiteOne run.
    unsafe {
        let _ = libc::kill(-(pid as i32), signal);
    }
}

#[cfg(not(unix))]
fn signal_process_group(_pid: u32, _signal: i32) {}

fn was_cancel_requested(state: &SharedState) -> bool {
    state
        .lock()
        .map(|guard| guard.cancel_requested)
        .unwrap_or(true)
}

fn make_unique_run_root(base: &Path) -> PathBuf {
    let mut candidate = base.to_path_buf();
    let mut suffix = 2;
    while candidate.exists() {
        let name = base
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("run");
        candidate = base.with_file_name(format!("{}-{}", name, suffix));
        suffix += 1;
    }
    candidate
}

fn run_child(
    app: &AppHandle,
    state: &SharedState,
    binary: &Path,
    arguments: &[String],
    cwd: &Path,
) -> Result<i32, CrawlError> {
    let mut command = Command::new(binary);
    command
        .args(arguments)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb");
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
    let child_pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    if let Ok(mut guard) = state.lock() {
        guard.child = Some(child);
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
        let finished = state
            .lock()
            .ok()
            .and_then(|mut guard| guard.child.as_mut().and_then(|child| child.try_wait().ok()));
        if let Some(result) = finished {
            let exit_code = result.map(|status| status.code().unwrap_or(1)).unwrap_or(1);
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

fn run_queue(
    app: AppHandle,
    state: SharedState,
    configuration: CrawlConfiguration,
    run_id: String,
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
    for (index, viewport) in enabled.iter().enumerate() {
        if was_cancel_requested(&state) {
            break;
        }
        let plan = plans[index].clone();
        let size_root = PathBuf::from(&plan.size_root);
        if let Err(error) = fs::create_dir_all(&plan.captures) {
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
            guard.status.size_index = index + 1;
            guard.status.current_size_id = Some(viewport.id.clone());
            guard.status.current_size_label = Some(viewport.label.clone());
            guard.status.current_plan = Some(plan.clone());
            guard.status.message = Some(format!("{} を撮影中", viewport.label));
        }
        emit_status(&app, &state);
        append_log(
            &app,
            &state,
            &format!(
                "\n[サイズ {}/{}] {} ({}x{})\n",
                index + 1,
                enabled.len(),
                viewport.label,
                viewport.width,
                viewport.height
            ),
        );
        let arguments = match build_arguments(&configuration, viewport, &plan, "Asia/Tokyo") {
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
        let child_result = run_child(&app, &state, &binary, &arguments, &size_root);
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
            guard.status.message = Some("残りのサイズをキャンセルしました".to_string());
        } else {
            guard.status.phase = RunPhase::Succeeded;
            guard.status.message = Some("すべてのサイズが完了しました".to_string());
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
    let _ = run_id;
}

fn list_capture_items(
    root: &Path,
    configured: &[CaptureViewport],
) -> Result<Vec<CaptureItem>, CrawlError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let allowed = ["png", "jpg", "jpeg", "webp"];
    let mut captures = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries =
            fs::read_dir(&directory).map_err(|error| CrawlError::Io(error.to_string()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let is_hidden = path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|name| name.starts_with('.'));
                if !is_hidden {
                    stack.push(path);
                }
                continue;
            }
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !allowed.contains(&extension.as_str()) {
                continue;
            }
            let metadata =
                fs::metadata(&path).map_err(|error| CrawlError::Io(error.to_string()))?;
            let size_slug = path
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .and_then(|value| value.to_str())
                .unwrap_or_default();
            let matching = configured
                .iter()
                .find(|viewport| safe_size_slug(viewport) == size_slug);
            let (size_id, size_label, width, height) = matching
                .map(|viewport| {
                    (
                        Some(viewport.id.clone()),
                        Some(viewport.label.clone()),
                        Some(viewport.width),
                        Some(viewport.height),
                    )
                })
                .unwrap_or((None, None, None, None));
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs())
                .unwrap_or_default();
            captures.push(CaptureItem {
                path: path.to_string_lossy().to_string(),
                filename: path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_string(),
                bytes: metadata.len(),
                modified_at,
                size_id,
                size_label,
                width,
                height,
            });
        }
    }
    captures.sort_by(|left, right| {
        left.filename
            .to_lowercase()
            .cmp(&right.filename.to_lowercase())
    });
    Ok(captures)
}

#[tauri::command]
fn load_configuration(app: AppHandle) -> Result<CrawlConfiguration, String> {
    let path = config_path(&app).map_err(|error| error.to_string())?;
    let data = match fs::read(path) {
        Ok(data) => data,
        Err(_) => return Ok(CrawlConfiguration::default()),
    };
    serde_json::from_slice(&data).map_err(|error| error.to_string())
}

#[tauri::command]
fn save_configuration(app: AppHandle, configuration: CrawlConfiguration) -> Result<(), String> {
    configuration
        .validate()
        .map_err(|error| error.to_string())?;
    let path = config_path(&app).map_err(|error| error.to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(&configuration).map_err(|error| error.to_string())?;
    let mut file = File::create(path).map_err(|error| error.to_string())?;
    file.write_all(&data).map_err(|error| error.to_string())
}

#[tauri::command]
fn validate_configuration(configuration: CrawlConfiguration) -> Result<(), String> {
    configuration.validate().map_err(|error| error.to_string())
}

#[tauri::command]
fn get_status(state: tauri::State<'_, AppState>) -> Result<CrawlStatus, String> {
    state
        .runtime
        .lock()
        .map(|guard| guard.status.clone())
        .map_err(|_| "状態を読み込めません".to_string())
}

#[tauri::command]
fn get_log(state: tauri::State<'_, AppState>) -> Result<String, String> {
    state
        .runtime
        .lock()
        .map(|guard| guard.log.clone())
        .map_err(|_| "ログを読み込めません".to_string())
}

#[tauri::command]
fn get_engine_version(app: AppHandle) -> Result<String, String> {
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
}

#[tauri::command]
fn start_crawl(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    configuration: CrawlConfiguration,
) -> Result<StartResponse, String> {
    configuration
        .validate()
        .map_err(|error| error.to_string())?;
    let enabled_count = configuration.enabled_captures().len();
    let run_captures = configuration.enabled_captures();
    let base = expand_path(&configuration.output_root);
    fs::create_dir_all(&base).map_err(|error| error.to_string())?;
    let url = Url::parse(configuration.target_url.trim())
        .map_err(|_| ValidationError::InvalidUrl.to_string())?;
    let timestamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let base_run = base.join(format!(
        "{}-{}",
        sanitize_component(url.host_str().unwrap_or("website"), "website"),
        timestamp
    ));
    let root = make_unique_run_root(&base_run);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let plans = output_plans_for_root(&root, &run_captures);
    let run_id = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("run")
        .to_string();
    {
        let mut guard = state
            .runtime
            .lock()
            .map_err(|_| "状態を更新できません".to_string())?;
        if guard.status.phase.is_active() {
            return Err(CrawlError::AlreadyRunning.to_string());
        }
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
        guard.allowed_open_roots = vec![base.clone(), root.clone()];
    }
    let thread_state = Arc::clone(&state.runtime);
    let thread_app = app.clone();
    let thread_run_id = run_id.clone();
    let response_root = root.to_string_lossy().to_string();
    let thread_captures = run_captures.clone();
    let thread_plans = plans.clone();
    thread::spawn(move || {
        run_queue(
            thread_app,
            thread_state,
            configuration,
            thread_run_id,
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
fn stop_crawl(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
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

#[tauri::command]
fn list_captures(
    state: tauri::State<'_, AppState>,
    root: String,
    _configuration: CrawlConfiguration,
) -> Result<Vec<CaptureItem>, String> {
    let root = expand_path(root.trim());
    let (allowed, configured) = state
        .runtime
        .lock()
        .map(|guard| (guard.allowed_open_roots.clone(), guard.run_captures.clone()))
        .map_err(|_| "パスの許可情報を読み込めません。".to_string())?;
    if !is_path_allowed(&root, &allowed) {
        return Err("実行で生成した保存先以外は列挙できません。".to_string());
    }
    list_capture_items(&root, &configured).map_err(|error| error.to_string())
}

#[tauri::command]
fn read_capture(state: tauri::State<'_, AppState>, path: String) -> Result<CaptureImage, String> {
    let path = expand_path(path.trim());
    let allowed = state
        .runtime
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
}

#[tauri::command]
fn read_capture_thumbnail(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<CaptureImage, String> {
    let path = expand_path(path.trim());
    let allowed = state
        .runtime
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
}

#[tauri::command]
fn open_path(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
    let path = expand_path(path.trim());
    if !path.exists() {
        return Err("指定したパスが見つかりません。".to_string());
    }
    let allowed = state
        .runtime
        .lock()
        .map_err(|_| "パスの許可情報を読み込めません。".to_string())?
        .allowed_open_roots
        .iter()
        .any(|root| is_path_allowed(&path, std::slice::from_ref(root)));
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
fn reveal_path(state: tauri::State<'_, AppState>, path: String) -> Result<(), String> {
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
fn select_output_folder(app: AppHandle) -> Result<Option<String>, String> {
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
fn clear_log(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state
        .runtime
        .lock()
        .map(|mut guard| guard.log.clear())
        .map_err(|_| "ログを消去できません".to_string())
}

#[tauri::command]
fn app_metadata() -> serde_json::Value {
    serde_json::json!({ "name": "MahoCrawl", "version": "0.1.0", "siteOneVersion": "2.5.1" })
}

pub fn run() {
    let runtime = Arc::new(Mutex::new(RuntimeState::default()));
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { runtime })
        .invoke_handler(tauri::generate_handler![
            load_configuration,
            save_configuration,
            validate_configuration,
            get_status,
            get_log,
            get_engine_version,
            start_crawl,
            stop_crawl,
            list_captures,
            read_capture,
            read_capture_thumbnail,
            open_path,
            reveal_path,
            select_output_folder,
            clear_log,
            app_metadata,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MahoCrawl");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> OutputPlan {
        OutputPlan {
            root: "/tmp/Maho Crawl/run".into(),
            size_root: "/tmp/Maho Crawl/run/desktop-1440x900".into(),
            captures: "/tmp/Maho Crawl/run/desktop-1440x900/screenshots".into(),
            http_cache_dir: "/tmp/Maho Crawl/run/desktop-1440x900/.siteone-http-cache".into(),
            html_report: "/tmp/Maho Crawl/run/desktop-1440x900/report.html".into(),
            json_report: "/tmp/Maho Crawl/run/desktop-1440x900/report.json".into(),
            text_report: "/tmp/Maho Crawl/run/desktop-1440x900/report.txt".into(),
            size_slug: "desktop-1440x900".into(),
        }
    }

    #[test]
    fn defaults_have_three_enabled_sizes() {
        let configuration = CrawlConfiguration::default();
        assert_eq!(configuration.captures.len(), 3);
        assert_eq!(configuration.enabled_captures().len(), 3);
        assert!(configuration.validate().is_ok());
    }

    #[test]
    fn rejects_duplicate_dimensions_and_no_enabled_sizes() {
        let mut configuration = CrawlConfiguration::default();
        configuration.captures[1].width = 1440;
        configuration.captures[1].height = 900;
        assert_eq!(
            configuration.validate(),
            Err(ValidationError::DuplicateViewport)
        );
        configuration.captures[1].width = 768;
        configuration.captures[1].height = 1024;
        for viewport in &mut configuration.captures {
            viewport.enabled = false;
        }
        assert_eq!(
            configuration.validate(),
            Err(ValidationError::NoEnabledCaptureSize)
        );
    }

    #[test]
    fn safe_size_slug_removes_unsafe_characters_and_keeps_dimensions() {
        let viewport = CaptureViewport::new("id", "My Phone / 2", 390, 844);
        assert_eq!(safe_size_slug(&viewport), "my-phone-2-390x844");
        let punctuation = CaptureViewport::new("id", "...", 390, 844);
        assert_eq!(safe_size_slug(&punctuation), "size-390x844");
    }

    #[test]
    fn command_builder_preserves_paths_as_one_arguments_and_exact_ua() {
        let mut configuration = CrawlConfiguration::default();
        configuration.target_url = "https://example.com/path?foo=hello%20world".into();
        configuration.user_agent = "Mozilla/5.0 Custom Agent".into();
        configuration.max_depth = 3;
        let arguments = build_arguments(
            &configuration,
            &configuration.captures[0],
            &plan(),
            "Asia/Tokyo",
        )
        .unwrap();
        assert!(arguments.contains(&"--url=https://example.com/path?foo=hello%20world".to_string()));
        assert!(arguments.contains(&"--user-agent=Mozilla/5.0 Custom Agent!".to_string()));
        assert!(arguments.contains(
            &"--screenshots-dir=/tmp/Maho Crawl/run/desktop-1440x900/screenshots".to_string()
        ));
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| argument.starts_with("--http-cache-dir="))
                .count(),
            1
        );
        assert!(arguments.contains(
            &"--http-cache-dir=/tmp/Maho Crawl/run/desktop-1440x900/.siteone-http-cache"
                .to_string()
        ));
        assert!(arguments.contains(&"--screenshot-viewport=1440x900".to_string()));
        assert!(arguments.contains(&"--timezone=Asia/Tokyo".to_string()));
    }

    #[test]
    fn single_page_excludes_depth() {
        let mut configuration = CrawlConfiguration::default();
        configuration.single_page = true;
        configuration.max_depth = 9;
        let arguments =
            build_arguments(&configuration, &configuration.captures[0], &plan(), "UTC").unwrap();
        assert!(arguments.contains(&"--single-page".to_string()));
        assert!(!arguments
            .iter()
            .any(|argument| argument.starts_with("--max-depth=")));
    }

    #[test]
    fn only_supported_v251_options_are_emitted() {
        let configuration = CrawlConfiguration::default();
        let arguments =
            build_arguments(&configuration, &configuration.captures[0], &plan(), "UTC").unwrap();
        let supported: HashSet<&str> = [
            "--url",
            "--browser",
            "--screenshots",
            "--screenshots-dir",
            "--screenshot-mode",
            "--screenshot-viewport",
            "--screenshot-format",
            "--browser-wait",
            "--browser-timeout",
            "--workers",
            "--browser-workers",
            "--max-reqs-per-sec",
            "--http-cache-dir",
            "--output-html-report",
            "--output-json-file",
            "--output-text-file",
            "--timezone",
            "--no-color",
            "--hide-progress-bar",
            "--single-page",
            "--max-depth",
            "--user-agent",
            "--browser-path",
            "--browser-auto-download",
        ]
        .into_iter()
        .collect();
        for argument in arguments {
            let option = argument.split('=').next().unwrap_or_default();
            assert!(supported.contains(option), "unsupported option: {}", option);
        }
    }

    #[test]
    fn output_plan_is_partitioned_by_size() {
        let configuration = CrawlConfiguration::default();
        let (root, plans) =
            make_output_plans(&configuration, UNIX_EPOCH + Duration::from_secs(1234)).unwrap();
        assert!(root.ends_with("example.com-1234"));
        assert_eq!(plans.len(), 3);
        assert!(plans[0].captures.ends_with("desktop-1440x900/screenshots"));
        assert!(plans[0]
            .http_cache_dir
            .ends_with("desktop-1440x900/.siteone-http-cache"));
        assert!(plans[1]
            .html_report
            .ends_with("tablet-768x1024/report.html"));
        assert!(plans[2].json_report.ends_with("mobile-390x844/report.json"));
    }

    #[test]
    fn output_plan_keeps_started_viewport_snapshot_after_configuration_edit() {
        let configuration = CrawlConfiguration::default();
        let (_, plans) =
            make_output_plans(&configuration, UNIX_EPOCH + Duration::from_secs(1234)).unwrap();
        let mut edited = configuration.clone();
        edited.captures[0].label = "Renamed after run".into();
        edited.captures[0].width = 1600;
        assert_eq!(plans[0].size_slug, "desktop-1440x900");
        assert_ne!(safe_size_slug(&edited.captures[0]), plans[0].size_slug);
    }

    #[test]
    fn canonical_open_boundary_rejects_parent_and_accepts_nested_file() {
        let root = std::env::temp_dir().join(format!("maho-crawl-boundary-{}", std::process::id()));
        let nested = root.join("run/desktop-1440x900/screenshots");
        std::fs::create_dir_all(&nested).unwrap();
        let image = nested.join("page.png");
        std::fs::write(&image, [0_u8, 1, 2]).unwrap();
        let allowed = vec![root.join("run")];
        assert!(is_path_allowed(&image, &allowed));
        assert!(!is_path_allowed(&root.join("outside.png"), &allowed));
        assert!(!is_path_allowed(&nested.join("../report.html"), &allowed));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn capture_listing_is_recursive_but_ignores_non_images() {
        let root = std::env::temp_dir().join(format!("maho-crawl-captures-{}", std::process::id()));
        let nested = root.join("desktop-1440x900/screenshots");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("one.png"), [0_u8, 1]).unwrap();
        std::fs::write(nested.join("two.webp"), [0_u8, 1, 2]).unwrap();
        let hidden = root.join(".siteone-http-cache");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("cached.png"), [0_u8, 1, 2]).unwrap();
        std::fs::write(nested.join("ignore.txt"), [0_u8]).unwrap();
        let mut config = CrawlConfiguration::default();
        config.captures.truncate(1);
        let items = list_capture_items(&root, &config.captures).unwrap();
        assert_eq!(
            items
                .iter()
                .map(|item| item.filename.as_str())
                .collect::<Vec<_>>(),
            vec!["one.png", "two.webp"]
        );
        assert_eq!(items[0].size_id.as_deref(), Some("desktop"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn thumbnail_encoder_reduces_dimensions_and_returns_jpeg() {
        let root =
            std::env::temp_dir().join(format!("maho-crawl-thumbnail-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let image_path = root.join("large.png");
        let image = image::RgbImage::from_pixel(1200, 900, image::Rgb([40, 80, 120]));
        image.save(&image_path).unwrap();
        let thumbnail = encode_thumbnail(&image_path).unwrap();
        assert_eq!(thumbnail.mime_type, "image/jpeg");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(thumbnail.data_base64)
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert!(decoded.width() <= 480);
        assert!(decoded.height() <= 320);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn ansi_cleaner_removes_siteone_version_color_codes() {
        assert_eq!(
            clean_ansi("\u{1b}[0;34mVersion: 2.5.1\u{1b}[0m"),
            "Version: 2.5.1"
        );
    }
}
