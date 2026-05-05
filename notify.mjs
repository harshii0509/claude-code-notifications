#!/usr/bin/env node
import { execSync, execFileSync } from 'child_process';

let raw = '';
process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) raw += chunk;

const payload = JSON.parse(raw || '{}');
const hookEvent = payload.hook_event_name;
const notifType = payload.notification_type;

const { title, message, sound, urgent } = resolveNotification(hookEvent, notifType);

if (process.platform === 'darwin') {
  const host = detectHost();

  // Skip notification for non-urgent (Stop) events if the user is already
  // looking at the terminal — no point interrupting an active session.
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
  let title = 'Claude Code';
  let message = 'Finished — check your terminal';
  let sound = null;
  let urgent = false;

  if (hookEvent === 'Notification') {
    if (notifType === 'permission_prompt') {
      message = 'Needs your permission — waiting for input';
      sound = 'Ping';
      urgent = true;
    } else if (notifType === 'idle_prompt') {
      message = 'Idle — waiting for your response';
      sound = 'Ping';
      urgent = true;
    }
  }

  return { title, message, sound, urgent };
}

function sendWindowsToast(title, message) {
  const t = title.replace(/'/g, "''");
  const m = message.replace(/'/g, "''");
  execSync(
    `powershell.exe -NoProfile -NonInteractive -Command "` +
    `[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; ` +
    `$xml = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); ` +
    `$xml.GetElementsByTagName('text')[0].AppendChild($xml.CreateTextNode('${t}')) | Out-Null; ` +
    `$xml.GetElementsByTagName('text')[1].AppendChild($xml.CreateTextNode('${m}')) | Out-Null; ` +
    `[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Claude Code').Show([Windows.UI.Notifications.ToastNotification]::new($xml))"`,
    { stdio: 'inherit' }
  );
}

// ---------------------------------------------------------------------------
// Detect which IDE/terminal is hosting this Claude Code session.
// TERM_PROGRAM is set by virtually every terminal and inherited by subprocesses,
// making it far more reliable than inspecting the process tree.
// ---------------------------------------------------------------------------
function detectHost() {
  const tp  = process.env.TERM_PROGRAM ?? '';
  const env = process.env;

  // Cursor sets TERM_PROGRAM=vscode too (it's a VS Code fork), but also
  // exposes CURSOR_CHANNEL or CURSOR_TRACE_ID.
  if (tp === 'vscode') {
    if (env.CURSOR_CHANNEL || env.CURSOR_TRACE_ID) {
      return { bundle: 'com.todesktop.230313mzl4w4u92' }; // Cursor
    }
    // Windsurf (Codeium fork) check
    if (env.WINDSURF_EXTENSION_PATH) {
      return { bundle: 'com.exafunction.windsurf' };
    }
    return { bundle: 'com.microsoft.VSCode' };
  }

  if (tp === 'iTerm.app')      return { bundle: 'com.googlecode.iterm2'    };
  if (tp === 'WarpTerminal')   return { bundle: 'dev.warp.Warp-Stable'     };
  if (tp === 'Apple_Terminal') return { bundle: 'com.apple.Terminal'       };
  if (tp === 'ghostty')        return { bundle: 'com.mitchellh.ghostty'    };
  if (tp === 'Hyper')          return { bundle: 'co.zeit.hyper'            };
  if (tp === 'WezTerm')        return { bundle: 'com.github.wez.wezterm'   };

  // kitty / alacritty / zed don't set TERM_PROGRAM; fall back to process tree scan
  function detectFromProcessTree() {
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
        const [rawPpid, ...parts] = out.trim().split(/\s+/);
        const comm = parts.join(' ');
        for (const c of candidates) {
          if (c.re.test(comm)) return { bundle: c.bundle };
        }
        pid = parseInt(rawPpid, 10);
      } catch { break; }
    }

    return { bundle: 'com.apple.Terminal' };
  }

  return detectFromProcessTree();
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
