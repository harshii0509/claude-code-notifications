use std::io::Read;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// Payload & notification types
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize, Default)]
struct HookPayload {
    hook_event_name:   Option<String>,
    notification_type: Option<String>,
}

struct Notification {
    title:   &'static str,
    message: &'static str,
    sound:   bool,
    urgent:  bool,
}

fn read_payload() -> HookPayload {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw).unwrap_or(0);
    serde_json::from_str(&raw).unwrap_or_default()
}

fn resolve_notification(event: &str, notif_type: &str) -> Notification {
    match (event, notif_type) {
        ("Notification", "permission_prompt") => Notification {
            title:   "Claude Code",
            message: "Needs your permission \u{2014} waiting for input",
            sound:   true,
            urgent:  true,
        },
        ("Notification", "idle_prompt") => Notification {
            title:   "Claude Code",
            message: "Idle \u{2014} waiting for your response",
            sound:   true,
            urgent:  true,
        },
        _ => Notification {
            title:   "Claude Code",
            message: "Finished \u{2014} check your terminal",
            sound:   false,
            urgent:  false,
        },
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

#[cfg(target_os = "macos")]
struct Host {
    bundle: &'static str,
}

#[cfg(target_os = "macos")]
fn detect_host() -> Host {
    let tp = std::env::var("TERM_PROGRAM").unwrap_or_default();

    if tp == "vscode" {
        if std::env::var("CURSOR_CHANNEL").is_ok() || std::env::var("CURSOR_TRACE_ID").is_ok() {
            return Host { bundle: "com.todesktop.230313mzl4w4u92" };
        }
        if std::env::var("WINDSURF_EXTENSION_PATH").is_ok() {
            return Host { bundle: "com.exafunction.windsurf" };
        }
        return Host { bundle: "com.microsoft.VSCode" };
    }

    match tp.as_str() {
        "iTerm.app"      => return Host { bundle: "com.googlecode.iterm2"     },
        "WarpTerminal"   => return Host { bundle: "dev.warp.Warp-Stable"      },
        "Apple_Terminal" => return Host { bundle: "com.apple.Terminal"        },
        "ghostty"        => return Host { bundle: "com.mitchellh.ghostty"     },
        "Hyper"          => return Host { bundle: "co.zeit.hyper"             },
        "WezTerm"        => return Host { bundle: "com.github.wez.wezterm"    },
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
                return Host { bundle };
            }
        }

        let Ok(next_pid) = ppid_str.parse::<u32>() else { break };
        pid = next_pid;
    }

    Host { bundle: "com.apple.Terminal" }
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

#[cfg(target_os = "macos")]
fn send_macos(notif: &Notification, host: &Host) {
    let execute_cmd = format!("open -b '{}'", host.bundle);
    Command::new("terminal-notifier")
        .args([
            "-title",    notif.title,
            "-message",  notif.message,
            "-execute",  &execute_cmd,
            "-sender",   host.bundle,
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
fn send_windows(notif: &Notification) {
    notify_rust::Notification::new()
        .summary(notif.title)
        .body(notif.message)
        .show()
        .ok();
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn send_linux(notif: &Notification) {
    use notify_rust::{Hint, Urgency};
    notify_rust::Notification::new()
        .summary(notif.title)
        .body(notif.message)
        .hint(Hint::Urgency(if notif.urgent {
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
    let notif      = resolve_notification(event, notif_type);

    #[cfg(target_os = "macos")]
    {
        let host = detect_host();
        if !notif.urgent && is_host_focused(host.bundle) {
            std::process::exit(0);
        }
        if notif.sound { play_ping(); }
        send_macos(&notif, &host);
    }

    #[cfg(target_os = "windows")]
    {
        if notif.sound { play_ping(); }
        send_windows(&notif);
    }

    #[cfg(target_os = "linux")]
    {
        if notif.sound { play_ping(); }
        send_linux(&notif);
    }
}
