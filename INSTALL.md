# Installing cc-hooks

`cc-hooks` is one native binary that answers Claude Code hooks. Each subcommand replaces a
PowerShell or Python hook script, which paid 150-500 ms of interpreter start on every call.

## 1. Build and copy the binary

```powershell
cargo build --release
New-Item -ItemType Directory -Force "$env:USERPROFILE\.claude\bin" | Out-Null
Copy-Item target\release\cc-hooks.exe "$env:USERPROFILE\.claude\bin\cc-hooks.exe"
```

## 2. Create the local config

Copy `cc-hooks.example.toml` to `%USERPROFILE%\.claude\cc-hooks.toml` and fill it in. Set
`CC_HOOKS_CONFIG` to read it from somewhere else. A hook whose section is missing does nothing.

| Section | Used by |
|---|---|
| `[git_identity]` | `git-identity` |
| `[quality_gate]` | `quality-gate`, `quality-gate --post` |
| `[[context_inject]]` | `context-inject` (one block per folder rule) |

## 3. Point settings.json at it

Replace `<you>` with your Windows user name. Only the `cc-hooks` entries are shown. Keep the other
hooks in each event (rtk, graphify, memory, skillmem, ...) where they are, and remove the script
entries these replace.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|PowerShell",
        "hooks": [
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe attribution"
          },
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe git-identity",
            "timeout": 10
          },
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe quality-gate",
            "timeout": 900,
            "statusMessage": "Running the AI-delivery quality gate..."
          }
        ]
      },
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe image-tools",
            "timeout": 10
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe grill",
            "timeout": 10,
            "statusMessage": "Checking for feature request..."
          },
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe design-mockup",
            "timeout": 10,
            "statusMessage": "Checking for mockup request..."
          },
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe image-tools",
            "timeout": 10
          }
        ]
      }
    ],
    "SessionStart": [
      {
        "matcher": "^(startup|resume|clear|compact|fork)$",
        "hooks": [
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe herdr-session",
            "timeout": 10
          }
        ]
      },
      {
        "matcher": "",
        "hooks": [
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe context-inject",
            "timeout": 10
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "Bash|PowerShell",
        "hooks": [
          {
            "type": "command",
            "command": "C:/Users/<you>/.claude/bin/cc-hooks.exe quality-gate --post",
            "timeout": 30,
            "statusMessage": "Queueing the post-PR quality gate..."
          }
        ]
      }
    ]
  }
}
```

`quality-gate` only checks whether the command creates a pull request. When it does, it runs the
configured PowerShell gate script with the original stdin and passes the script's stdout and exit
code back unchanged. `$env:QUALITY_GATE = 'off'` turns both stages off.

## 4. Verify

```powershell
cargo test                                         # unit tests
$env:CC_HOOKS_PARITY = '1'; cargo test --test parity -- --nocapture
```

The parity test feeds the same payloads to the old scripts (from `%USERPROFILE%\.claude\hooks`) and
to the binary, then compares the parsed JSON. It prints a table per hook.

## Differences from the old scripts

- **UTF-8 BOM on stdin.** The PowerShell scripts could not parse a payload that starts with a BOM
  and stayed silent. `cc-hooks` strips the BOM and answers.
- **Encoding.** Windows PowerShell 5.1 decodes piped stdin and encodes stdout with the console code
  page, not UTF-8, and Python decoded stdin as cp1252. Depending on the console, accented prompts
  could be misclassified and non-ASCII context text garbled. `cc-hooks` always uses UTF-8. The
  parity harness forces UTF-8 on the old scripts so it compares logic, and it reports the raw-mode
  differences separately.
- **image-tools dedupe without a uuid.** Python keyed it on `hash(prompt)`, which is salted per
  process, so it never deduped (only the empty prompt hashes to a stable 0). `cc-hooks` keeps that
  behavior on purpose: `prompt:0` for an empty prompt, a unique key otherwise. Entries that have a
  uuid use the same state files as before (`%TEMP%\claude-img\reminded-<session>.txt`).
- **image-tools tail read.** It still reads at most the last 8 MB of the transcript, but it starts
  with a 256 KB window and grows it only when the latest human entry is further back. Lines that
  hold a `"type":"tool_result"` are skipped before JSON parsing.
- **git-identity** is new. Besides `git -C <dir>`, it follows a `cd <dir>` earlier in the same
  command line and an inline `-c user.email=...`.
