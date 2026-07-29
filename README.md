# m365-tui

A unified **terminal client for Outlook and Microsoft Teams**, built on the
Microsoft Graph API in Rust. One process, two screens (switch with `F2`), with
cross-navigation between them and near-real-time updates via a
Cloudflare-tunnelled webhook.

- **Outlook** — folders, message list with scroll-to-load-more, an HTML-rendered
  reading pane, compose / reply / reply-all / forward, full-text search, and a
  7-day calendar view with RSVP.
- **Teams** — chats and channels, a message pane with per-message selection and
  a pinned Today/Yesterday/date header, an inline composer, **emoji reactions**,
  and presence-aware labels.
- **Paging** — mail and Teams messages load 50 at a time; scrolling to the end
  of either list pulls the next page, and periodic refreshes merge in new items
  without dropping the pages you've already scrolled back through.
- **Rich rendering** — mail and Teams message bodies are HTML; they're rendered
  directly to styled terminal text (headings, bold/italic, lists, code,
  blockquotes, links) — no raw tags.
- **Cross-navigation** — from an email, open a Teams chat with its sender
  (command palette → *chat with selected email's sender*).
- **Live updates** — a background poll refreshes the current view every 20s
  (a `⟳ synced HH:MM:SS` indicator shows in the tab bar). For *instant* push,
  Graph change notifications hit a webhook behind a Cloudflare tunnel, which
  publishes to Redis; the TUI reacts with a targeted delta fetch
  ("notify-then-delta").

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

- Rust toolchain. On NixOS the rustup toolchain may be broken, so use the flake
  dev shell (nixpkgs `rustc` + `cargo`):
  ```sh
  nix develop                      # drop into a shell with the toolchain
  nix develop -c cargo build       # or run a single command in it
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
   - Optionally `Presence.ReadWrite` to *change* your own status — see
     [Changing your status](#changing-your-status).
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
nix develop -c cargo build --release

# Auth smoke test (device-code login, prints your identity):
nix develop -c cargo run -p m365-tui -- whoami

# Launch the TUI:
nix develop -c cargo run -p m365-tui
```

### Via the Nix flake

```sh
nix run  github:rootHytx/m365-tui          # run without installing
nix build github:rootHytx/m365-tui         # build ./result/bin/m365
```

Or add it to a system/home-manager flake as an input (`m365-tui.url =
"github:rootHytx/m365-tui"`) and reference
`m365-tui.packages.${system}.default`.

### Prebuilt binary

Grab the static `x86_64-linux` tarball from the
[latest release](https://github.com/rootHytx/m365-tui/releases/latest) — it
contains `m365` and `m365-webhook` and runs as-is (musl, no glibc dependency, so
it works on NixOS too).

On first launch you'll get a device-code prompt: open the URL, enter the code,
and sign in. The token is cached at `~/.config/m365-tui/token-cache.json`
(mode `0600`) and refreshed silently thereafter.

### Keys

| Scope   | Keys |
|---------|------|
| Global  | `F2` switch app · `Ctrl+P` command palette · `p` set presence · `?` help · `q`/`Ctrl+C` quit |
| Outlook | `Tab` cycle panes · `j`/`k` move (scroll to bottom loads more) · `Enter` open · `c` compose · `r` reply · `a` reply-all · `f` forward · `/` search · `g` calendar |
| Teams   | `Tab` cycle panes · `Enter` open · `t` chats↔channels · `j`/`k` select message (scroll to end loads older) · `e` react · `Esc`/`←`/`h` back to list · `i` type · `Enter` send |
| Copying | `y` yank focused message · `Y` yank whole view · `z` copy mode |
| Compose | `Tab`/`Shift+Tab` field · `←→↑↓` move · `Ctrl+←→` word · `Home`/`End` line · `Ctrl+Home`/`Ctrl+End` all · `Backspace`/`Delete` · `Ctrl+W` word · `Ctrl+U`/`Ctrl+K` to line start/end · `Ctrl+S` send · `Esc` cancel |
| React   | `1`–`7` pick emoji · `Esc` cancel |

### Writing mail

The compose/reply window is a real text editor: a visible cursor, arrow-key
movement, word jumps (`Ctrl+←/→`), per-line `Home`/`End`, `Ctrl+Home`/`Ctrl+End`
for the whole field, `Delete` as well as `Backspace`, and the usual `Ctrl+W`
(word), `Ctrl+U` (to line start) and `Ctrl+K` (to line end) deletions. The body
soft-wraps and scrolls with the cursor, and terminal **paste** is supported via
bracketed paste. The Teams composer uses the same editing keys.

### Changing your status

`p` shows your presence and offers Available / Busy / DND / Be right back / Away
/ Appear offline (Graph `setUserPreferredPresence`, same as Teams' sticky status).

Reading your status works out of the box. **Setting** it needs the extra
`Presence.ReadWrite` scope, which is **opt-in** — adding a scope invalidates any
existing consent grant, and in tenants that disallow user consent every sign-in
then requires fresh admin approval. To enable it:

1. Add the `Presence.ReadWrite` delegated permission to the app registration and
   grant admin consent.
2. Set `M365_PRESENCE_WRITE=1` in `.env`.
3. Re-authenticate: `rm ~/.config/m365-tui/token-cache.json` then
   `m365 login`.

Without it, the picker is read-only and says so.

> **If sign-in starts asking for admin approval,** the requested scope set no
> longer matches what was consented. Either get the new set approved, or pin the
> old one via `M365_SCOPES` in `.env` and re-login.

### Copying text

Terminal selection is linear across the whole screen, so dragging over the
reading pane would otherwise also grab the list pane beside it. Two ways around
that:

- **`y` / `Y`** — copy straight to the system clipboard, no mouse involved. `y`
  copies the focused item (open email body, or selected Teams message); `Y`
  copies the whole view (email with headers, or the entire conversation). Uses
  `wl-copy`/`xclip`/`xsel` when available, else the OSC 52 escape sequence (which
  also works over SSH).
- **`z` copy mode** — redraws the current message/conversation full-width with no
  borders and no side panes, so a normal terminal drag-select captures exactly the
  text. `j`/`k` scroll, `y` yanks everything, `z`/`Esc` exits.

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
nix develop -c cargo test --workspace
```

## Releases

Pushing a tag matching `v*.*` triggers `.github/workflows/release.yml`, which
builds a static `x86_64-unknown-linux-musl` binary of both `m365` and
`m365-webhook`, packages them into a tarball (with a SHA-256 checksum), and
publishes a GitHub Release with auto-generated notes.

```sh
git tag v0.1.0
git push origin v0.1.0
```

## Out of scope

Joining Teams audio/video/screen-share calls (not a terminal capability) and
bulk chat export (a separately-approved "protected" Graph API). You can list and
schedule meetings, just not join their media.
