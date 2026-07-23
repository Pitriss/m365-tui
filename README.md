# m365-tui

A unified **terminal client for Outlook and Microsoft Teams**, built on the
Microsoft Graph API in Rust. One process, two screens (switch with `F2`), with
cross-navigation between them and near-real-time updates via a
Cloudflare-tunnelled webhook.

- **Outlook** — folders, message list, reading pane, compose/reply, search, and
  a 7-day calendar view with RSVP.
- **Teams** — chats and channels, message view, an inline composer, and
  presence-aware labels.
- **Cross-navigation** — from an email, open a Teams chat with its sender
  (command palette → *chat with selected email's sender*).
- **Real-time** — Graph change notifications hit a webhook behind a Cloudflare
  tunnel, which publishes to Redis; the TUI reacts with a targeted delta fetch
  ("notify-then-delta"). Falls back to polling when no tunnel is configured.

## Architecture

```
crates/
  m365-core/   auth (device-code), Graph client, models, endpoints, subscriptions, event bus
  m365-tui/    the ratatui app (binary: `m365`)
  webhook/     axum change-notification receiver (binary: `m365-webhook`)
docker-compose.yml   redis + webhook + cloudflared
```

Real-time data flow:

```
Graph --HTTPS--> cloudflared --> webhook --> Redis --> TUI --delta fetch--> Graph
  ^                                                     |
  └──────────── TUI creates/renews subscriptions ───────┘
```

The webhook holds no Graph token and never calls Graph — it only validates,
verifies `clientState`, and republishes a small "something changed" signal.

## Prerequisites

- Rust toolchain. **On this NixOS machine** the rustup toolchain is broken, so
  use the provided dev shell (nixpkgs `rustc` + `cargo`):
  ```sh
  nix-shell            # or: nix-shell --run 'cargo build'
  ```
- Docker + Docker Compose (only for real-time push).
- A **work/school Microsoft 365 account** (Teams messaging APIs don't work with
  personal accounts).

## 1. Register an Entra application

In the [Entra admin center](https://entra.microsoft.com) → *App registrations* →
*New registration*:

1. **Supported account types**: single tenant (your org) is fine.
2. **Authentication** → *Advanced settings* → **Allow public client flows** →
   **Yes**. (Device-code flow needs this; no redirect URI is required.)
3. **API permissions** → *Microsoft Graph* → *Delegated*, add:
   `User.Read`, `People.Read`, `Mail.ReadWrite`, `Mail.Send`,
   `Calendars.ReadWrite`, `Chat.ReadWrite`, `ChannelMessage.Send`,
   `ChannelMessage.Read.All`, `Presence.Read.All` (and `offline_access`,
   `openid`, `profile`).
   - **`ChannelMessage.Read.All` requires admin consent.** If you're not an
     admin, click *Grant admin consent* is unavailable — ask IT, or start
     Outlook-only by trimming scopes (see `M365_SCOPES` in `.env`).
4. Copy the **Application (client) ID** and (optionally) the **Directory
   (tenant) ID**.

> **Most likely blocker:** your tenant may forbid third-party app registrations
> or require admin approval. That's an org policy, not a code issue.

## 2. Configure

```sh
cp .env.example .env
# edit .env: set at least M365_CLIENT_ID (and M365_TENANT_ID if not `organizations`)
```

For **poll-only mode** (no push), leave `M365_TUNNEL_BASE_URL` empty — the app is
fully functional, just not instant.

## 3. Build & run

```sh
nix-shell --run 'cargo build --release'

# Auth smoke test (device-code login, prints your identity):
nix-shell --run 'cargo run -p m365-tui -- whoami'

# Launch the TUI:
nix-shell --run 'cargo run -p m365-tui'
```

On first launch you'll get a device-code prompt: open the URL, enter the code,
and sign in. The token is cached at `~/.config/m365-tui/token-cache.json`
(mode `0600`) and refreshed silently thereafter.

### Keys

| Scope   | Keys |
|---------|------|
| Global  | `F2` switch app · `Ctrl+P` command palette · `?` help · `q`/`Ctrl+C` quit |
| Outlook | `Tab` cycle panes · `j`/`k` move · `Enter` open · `c` compose · `r` reply · `/` search · `g` calendar |
| Teams   | `Tab` cycle panes · `j`/`k` move · `Enter` open · `t` chats↔channels · `i` type · `Enter` send · `Esc` leave composer |
| Compose | `Tab` next field · `Ctrl+S` send · `Esc` cancel |

Logs go to `$TMPDIR/m365-tui.log` (set `RUST_LOG=debug` for detail).

## 4. Real-time (tunnel + Docker)

1. Create a **Cloudflare Tunnel** (Zero Trust dashboard → *Networks* →
   *Tunnels*), add a public hostname routing to `http://webhook:8080`, and copy
   the tunnel **token**.
2. In `.env` set:
   - `CLOUDFLARE_TUNNEL_TOKEN=` the token
   - `M365_TUNNEL_BASE_URL=https://<your-tunnel-hostname>` (no trailing slash)
   - `M365_CLIENT_STATE=` a shared secret (`openssl rand -hex 16`)
3. Start the stack:
   ```sh
   docker compose up -d --build
   docker compose logs -f cloudflared   # confirm the tunnel is up
   ```
4. Launch the TUI (on the host). Because `M365_TUNNEL_BASE_URL` is now set, it
   creates Graph subscriptions pointing at the tunnel and renews them every
   45 min (chat subscriptions expire in ~1h).

For a throwaway URL without a Cloudflare account, use a quick tunnel — see the
commented `command:` in `docker-compose.yml`, then read the printed
`https://*.trycloudflare.com` host into `M365_TUNNEL_BASE_URL`.

## 5. Verify end-to-end

1. **Auth**: `cargo run -p m365-tui -- whoami` prints your name/email; relaunch
   and confirm no second login (silent refresh from cache).
2. **Outlook**: read an email, `c` to send yourself a test mail, `r` to reply,
   `g` for the calendar — cross-check in Outlook web.
3. **Teams**: open a chat, `i` then type, `Enter` to send — confirm in Teams.
4. **Handshake**: `docker compose up`; the first TUI launch with a tunnel set
   should create subscriptions without error (the webhook echoes Graph's
   `validationToken`). Check `docker compose logs webhook`.
5. **Live push**: from your phone, send yourself an email and a Teams chat →
   both appear in the TUI within ~1–2s.
6. **Resilience**: `docker compose stop cloudflared` → new messages still show
   up via polling (slower); restart → push resumes after renewal.

## Tests

```sh
nix-shell --run 'cargo test --workspace'
```

## Out of scope

Joining Teams audio/video/screen-share calls (not a terminal capability) and
bulk chat export (a separately-approved "protected" Graph API). You can list and
schedule meetings, just not join their media.
