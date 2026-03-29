# OZ Relay — Roadmap

Waves are ordered by dependency. Each wave is self-contained with exit criteria.

## Status

### Completed
- **RLY-0001**: OIP types, A2A schema, CLI validation (arcflow-core)
- **RLY-0002**: Relay server — auth, rate limiting, task management (oz-relay)
- **RLY-0003**: Security hardening — 15-item audit, all fixed
- **RLY-0004**: Entitlement gate — pending/active/suspended key lifecycle
- **RLY-0005**: Source privacy — agent CLAUDE.md, response filter, prompt injection sanitizer
- **RLY-0006**: Agent bridge — Claude Code sessions in worktrees, wired into message/send
- **RLY-0007**: Filesystem state machine — tasks/, ledger/, promotions/, bugs/ directories
- **RLY-0008**: Clarity gate — EARS-inspired scoring, rejects vague intents (saves tokens)
- **RLY-0009**: Deployment — systemd, Cloudflare tunnel, Ed25519 keys, firewall

- **RLY-0010**: OTEL error bridge — `/bugs/report` endpoint, BugReport schema
- **RLY-0011**: Bug triage automation — auto-generate intents from clear bug reports
- **RLY-0012**: Token tracking — cost estimation in build reports and ledger
- **RLY-0013**: Promotion queue — `/promotions/list`, approve, reject endpoints
- **RLY-0014**: Artifact report — naming convention, target triple, report structure
- **RLY-0015**: SSE progress streaming — real-time build updates via Server-Sent Events
- **RLY-0016**: TUI monitor — `oz-relay-monitor` ratatui dashboard
- **RLY-0018**: nsjail v3.4 installed — production sandbox ready
- **RLY-0020**: Rate limit persistence — counters restored from ledger on startup

### In Progress
- **RLY-0017**: CLI consolidation — `arcflow relay` subcommand (standalone `arcflow-relay` works)

### Blocked
- **RLY-0019**: Named Cloudflare tunnel — waiting for Route53/DNS access

---

## Wave RLY-0010: OTEL Error Bridge

**Goal:** Route ArcFlow runtime errors from end users into the relay's bugs/incoming/ directory.

**Context:** The OTEL collector is running on the server (dev-otel-collector-1). ArcFlow bindings
have an `exportErrorTasks()` API that produces structured error reports. These need to flow into
the relay filesystem so they can be triaged, converted to intents, and fixed.

**Tasks:**
1. Define the bug report schema (JSON): error message, stack trace, ArcFlow version, user context, OTEL trace ID, timestamp
2. Add a `/bugs/report` HTTP endpoint to the relay server (no auth required — end users report errors without API keys)
3. The endpoint validates the report, writes to `bugs/incoming/{timestamp}-{hash}.json`, appends to ledger
4. Add rate limiting on bug reports (by IP, 10/hour) to prevent abuse
5. Add a `/bugs/list` authenticated endpoint for OZ to view incoming bugs
6. Write a simple exporter script that reads OTEL collector output and POSTs to `/bugs/report`

**Exit criteria:**
- `curl -X POST relay/bugs/report -d '{"error":"...","version":"1.7.0"}'` → file appears in bugs/incoming/
- Ledger records `bug.reported` event
- Rate limiting prevents spam
- End users don't need API keys to report bugs

**Crate changes:** oz-relay-server (new routes), oz-relay-common (BugReport schema)

---

## Wave RLY-0011: Bug Triage Automation

**Goal:** Automatically classify incoming bugs and convert clear ones to intents.

**Tasks:**
1. Add a triage CLI command: `oz-relay-server triage` — scans bugs/incoming/, scores each for clarity
2. Bugs with clear reproduction steps + error messages → auto-generate an Intent from the bug report
3. Move triaged bugs to bugs/triaged/ with the generated intent attached
4. Bugs that are too vague → leave in incoming/ with a `needs_info` flag
5. Add ledger events: `bug.triaged`, `bug.needs_info`, `bug.converted_to_intent`

**Exit criteria:**
- A clear bug report with stack trace auto-converts to an intent
- Vague bug reports get flagged, not auto-converted
- Ledger tracks the triage pipeline

**Depends on:** RLY-0010

---

## Wave RLY-0012: Token Tracking + Cost Controls

**Goal:** Log token usage per build, enforce per-task budgets, choose cheaper models where possible.

**Tasks:**
1. Capture claude `--print` token usage from stdout/stderr (Claude Code reports usage)
2. Log token counts in ledger events: `claude.finished` gets `input_tokens`, `output_tokens`, `cost_usd`
3. Add `RELAY_MAX_TOKENS_PER_TASK` config (default: 200,000) — kill claude session if exceeded
4. Add `RELAY_AGENT_MODEL` config — use `haiku` for implementation, `sonnet` for review (default: sonnet)
5. Track cumulative daily cost in ledger, add to clarity report response: `"estimatedCost": "$3-5"`
6. Add cost summary to build report: `"tokensUsed": 142000, "costUsd": 4.26`

**Exit criteria:**
- Ledger shows token usage per task
- Tasks killed if they exceed token budget
- Daily cost visible in logs
- Build report includes cost

**Crate changes:** oz-relay-server (agent_bridge, routes)

---

## Wave RLY-0013: Promotion Queue Workflow

**Goal:** Successful builds persist as branches, enter a review queue, get promoted to main.

**Tasks:**
1. On build success: keep the git branch (don't delete), write metadata to promotions/pending/
2. Add `oz-relay-server promote` CLI: review pending promotions, approve/reject
3. Approved promotions: cherry-pick into main, move to promotions/merged/, tag with attribution
4. Rejected promotions: move to promotions/rejected/ with reason
5. Add developer attribution: commit message includes `Contributed-by: dev_alice via OZ Relay`
6. Ledger events: `promotion.pending`, `promotion.approved`, `promotion.merged`, `promotion.rejected`
7. Notify developer (via task status update) when their intent is promoted

**Exit criteria:**
- `ls promotions/pending/` shows successful builds awaiting review
- `oz-relay-server promote --approve <id>` merges into main
- Changelog attribution works
- Developer can poll task status and see "promoted"

**Depends on:** RLY-0012 (cost tracking in promotion metadata)

---

## Wave RLY-0014: Delivery Report + Artifact Compilation

**Goal:** Compile signed binaries from successful builds, return them to developers.

**Tasks:**
1. After tests pass, run `cargo build --release --target <target>` in the worktree
2. Strip debug symbols from the binary
3. Sign with Ed25519 (artifact_signer.rs already exists)
4. Write artifact manifest: name, sha256, signature, target triple, ArcFlow version
5. Return artifact as a Binary Part in the A2A response (base64-encoded)
6. Add `arcflow-relay install --task-id <id>` CLI command to download + verify + install
7. Artifact naming: `arcflow-{developer}-{slug}-{short_id}.{so|dylib|wasm}`

**Exit criteria:**
- Successful build returns a signed binary
- `arcflow-relay verify --artifact <file> --manifest <file> --pubkey <hex>` passes
- `arcflow-relay install` downloads, verifies, and places the binary

**Depends on:** RLY-0013 (promotion decides if artifact stays custom or goes to release)

---

## Wave RLY-0015: SSE Progress Streaming

**Goal:** Real-time build progress via Server-Sent Events.

**Tasks:**
1. Replace the single-event `message/stream` with a real progress channel
2. The build pipeline emits progress events: `analyzing`, `implementing`, `testing`, `compiling`, `signing`
3. Each event includes elapsed time and phase description
4. CLI shows spinner with live progress: `⠋ [03:22] Implementing trim() function...`
5. Developer's agent can consume SSE for autonomous operation

**Exit criteria:**
- `curl -N relay/a2a` with `message/stream` returns multiple SSE events over time
- CLI shows real-time progress
- Agent-to-agent flow works (developer agent polls SSE, loads binary when done)

**Depends on:** RLY-0014 (streaming includes artifact-ready event)

---

## Wave RLY-0016: TUI Monitor (`oz-relay-monitor`)

**Goal:** Terminal UI for engineering/product managers to observe the relay pipeline.

**Tasks:**
1. New binary: `oz-relay-monitor` (uses `ratatui` for TUI)
2. Reads filesystem state directly (no server API needed):
   - `tasks/{submitted,working,completed,failed}/` — directory counts + file details
   - `promotions/{pending,approved,merged,rejected}/` — promotion pipeline
   - `bugs/{incoming,triaged,resolved}/` — bug pipeline
   - `ledger/events.jsonl` — real-time event stream (tail -f)
3. Dashboard panels:
   - **Pipeline** — counts per directory, bar chart
   - **Active builds** — task ID, developer, branch, elapsed time, phase
   - **Cost** — tokens today, cost today, avg cost/build, balance
   - **Promotions** — pending count, approval rate, merge rate
   - **Bugs** — incoming count, triage rate, resolution rate
   - **Events** — scrolling log of recent ledger events
4. Keyboard: `q` quit, `r` refresh, `l` full ledger, `t` task detail, `b` bugs
5. Auto-refresh every 2 seconds (watches filesystem with inotify)
6. Run with: `oz-relay-monitor --data-dir /opt/oz-relay`

**Exit criteria:**
- Running `oz-relay-monitor` shows a live dashboard
- All metrics derived from filesystem (no server dependency)
- Responsive to file changes in real-time

**Depends on:** RLY-0012 (cost data in ledger), RLY-0010 (bugs directory populated)

---

## Wave RLY-0017: CLI Consolidation

**Goal:** `arcflow-relay` commands become subcommands of the main `arcflow` CLI.

**Tasks:**
1. Move relay CLI into arcflow CLI: `arcflow relay submit`, `arcflow relay status`, `arcflow relay discover`
2. Keep `arcflow-relay` as an alias (backwards compatibility)
3. Add `arcflow relay bugs report` — report a bug from the CLI
4. Add `arcflow relay install` — download and install a custom build
5. Default server URL from ArcFlow config, not hardcoded

**Exit criteria:**
- `arcflow relay submit --intent "..."` works
- `arcflow relay bugs report --error "..." --version 1.7.0` works
- Old `arcflow-relay` command still works

**Depends on:** RLY-0014 (install command), RLY-0010 (bug reporting)

---

## Wave RLY-0018: nsjail Production Sandbox

**Goal:** Install nsjail on the server, wire it into the build pipeline.

**Tasks:**
1. Install nsjail v3.4 on the Ubuntu server (setup.sh already has the script)
2. Test nsjail with a sample cargo build inside the sandbox
3. Wire `run_sandboxed_with_config()` to use nsjail when config is present
4. Verify: no network from inside sandbox, filesystem isolation works, GPU passthrough works
5. Run a full intent submission through nsjail

**Exit criteria:**
- `nsjail --version` returns 3.4
- Builds run inside nsjail with no network access
- cargo test passes inside the sandbox
- GPU tests work (CUDA device passthrough)

**Depends on:** RLY-0012 (resource limits in sandbox config)

---

## Wave RLY-0019: Named Cloudflare Tunnel + DNS

**Goal:** `relay.ozapi.net` points to the relay via a named Cloudflare tunnel.

**Tasks:**
1. Add `ozapi.net` to Cloudflare (or configure CNAME in Route53)
2. Create a named tunnel: `cloudflared tunnel create oz-relay`
3. Route DNS: `cloudflared tunnel route dns oz-relay relay.ozapi.net`
4. Install as systemd service: `cloudflared service install`
5. Update config: `RELAY_URL=https://relay.ozapi.net`, `RELAY_BIND_ADDR=127.0.0.1:3400`
6. Remove quick tunnel, verify everything works on the permanent URL
7. Update AgentCard URL

**Exit criteria:**
- `curl https://relay.ozapi.net/.well-known/agent.json` returns AgentCard
- TLS via Cloudflare, no Caddy needed
- Tunnel survives server reboots (systemd)

**Depends on:** Route53 access

---

## Wave RLY-0020: Rate Limit Persistence

**Goal:** Rate limit counters survive server restarts.

**Tasks:**
1. Write rate limit state to `ledger/rate_limits.json` on every check
2. Load on startup
3. Or: derive from ledger events (count `task.created` per developer per day)

**Exit criteria:**
- Restart server, developer's daily quota is preserved
- No SQLite dependency

**Depends on:** RLY-0007 (filesystem state machine)

---

## Dependency Graph

```
RLY-0010 (OTEL bridge)
  └── RLY-0011 (bug triage)
       └── RLY-0016 (TUI monitor) ← also needs RLY-0012

RLY-0012 (cost controls)
  └── RLY-0013 (promotion queue)
       └── RLY-0014 (artifact compilation)
            └── RLY-0015 (SSE streaming)

RLY-0017 (CLI consolidation) ← needs RLY-0010, RLY-0014
RLY-0018 (nsjail) ← needs RLY-0012
RLY-0019 (DNS) ← needs Route53 access
RLY-0020 (rate limit persistence) ← independent
```

## Execution Order (recommended)

1. **RLY-0010** — OTEL error bridge (unblocks bug pipeline)
2. **RLY-0012** — Token tracking (saves money immediately)
3. **RLY-0011** — Bug triage automation
4. **RLY-0013** — Promotion queue
5. **RLY-0016** — TUI monitor (needs 0010 + 0012)
6. **RLY-0014** — Artifact compilation
7. **RLY-0015** — SSE streaming
8. **RLY-0020** — Rate limit persistence
9. **RLY-0017** — CLI consolidation
10. **RLY-0018** — nsjail sandbox
11. **RLY-0019** — Named tunnel (when DNS available)
