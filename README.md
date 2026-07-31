# m365-tui

A terminal client for **Outlook and Microsoft Teams**, in one app. Switch
between the two with `F2`.

- **Outlook** — read mail with proper HTML rendering, compose / reply /
  reply-all / forward, send and save attachments, search, and a 7-day calendar
  with RSVP.
- **Teams** — chats and channels, emoji reactions, shared files, and your
  presence status.
- **Live** — refreshes every 20 seconds out of the box; add a tunnel for
  instant push notifications.

Built on the Microsoft Graph API in Rust. For how it works internally, see
[ARCHITECTURE.md](ARCHITECTURE.md).

---

## Requirements

- A **work or school Microsoft 365 account**. Personal accounts can't use the
  Teams messaging APIs.
- Rust (or Nix — see [Install](#3-install)).
- Docker, **only** if you want instant push notifications.

## 1. Register an app in Entra

You need an app registration to get a client ID. Sign in at
[entra.microsoft.com](https://entra.microsoft.com) → *Applications* →
*App registrations* → **New registration**.

1. **Name**: anything, e.g. `m365-tui`.
2. **Supported account types**: *Accounts in this organizational directory only*.
3. **Redirect URI**: leave blank.
4. Click **Register**, then copy the **Application (client) ID** and
   **Directory (tenant) ID** from the Overview page.

Then two settings on that app:

5. **Authentication** → *Advanced settings* → **Allow public client flows** →
   **Yes**. Sign-in fails without this.
6. **API permissions** → *Add a permission* → *Microsoft Graph* →
   **Delegated permissions**, and add:

   ```
   User.Read  People.Read
   Mail.ReadWrite  Mail.Send
   Calendars.ReadWrite
   Chat.ReadWrite  ChannelMessage.Send  ChannelMessage.Read.All
   Presence.Read.All
   offline_access  openid  profile
   ```

7. Click **Grant admin consent**.

> **If you're not an admin**, that button is greyed out. `ChannelMessage.Read.All`
> and `Presence.Read.All` need an administrator — ask them to register the app
> and grant consent, then use the client ID they give you. You sign in as
> yourself; the app acts on your behalf.
>
> To start without an admin, use Outlook-only scopes by setting `M365_SCOPES` in
> `.env` (see [.env.example](.env.example)).

## 2. Configure

```sh
cp .env.example .env
```

Set at minimum:

```dotenv
M365_CLIENT_ID=<Application (client) ID>
M365_TENANT_ID=<Directory (tenant) ID>
```

Leave `M365_TUNNEL_BASE_URL` empty for now — the app works fully without it,
just polling instead of instant push.

## 3. Install

**With Nix:**

```sh
nix run github:rootHytx/m365-tui         # try it
nix profile install github:rootHytx/m365-tui
```

Or as a flake input, referencing `m365-tui.packages.${system}.default`.

**Prebuilt binary:** download the `x86_64-linux` tarball from the
[latest release](https://github.com/rootHytx/m365-tui/releases/latest). It's
statically linked, so it runs anywhere including NixOS.

**From source:**

```sh
cargo build --release            # binary at target/release/m365
```

On NixOS, or if your Rust toolchain misbehaves, use the bundled dev shell:

```sh
nix develop -c cargo build --release
```

## 4. Run

```sh
m365 whoami     # sign in and print your identity — a good first check
m365            # launch
```

The first run prints a URL and a code: open the URL, enter the code, sign in.
The token is cached at `~/.config/m365-tui/token-cache.json` and refreshed
automatically, so you only do this once.

---

## Keys

Press `?` in the app for this list at any time.

| Scope | Keys |
|---|---|
| **Global** | `F2` switch Outlook/Teams · `Ctrl+P` command palette · `p` presence · `?` help · `q` quit |
| **Outlook** | `Tab` change pane · `j`/`k` move · `Enter` open · `c` compose · `r` reply · `a` reply-all · `f` forward · `/` search · `g` calendar |
| **Teams** | `Tab` change pane · `Enter` open · `t` chats↔channels · `j`/`k` select message · `e` react · `i` write · `Enter` send · `Esc` back |
| **Attachments** | `A` list · `1`–`9` save to Downloads |
| **Links** | `o` list · `1`–`9` open in browser |
| **Copying** | `y` copy message · `Y` copy everything · `z` copy mode |
| **Writing** | `←→↑↓` move · `Ctrl+←→` by word · `Home`/`End` · `Ctrl+W`/`Ctrl+U`/`Ctrl+K` delete · `Ctrl+S` send · `Esc` cancel |

Scrolling to the end of a message list or conversation loads the next 50 items.

## Things worth knowing

**Sending attachments.** In the compose window, `Tab` to the `Attach:` field,
type a file path (`~` works) and press `Enter` to stage it. `Ctrl+X` removes the
last one. Files over 3 MB upload in chunks automatically; Graph's own ceiling is
150 MB per message.

**Saving attachments.** Messages with attachments show 📎. Press `A`, then a
number, to save to your Downloads folder. Files are never overwritten.

**Links.** Long URLs are kept out of the text — a link shows as its text plus
`[1]`. Press `o` to list them and a number to open. Microsoft "Safelinks"
tracking wrappers are unwrapped back to the real destination.

**Copying text.** Press `y` to copy a message straight to the clipboard, or `z`
for copy mode — a borderless full-width view where a normal mouse drag selects
only the message text, with no side panes in the way.

**Your status.** `p` sets Available / Busy / DND / Away / Appear offline. This
needs one extra permission (`Presence.ReadWrite`) plus `M365_PRESENCE_WRITE=1`
in `.env`; without them the picker is read-only.

Note that Teams overrides `Available` with `Away` when its client is idle. If
you want a status that sticks while you work in the terminal, use **Busy** or
**Do not disturb**.

## Instant push (optional)

Polling every 20 seconds is the default and needs nothing. For ~1-second
updates, run the webhook behind a Cloudflare tunnel.

1. Create a tunnel at [one.dash.cloudflare.com](https://one.dash.cloudflare.com)
   → *Networks* → *Tunnels*, and copy its token.
2. Add a route: **Published application** → a hostname on a domain in your
   Cloudflare account → service `http://webhook:8080`.
3. In `.env`:
   ```dotenv
   M365_TUNNEL_BASE_URL=https://<your-hostname>
   CLOUDFLARE_TUNNEL_TOKEN=<the token>
   M365_CLIENT_STATE=<openssl rand -hex 16>
   ```
4. Start it, **before** launching the app:
   ```sh
   docker compose up -d --build
   curl https://<your-hostname>/healthz     # expect: ok
   ```

No domain? Use a throwaway tunnel instead — see the commented `command:` in
[docker-compose.yml](docker-compose.yml) — and put the printed
`https://*.trycloudflare.com` URL in `M365_TUNNEL_BASE_URL`.

## Troubleshooting

**Sign-in asks for admin approval.** The requested scopes no longer match what
was consented. Either have an admin approve the new set, or pin the old one with
`M365_SCOPES` in `.env` and sign in again.

**Nothing arrives instantly.** Check `docker compose logs webhook`, and confirm
`M365_TUNNEL_BASE_URL` exactly matches your tunnel hostname. Subscription errors
are recorded in `$TMPDIR/m365-tui.log`.

**Anything else.** Logs go to `$TMPDIR/m365-tui.log`; run with `RUST_LOG=debug`
for detail.

## Development

```sh
cargo test --workspace
cargo clippy --workspace
```

Tagging a release (`v0.1.0`) builds static binaries and publishes them via
GitHub Actions. Design notes and internals live in
[ARCHITECTURE.md](ARCHITECTURE.md).

## Not supported

Joining Teams calls or meetings (audio/video isn't a terminal thing — you can
still list and schedule them), and bulk chat export, which needs separately
approved Graph permissions.
