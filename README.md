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

### To run it

| | |
|---|---|
| **A work or school Microsoft 365 account** | Personal accounts can't use the Teams messaging APIs |
| **An Entra app registration** | See [step 1](#1-register-an-app-in-entra) — you need a client ID |
| **Linux on x86_64** | The release binary is statically linked against musl: no libc, no shared libraries, runs on any distribution including NixOS |

Nothing else. The binary has no runtime library dependencies.

### Optional helpers

These are external commands the app calls when present. Each has a fallback, so
nothing breaks if one is missing.

| Feature | Command | Package | Without it |
|---|---|---|---|
| Open links (`o`) | `xdg-open` | `xdg-utils` | Links can still be copied |
| Copy (`y`, `Y`) | `wl-copy` (Wayland), or `xclip` / `xsel` (X11) | `wl-clipboard`, `xclip`, `xsel` | Falls back to the OSC 52 escape, which most modern terminals accept |
| Notifications | `notify-send` | `libnotify` (Debian/Ubuntu: `libnotify-bin`) | Falls back to the terminal bell |

Check what you have:

```sh
for c in xdg-open wl-copy xclip xsel notify-send; do
  command -v "$c" >/dev/null && echo "✓ $c" || echo "✗ $c"
done
```

You only need **one** clipboard tool — `wl-copy` on Wayland, `xclip` or `xsel` on X11.

### For instant push (optional)

Only if you want ~1-second updates instead of the 20-second refresh:

- **Docker** and **Docker Compose** — they run the webhook, Redis and the tunnel;
  you don't install `cloudflared` or Redis yourself.
- **A Cloudflare account with a domain on it**, for a named tunnel. Without a
  domain you can use a throwaway `trycloudflare.com` tunnel instead.

### To build from source

Not needed if you use the release binary or the Nix flake.

- **Rust**, recent stable (developed against 1.96). Cargo fetches the crate
  dependencies itself; `Cargo.lock` is committed.
- **or Nix**, which supplies the toolchain via `nix develop`.

No system libraries are required: TLS is handled by rustls, so there's no
OpenSSL dependency.

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

**Download the release binary** — nothing else required:

```sh
curl -fsSL -o m365-tui.tar.gz \
  https://github.com/rootHytx/m365-tui/releases/latest/download/m365-tui-x86_64-linux-musl.tar.gz
tar xzf m365-tui.tar.gz
sudo install m365-tui-*/m365 /usr/local/bin/
```

Each release also publishes a `.sha256` next to the tarball if you want to
verify it. `m365-webhook` is in the same archive; you only need it for
[instant push](#instant-push-optional), and Docker builds it for you there.

<details>
<summary>Other ways to install</summary>

**Nix** — run it without installing, or add it to a system/home-manager flake
(`m365-tui.packages.${system}.default`):

```sh
nix run github:rootHytx/m365-tui
nix profile install github:rootHytx/m365-tui
```

**From source** — needs a Rust toolchain:

```sh
cargo build --release            # binary at target/release/m365
nix develop -c cargo build --release    # or via the bundled dev shell
```

</details>

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
| **Reading a mail** | `j`/`k` scroll · `Home`/`End` · `Esc` back to the list |
| **Teams** | `Tab` change pane · `Enter` open · `t` chats↔channels · `j`/`k` select message · `g` newest · `e` react · `i` write · `Enter` send · `Esc` back |
| **Attachments** | `A` list · `1`–`9` save to Downloads |
| **Links** | `o` list · `1`–`9` open in browser |
| **Copying** | `y` copy message · `Y` copy everything · `z` copy mode |
| **Writing** | `←→↑↓` move · `Ctrl+←→` by word · `Home`/`End` · `Ctrl+W`/`Ctrl+U`/`Ctrl+K` delete · `Ctrl+S` send · `Esc` cancel |

Scrolling to the end of a message list or conversation loads the next 50 items.

The **top-right** shows live state — your presence, push health, memory use and
the last sync time. The **bottom-right** shows the keys available right now,
changing with whatever has focus. The bottom-left carries the most recent
message, which clears itself after a few seconds.

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

**Notifications.** You're told about things actually addressed to you:

| | Notifies |
|---|---|
| New inbox mail | Always, unless already read elsewhere |
| Direct messages | Always |
| Group chats, meeting chats | Only when someone `@mentions` you |

Mail is announced even while you're reading another folder. Uses `notify-send`
if installed, otherwise the terminal bell. Turn it off with `M365_NOTIFY=0` in
`.env`.

**Conversations read like a chat.** Consecutive messages from the same person
are grouped under one name, each with its own timestamp down the left, and a
`Today`/`Yesterday` header pinned at the top of the pane as you scroll.

**Open chats update themselves.** New messages appear in the conversation you're
reading — within a second with push enabled, otherwise on the 20-second refresh.
If you're on the newest message the view follows along; if you've scrolled back
to read history it stays put and the title shows `▲ 2 new`. Press `g` to jump
back to the newest.

**Your status.** `p` sets Available / Busy / DND / Be right back / Away / Appear
offline, and it works **without Teams running** — the app publishes its own
presence session, so colleagues see the status you pick. Quitting the app clears
it, and the session is renewed automatically while the app is open.

Needs one extra permission (`Presence.ReadWrite`) plus `M365_PRESENCE_WRITE=1` in
`.env`; without them the picker is read-only.

Two quirks worth knowing:

- A **signed-in Teams client outranks the app.** If Teams is running and goes
  idle, it can pull `Available` down to `Away`. With no Teams client, what you
  pick is what shows.
- The session API's vocabulary is narrower than the picker's, so **Busy** shows
  as "In a call" and **Do not disturb** as "Presenting". The colour and the
  do-not-disturb behaviour are right; the sub-label is Microsoft's, not ours.

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

### Is it actually working?

**The top-right corner tells you**, alongside your presence, memory use and last
sync time:

| | Meaning |
|---|---|
| `push live` (green) | Subscriptions are live — changes arrive in seconds |
| `push …` (yellow) | Still subscribing |
| `push FAILED` (red) | Graph rejected it; the reason appears in the status bar, and it falls back to polling |
| `push off` (grey) | No tunnel configured — normal poll-only mode |

To confirm end-to-end, send yourself an email from your phone: it should appear
within a second or two, well before the `⟳ synced` clock changes.

If it says failed, the usual cause is `M365_TUNNEL_BASE_URL` not exactly matching
your tunnel hostname — Graph reports `Failed to resolve domain ...`. Check
`docker compose logs webhook` to see requests arriving.

## Troubleshooting

**Sign-in asks for admin approval.** The requested scopes no longer match what
was consented. Either have an admin approve the new set, or pin the old one with
`M365_SCOPES` in `.env` and sign in again.

**Nothing arrives instantly.** Look at the push indicator in the tab bar — see
[Is it actually working?](#is-it-actually-working). Subscription errors are also
recorded in `$TMPDIR/m365-tui.log`.

**Anything else.** Logs go to `$TMPDIR/m365-tui.log`; run with `RUST_LOG=debug`
for detail.

## Development

Only needed if you're changing the code — running it doesn't require any of this.

```sh
cargo test --workspace
cargo clippy --workspace
```

Pushing a `v*.*` tag builds the static `x86_64-linux-musl` binaries and publishes
them as a GitHub release, which is what the install step downloads. Design notes
and internals live in [ARCHITECTURE.md](ARCHITECTURE.md).

## Not supported

Joining Teams calls or meetings (audio/video isn't a terminal thing — you can
still list and schedule them), and bulk chat export, which needs separately
approved Graph permissions.
