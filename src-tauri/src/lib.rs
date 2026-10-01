use base64::Engine;
use chrono::Local;
use image::{codecs::jpeg::JpegEncoder, ImageReader};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufReader, ErrorKind, Read, Write};
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
const MAX_REPORT_LENGTH: u64 = 16 * 1024 * 1024;
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
    pub http_auth_user: String,
    #[serde(default, skip_serializing)]
    pub http_auth_password: String,
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
            http_auth_user: String::new(),
            http_auth_password: String::new(),
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
        self.http_auth_value()?;
        let root = expand_path(self.output_root.trim());
        if !root.is_absolute() {
            return Err(ValidationError::OutputFolderMustBeAbsolute);
        }
        let mut ids = HashSet::new();
        let mut dimensions = HashSet::new();
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

    fn http_auth_value(&self) -> Result<Option<String>, ValidationError> {
        let user = self.http_auth_user.trim();
        let password = self.http_auth_password.as_str();
        if user.is_empty() && password.is_empty() {
            return Ok(None);
        }
        if user.is_empty() {
            return Err(ValidationError::MissingHttpAuthUser);
        }
        if user.contains(':') {
            return Err(ValidationError::InvalidHttpAuthUser);
        }
        if contains_disallowed_auth_char(user) || contains_disallowed_auth_char(password) {
            return Err(ValidationError::InvalidHttpAuthValue);
        }
        Ok(Some(format!("{user}:{password}")))
    }

    fn for_storage(&self) -> Self {
        let mut stored = self.clone();
        stored.http_auth_password.clear();
        stored
    }
}

fn contains_disallowed_auth_char(value: &str) -> bool {
    value.chars().any(|character| character.is_control())
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
pub struct ScanRunSummary {
    pub run_id: String,
    pub path: String,
    pub modified_at: u64,
    pub size_count: usize,
    pub capture_count: usize,
    pub has_html_report: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SeoPageItem {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub canonical: Option<String>,
    pub og_title: Option<String>,
    pub og_description: Option<String>,
    pub og_image: Option<String>,
    pub twitter_title: Option<String>,
    pub twitter_description: Option<String>,
    pub twitter_image: Option<String>,
    pub h1: Option<String>,
    pub h2: Option<String>,
    pub size_ids: Vec<String>,
    pub size_labels: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub capture_by_size: HashMap<String, CaptureItem>,
    pub status: Option<String>,
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
    #[error("キャプチャサイズのIDが不正です。")]
    InvalidCaptureId,
    #[error("キャプチャサイズ名は1〜80文字で指定してください。")]
    InvalidCaptureLabel,
    #[error("キャプチャサイズIDが重複しています。")]
    DuplicateCaptureId,
    #[error("キャプチャサイズの幅・高さが重複しています。")]
    DuplicateViewport,
    #[error("Basic認証のユーザー名を入力してください。")]
    MissingHttpAuthUser,
    #[error("Basic認証のユーザー名にコロンは使えません。")]
    InvalidHttpAuthUser,
    #[error("Basic認証に使用できない文字が含まれています。")]
    InvalidHttpAuthValue,
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

#[derive(Debug, Default)]
struct RuntimeState {
    status: CrawlStatus,
    log: String,
    cancel_requested: bool,
    start_in_progress: bool,
    child: Option<Child>,
    allowed_open_roots: Vec<PathBuf>,
    history_browse_roots: Vec<PathBuf>,
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

fn output_plan_for(root: &Path, viewport: Option<&CaptureViewport>) -> OutputPlan {
    let size_slug = viewport
        .map(safe_size_slug)
        .unwrap_or_else(|| "metadata".into());
    existing_output_plan(root, &root.join(size_slug))
}

fn output_plans_for_root(root: &Path, captures: &[CaptureViewport]) -> Vec<OutputPlan> {
    if captures.is_empty() {
        return vec![output_plan_for(root, None)];
    }
    captures
        .iter()
        .map(|viewport| output_plan_for(root, Some(viewport)))
        .collect()
}

fn existing_output_plan(root: &Path, size_root: &Path) -> OutputPlan {
    let size_slug = size_root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("size")
        .to_string();
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

fn infer_viewport_from_slug(slug: &str) -> Option<CaptureViewport> {
    if slug == "metadata" {
        return None;
    }
    let (label_slug, dimensions) = slug.rsplit_once('-')?;
    let (width, height) = dimensions.split_once('x')?;
    let width = width.parse::<u32>().ok()?;
    let height = height.parse::<u32>().ok()?;
    if !(MIN_DIMENSION..=MAX_DIMENSION).contains(&width)
        || !(MIN_DIMENSION..=MAX_DIMENSION).contains(&height)
        || label_slug.is_empty()
    {
        return None;
    }
    let label = match label_slug {
        "desktop" => "Desktop".to_string(),
        "tablet" => "Tablet".to_string(),
        "mobile" => "Mobile".to_string(),
        value => value.replace(['-', '_'], " "),
    };
    Some(CaptureViewport::new(slug, label, width, height))
}

fn count_capture_files(directory: &Path) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_file() && image_mime_type(&entry.path()).is_some())
        .count()
}

fn discover_scan_run(path: &Path) -> Result<Option<(ScanRunSummary, CrawlStatus)>, CrawlError> {
    if !path.is_dir() {
        return Ok(None);
    }
    let root = fs::canonicalize(path).map_err(|error| CrawlError::Io(error.to_string()))?;
    let mut size_directories = fs::read_dir(&root)
        .map_err(|error| CrawlError::Io(error.to_string()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|entry| entry.is_dir())
        .filter(|entry| {
            !entry
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with('.'))
        })
        .filter(|entry| {
            entry.join("report.html").is_file()
                || entry.join("report.json").is_file()
                || entry.join("report.txt").is_file()
                || entry.join("screenshots").is_dir()
        })
        .collect::<Vec<_>>();
    size_directories.sort();
    if size_directories.is_empty() {
        return Ok(None);
    }

    let plans = size_directories
        .iter()
        .map(|directory| existing_output_plan(&root, directory))
        .collect::<Vec<_>>();
    let run_captures = plans
        .iter()
        .filter_map(|plan| infer_viewport_from_slug(&plan.size_slug))
        .collect::<Vec<_>>();
    let capture_count = size_directories
        .iter()
        .map(|directory| count_capture_files(&directory.join("screenshots")))
        .sum();
    let modified_at = fs::metadata(&root)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    let run_id = root
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("scan")
        .to_string();
    let summary = ScanRunSummary {
        run_id: run_id.clone(),
        path: root.to_string_lossy().to_string(),
        modified_at,
        size_count: run_captures.len(),
        capture_count,
        has_html_report: plans
            .iter()
            .any(|plan| Path::new(&plan.html_report).is_file()),
    };
    let status = CrawlStatus {
        phase: RunPhase::Succeeded,
        run_id: Some(run_id),
        size_index: plans.len(),
        size_total: plans.len(),
        current_size_id: None,
        current_size_label: None,
        current_plan: plans.first().cloned(),
        plans,
        run_captures,
        message: Some("過去のスキャンを開きました".to_string()),
    };
    Ok(Some((summary, status)))
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
    viewport: Option<&CaptureViewport>,
    plan: &OutputPlan,
    timezone: &str,
) -> Result<Vec<String>, ValidationError> {
    configuration.validate()?;
    let url =
        Url::parse(configuration.target_url.trim()).map_err(|_| ValidationError::InvalidUrl)?;
    let mut arguments = vec![
        format!("--url={}", url.as_str()),
        format!("--workers={}", configuration.workers),
        format!(
            "--max-reqs-per-sec={}",
            configuration.max_requests_per_second
        ),
        format!("--http-cache-dir={}", plan.http_cache_dir),
        format!("--output-html-report={}", plan.html_report),
        format!("--output-json-file={}", plan.json_report),
        format!("--output-text-file={}", plan.text_report),
        "--extra-columns=Canonical=xpath://link[@rel='canonical']/@href(200>)".to_string(),
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
    if configuration.http_auth_value()?.is_some() {
        arguments.push("--http-auth-stdin".to_string());
    }
    if let Some(viewport) = viewport {
        arguments.extend([
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
            format!("--browser-workers={}", configuration.browser_workers),
        ]);
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
    // 開発時のみソースツリー内のバイナリを探す。リリースビルドにビルド環境の絶対パスを埋め込まない。
    #[cfg(debug_assertions)]
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

fn trim_log_to_limit(log: &mut String) {
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

fn append_log(app: &AppHandle, state: &SharedState, text: &str) {
    if let Ok(mut guard) = state.lock() {
        guard.log.push_str(text);
        trim_log_to_limit(&mut guard.log);
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

fn reserve_start(state: &SharedState) -> Result<(), String> {
    let mut guard = state
        .lock()
        .map_err(|_| "状態を更新できません".to_string())?;
    if guard.start_in_progress || guard.status.phase.is_active() {
        return Err(CrawlError::AlreadyRunning.to_string());
    }
    guard.start_in_progress = true;
    Ok(())
}

fn clear_start_reservation(state: &SharedState) {
    if let Ok(mut guard) = state.lock() {
        guard.start_in_progress = false;
    }
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

fn child_exit_code(child: &mut Child) -> Result<Option<i32>, CrawlError> {
    child
        .try_wait()
        .map(|status| status.map(|status| status.code().unwrap_or(1)))
        .map_err(|error| CrawlError::Io(error.to_string()))
}

fn run_child(
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
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(payload.as_bytes())
                .map_err(|error| CrawlError::Io(error.to_string()))?;
        }
    }
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
        let finished = {
            let mut guard = state
                .lock()
                .map_err(|error| CrawlError::Io(error.to_string()))?;
            let child = guard
                .child
                .as_mut()
                .ok_or_else(|| CrawlError::Io("クロールプロセスが見つかりません。".into()))?;
            child_exit_code(child)?
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
        let arguments = match build_arguments(&configuration, viewport, plan, "Asia/Tokyo") {
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

fn is_page_screenshot_url(raw_url: &str) -> bool {
    let Ok(url) = Url::parse(raw_url) else {
        return true;
    };
    let Some(last_segment) = url.path().rsplit('/').next() else {
        return true;
    };
    let Some((_, extension)) = last_segment.rsplit_once('.') else {
        return true;
    };
    if extension.is_empty() {
        return true;
    }
    !matches!(
        extension.to_ascii_lowercase().as_str(),
        "7z" | "7zip"
            | "aac"
            | "avif"
            | "avi"
            | "bmp"
            | "css"
            | "csv"
            | "doc"
            | "docx"
            | "eot"
            | "gif"
            | "gz"
            | "ico"
            | "jpeg"
            | "jpg"
            | "js"
            | "json"
            | "m4a"
            | "m4v"
            | "map"
            | "mjs"
            | "mov"
            | "mp3"
            | "mp4"
            | "ogg"
            | "otf"
            | "pdf"
            | "png"
            | "ppt"
            | "pptx"
            | "rar"
            | "svg"
            | "tar"
            | "tif"
            | "tiff"
            | "ttf"
            | "txt"
            | "wav"
            | "wasm"
            | "webmanifest"
            | "webm"
            | "webp"
            | "woff"
            | "woff2"
            | "xls"
            | "xlsx"
            | "xml"
            | "zip"
    )
}

/// Resolve a screenshot path from a report row without ever leaving that
/// report's own `screenshots` directory. SiteOne currently emits absolute
/// paths, but a relative path is interpreted relative to the directory that
/// contains `report.json` (for example, `screenshots/page.png`).
fn resolve_screenshot_path(
    report_path: &Path,
    screenshot_dir: &Path,
    raw_path: &str,
) -> Option<PathBuf> {
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let report_root = report_path.parent()?;
    let canonical_report_root = canonical_path(report_root)?;
    let expected_screenshot_dir = report_root.join("screenshots");
    let canonical_expected_screenshot_dir = canonical_path(&expected_screenshot_dir)?;
    let canonical_screenshot_dir = canonical_path(screenshot_dir)?;
    // Keep the caller's directory tied to this report's size folder. This
    // rejects a report row that points to another size's screenshots and also
    // rejects a `screenshots` symlink that escapes the size folder.
    if canonical_screenshot_dir != canonical_expected_screenshot_dir
        || !canonical_screenshot_dir.starts_with(&canonical_report_root)
    {
        return None;
    }

    let raw = Path::new(raw_path);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        report_root.join(raw)
    };
    let canonical_target = canonical_path(&candidate)?;
    if !canonical_target.starts_with(&canonical_screenshot_dir) {
        return None;
    }
    Some(canonical_target)
}

fn capture_item_for_path(path: &Path, viewport: &CaptureViewport) -> Option<CaptureItem> {
    image_mime_type(path)?;
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let filename = path.file_name()?.to_str()?.to_string();
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    Some(CaptureItem {
        path: path.to_string_lossy().to_string(),
        filename,
        bytes: metadata.len(),
        modified_at,
        size_id: Some(viewport.id.clone()),
        size_label: Some(viewport.label.clone()),
        width: Some(viewport.width),
        height: Some(viewport.height),
    })
}

fn filter_non_page_screenshots(
    report_path: &Path,
    screenshot_dir: &Path,
) -> Result<usize, CrawlError> {
    let metadata = fs::metadata(report_path).map_err(|error| CrawlError::Io(error.to_string()))?;
    if metadata.len() > MAX_REPORT_LENGTH {
        return Ok(0);
    }
    let data = fs::read(report_path).map_err(|error| CrawlError::Io(error.to_string()))?;
    let report: serde_json::Value = serde_json::from_slice(&data)
        .map_err(|error| CrawlError::Io(format!("JSONレポートを読み込めません: {}", error)))?;
    let mut removed = 0;
    for row in report_rows(&report, "browser-screenshots") {
        let Some(raw_url) = row.get("url").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if is_page_screenshot_url(raw_url) {
            continue;
        }
        let Some(raw_path) = row.get("path").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(path) = resolve_screenshot_path(report_path, screenshot_dir, raw_path) else {
            continue;
        };
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

fn report_rows<'a>(
    report: &'a serde_json::Value,
    table_name: &str,
) -> Vec<&'a serde_json::Map<String, serde_json::Value>> {
    report
        .get("tables")
        .and_then(|tables| tables.get(table_name))
        .and_then(|table| table.get("rows"))
        .and_then(serde_json::Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(serde_json::Value::as_object)
                .collect()
        })
        .unwrap_or_default()
}

fn row_text(row: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        row.get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn report_url(base_url: &str, raw_url: &str) -> String {
    if let Ok(url) = Url::parse(raw_url) {
        return url.to_string();
    }
    Url::parse(base_url)
        .ok()
        .and_then(|base| base.join(raw_url).ok())
        .map(|url| url.to_string())
        .unwrap_or_else(|| raw_url.to_string())
}

fn remove_html_block(value: &str, tag: &str) -> String {
    let lowered = value.to_ascii_lowercase();
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut output = String::new();
    let mut cursor = 0;
    while let Some(relative_start) = lowered[cursor..].find(&open) {
        let start = cursor + relative_start;
        output.push_str(&value[cursor..start]);
        let after_start = start + open.len();
        let Some(relative_end) = lowered[after_start..].find(&close) else {
            return output;
        };
        cursor = after_start + relative_end + close.len();
    }
    output.push_str(&value[cursor..]);
    output
}

fn normalize_heading_text(value: &str) -> String {
    let value = remove_html_block(&remove_html_block(value, "script"), "style");
    let mut output = String::new();
    let mut in_tag = false;
    for ch in value.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    let mut decoded = String::new();
    let mut remaining = output.as_str();
    while let Some(index) = remaining.find('&') {
        decoded.push_str(&remaining[..index]);
        remaining = &remaining[index..];
        if let Some(end) = remaining.find(';') {
            let entity = &remaining[1..end];
            let character = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|value| u32::from_str_radix(value, 16).ok())
                    .or_else(|| {
                        entity
                            .strip_prefix('#')
                            .and_then(|value| value.parse().ok())
                    })
                    .and_then(char::from_u32),
            };
            if let Some(character) = character {
                decoded.push(character);
                remaining = &remaining[end + 1..];
                continue;
            }
        }
        decoded.push('&');
        remaining = &remaining[1..];
    }
    decoded.push_str(remaining);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn extract_heading(value: &str, level: u8) -> Option<String> {
    let lowered = value.to_ascii_lowercase();
    let marker = format!("<h{}", level);
    let mut search_from = 0;
    while let Some(relative_start) = lowered[search_from..].find(&marker) {
        let start = search_from + relative_start;
        let Some(relative_end_tag) = lowered[start..].find('>') else {
            break;
        };
        let content_start = start + relative_end_tag + 1;
        let content_end = (1..=6)
            .filter_map(|next_level| lowered[content_start..].find(&format!("<h{}", next_level)))
            .map(|offset| content_start + offset)
            .min()
            .unwrap_or(value.len());
        let text = normalize_heading_text(&value[content_start..content_end]);
        if !text.is_empty() {
            return Some(text);
        }
        search_from = content_start;
    }
    None
}

fn size_for_report_path<'a>(
    path: &Path,
    configured: &'a [CaptureViewport],
) -> Option<&'a CaptureViewport> {
    let size_slug = path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    configured
        .iter()
        .find(|viewport| safe_size_slug(viewport) == size_slug)
}

fn ensure_seo_page(
    pages: &mut HashMap<String, SeoPageItem>,
    base_url: &str,
    row: &serde_json::Map<String, serde_json::Value>,
    size: Option<&CaptureViewport>,
) -> Option<String> {
    let raw_url = row_text(row, &["urlPathAndQuery", "url"])?;
    let url = report_url(base_url, &raw_url);
    let page = pages.entry(url.clone()).or_insert_with(|| SeoPageItem {
        url: url.clone(),
        ..SeoPageItem::default()
    });
    if let Some(viewport) = size {
        if !page.size_ids.iter().any(|id| id == &viewport.id) {
            page.size_ids.push(viewport.id.clone());
        }
        if !page
            .size_labels
            .iter()
            .any(|label| label == &viewport.label)
        {
            page.size_labels.push(viewport.label.clone());
        }
    }
    Some(url)
}

fn fill_if_missing(target: &mut Option<String>, value: Option<String>) {
    if target.is_none() {
        *target = value;
    }
}

fn parse_seo_report(
    path: &Path,
    configured: &[CaptureViewport],
) -> Result<Vec<SeoPageItem>, CrawlError> {
    let metadata = fs::metadata(path).map_err(|error| CrawlError::Io(error.to_string()))?;
    if metadata.len() > MAX_REPORT_LENGTH {
        return Err(CrawlError::Io("SEOレポートが大きすぎます。".to_string()));
    }
    let data = fs::read(path).map_err(|error| CrawlError::Io(error.to_string()))?;
    let report: serde_json::Value = serde_json::from_slice(&data)
        .map_err(|error| CrawlError::Io(format!("SEOレポートを読み込めません: {}", error)))?;
    let base_url = report
        .pointer("/options/url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let size = size_for_report_path(path, configured);
    let mut pages = HashMap::<String, SeoPageItem>::new();

    for row in report_rows(&report, "seo") {
        if let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) {
            let page = pages.get_mut(&url).expect("SEO page was inserted");
            fill_if_missing(&mut page.title, row_text(row, &["title"]));
            fill_if_missing(&mut page.description, row_text(row, &["description"]));
            fill_if_missing(
                &mut page.canonical,
                row_text(row, &["canonical", "canonicalUrl", "canonicalURL"]),
            );
            fill_if_missing(&mut page.h1, row_text(row, &["h1"]));
            fill_if_missing(&mut page.status, row_text(row, &["indexing"]));
        }
    }
    for row in report_rows(&report, "open-graph") {
        if let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) {
            let page = pages.get_mut(&url).expect("SEO page was inserted");
            fill_if_missing(&mut page.og_title, row_text(row, &["ogTitle"]));
            fill_if_missing(&mut page.og_description, row_text(row, &["ogDescription"]));
            fill_if_missing(&mut page.og_image, row_text(row, &["ogImage"]));
            fill_if_missing(&mut page.twitter_title, row_text(row, &["twitterTitle"]));
            fill_if_missing(
                &mut page.twitter_description,
                row_text(row, &["twitterDescription"]),
            );
            fill_if_missing(&mut page.twitter_image, row_text(row, &["twitterImage"]));
        }
    }
    for row in report_rows(&report, "seo-headings") {
        if let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) {
            let page = pages.get_mut(&url).expect("SEO page was inserted");
            fill_if_missing(&mut page.h1, row_text(row, &["h1"]));
            fill_if_missing(&mut page.h2, row_text(row, &["h2"]));
            if page.h1.is_none() {
                page.h1 = row_text(row, &["headings"]).and_then(|value| extract_heading(&value, 1));
            }
            if page.h2.is_none() {
                page.h2 = row_text(row, &["headings"]).and_then(|value| extract_heading(&value, 2));
            }
        }
    }
    if let Some(results) = report.get("results").and_then(serde_json::Value::as_array) {
        for result in results.iter().filter_map(serde_json::Value::as_object) {
            if result.get("type").and_then(serde_json::Value::as_i64) != Some(1) {
                continue;
            }
            let Some(raw_url) = row_text(result, &["url", "urlPathAndQuery"]) else {
                continue;
            };
            if !is_page_screenshot_url(&raw_url) {
                continue;
            }
            let Some(url) = ensure_seo_page(&mut pages, base_url, result, size) else {
                continue;
            };
            let page = pages.get_mut(&url).expect("SEO result page was inserted");
            fill_if_missing(&mut page.status, row_text(result, &["status"]));
            let canonical = result
                .get("extras")
                .and_then(serde_json::Value::as_object)
                .and_then(|extras| row_text(extras, &["Canonical", "canonical", "canonicalUrl"]));
            fill_if_missing(
                &mut page.canonical,
                canonical.map(|value| report_url(base_url, &value)),
            );
        }
    }
    if let Some(viewport) = size {
        let Some(report_root) = path.parent() else {
            return Ok(pages.into_values().collect());
        };
        let screenshot_dir = report_root.join("screenshots");
        let mut ambiguous_captures = HashSet::<(String, String)>::new();
        for row in report_rows(&report, "browser-screenshots") {
            let Some(raw_url) = row_text(row, &["url"]) else {
                continue;
            };
            if !is_page_screenshot_url(&raw_url) {
                continue;
            }
            let Some(raw_path) = row_text(row, &["path"]) else {
                continue;
            };
            let Some(capture_path) = resolve_screenshot_path(path, &screenshot_dir, &raw_path)
            else {
                continue;
            };
            let Some(capture) = capture_item_for_path(&capture_path, viewport) else {
                continue;
            };
            let Some(url) = ensure_seo_page(&mut pages, base_url, row, size) else {
                continue;
            };
            let key = (url.clone(), viewport.id.clone());
            if ambiguous_captures.contains(&key) {
                continue;
            }
            let page = pages.get_mut(&url).expect("screenshot page was inserted");
            match page.capture_by_size.get(&viewport.id) {
                None => {
                    page.capture_by_size.insert(viewport.id.clone(), capture);
                }
                Some(existing) if existing.path == capture.path => {}
                Some(_) => {
                    page.capture_by_size.remove(&viewport.id);
                    ambiguous_captures.insert(key);
                }
            }
        }
    }
    Ok(pages.into_values().collect())
}

fn collect_report_files(
    directory: &Path,
    allowed_roots: &[PathBuf],
    reports: &mut Vec<PathBuf>,
) -> Result<(), CrawlError> {
    let entries = fs::read_dir(directory).map_err(|error| CrawlError::Io(error.to_string()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !is_path_allowed(&path, allowed_roots) {
            continue;
        }
        if path.is_dir() {
            let hidden = path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.starts_with('.'));
            if !hidden {
                collect_report_files(&path, allowed_roots, reports)?;
            }
        } else if path.file_name().and_then(|value| value.to_str()) == Some("report.json")
            && is_path_allowed(&path, allowed_roots)
        {
            reports.push(path);
        }
    }
    Ok(())
}

fn list_seo_page_items(
    root: &Path,
    configured: &[CaptureViewport],
    allowed_roots: &[PathBuf],
) -> Result<Vec<SeoPageItem>, CrawlError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut reports = Vec::new();
    collect_report_files(root, allowed_roots, &mut reports)?;
    reports.sort();
    let mut pages = HashMap::<String, SeoPageItem>::new();
    let mut ambiguous_captures = HashSet::<(String, String)>::new();
    for report_path in reports {
        let Ok(report_pages) = parse_seo_report(&report_path, configured) else {
            continue;
        };
        for incoming in report_pages {
            let page = pages.entry(incoming.url.clone()).or_default();
            if page.url.is_empty() {
                page.url = incoming.url.clone();
            }
            for (target, value) in [
                (&mut page.title, incoming.title),
                (&mut page.description, incoming.description),
                (&mut page.canonical, incoming.canonical),
                (&mut page.og_title, incoming.og_title),
                (&mut page.og_description, incoming.og_description),
                (&mut page.og_image, incoming.og_image),
                (&mut page.twitter_title, incoming.twitter_title),
                (&mut page.twitter_description, incoming.twitter_description),
                (&mut page.twitter_image, incoming.twitter_image),
                (&mut page.h1, incoming.h1),
                (&mut page.h2, incoming.h2),
                (&mut page.status, incoming.status),
            ] {
                fill_if_missing(target, value);
            }
            for id in incoming.size_ids {
                if !page.size_ids.iter().any(|known| known == &id) {
                    page.size_ids.push(id);
                }
            }
            for label in incoming.size_labels {
                if !page.size_labels.iter().any(|known| known == &label) {
                    page.size_labels.push(label);
                }
            }
            for (size_id, capture) in incoming.capture_by_size {
                let key = (incoming.url.clone(), size_id.clone());
                if ambiguous_captures.contains(&key) {
                    continue;
                }
                match page.capture_by_size.get(&size_id) {
                    None => {
                        page.capture_by_size.insert(size_id, capture);
                    }
                    Some(existing) if existing.path == capture.path => {}
                    Some(_) => {
                        page.capture_by_size.remove(&size_id);
                        ambiguous_captures.insert(key);
                    }
                }
            }
        }
    }
    let mut result = pages.into_values().collect::<Vec<_>>();
    result.sort_by(|left, right| left.url.cmp(&right.url));
    Ok(result)
}

fn read_configuration(path: &Path) -> Result<Option<CrawlConfiguration>, CrawlError> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|error| CrawlError::Configuration(error.to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CrawlError::Configuration(error.to_string())),
    }
}

#[tauri::command]
fn load_configuration(app: AppHandle) -> Result<Option<CrawlConfiguration>, String> {
    let path = config_path(&app).map_err(|error| error.to_string())?;
    read_configuration(&path).map_err(|error| error.to_string())
}

fn write_configuration(path: &Path, configuration: &CrawlConfiguration) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let data = serde_json::to_vec_pretty(&configuration.for_storage())
        .map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
    let result = (|| {
        file.write_all(&data)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error: std::io::Error| error.to_string())
}

#[tauri::command]
fn save_configuration(app: AppHandle, configuration: CrawlConfiguration) -> Result<(), String> {
    configuration
        .validate()
        .map_err(|error| error.to_string())?;
    let path = config_path(&app).map_err(|error| error.to_string())?;
    write_configuration(&path, &configuration)
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
    let url = Url::parse(configuration.target_url.trim())
        .map_err(|_| ValidationError::InvalidUrl.to_string())?;
    reserve_start(&state.runtime)?;
    let setup = (|| -> Result<(PathBuf, PathBuf, Vec<OutputPlan>, String), String> {
        let requested_base = expand_path(&configuration.output_root);
        fs::create_dir_all(&requested_base).map_err(|error| error.to_string())?;
        // Resolve aliases such as ~/Pictures -> /Volumes/... before passing paths
        // to the sidecar. This keeps reports and child-process working directories
        // on the actual mounted volume, which is also the path macOS protects.
        let base = fs::canonicalize(&requested_base).unwrap_or(requested_base);
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
}

#[tauri::command]
fn list_seo_pages(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<Vec<SeoPageItem>, String> {
    let root = expand_path(root.trim());
    let (allowed, configured) = state
        .runtime
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
}

#[tauri::command]
fn list_scan_runs(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<Vec<ScanRunSummary>, String> {
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
            .runtime
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
}

#[tauri::command]
fn load_scan_run(state: tauri::State<'_, AppState>, path: String) -> Result<CrawlStatus, String> {
    let path = expand_path(path.trim());
    let browse_roots = state
        .runtime
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
        .runtime
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

// シェルを経由せずopenに渡せるHTTP(S) URLを検証する。
fn validate_external_url(raw: &str) -> Result<String, String> {
    let invalid = || "HTTPまたはHTTPSの有効なURLを指定してください。".to_string();
    if raw.chars().any(char::is_control) {
        return Err(invalid());
    }
    let trimmed = raw.trim();
    let (scheme, rest) = trimmed.split_once("://").ok_or_else(invalid)?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        || rest.is_empty()
        || rest.starts_with(['/', '\\', '?', '#'])
        || trimmed.starts_with('-')
    {
        return Err(invalid());
    }
    let parsed = Url::parse(trimmed).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none_or(str::is_empty) {
        return Err(invalid());
    }
    Ok(trimmed.to_string())
}

#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
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
fn select_scan_folder(app: AppHandle) -> Result<Option<String>, String> {
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
            list_seo_pages,
            list_scan_runs,
            load_scan_run,
            read_capture,
            read_capture_thumbnail,
            open_path,
            open_url,
            reveal_path,
            select_output_folder,
            select_scan_folder,
            clear_log,
            app_metadata,
        ])
        .run(tauri::generate_context!())
        .expect("error while running MahoCrawl");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_non_page_screenshots_are_removed_and_outside_files_preserved() {
        let root = std::env::temp_dir().join(format!("maho-relative-{}", std::process::id()));
        let shots = root.join("screenshots");
        fs::create_dir_all(&shots).unwrap();
        fs::write(shots.join("font.png"), b"image").unwrap();
        fs::write(root.join("outside.png"), b"keep").unwrap();
        let report = root.join("report.json");
        fs::write(
            &report,
            serde_json::to_vec(
                &serde_json::json!({"tables": {"browser-screenshots": {"rows": [
                    {"url":"https://example.com/font.woff2", "path":"screenshots/font.png"},
                    {"url":"https://example.com/font.woff2", "path":"outside.png"}
                ]}}}),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(filter_non_page_screenshots(&report, &shots).unwrap(), 1);
        assert!(root.join("outside.png").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovered_metadata_and_unknown_sizes_have_consistent_progress() {
        let root = std::env::temp_dir().join(format!("maho-progress-{}", std::process::id()));
        for slug in ["metadata", "custom", "desktop-1440x900"] {
            fs::create_dir_all(root.join(slug)).unwrap();
            fs::write(root.join(slug).join("report.json"), b"{}").unwrap();
        }
        let (_, status) = discover_scan_run(&root).unwrap().unwrap();
        assert_eq!(status.size_index, status.size_total);
        assert_eq!(status.size_total, 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn heading_entities_are_decoded_once() {
        assert_eq!(
            normalize_heading_text("&amp;lt; &lt; &#65; &#x1F600; &unknown;"),
            "&lt; < A 😀 &unknown;"
        );
    }

    #[test]
    fn log_limit_preserves_utf8_tail() {
        let mut log = format!("あ{}", "x".repeat(MAX_LOG_LENGTH - 1));
        trim_log_to_limit(&mut log);
        assert_eq!(log, "x".repeat(MAX_LOG_LENGTH - 1));
    }

    #[test]
    fn configuration_save_replaces_without_following_destination_symlink() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("maho-atomic-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let target = root.join("original");
        let path = root.join("configuration.json");
        fs::write(&target, b"original").unwrap();
        symlink(&target, &path).unwrap();
        write_configuration(&path, &CrawlConfiguration::default()).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"original");
        assert!(read_configuration(&path).unwrap().is_some());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_urls_allow_only_http_with_a_host() {
        for raw in [
            "https://example.com/path?q=1",
            " HTTP://example.com/path ",
            "https://例え.jp/",
        ] {
            let validated = validate_external_url(raw).unwrap();
            assert!(!validated.starts_with('-'));
            let parsed = Url::parse(&validated).unwrap();
            assert!(matches!(parsed.scheme(), "http" | "https"));
            assert!(parsed.host_str().is_some());
        }
        for raw in [
            "",
            "https://",
            "https:///path",
            "http:/example.com",
            "file:///tmp/test",
            "javascript:alert(1)",
            "-https://example.com",
            "https://exa\nmple.com",
            "\thttps://example.com",
            "https://example.com/\u{7f}",
        ] {
            assert!(
                validate_external_url(raw).unwrap_err().contains("URL"),
                "{raw:?}"
            );
        }
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
    fn discovers_an_existing_scan_and_reconstructs_its_viewports() {
        let base = std::env::temp_dir().join(format!(
            "maho-crawl-history-discovery-{}",
            std::process::id()
        ));
        let run = base.join("example.com-20260915-120000");
        let desktop = run.join("desktop-1440x900");
        let custom = run.join("landing-page-800x1200");
        let metadata = run.join("metadata");
        std::fs::create_dir_all(desktop.join("screenshots")).unwrap();
        std::fs::create_dir_all(custom.join("screenshots")).unwrap();
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::write(desktop.join("screenshots/home.png"), b"image").unwrap();
        std::fs::write(desktop.join("report.json"), b"{}").unwrap();
        std::fs::write(custom.join("report.html"), b"report").unwrap();
        std::fs::write(metadata.join("report.txt"), b"metadata").unwrap();

        let (summary, status) = discover_scan_run(&run).unwrap().unwrap();

        assert_eq!(summary.run_id, "example.com-20260915-120000");
        assert_eq!(summary.size_count, 2);
        assert_eq!(summary.capture_count, 1);
        assert!(summary.has_html_report);
        assert_eq!(status.phase, RunPhase::Succeeded);
        assert_eq!(status.plans.len(), 3);
        assert_eq!(status.run_captures[0].label, "Desktop");
        assert_eq!(status.run_captures[0].width, 1440);
        assert_eq!(status.run_captures[0].height, 900);
        assert_eq!(safe_size_slug(&status.run_captures[0]), "desktop-1440x900");
        assert_eq!(status.run_captures[1].label, "landing page");
        assert_eq!(
            safe_size_slug(&status.run_captures[1]),
            "landing-page-800x1200"
        );
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn stored_configuration_omits_http_auth_password() {
        let configuration = CrawlConfiguration {
            http_auth_user: "user".into(),
            http_auth_password: "secret".into(),
            ..Default::default()
        };
        let stored = configuration.for_storage();
        let json = serde_json::to_string(&stored).unwrap();
        assert!(!json.contains("secret"));
        assert!(json.contains("httpAuthUser"));
    }

    #[test]
    fn configuration_reader_distinguishes_missing_valid_and_invalid_data() {
        let root = std::env::temp_dir().join(format!(
            "maho-crawl-configuration-reader-{}",
            std::process::id()
        ));
        let path = root.join("configuration.json");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(read_configuration(&path).unwrap(), None);

        let expected = CrawlConfiguration::default();
        std::fs::write(&path, serde_json::to_vec(&expected).unwrap()).unwrap();
        assert_eq!(read_configuration(&path).unwrap(), Some(expected));

        std::fs::write(&path, b"{not-json").unwrap();
        assert!(read_configuration(&path).is_err());
        let _ = std::fs::remove_dir_all(root);
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

    #[test]
    fn rejects_duplicate_dimensions_but_allows_no_enabled_sizes() {
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
        assert_eq!(configuration.validate(), Ok(()));
        configuration.captures.clear();
        assert_eq!(configuration.validate(), Ok(()));
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
        let configuration = CrawlConfiguration {
            target_url: "https://example.com/path?foo=hello%20world".into(),
            user_agent: "Mozilla/5.0 Custom Agent".into(),
            max_depth: 3,
            ..Default::default()
        };
        let arguments = build_arguments(
            &configuration,
            Some(&configuration.captures[0]),
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
        assert!(arguments.contains(
            &"--extra-columns=Canonical=xpath://link[@rel='canonical']/@href(200>)".to_string()
        ));
        assert!(arguments.contains(&"--screenshot-viewport=1440x900".to_string()));
        assert!(arguments.contains(&"--timezone=Asia/Tokyo".to_string()));
    }

    #[test]
    fn single_page_excludes_depth() {
        let configuration = CrawlConfiguration {
            single_page: true,
            max_depth: 9,
            ..Default::default()
        };
        let arguments = build_arguments(
            &configuration,
            Some(&configuration.captures[0]),
            &plan(),
            "UTC",
        )
        .unwrap();
        assert!(arguments.contains(&"--single-page".to_string()));
        assert!(!arguments
            .iter()
            .any(|argument| argument.starts_with("--max-depth=")));
    }

    #[test]
    fn http_auth_is_optional_and_emitted_as_stdin_flag() {
        let mut configuration = CrawlConfiguration::default();
        let arguments = build_arguments(
            &configuration,
            Some(&configuration.captures[0]),
            &plan(),
            "UTC",
        )
        .unwrap();
        assert!(!arguments
            .iter()
            .any(|argument| argument == "--http-auth-stdin"));

        configuration.http_auth_user = " staging ".into();
        configuration.http_auth_password = "p:ass word".into();
        let arguments = build_arguments(
            &configuration,
            Some(&configuration.captures[0]),
            &plan(),
            "UTC",
        )
        .unwrap();
        assert!(arguments.contains(&"--http-auth-stdin".to_string()));
        assert_eq!(
            configuration.http_auth_value().unwrap(),
            Some("staging:p:ass word".to_string())
        );
        let stored = configuration.for_storage();
        assert!(stored.http_auth_password.is_empty());

        configuration.http_auth_user = "user:name".into();
        assert_eq!(
            configuration.validate(),
            Err(ValidationError::InvalidHttpAuthUser)
        );
        configuration.http_auth_user = String::new();
        configuration.http_auth_password = "secret".into();
        assert_eq!(
            configuration.validate(),
            Err(ValidationError::MissingHttpAuthUser)
        );
        configuration.http_auth_user = "user".into();
        configuration.http_auth_password = "secret\n".into();
        assert_eq!(
            configuration.validate(),
            Err(ValidationError::InvalidHttpAuthValue)
        );
    }

    #[test]
    fn only_supported_v251_options_are_emitted() {
        let configuration = CrawlConfiguration::default();
        let arguments = build_arguments(
            &configuration,
            Some(&configuration.captures[0]),
            &plan(),
            "UTC",
        )
        .unwrap();
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
            "--extra-columns",
            "--timezone",
            "--no-color",
            "--hide-progress-bar",
            "--single-page",
            "--max-depth",
            "--user-agent",
            "--http-auth-stdin",
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
    fn metadata_only_crawl_has_one_report_plan_without_browser_or_screenshot_options() {
        let mut configuration = CrawlConfiguration {
            user_agent: "Metadata Checker".into(),
            browser_path: "/missing/browser".into(),
            auto_download_browser: true,
            ..Default::default()
        };
        for capture in &mut configuration.captures {
            capture.enabled = false;
        }
        for captures in [configuration.captures.clone(), Vec::new()] {
            configuration.captures = captures;
            let (_, plans) = make_output_plans(&configuration, UNIX_EPOCH).unwrap();
            assert_eq!(plans.len(), 1);
            let plan = &plans[0];
            assert_eq!(plan.size_slug, "metadata");
            assert!(plan.json_report.ends_with("metadata/report.json"));
            let arguments = build_arguments(&configuration, None, plan, "Asia/Tokyo").unwrap();
            assert!(!arguments
                .iter()
                .any(|argument| argument.starts_with("--browser")
                    || argument.starts_with("--screenshot")));
            assert!(arguments.contains(&format!("--output-json-file={}", plan.json_report)));
            assert!(arguments.contains(&format!("--output-html-report={}", plan.html_report)));
            assert!(arguments.contains(&format!("--output-text-file={}", plan.text_report)));
            assert!(arguments.contains(&"--max-depth=2".to_string()));
            assert!(arguments.contains(&"--user-agent=Metadata Checker!".to_string()));
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
    fn page_screenshot_url_filter_excludes_known_static_resources() {
        assert!(is_page_screenshot_url("https://example.test/about"));
        assert!(is_page_screenshot_url("https://example.test/about.html"));
        assert!(is_page_screenshot_url(
            "https://example.test/about?view=mobile"
        ));
        assert!(!is_page_screenshot_url(
            "https://example.test/assets/fonts/site.woff2?v=1"
        ));
        assert!(!is_page_screenshot_url(
            "https://example.test/assets/images/logo.svg"
        ));
        assert!(!is_page_screenshot_url(
            "https://example.test/assets/app.js?v=1"
        ));
    }

    #[test]
    fn non_page_screenshot_filter_removes_only_reported_static_resources() {
        let root = std::env::temp_dir().join(format!(
            "maho-crawl-screenshot-filter-{}",
            std::process::id()
        ));
        let screenshot_dir = root.join("screenshots");
        let report_path = root.join("report.json");
        std::fs::create_dir_all(&screenshot_dir).unwrap();
        let page = screenshot_dir.join("page.png");
        let font = screenshot_dir.join("font.png");
        let unknown = screenshot_dir.join("unknown.png");
        std::fs::write(&page, [0_u8, 1]).unwrap();
        std::fs::write(&font, [0_u8, 1]).unwrap();
        std::fs::write(&unknown, [0_u8, 1]).unwrap();
        let report = serde_json::json!({
            "tables": {
                "browser-screenshots": {
                    "rows": [
                        { "path": page.to_string_lossy(), "url": "https://example.test/about" },
                        { "path": font.to_string_lossy(), "url": "https://example.test/assets/site.woff2" },
                        { "path": unknown.to_string_lossy(), "url": "https://example.test/assets/custom.page" }
                    ]
                }
            }
        });
        std::fs::write(&report_path, serde_json::to_vec(&report).unwrap()).unwrap();

        assert_eq!(
            filter_non_page_screenshots(&report_path, &screenshot_dir).unwrap(),
            1
        );
        assert!(page.exists());
        assert!(!font.exists());
        assert!(unknown.exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn seo_report_parser_merges_sizes_and_extracts_safe_standard_fields() {
        let root = std::env::temp_dir().join(format!("maho-crawl-seo-{}", std::process::id()));
        let desktop = root.join("desktop-1440x900");
        let tablet = root.join("tablet-768x1024");
        std::fs::create_dir_all(&desktop).unwrap();
        std::fs::create_dir_all(&tablet).unwrap();
        let report = |title: &str| {
            serde_json::json!({
                "options": { "url": "https://example.test/" },
            "tables": {
                    "seo": { "rows": [{
                        "urlPathAndQuery": "/about?from=report",
                        "title": title,
                        "description": "Description",
                        "h1": "Heading one",
                        "indexing": "Allowed"
                    }] },
                    "open-graph": { "rows": [{
                        "urlPathAndQuery": "/about?from=report",
                        "ogTitle": "OG title",
                        "ogDescription": "OG description",
                        "ogImage": "https://example.test/og.png",
                        "twitterTitle": "Twitter title"
                    }] },
                    "seo-headings": { "rows": [{
                        "urlPathAndQuery": "/about?from=report",
                        "headings": "<h1> Heading one <h2> Heading two <script>ignored</script>"
                    }] }
                },
                "results": [{
                    "url": "https://example.test/about?from=report",
                    "type": 1,
                    "status": "200",
                    "extras": { "Canonical": "https://example.test/about" }
                }, {
                    "url": "https://example.test/not-in-standard-tables",
                    "type": 1,
                    "status": "204"
                }, {
                    "url": "https://example.test/assets/site.woff2",
                    "type": 1,
                    "status": "404"
                }]
            })
        };
        std::fs::write(
            desktop.join("report.json"),
            serde_json::to_vec(&report("Desktop title")).unwrap(),
        )
        .unwrap();
        std::fs::write(
            tablet.join("report.json"),
            serde_json::to_vec(&report("Tablet title")).unwrap(),
        )
        .unwrap();

        let pages = list_seo_page_items(
            &root,
            &CrawlConfiguration::default().captures,
            std::slice::from_ref(&root),
        )
        .unwrap();
        assert_eq!(pages.len(), 2);
        let page = pages
            .iter()
            .find(|page| page.url.ends_with("/about?from=report"))
            .unwrap();
        assert_eq!(page.url, "https://example.test/about?from=report");
        assert_eq!(page.title.as_deref(), Some("Desktop title"));
        assert_eq!(page.description.as_deref(), Some("Description"));
        assert_eq!(
            page.canonical.as_deref(),
            Some("https://example.test/about")
        );
        assert_eq!(page.og_title.as_deref(), Some("OG title"));
        assert_eq!(page.twitter_title.as_deref(), Some("Twitter title"));
        assert_eq!(page.h1.as_deref(), Some("Heading one"));
        assert_eq!(page.h2.as_deref(), Some("Heading two"));
        assert_eq!(page.size_ids, vec!["desktop", "tablet"]);
        assert_eq!(page.status.as_deref(), Some("Allowed"));
        let tableless_page = pages
            .iter()
            .find(|page| page.url.ends_with("/not-in-standard-tables"))
            .unwrap();
        assert_eq!(tableless_page.title, None);
        assert_eq!(tableless_page.status.as_deref(), Some("204"));
        assert_eq!(tableless_page.size_ids, vec!["desktop", "tablet"]);
        assert!(!pages
            .iter()
            .any(|page| page.url.ends_with("/assets/site.woff2")));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn seo_report_parser_maps_page_captures_by_viewport_id_and_serializes_contract() {
        let root =
            std::env::temp_dir().join(format!("maho-crawl-seo-captures-{}", std::process::id()));
        let wide = CaptureViewport::new("wide", "Same label", 1440, 900);
        let narrow = CaptureViewport::new("narrow", "Same label", 768, 1024);
        let configured = vec![wide.clone(), narrow.clone()];
        let wide_root = root.join(safe_size_slug(&wide));
        let narrow_root = root.join(safe_size_slug(&narrow));
        let wide_screenshots = wide_root.join("screenshots");
        let narrow_screenshots = narrow_root.join("screenshots");
        std::fs::create_dir_all(&wide_screenshots).unwrap();
        std::fs::create_dir_all(&narrow_screenshots).unwrap();

        let wide_about = wide_screenshots.join("about-wide.png");
        let wide_child = wide_screenshots.join("child-wide.jpg");
        let narrow_about = narrow_screenshots.join("about-narrow.webp");
        let narrow_child = narrow_screenshots.join("child-narrow.png");
        let outside = root.join("outside.png");
        let static_resource = wide_screenshots.join("font.png");
        std::fs::write(&wide_about, [1_u8, 2, 3]).unwrap();
        std::fs::write(&wide_child, [4_u8, 5]).unwrap();
        std::fs::write(&narrow_about, [6_u8, 7, 8, 9]).unwrap();
        std::fs::write(&narrow_child, [10_u8]).unwrap();
        std::fs::write(&outside, [11_u8]).unwrap();
        std::fs::write(&static_resource, [12_u8]).unwrap();

        let report = |title: &str, screenshot_rows: Vec<serde_json::Value>| {
            serde_json::json!({
                "options": { "url": "https://example.test:8443/base/" },
                "tables": {
                    "seo": { "rows": [
                        {
                            "urlPathAndQuery": "/parent/about?from=report",
                            "title": title,
                            "indexing": "Allowed"
                        },
                        {
                            "urlPathAndQuery": "/parent/child?from=report",
                            "title": "Child title"
                        }
                    ] },
                    "browser-screenshots": { "rows": screenshot_rows }
                },
                "results": [{
                    "url": "https://example.test:8443/parent/child?from=report",
                    "type": 1,
                    "status": "200"
                }]
            })
        };
        let wide_report = report(
            "Wide title",
            vec![
                serde_json::json!({
                    "url": "https://example.test:8443/parent/about?from=report",
                    "path": "screenshots/about-wide.png"
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/child?from=report",
                    "path": wide_child.to_string_lossy()
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/assets/site.woff2",
                    "path": static_resource.to_string_lossy()
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/about?from=report",
                    "path": "../outside.png"
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/child?from=report",
                    "path": "screenshots/missing.png"
                }),
            ],
        );
        let narrow_report = report(
            "Narrow title",
            vec![
                serde_json::json!({
                    "url": "https://example.test:8443/parent/about?from=report",
                    "path": narrow_about.to_string_lossy()
                }),
                serde_json::json!({
                    "url": "https://example.test:8443/parent/child?from=report",
                    "path": narrow_child.to_string_lossy()
                }),
            ],
        );
        std::fs::write(
            wide_root.join("report.json"),
            serde_json::to_vec(&wide_report).unwrap(),
        )
        .unwrap();
        std::fs::write(
            narrow_root.join("report.json"),
            serde_json::to_vec(&narrow_report).unwrap(),
        )
        .unwrap();

        let pages = list_seo_page_items(&root, &configured, std::slice::from_ref(&root)).unwrap();
        assert_eq!(pages.len(), 2);
        let about = pages
            .iter()
            .find(|page| page.url.ends_with("/parent/about?from=report"))
            .unwrap();
        let child = pages
            .iter()
            .find(|page| page.url.ends_with("/parent/child?from=report"))
            .unwrap();
        assert_eq!(
            about.url,
            "https://example.test:8443/parent/about?from=report"
        );
        assert_eq!(about.size_ids, vec!["wide", "narrow"]);
        assert_eq!(about.size_labels, vec!["Same label"]);
        assert_eq!(about.capture_by_size.len(), 2);
        assert_eq!(about.capture_by_size["wide"].filename, "about-wide.png");
        assert_eq!(
            about.capture_by_size["narrow"].filename,
            "about-narrow.webp"
        );
        assert_eq!(child.capture_by_size.len(), 2);
        assert_eq!(child.capture_by_size["wide"].filename, "child-wide.jpg");
        assert_eq!(child.capture_by_size["narrow"].filename, "child-narrow.png");
        assert_eq!(
            child.capture_by_size["wide"].size_label.as_deref(),
            Some("Same label")
        );
        assert!(!pages
            .iter()
            .any(|page| page.url.ends_with("/assets/site.woff2")));

        let serialized = serde_json::to_value(about).unwrap();
        let captures = serialized
            .get("captureBySize")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        assert_eq!(captures["wide"]["sizeId"], "wide");
        assert_eq!(captures["wide"]["sizeLabel"], "Same label");
        assert_eq!(captures["wide"]["width"], 1440);
        assert_eq!(captures["narrow"]["sizeId"], "narrow");
        assert_eq!(captures["narrow"]["height"], 1024);

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
