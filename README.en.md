# MahoCrawl

[日本語](README.md) | English

MahoCrawl is a macOS app that crawls a website and captures screenshots and SEO information for each page. It captures multiple screen sizes, such as Desktop, Tablet, and Mobile, one after another in a single run, and lets you review the results in a gallery.

Crawling and capturing are powered by [SiteOne Crawler](https://github.com/janreges/siteone-crawler). It is bundled with the app, so no separate installation is required.

![Grid view: browse captured screenshots at a glance](docs/images/screenshot-grid.jpg)

![Row view: review URL, captures, and SEO information for each page](docs/images/screenshot-list.jpg)

> **Note:** The app's user interface is currently available in Japanese only. In this guide, on-screen labels are shown in Japanese followed by an English translation.

## Requirements

- macOS 13 or later
- A Mac with Apple Silicon (M1 or later) or an Intel processor
- A Chromium-based browser such as Google Chrome (used for taking screenshots)

## Download and installation

1. Open the [Releases page](https://github.com/dtsuka/MahoCrawl/releases/latest) and download the zip file for your Mac.
   - Apple Silicon (M1 or later): `MahoCrawl_<version>_aarch64.zip`
   - Intel: `MahoCrawl_<version>_x86_64.zip`
   - If you are not sure which one you have, choose Apple menu → "About This Mac" from the menu bar and check "Chip" (or "Processor"). If it starts with "Apple M", you have Apple Silicon. If it says "Intel", you have an Intel Mac.
2. Double-click the downloaded zip file to extract it.
3. Move `MahoCrawl.app` to your Applications folder.

### First launch

MahoCrawl is not signed with an Apple Developer ID or notarized, so macOS shows a warning such as "cannot be opened because the developer cannot be verified" the first time you open it. Use one of the following methods to open it:

- In Finder, right-click (or Control-click) `MahoCrawl.app` and choose "Open".
- Try to open the app once, then go to System Settings → Privacy & Security and click "Open Anyway".

After the first launch, you can open the app normally by double-clicking it.

## Usage

### Basic workflow

1. In the left sidebar, enter the URL of the site you want to crawl under 「クロール対象」 (Crawl target).
2. Adjust the maximum depth, concurrent workers, request rate, and other options under 「クロール設定」 (Crawl settings).
3. Turn on the screen sizes you want to capture under 「キャプチャサイズ」 (Capture sizes).
4. Start the crawl. The app captures each enabled size in turn and shows the results in the gallery.

Your settings are saved automatically as you edit them.

### Capture sizes

By default, Desktop `1440x900`, Tablet `768x1024`, and Mobile `390x844` are available. You can add, remove, and turn each size on or off.

If you turn off or remove all capture sizes before starting a crawl, the app skips screenshots, collects only the metadata found in the HTML (title, description, and so on), and shows the results in row view. Because no browser is launched in this mode, information added later by JavaScript is not included.

### Reviewing results

- Switch the gallery between 「グリッド」 (Grid) and 「行」 (Rows). Row view shows one page per row.
- Use the search box to search by URL, SEO fields, or file name. Click the clear button to reset the search.
- In row view, click a capture size name to open that page's screenshot for that size in a preview. In the preview, you can switch between 「全体」 (Fit) and 「100%」 (actual size). Press `Esc` to close it.
- Open SEO 「詳細」 (Details) to see the full, untruncated page information.
- You can also open the HTML report and the output folder from the app.

### Past scans

Click 「過去のスキャン」 (Past scans) at the top of the window to list the results in the current output folder, newest first. Click 「開く」 (Open) to load the saved screenshots and SEO information.

If your results are in a previous output folder or on an external drive, click 「別のフォルダ」 (Another folder) and select a run folder or a parent folder that contains run folders.

### Sites with Basic authentication

To crawl a site protected by Basic authentication, enter the user name and password. The password is never saved and is sent only to the same origin as the target URL during the crawl. You need to enter it again after restarting the app.

### About the browser

MahoCrawl uses a Chromium-based browser installed on your Mac, such as Google Chrome, to take screenshots. If it cannot find one automatically, specify it in 「ブラウザのパス」 (Browser path). If you turn on 「見つからない場合に自動取得」 (Download automatically if not found), the app downloads a browser for capturing when none is found.

## Output files

By default, results are saved to `~/Pictures/MahoCrawl/` (you can change the output folder). Each run gets a folder named with the date and time, and each capture size gets its own subfolder.

```text
~/Pictures/MahoCrawl/example.com-20260818-120000/
├── desktop-1440x900/
│   ├── screenshots/
│   ├── report.html
│   ├── report.json
│   └── report.txt
├── tablet-768x1024/
│   └── ...
└── mobile-390x844/
    └── ...
```

When only metadata is collected, the reports are saved in the `metadata/` folder inside the run folder.

## Please note

- Only crawl and capture sites whose owners have given you permission, or sites where doing so does not violate their terms of use or robots.txt.
- Set the request rate and concurrency to values that do not put excessive load on the target site.
- Logs and reports contain information about the target site. Handle them with care.
- Automatic cookie banner hiding currently does not work because the bundled SiteOne Crawler does not support it.

## For developers

To build from source, develop, test, or publish a release, see the [Development and Build Guide](docs/DEVELOPMENT.en.md).

## License

MahoCrawl is released under the [MIT License](LICENSE). Licenses for bundled third-party software, including SiteOne Crawler (MIT License), are included in the app and in [`Resources/Licenses/`](Resources/Licenses/).

MahoCrawl is an unofficial wrapper for SiteOne Crawler and is not affiliated with the SiteOne Crawler developers.
