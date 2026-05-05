# Claude Code Desktop Notifications

Sends native desktop notifications when Claude Code finishes a task or needs your attention.

## What it does

| Event | Notification |
|-------|-------------|
| Claude finishes responding | "Finished — check your terminal" |
| Claude needs your permission | "Needs your permission — waiting for input" (with sound) |
| Claude is idle/waiting | "Idle — waiting for your response" (with sound) |

## Setup

### 1. Wire up the hooks in `~/.claude/settings.json`

Add these entries inside the `"hooks"` object:

```json
"Stop": [
  {
    "hooks": [
      {
        "type": "command",
        "command": "node /Users/harshii/Developer/side-projects/claude-code-notifications/notify.mjs",
        "timeout": 5
      }
    ]
  }
],
"Notification": [
  {
    "matcher": "permission_prompt|idle_prompt",
    "hooks": [
      {
        "type": "command",
        "command": "node /Users/harshii/Developer/side-projects/claude-code-notifications/notify.mjs",
        "timeout": 5
      }
    ]
  }
]
```

### 2. Mac — allow notifications

macOS may ask you to allow notifications from Terminal (or whichever app runs Claude Code). Accept the prompt, or go to:

**System Settings → Notifications → Terminal → Allow Notifications**

### 3. Windows — no extra setup

Uses Windows Runtime toast notifications built into Windows 10+. No extra packages needed.

## Requirements

- Node.js (already required by Claude Code)
- Mac: built-in `osascript`
- Windows: PowerShell + Windows 10+
- Linux: `notify-send` (install via `sudo apt install libnotify-bin`)
