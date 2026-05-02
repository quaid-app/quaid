---
name: "local-ipc-auth"
description: "Review pattern for local socket auth without trusting spoofable protocol identity"
domain: "security"
confidence: "high"
source: "extracted"
---

## Pattern

For local IPC that forwards privileged writes, treat **kernel-backed peer credentials as the authority** and treat protocol identity (`whoami`, session id, socket-path token) as a **cross-check only**.

## Use when

- A CLI talks to a local daemon over a Unix socket.
- The daemon exposes write-capable operations.
- A same-UID spoofing race is plausible.

## Requirements

1. Publish the endpoint only **after** bind + permission audit succeed.
2. Clear any published endpoint metadata and unlink the socket on shutdown/startup-abort so clients cannot chase stale coordinates.
3. Client verifies socket owner/mode before connect.
4. Client verifies kernel peer UID/PID after connect against the expected process.
5. Server verifies peer UID on every accept.
6. If auth/setup fails while a live owner exists, **refuse**; do not fall back to an unsafe direct path.

## Anti-patterns

- Trusting `whoami` or session id as the primary auth primitive.
- Trusting the socket path alone.
- Falling back to direct writes when the proxy channel is unavailable.
- Blocking the daemon's main supervision loop on socket I/O.
