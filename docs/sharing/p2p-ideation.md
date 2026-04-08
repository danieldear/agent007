# Ideation: P2P Team Sharing with libp2p

> Status: Brainstorm — extending `docs/sharing/ideation.md` for cross-machine team sharing

## The Problem (Extended)

The original ideation covered same-machine sharing (global `~/.agent007/`).
This document covers **cross-machine team sharing** — teammates on different laptops,
possibly behind NAT, possibly on different networks.

## Why libp2p Fits Well

| Requirement | libp2p Feature | Notes |
|-------------|---------------|-------|
| Same office auto-discovery | **mDNS** | Zero config, finds peers on same WiFi |
| Remote team sharing | **Kademlia DHT** + rendezvous | Works over internet |
| Push skill updates to team | **gossipsub** | Pub/sub, like Slack for skills |
| Fetch skill on demand | **request-response** | Pull, not push — respects privacy |
| Content integrity | **Content IDs (CID)** | Skills identified by hash → tamper-proof |
| NAT traversal | **relay + dcutr** | Works behind corporate firewalls |

## Scenarios

### Scenario A — Same office / team WiFi
```
Alice's machine ──mDNS─► Bob's agent007 (auto-discovered!)
                          Carol's agent007 (auto-discovered!)
Dashboard shows "Team" panel automatically. Zero config.
```

### Scenario B — Remote team (different networks)
```
                  ┌─ Bootstrap/Rendezvous node (lightweight, hosted) ─┐
Alice (home) ─────┤                                                   ├── Bob (office)
                  └───────────────────────────────────────────────────┘
Team subscribes to gossipsub topic:  /agent007/team/<team-id>
Alice publishes skill → Bob and Carol receive it automatically.
```

### Scenario C — Hybrid (pragmatic)
- mDNS for same-network discovery (works immediately, no internet)
- Git URL install as fallback for remote (works everywhere, no server)
- libp2p gossipsub for "live sync" teams who want real-time updates

## Architecture

```
crates/p2p/                         (new crate)
├── swarm.rs     — libp2p Swarm with all behaviours
├── protocol.rs  — SkillRequest/SkillResponse codec
├── discovery.rs — mDNS (local) + Kademlia (internet)
├── sync.rs      — gossipsub topic subscription + publish
└── identity.rs  — Ed25519 keypair stored in ~/.agent007/peer_id

Swarm behaviours:
  ┌─ mdns::Behaviour        → local peer discovery
  ├─ kad::Behaviour         → internet peer discovery + routing
  ├─ gossipsub::Behaviour   → team pub/sub channel
  ├─ request_response::Behaviour<SkillCodec>  → pull skills on demand
  └─ relay::client::Behaviour + dcutr::Behaviour → NAT traversal
```

### Team Channel Model (gossipsub)
```
Team ID = hash of team name / invite code
Topic   = /agent007/skills/v1/<team-id>

Publish message:
  {
    "kind": "skill_published",
    "trigger": "/code-review",
    "cid": "bafybeig...",   ← SHA-256 content ID
    "author_peer": "12D3Ko...",
    "timestamp": "2026-04-08T..."
  }

Subscriber receives → sees notification in dashboard
  "Bob shared /code-review" → [Install] button
  Clicking Install → request-response pull from Bob's peer
```

### Peer Identity
```
~/.agent007/peer_key.pem    ← Ed25519 keypair generated on first run
~/.agent007/peer_id         ← corresponding PeerId (public key hash)

This identity:
  - is your agent007 "name" on the P2P network
  - is NOT linked to any account
  - can be regenerated if lost
```

### Content Addressing
Skills are identified by their SHA-256 hash encoded as a CID:
```
CID = base58(sha256(skill_content))
```
When you install a skill by CID, you get exactly the content you expected.
No central authority needed.

## libp2p Cargo Features Needed

```toml
[dependencies]
libp2p = { version = "0.56", features = [
  "tokio",
  "mdns",           # local discovery
  "kad",            # internet discovery
  "gossipsub",      # team pub/sub
  "request-response",# pull skills
  "tcp",            # transport
  "noise",          # encryption
  "yamux",          # multiplexing
  "relay",          # NAT traversal
  "dcutr",          # direct connection upgrade
] }
```

Binary size impact: ~15-25 MB additional (libp2p is modular, feature-gated).

## CLI Interface

```sh
# Join/create a team channel
agent007 team join <invite-code>
agent007 team create --name "acme-corp"

# Publish a skill to your team
agent007 skill publish /code-review --to team

# See what peers/teams are active
agent007 peers list

# Install a skill from a peer
agent007 skill install peer:<peer-id>/<trigger>
agent007 skill install cid:<content-id>
```

## Dashboard UX

```
┌─ Team Panel ───────────────────────────────────────────────────┐
│ 🟢 Bob (same network)    3 skills available    [Browse]        │
│ 🟡 Carol (remote)        2 skills available    [Browse]        │
│ 🔴 Dave (offline)                                              │
│                                                                │
│ 📢 Bob just published /commit-msg               [Install]      │
│ 📢 Carol published /security-audit             [Install]       │
└────────────────────────────────────────────────────────────────┘
```

## Tradeoffs vs. Simpler Approaches

| Approach | Pros | Cons |
|----------|------|------|
| **libp2p** | Auto-discovery, real-time, no server needed | Complexity, ~20MB extra |
| **Git registry** | Simple, versioned, works everywhere | No real-time, requires git push |
| **Simple HTTP server** | Dead simple | Manual IP sharing, no discovery |
| **iroh (alt to libp2p)** | Simpler API, QUIC-based, smaller | Less mature than libp2p |

## Recommendation

**Start with mDNS-only** (just `libp2p-mdns` crate, ~2MB):
- Auto-discover teammates on same network in a single `crates/p2p`
- No internet, no server, no config
- Later: add gossipsub + Kademlia for remote teams

This is the **lowest-risk entry point** into P2P sharing.

## Open Questions

1. **Bootstrap node**: For remote teams (Scenario B), do we host a bootstrap node
   at `bootstrap.agent007.dev`, or require teams to self-host one?

2. **Team invite flow**: How does Alice invite Bob to the team channel?
   Options: QR code, invite code copied from dashboard, shared config file?

3. **Privacy**: Should skill _content_ be encrypted at rest in transit (E2E), or is
   transport encryption (noise protocol) sufficient?

4. **iroh vs libp2p**: iroh (from the IPFS team) offers a simpler Rust API with QUIC
   transport. Worth evaluating alongside libp2p before committing.
