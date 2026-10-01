//! Finding, launching and stopping the installed Chrome a dot browses with (Open Dot's
//! `launch` and `freeProfile` in `computer/browser.ts`).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;

use super::cdp::{http_json, Cdp};
use super::{SCREEN_HEIGHT, SCREEN_WIDTH};

/// The installed Chrome the dots use, if any: Google Chrome first, then Chromium, Edge and Brave.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// The dots use the browser already installed on the computer, not a bundled Chromium: Google refuses sign-in in automation builds of Chromium (Open Dot found the same), and bundling one would add hundreds of megabytes to Ghostex.
pub(super) fn find_chrome() -> Option<PathBuf> {
    candidates().into_iter().find(|path| path.is_file())
}

fn candidates() -> Vec<PathBuf> {
    let mut list = Vec::new();
    if cfg!(target_os = "macos") {
        let apps = [
            "Google Chrome.app/Contents/MacOS/Google Chrome",
            "Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            "Chromium.app/Contents/MacOS/Chromium",
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "Brave Browser.app/Contents/MacOS/Brave Browser",
        ];
        let home_apps =
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Applications"));
        for app in apps {
            list.push(Path::new("/Applications").join(app));
            if let Some(home_apps) = &home_apps {
                list.push(home_apps.join(app));
            }
        }
    } else if cfg!(windows) {
        let roots = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        let apps = [
            "Google\\Chrome\\Application\\chrome.exe",
            "Chromium\\Application\\chrome.exe",
            "Microsoft\\Edge\\Application\\msedge.exe",
            "BraveSoftware\\Brave-Browser\\Application\\brave.exe",
        ];
        for app in apps {
            for root in &roots {
                list.push(root.join(app));
            }
        }
    } else {
        let names = [
            "google-chrome",
            "google-chrome-stable",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ];
        let dirs = std::env::var_os("PATH")
            .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
            .unwrap_or_default();
        for name in names {
            for dir in dirs
                .iter()
                .map(PathBuf::as_path)
                .chain([Path::new("/usr/bin"), Path::new("/snap/bin")])
            {
                list.push(dir.join(name));
            }
        }
    }
    list
}

/// The user agent with "HeadlessChrome" replaced, learned on the first launch of this process.
fn fixed_user_agent() -> &'static Mutex<Option<String>> {
    static UA: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    UA.get_or_init(|| Mutex::new(None))
}

/// One running Chrome with its DevTools port.
pub(super) struct ChromeProc {
    child: Child,
    pub(super) port: u16,
    browser_ws_path: String,
}

impl ChromeProc {
    pub(super) fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub(super) fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Closes Chrome gracefully (so it flushes cookies and the profile), killing it when it
    /// does not exit in time.
    pub(super) fn shutdown(mut self) {
        if self.alive() {
            let url = format!("ws://127.0.0.1:{}{}", self.port, self.browser_ws_path);
            if let Ok(mut cdp) = Cdp::connect(&url) {
                let _ = cdp.call_with_timeout("Browser.close", json!({}), Duration::from_secs(2));
                cdp.close();
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.alive() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        if self.alive() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Starts Chrome headless on the dot's profile.
///
/// CDXC:OpenDots 2026-10-01 WHY:
/// Headless Chrome announces itself as "HeadlessChrome" in its user agent, which sites treat as a bot. As in Open Dot, the first launch of each gxserver run learns the real version and relaunches with the normal user agent; later launches pass it straight away.
pub(super) fn launch(chrome: &Path, profile: &Path) -> Result<ChromeProc, String> {
    std::fs::create_dir_all(profile)
        .map_err(|error| format!("Couldn't create the dot's browser profile: {error}"))?;
    free_profile(profile);
    let known = fixed_user_agent()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let mut proc = spawn(chrome, profile, known.as_deref())?;
    if known.is_none() {
        let reported = http_json(proc.port, "/json/version", Duration::from_secs(5))
            .ok()
            .and_then(|version| {
                version
                    .get("User-Agent")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            });
        if let Some(reported) = reported {
            let fixed = reported.replace("HeadlessChrome/", "Chrome/");
            *fixed_user_agent()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(fixed.clone());
            if fixed != reported {
                proc.shutdown();
                proc = spawn(chrome, profile, Some(&fixed))?;
            }
        }
    }
    Ok(proc)
}

fn spawn(chrome: &Path, profile: &Path, user_agent: Option<&str>) -> Result<ChromeProc, String> {
    let port_file = profile.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);
    let mut command = Command::new(chrome);
    command
        .arg("--headless=new")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-port=0")
        .arg(format!("--window-size={SCREEN_WIDTH},{SCREEN_HEIGHT}"))
        .arg("--force-device-scale-factor=1")
        // No "controlled by automated software" signals, and no navigator.webdriver.
        .arg("--disable-blink-features=AutomationControlled")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-default-apps")
        .arg("--disable-search-engine-choice-screen")
        // The profile's cookies are sealed with Chrome's built-in key instead of a Keychain item,
        // so a background Chrome never raises a Keychain prompt.
        .arg("--use-mock-keychain")
        .arg("--password-store=basic")
        .arg("--disable-background-timer-throttling")
        .arg("--disable-backgrounding-occluded-windows")
        .arg("--disable-renderer-backgrounding")
        .arg("--disable-breakpad")
        .arg("--mute-audio");
    if let Some(user_agent) = user_agent {
        command.arg(format!("--user-agent={user_agent}"));
    }
    command
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command
        .spawn()
        .map_err(|error| format!("Couldn't start Chrome: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(text) = std::fs::read_to_string(&port_file) {
            let mut lines = text.lines();
            let port = lines
                .next()
                .and_then(|line| line.trim().parse::<u16>().ok());
            let path = lines
                .next()
                .map(str::trim)
                .filter(|path| path.starts_with('/'));
            if let (Some(port), Some(path)) = (port, path) {
                return Ok(ChromeProc {
                    child,
                    port,
                    browser_ws_path: path.to_string(),
                });
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!("Chrome quit while starting ({status})."));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Chrome took too long to start.".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A Chrome left over from an earlier gxserver run keeps the profile locked, and a new launch on a
/// locked profile hands off to it and exits. The profile only ever belongs to this dot's browser,
/// so close that Chrome (only after checking it really runs on this profile, since a pid can be
/// reused). A lock whose pid is gone, or now belongs to a process that does not use this profile,
/// is stale and removed; that other process is left alone.
pub(super) fn free_profile(profile: &Path) {
    #[cfg(unix)]
    {
        let lock = profile.join("SingletonLock");
        let Ok(target) = std::fs::read_link(&lock) else {
            return;
        };
        let pid = target
            .to_string_lossy()
            .rsplit('-')
            .next()
            .and_then(|pid| pid.parse::<i32>().ok())
            .unwrap_or(0);
        let holds_profile = |pid: i32| {
            pid > 0
                && pid != std::process::id() as i32
                && alive(pid)
                && runs_on_profile(pid, profile)
        };
        if holds_profile(pid) {
            terminate(pid);
        }
        if !holds_profile(pid) {
            for name in ["SingletonLock", "SingletonSocket", "SingletonCookie"] {
                let _ = std::fs::remove_file(profile.join(name));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = profile;
    }
}

#[cfg(unix)]
fn alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// SIGTERM, then SIGKILL when the process is still there after five seconds.
#[cfg(unix)]
fn terminate(pid: i32) {
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    if alive(pid) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The full command line of `pid` (`ps -ww`, so a long one is not cut at the terminal width).
#[cfg(unix)]
fn command_line(pid: i32) -> Option<String> {
    let output = Command::new("ps")
        .args(["-ww", "-o", "command=", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(unix)]
fn runs_on_profile(pid: i32, profile: &Path) -> bool {
    command_line(pid)
        .is_some_and(|command| command.contains(&format!("--user-data-dir={}", profile.display())))
}

/// Closes dot Chromes an earlier gxserver left running (it crashed or was killed before its
/// shutdown closed them): main Chrome processes whose `--user-data-dir` is a profile under
/// `browsers_root`, except `keep` (this gxserver's own). Their helper processes exit with them.
/// Returns how many were closed.
pub(super) fn sweep_orphans(browsers_root: &Path, keep: &[u32]) -> usize {
    #[cfg(unix)]
    {
        let Ok(output) = Command::new("ps")
            .args(["-axww", "-o", "pid=,ppid=,command="])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
        else {
            return 0;
        };
        let marker = format!(
            "--user-data-dir={}/",
            browsers_root.display().to_string().trim_end_matches('/')
        );
        let own = std::process::id();
        let orphans: Vec<i32> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                let pid = fields.next()?.parse::<u32>().ok()?;
                let ppid = fields.next()?.parse::<u32>().ok()?;
                let command = line.trim_start();
                let is_main = command.contains(&marker) && !command.contains(" --type=");
                (is_main && pid != own && ppid != own && !keep.contains(&pid)).then_some(pid as i32)
            })
            .collect();
        for pid in &orphans {
            unsafe {
                libc::kill(*pid, libc::SIGTERM);
            }
        }
        for pid in &orphans {
            if alive(*pid) {
                terminate(*pid);
            }
        }
        orphans.len()
    }
    #[cfg(not(unix))]
    {
        let _ = (browsers_root, keep);
        0
    }
}
