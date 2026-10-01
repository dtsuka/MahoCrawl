use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use thiserror::Error;
use url::Url;

use crate::paths::expand_path;

pub(crate) const MIN_DIMENSION: u32 = 320;

pub(crate) const MAX_DIMENSION: u32 = 8192;

pub(crate) const MAX_LOG_LENGTH: usize = 400_000;

pub(crate) const MAX_REPORT_LENGTH: u64 = 16 * 1024 * 1024;

pub(crate) const CONFIG_FILENAME: &str = "configuration.json";

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

    pub(crate) fn http_auth_value(&self) -> Result<Option<String>, ValidationError> {
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

    pub(crate) fn for_storage(&self) -> Self {
        let mut stored = self.clone();
        stored.http_auth_password.clear();
        stored
    }
}

pub(crate) fn contains_disallowed_auth_char(value: &str) -> bool {
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
    pub(crate) fn as_cli(&self) -> &'static str {
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
    pub(crate) fn as_cli(&self) -> &'static str {
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
    pub(crate) fn as_cli(&self) -> &'static str {
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
    pub(crate) fn is_active(&self) -> bool {
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
pub(crate) enum CrawlError {
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

pub(crate) fn default_target_url() -> String {
    "https://example.com".to_string()
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_max_depth() -> u32 {
    2
}

pub(crate) fn default_workers() -> u32 {
    3
}

pub(crate) fn default_browser_workers() -> u32 {
    2
}

pub(crate) fn default_request_rate() -> u32 {
    5
}

pub(crate) fn default_timeout() -> u32 {
    30
}

pub(crate) fn default_output_root() -> String {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Pictures/MahoCrawl"))
        .unwrap_or_else(|| PathBuf::from("/tmp/MahoCrawl"))
        .to_string_lossy()
        .to_string()
}

pub(crate) fn default_viewports() -> Vec<CaptureViewport> {
    vec![
        CaptureViewport::desktop(),
        CaptureViewport::tablet(),
        CaptureViewport::mobile(),
    ]
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

pub(crate) fn config_path(app: &AppHandle) -> Result<PathBuf, CrawlError> {
    app.path()
        .app_config_dir()
        .map(|directory| directory.join(CONFIG_FILENAME))
        .map_err(|error| CrawlError::Configuration(error.to_string()))
}

pub(crate) fn read_configuration(path: &Path) -> Result<Option<CrawlConfiguration>, CrawlError> {
    match fs::read(path) {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|error| CrawlError::Configuration(error.to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(CrawlError::Configuration(error.to_string())),
    }
}

pub(crate) fn write_configuration(
    path: &Path,
    configuration: &CrawlConfiguration,
) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::plan;
    use crate::paths::{make_output_plans, safe_size_slug};
    use std::time::Duration;

    #[test]
    #[cfg(unix)]
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
    fn defaults_have_three_enabled_sizes() {
        let configuration = CrawlConfiguration::default();
        assert_eq!(configuration.captures.len(), 3);
        assert_eq!(configuration.enabled_captures().len(), 3);
        assert!(configuration.validate().is_ok());
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
    fn default_output_root_is_pictures_folder_in_home() {
        let expected = crate::platform::home_dir().join("Pictures").join("MahoCrawl");
        assert_eq!(PathBuf::from(default_output_root()), expected);
    }
}
