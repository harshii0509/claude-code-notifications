#!/usr/bin/env node
import { existsSync } from 'fs';
import { execSync, execFileSync } from 'child_process';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dir = dirname(fileURLToPath(import.meta.url));

let raw = '';
process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) raw += chunk;

// ---------------------------------------------------------------------------
// Prefer the compiled Rust binary — it has richer host detection and handles
// all platforms natively. Node.js is a fallback for users who haven't built
// or downloaded it yet.
// ---------------------------------------------------------------------------
const binaryCandidates = [
  resolve(__dir, 'notify'),                 // installed next to this script (GitHub Release)
  resolve(__dir, 'target/release/notify'),  // local dev build
];
const binary = binaryCandidates.find(p => existsSync(p));
if (binary) {
  execFileSync(binary, [], { input: raw, stdio: ['pipe', 'inherit', 'inherit'] });
  process.exit(0);
}

// ---------------------------------------------------------------------------
// Fallback: pure Node.js implementation
// ---------------------------------------------------------------------------

const payload   = JSON.parse(raw || '{}');
const hookEvent = payload.hook_event_name;
const notifType = payload.notification_type;

const { title, message, sound, urgent } = resolveNotification(hookEvent, notifType);

if (process.platform === 'darwin') {
  const host = detectHost();

  // Skip non-urgent (Stop) events if the user is already looking at the terminal.
  if (!urgent && isHostFocused(host.bundle)) process.exit(0);

  const args = [
    '-title',    title,
    '-message',  message,
    '-execute',  `open -b '${host.bundle}'`,
    '-sender',   host.bundle,
    '-group',    'claude-code',
  ];
  if (sound) args.push('-sound', sound);
  execFileSync('terminal-notifier', args, { stdio: 'ignore' });

} else if (process.platform === 'win32') {
  sendWindowsToast(title, message);

} else {
  const urgency = urgent ? '--urgency=critical' : '--urgency=normal';
  execFileSync('notify-send', [urgency, title, message]);
}

// ---------------------------------------------------------------------------

function resolveNotification(hookEvent, notifType) {
  if (hookEvent === 'Notification') {
    if (notifType === 'permission_prompt') {
      return { title: 'Claude Code', message: 'Needs your permission — waiting for input', sound: 'Ping', urgent: true };
    }
    if (notifType === 'idle_prompt') {
      return { title: 'Claude Code', message: 'Idle — waiting for your response', sound: 'Ping', urgent: true };
    }
  }
  return { title: 'Claude Code', message: 'Finished — check your terminal', sound: null, urgent: false };
}

function sendWindowsToast(title, message) {
  // Values are passed via environment variables so no escaping or injection is possible,
  // regardless of what characters the title or message contain.
  const script = [
    '[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null',
    '$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02)',
    '$xml.GetElementsByTagName("text")[0].AppendChild($xml.CreateTextNode($env:TOAST_TITLE)) | Out-Null',
    '$xml.GetElementsByTagName("text")[1].AppendChild($xml.CreateTextNode($env:TOAST_MSG)) | Out-Null',
    '[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier("Claude Code").Show([Windows.UI.Notifications.ToastNotification]::new($xml))',
  ].join('; ');

  execSync(`powershell.exe -NoProfile -NonInteractive -Command "${script}"`, {
    stdio: 'inherit',
    env: { ...process.env, TOAST_TITLE: title, TOAST_MSG: message },
  });
}

// ---------------------------------------------------------------------------
// Detect which IDE/terminal is hosting this Claude Code session.
// TERM_PROGRAM is set by virtually every terminal and inherited by subprocesses,
// making it far more reliable than inspecting the process tree.
// ---------------------------------------------------------------------------
function detectHost() {
  const tp  = process.env.TERM_PROGRAM ?? '';
  const env = process.env;

  if (tp === 'vscode') {
    if (env.CURSOR_CHANNEL || env.CURSOR_TRACE_ID) return { bundle: 'com.todesktop.230313mzl4w4u92' };
    if (env.WINDSURF_EXTENSION_PATH)               return { bundle: 'com.exafunction.windsurf' };
    return { bundle: 'com.microsoft.VSCode' };
  }

  if (tp === 'iTerm.app')      return { bundle: 'com.googlecode.iterm2'  };
  if (tp === 'WarpTerminal')   return { bundle: 'dev.warp.Warp-Stable'   };
  if (tp === 'Apple_Terminal') return { bundle: 'com.apple.Terminal'     };
  if (tp === 'ghostty')        return { bundle: 'com.mitchellh.ghostty'  };
  if (tp === 'Hyper')          return { bundle: 'co.zeit.hyper'          };
  if (tp === 'WezTerm')        return { bundle: 'com.github.wez.wezterm' };

  // kitty / alacritty / zed don't set TERM_PROGRAM; fall back to process tree scan
  const candidates = [
    { re: /Cursor/,       bundle: 'com.todesktop.230313mzl4w4u92' },
    { re: /Code Helper/,  bundle: 'com.microsoft.VSCode'          },
    { re: /iTerm2/,       bundle: 'com.googlecode.iterm2'         },
    { re: /Warp/,         bundle: 'dev.warp.Warp-Stable'          },
    { re: /WezTerm/,      bundle: 'com.github.wez.wezterm'        },
    { re: /alacritty/i,   bundle: 'io.alacritty'                  },
    { re: /kitty/i,       bundle: 'net.kovidgoyal.kitty'          },
    { re: /Zed/,          bundle: 'dev.zed.Zed'                   },
    { re: /Terminal/,     bundle: 'com.apple.Terminal'            },
  ];

  let pid = process.ppid;
  const seen = new Set();

  while (pid > 1 && !seen.has(pid)) {
    seen.add(pid);
    try {
      const out = execSync(`ps -p ${pid} -o ppid=,comm=`, {
        encoding: 'utf8', stdio: ['pipe', 'pipe', 'ignore'],
      }).trim();
      const [rawPpid, ...parts] = out.split(/\s+/);
      const comm = parts.join(' ');
      for (const c of candidates) {
        if (c.re.test(comm)) return { bundle: c.bundle };
      }
      pid = parseInt(rawPpid, 10);
    } catch { break; }
  }

  return { bundle: 'com.apple.Terminal' };
}

// Returns true if the given app is already the frontmost window on macOS.
function isHostFocused(bundleId) {
  try {
    const front = execSync(
      `osascript -e 'tell application "System Events" to get bundle identifier of first process whose frontmost is true'`,
      { encoding: 'utf8', stdio: ['pipe', 'pipe', 'ignore'] }
    ).trim();
    return front === bundleId;
  } catch { return false; }
}
