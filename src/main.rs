use std::io::Read;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// Payload
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
struct HookPayload {
    hook_event_name:   Option<String>,
    notification_type: Option<String>,
}

fn read_payload() -> HookPayload {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).unwrap_or(0);
    serde_json::from_str(&raw).unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Decision: urgency + sound flags (independently testable, no I/O)
// ---------------------------------------------------------------------------

struct NotifClass {
    urgent: bool,
    sound:  bool,
}

fn classify(event: &str, notif_type: &str) -> NotifClass {
    match (event, notif_type) {
        ("Notification", "permission_prompt") |
        ("Notification", "idle_prompt") => NotifClass { urgent: true, sound: true },
        _ => NotifClass { urgent: false, sound: false },
    }
}

// ---------------------------------------------------------------------------
// Rendering: display strings (independently testable, no I/O)
// ---------------------------------------------------------------------------

fn render(event: &str, notif_type: &str) -> (&'static str, &'static str) {
    match (event, notif_type) {
        ("Notification", "permission_prompt") =>
            ("Claude Code", "Needs your permission \u{2014} waiting for input"),
        ("Notification", "idle_prompt") =>
            ("Claude Code", "Idle \u{2014} waiting for your response"),
        _ =>
            ("Claude Code", "Finished \u{2014} check your terminal"),
    }
}

// ---------------------------------------------------------------------------
// Sound: 880 Hz sine oscillator with exponential decay envelope.
//
// Translates the Web Audio API concepts from the article:
//   - Oscillator  → sine wave at 880 Hz (A5, bright and attention-grabbing)
//   - Envelope    → amplitude × exp(-k × t), real sounds decay exponentially
//   - Duration    → 0.6 s (26 460 samples at 44 100 Hz); inaudible by 0.3 s
// ---------------------------------------------------------------------------

struct PingSource {
    total: u64,
    pos:   u64,
}

impl PingSource {
    fn new() -> Self {
        Self { total: 26_460, pos: 0 }
    }
}

impl Iterator for PingSource {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.total {
            return None;
        }
        let t = self.pos as f32 / 44_100.0;
        self.pos += 1;
        Some(
            (2.0 * std::f32::consts::PI * 880.0 * t).sin()
                * (-8.0_f32 * t).exp()
                * 0.7,
        )
    }
}

impl rodio::Source for PingSource {
    fn channels(&self)          -> u16                        { 1 }
    fn sample_rate(&self)       -> u32                        { 44_100 }
    fn current_frame_len(&self) -> Option<usize>              { None }
    fn total_duration(&self)    -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(600))
    }
}

fn play_ping() {
    // _stream MUST stay alive until sleep_until_end() returns —
    // dropping it early silently disconnects the audio backend.
    let Ok((_stream, handle)) = rodio::OutputStream::try_default() else { return };
    let Ok(sink) = rodio::Sink::try_new(&handle) else { return };
    sink.append(PingSource::new());
    sink.sleep_until_end();
}

// ---------------------------------------------------------------------------
// macOS: host detection + focus check + terminal-notifier
// ---------------------------------------------------------------------------

/// How the host terminal was identified. Exposed so callers can log or
/// act on detection confidence (TermProgram > ProcessTree > Fallback).
#[cfg(target_os = "macos")]
#[allow(dead_code)]
enum Detection {
    TermProgram,  // matched TERM_PROGRAM env var — most reliable
    ProcessTree,  // found by walking parent processes — medium confidence
    Fallback,     // defaulted to com.apple.Terminal — may be wrong
}

#[cfg(target_os = "macos")]
struct Host {
    bundle:              &'static str,
    #[allow(dead_code)]  // exposed for callers that want to log detection confidence
    detection:           Detection,
}

#[cfg(target_os = "macos")]
fn detect_host() -> Host {
    let tp = std::env::var("TERM_PROGRAM").unwrap_or_default();

    if tp == "vscode" {
        if std::env::var("CURSOR_CHANNEL").is_ok() || std::env::var("CURSOR_TRACE_ID").is_ok() {
            return Host { bundle: "com.todesktop.230313mzl4w4u92", detection: Detection::TermProgram };
        }
        if std::env::var("WINDSURF_EXTENSION_PATH").is_ok() {
            return Host { bundle: "com.exafunction.windsurf", detection: Detection::TermProgram };
        }
        return Host { bundle: "com.microsoft.VSCode", detection: Detection::TermProgram };
    }

    match tp.as_str() {
        "iTerm.app"      => return Host { bundle: "com.googlecode.iterm2",  detection: Detection::TermProgram },
        "WarpTerminal"   => return Host { bundle: "dev.warp.Warp-Stable",   detection: Detection::TermProgram },
        "Apple_Terminal" => return Host { bundle: "com.apple.Terminal",     detection: Detection::TermProgram },
        "ghostty"        => return Host { bundle: "com.mitchellh.ghostty",  detection: Detection::TermProgram },
        "Hyper"          => return Host { bundle: "co.zeit.hyper",          detection: Detection::TermProgram },
        "WezTerm"        => return Host { bundle: "com.github.wez.wezterm", detection: Detection::TermProgram },
        _ => {}
    }

    walk_process_tree()
}

#[cfg(target_os = "macos")]
fn walk_process_tree() -> Host {
    // (pattern, case_insensitive, bundle)
    const CANDIDATES: &[(&str, bool, &str)] = &[
        ("Cursor",      false, "com.todesktop.230313mzl4w4u92"),
        ("Code Helper", false, "com.microsoft.VSCode"         ),
        ("iTerm2",      false, "com.googlecode.iterm2"        ),
        ("Warp",        false, "dev.warp.Warp-Stable"         ),
        ("WezTerm",     false, "com.github.wez.wezterm"       ),
        ("alacritty",   true,  "io.alacritty"                 ),
        ("kitty",       true,  "net.kovidgoyal.kitty"         ),
        ("Zed",         false, "dev.zed.Zed"                  ),
        ("Terminal",    false, "com.apple.Terminal"           ),
    ];

    let mut pid = unsafe { libc::getppid() } as u32;
    let mut seen = std::collections::HashSet::new();

    while pid > 1 && seen.insert(pid) {
        let Ok(out) = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "ppid=,comm="])
            .stderr(Stdio::null())
            .output()
        else {
            break;
        };

        let line = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let mut parts = line.split_whitespace();
        let Some(ppid_str) = parts.next() else { break };
        let comm: String = parts.collect::<Vec<_>>().join(" ");

        for &(pattern, ci, bundle) in CANDIDATES {
            let matched = if ci {
                comm.to_lowercase().contains(&pattern.to_lowercase())
            } else {
                comm.contains(pattern)
            };
            if matched {
                return Host { bundle, detection: Detection::ProcessTree };
            }
        }

        let Ok(next_pid) = ppid_str.parse::<u32>() else { break };
        pid = next_pid;
    }

    Host { bundle: "com.apple.Terminal", detection: Detection::Fallback }
}

#[cfg(target_os = "macos")]
fn is_host_focused(bundle: &str) -> bool {
    let Ok(out) = Command::new("osascript")
        .args([
            "-e",
            "tell application \"System Events\" to get bundle identifier of first process whose frontmost is true",
        ])
        .stderr(Stdio::null())
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).trim() == bundle
}

/// Returns true when the notification should be silently dropped.
/// Non-urgent (Stop) events are suppressed if the user is already looking
/// at the terminal — no point interrupting an active session.
#[cfg(target_os = "macos")]
fn should_suppress(urgent: bool, bundle: &str) -> bool {
    !urgent && is_host_focused(bundle)
}

#[cfg(target_os = "macos")]
fn send_macos(title: &str, message: &str, bundle: &str) {
    let execute_cmd = format!("open -b '{}'", bundle);
    Command::new("terminal-notifier")
        .args([
            "-title",    title,
            "-message",  message,
            "-execute",  &execute_cmd,
            "-sender",   bundle,
            "-group",    "claude-code",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn send_windows(title: &str, message: &str) {
    notify_rust::Notification::new()
        .summary(title)
        .body(message)
        .show()
        .ok();
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn send_linux(title: &str, message: &str, urgent: bool) {
    use notify_rust::{Hint, Urgency};
    notify_rust::Notification::new()
        .summary(title)
        .body(message)
        .hint(Hint::Urgency(if urgent {
            Urgency::Critical
        } else {
            Urgency::Normal
        }))
        .show()
        .ok();
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let payload    = read_payload();
    let event      = payload.hook_event_name.as_deref().unwrap_or("");
    let notif_type = payload.notification_type.as_deref().unwrap_or("");

    let class            = classify(event, notif_type);
    let (title, message) = render(event, notif_type);

    #[cfg(target_os = "macos")]
    {
        let host = detect_host();
        if should_suppress(class.urgent, host.bundle) {
            std::process::exit(0);
        }
        if class.sound { play_ping(); }
        send_macos(title, message, host.bundle);
    }

    #[cfg(target_os = "windows")]
    {
        if class.sound { play_ping(); }
        send_windows(title, message);
    }

    #[cfg(target_os = "linux")]
    {
        if class.sound { play_ping(); }
        send_linux(title, message, class.urgent);
    }
}
