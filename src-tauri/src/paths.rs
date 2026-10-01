use chrono::Local;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

use crate::config::{
    CaptureViewport, CrawlConfiguration, CrawlError, CrawlStatus, OutputPlan, RunPhase,
    ScanRunSummary, ValidationError, MAX_DIMENSION, MIN_DIMENSION,
};
use crate::process::RuntimeState;
use crate::report::image_mime_type;

pub(crate) fn expand_path(value: &str) -> PathBuf {
    if value == "~" {
        return dirs_home();
    }
    if let Some(rest) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    {
        return dirs_home().join(rest);
    }
    PathBuf::from(value)
}

pub(crate) fn dirs_home() -> PathBuf {
    crate::platform::home_dir()
}

pub(crate) fn sanitize_component(value: &str, fallback: &str) -> String {
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

pub(crate) fn output_plan_for(root: &Path, viewport: Option<&CaptureViewport>) -> OutputPlan {
    let size_slug = viewport
        .map(safe_size_slug)
        .unwrap_or_else(|| "metadata".into());
    existing_output_plan(root, &root.join(size_slug))
}

pub(crate) fn output_plans_for_root(root: &Path, captures: &[CaptureViewport]) -> Vec<OutputPlan> {
    if captures.is_empty() {
        return vec![output_plan_for(root, None)];
    }
    captures
        .iter()
        .map(|viewport| output_plan_for(root, Some(viewport)))
        .collect()
}

pub(crate) fn existing_output_plan(root: &Path, size_root: &Path) -> OutputPlan {
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

pub(crate) fn infer_viewport_from_slug(slug: &str) -> Option<CaptureViewport> {
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

pub(crate) fn count_capture_files(directory: &Path) -> usize {
    let Ok(entries) = fs::read_dir(directory) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_file() && image_mime_type(&entry.path()).is_some())
        .count()
}

pub(crate) fn discover_scan_run(
    path: &Path,
) -> Result<Option<(ScanRunSummary, CrawlStatus)>, CrawlError> {
    if !path.is_dir() {
        return Ok(None);
    }
    let root =
        crate::platform::canonicalize(path).map_err(|error| CrawlError::Io(error.to_string()))?;
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
    let timestamp = chrono::DateTime::<Local>::from(now).format("%Y%m%d-%H%M%S");
    let run_id = format!("{host}-{timestamp}");
    let root = make_unique_run_root(&expand_path(&configuration.output_root).join(&run_id));
    let mut plans = Vec::new();
    plans.extend(output_plans_for_root(
        &root,
        &configuration.enabled_captures(),
    ));
    Ok((root.to_string_lossy().to_string(), plans))
}

pub(crate) fn canonical_path(path: &Path) -> Option<PathBuf> {
    crate::platform::canonicalize(path).ok()
}

pub(crate) fn is_path_allowed(path: &Path, allowed_roots: &[PathBuf]) -> bool {
    let Some(canonical_target) = canonical_path(path) else {
        return false;
    };
    allowed_roots
        .iter()
        .filter_map(|root| canonical_path(root))
        .any(|root| canonical_target.starts_with(root))
}

pub(crate) fn register_output_paths(runtime: &mut RuntimeState, base: &Path, root: &Path) {
    runtime.allowed_open_exact_paths = vec![base.to_path_buf()];
    runtime.allowed_open_roots = vec![root.to_path_buf()];
}

pub(crate) fn is_open_path_allowed(path: &Path, runtime: &RuntimeState) -> bool {
    is_path_allowed(path, &runtime.allowed_open_roots)
        || canonical_path(path).is_some_and(|target| {
            runtime
                .allowed_open_exact_paths
                .iter()
                .filter_map(|path| canonical_path(path))
                .any(|path| path == target)
        })
}

pub(crate) fn make_unique_run_root(base: &Path) -> PathBuf {
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

// シェルを経由せずopenに渡せるHTTP(S) URLを検証する。
pub(crate) fn validate_external_url(raw: &str) -> Result<String, String> {
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

#[cfg(test)]
pub(crate) fn plan() -> OutputPlan {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn saving_to_filesystem_or_home_root_never_grants_descendant_access() {
        let root = std::env::temp_dir().join(format!("maho-scope-{}", std::process::id()));
        fs::create_dir_all(root.join("run")).unwrap();
        fs::write(root.join("private.png"), b"private").unwrap();
        fs::write(root.join("run/capture.png"), b"capture").unwrap();
        let filesystem_root = std::env::temp_dir()
            .ancestors()
            .last()
            .unwrap()
            .to_path_buf();
        for base in [filesystem_root, dirs_home(), root.clone()] {
            let mut runtime = RuntimeState::default();
            register_output_paths(&mut runtime, &base, &root.join("run"));
            assert!(is_open_path_allowed(&base, &runtime));
            assert!(!is_open_path_allowed(&root.join("private.png"), &runtime));
            assert!(!is_path_allowed(&base, &runtime.allowed_open_roots));
            assert!(is_path_allowed(
                &root.join("run/capture.png"),
                &runtime.allowed_open_roots
            ));
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn output_planning_uses_calendar_timestamp_and_unique_run_folder() {
        use chrono::TimeZone;
        let base = std::env::temp_dir().join(format!("maho-plans-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let configuration = CrawlConfiguration {
            target_url: "https://example.com".into(),
            output_root: base.to_string_lossy().into(),
            ..Default::default()
        };
        let now = Local
            .with_ymd_and_hms(2026, 10, 1, 12, 34, 56)
            .single()
            .unwrap();
        let (root, plans) = make_output_plans(&configuration, now.into()).unwrap();
        assert!(root.ends_with("example.com-20261001-123456"));
        fs::create_dir_all(&root).unwrap();
        let (second, second_plans) = make_output_plans(&configuration, now.into()).unwrap();
        assert!(second.ends_with("example.com-20261001-123456-2"));
        assert_eq!(plans[0].size_slug, second_plans[0].size_slug);
        assert_eq!(second_plans[0].root, second);
        fs::remove_dir_all(base).unwrap();
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
    fn safe_size_slug_removes_unsafe_characters_and_keeps_dimensions() {
        let viewport = CaptureViewport::new("id", "My Phone / 2", 390, 844);
        assert_eq!(safe_size_slug(&viewport), "my-phone-2-390x844");
        let punctuation = CaptureViewport::new("id", "...", 390, 844);
        assert_eq!(safe_size_slug(&punctuation), "size-390x844");
    }
    #[test]
    fn output_plan_is_partitioned_by_size() {
        let configuration = CrawlConfiguration::default();
        let (root, plans) =
            make_output_plans(&configuration, UNIX_EPOCH + Duration::from_secs(1234)).unwrap();
        assert!(root.ends_with(&format!(
            "example.com-{}",
            chrono::DateTime::<Local>::from(UNIX_EPOCH + Duration::from_secs(1234))
                .format("%Y%m%d-%H%M%S")
        )));
        assert_eq!(plans.len(), 3);
        let ends_with = |path: &str, parent: &str, name: &str| {
            Path::new(path).ends_with(Path::new(parent).join(name))
        };
        assert!(ends_with(
            &plans[0].captures,
            "desktop-1440x900",
            "screenshots"
        ));
        assert!(ends_with(
            &plans[0].http_cache_dir,
            "desktop-1440x900",
            ".siteone-http-cache"
        ));
        assert!(ends_with(
            &plans[1].html_report,
            "tablet-768x1024",
            "report.html"
        ));
        assert!(ends_with(
            &plans[2].json_report,
            "mobile-390x844",
            "report.json"
        ));
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
    fn expands_home_prefix_with_either_separator() {
        let home = crate::platform::home_dir();
        assert_eq!(expand_path("~"), home);
        assert_eq!(expand_path("~/captures"), home.join("captures"));
        assert_eq!(expand_path("~\\captures"), home.join("captures"));
        assert_eq!(expand_path("captures"), PathBuf::from("captures"));
    }
}
