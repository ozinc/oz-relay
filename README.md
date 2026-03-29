# OZ Relay

A2A-compliant build relay for the [Intent-Source License](https://github.com/ozinc/arcflow/blob/main/legal/INTENT-SOURCE-TERM-SHEET.md). Receives structured change intents from developer agents, implements them in sandboxed build sessions, and returns signed binaries — without exposing source code.

Built on [Google's A2A protocol](https://github.com/google/A2A) for agent-to-agent communication and [Anthropic's MCP](https://modelcontextprotocol.io) for tool integration.

## What this does

A developer's coding agent hits a limitation in your software. Instead of filing a ticket or reading your source, it sends a structured intent:

```json
{
  "description": "Add OPTIONAL MATCH support to WorldCypher",
  "motivation": "Need to query optional relationships that may not exist",
  "category": "feature",
  "test_cases": [{
    "query": "OPTIONAL MATCH (a:Person)-[:KNOWS]->(b) RETURN a.name, b.name",
    "expected_behavior": "Returns b.name as null when no KNOWS edge exists"
  }],
  "context": {
    "arcflow_version": "1.7.0"
  }
}
```

The relay:

1. Validates the intent schema
2. Spawns an isolated sandbox (nsjail, no network, 30-minute timeout)
3. Runs a server-side coding agent against the proprietary source
4. Compiles a release binary and signs it with Ed25519
5. Returns the artifact, test results, and a behavioral summary
6. Strips all source code, file paths, and internal details from the response

The developer gets a working binary. The source never leaves the server.

## Architecture

```
Developer's Agent                           OZ Relay Server
       │                                          │
       │  ── capability discovery ──────────────►  │  /.well-known/agent.json
       │  ◄── AgentCard ────────────────────────  │
       │                                          │
       │  ── message/send (Intent) ────────────►  │  /a2a (JSON-RPC 2.0)
       │                                          │
       │                                          │  ┌──────────────────────┐
       │  ◄── SSE progress ────────────────────  │  │  nsjail sandbox      │
       │      "implementing feature..."           │  │   └─ Claude Code     │
       │      "847 tests pass, 0 fail..."         │  │      └─ cargo test   │
       │                                          │  │      └─ cargo build  │
       │                                          │  └──────────────────────┘
       │  ◄── Artifact (signed .so/.dylib) ────  │
       │      + manifest (sha256, Ed25519 sig)    │
       │      + behavioral summary                │
       │                                          │
```

**A2A** connects agents to each other. **MCP** connects agents to tools. **OIP** (Open Intent Protocol) defines the intent schema, source-privacy boundary, and artifact signing format on top of A2A.

## Crates

| Crate | Description |
|-------|-------------|
| `oz-relay-common` | OIP types: A2A primitives, intent schema, validation, artifact manifest |
| `oz-relay-server` | Axum-based A2A server with JWT auth, rate limiting, sandbox executor, response filter, Ed25519 signing |

## Security model

| Layer | Mechanism |
|-------|-----------|
| **Authentication** | HS256 JWT tokens (`bsk_` prefixed), per-developer |
| **Rate limiting** | Per-developer sliding window: community 3/day, professional 10/day, enterprise 1000/day |
| **Isolation** | nsjail: no network, separate PID namespace, read-only filesystem, 30-minute timeout |
| **Source privacy** | Response filter strips file paths, code blocks, and internal identifiers before delivery |
| **Artifact integrity** | Ed25519 signatures on SHA-256 hashes of compiled binaries |
| **Tenant isolation** | Tasks are owned by the JWT `sub` claim; developers can only access their own tasks |
| **Body limits** | 1MB request size limit on all endpoints |

## Deploy

```bash
# On your Ubuntu server:
git clone https://github.com/ozinc/oz-relay.git /tmp/oz-relay
cd /tmp/oz-relay/deploy

# Install nsjail, Caddy, create service user, generate signing keys
sudo bash setup.sh

# Clone your proprietary source (bare repo)
sudo -u oz-relay git clone --bare git@github.com:your-org/your-product.git /opt/arcflow/repo.git

# Configure secrets
sudo vim /opt/oz-relay/config/config.env
# Set: RELAY_JWT_SECRET, ANTHROPIC_API_KEY, RELAY_URL

# Build and install
cargo build --release -p oz-relay-server
sudo cp target/release/oz-relay-server /opt/oz-relay/bin/

# Start
sudo systemctl enable --now oz-relay-server
sudo systemctl enable --now caddy

# Verify
curl https://your-relay.example.com/.well-known/agent.json

# Generate a developer key
./generate-api-key.sh dev_alice community
```

## CI/CD

Push to `main` triggers: test, build release binary, deploy via SSH, health check. Three GitHub secrets required:

- `RELAY_SSH_HOST` — server hostname
- `RELAY_SSH_USER` — deployment user
- `RELAY_SSH_KEY` — SSH private key

## Multi-product

The relay is product-agnostic. Configure products in `products/`:

```toml
# products/your-product/product.toml
[product]
name = "your-product"
source_repo = "/opt/your-product/repo.git"
build_command = "cargo build --release"
test_command = "cargo test"
```

Each product gets its own source repo, signing keys, and agent instructions. One relay, many products.

## OIP — Open Intent Protocol

OIP is not a wire protocol. It's three things on top of A2A:

1. **Intent schema** (`application/vnd.oip.intent+json`) — structured change requests with test cases
2. **Source-privacy boundary** — response filter that strips internal details before delivery
3. **Artifact signing** — Ed25519 signatures for verifiable binary delivery

Any project can implement an OIP-compatible relay. The format is open, the transport is standard A2A.

## JSON-RPC methods

All methods are called via `POST /a2a` with Bearer authentication.

| Method | Description |
|--------|-------------|
| `message/send` | Submit an intent, receive a task ID |
| `message/stream` | SSE stream of task progress |
| `tasks/get` | Retrieve task status by ID |
| `tasks/cancel` | Cancel a running task |

## GPU passthrough

The nsjail config mounts `/dev/nvidia0`, `/dev/nvidiactl`, and `/dev/nvidia-uvm` into the sandbox. Builds that require GPU (CUDA tests, GPU-accelerated compilation) work against the host's GPU hardware. Rate limiting prevents contention.

## Intent-Source License

OZ Relay is the infrastructure for a new licensing model:

- **Compiled artifacts are free** — embed them in commercial products, no restrictions
- **Source stays proprietary** — never leaves the server, not "source-available with restrictions"
- **Contributions happen through intents** — agents describe behavioral changes, the relay implements them

This fills the gap between open source (everyone sees the code) and closed source (nobody can contribute). Developers contribute through their AI agents without ever seeing the implementation.

The relay is Apache-2.0. What you build on it, and what source code it protects, is your business.

## License

Apache-2.0. See [LICENSE](LICENSE).
