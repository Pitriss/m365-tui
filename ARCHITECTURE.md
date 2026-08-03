# Architecture

How m365-tui is put together and why. For installing and using it, see the
[README](README.md).

## Crate layout

A Cargo workspace with one library and two binaries:

```
crates/
  m365-core/   library — auth, Graph client, models, endpoint wrappers,
               subscriptions, and the change-event bus
  m365-tui/    binary `m365` — the ratatui application
  webhook/     binary `m365-webhook` — axum change-notification receiver
```

`m365-core` knows nothing about the terminal, and the TUI holds no HTTP logic.
The webhook shares the core only for its event types.

### m365-core

| Module | Responsibility |
|---|---|
| `auth.rs` | OAuth2 device-code flow, token cache, silent refresh |
| `graph.rs` | HTTP client: throttling, retries, paging, delta, uploads |
| `config.rs` | Environment-driven settings and scope selection |
| `models.rs` | Serde structs for the Graph resources actually rendered |
| `mail.rs` `calendar.rs` `chats.rs` `channels.rs` `people.rs` | Endpoint wrappers |
| `subscriptions.rs` | Change-notification lifecycle |
| `events.rs` | Redis subscriber → typed UI events |
| `util.rs` | base64, HTML escaping |

### m365-tui

| Module | Responsibility |
|---|---|
| `app.rs` | All state and the async orchestration; key handling |
| `ui.rs` | Rendering — a pure function of `&App` |
| `content.rs` | HTML → styled terminal text, link extraction |
| `editor.rs` | Text buffer with cursor, wrapping, and editing operations |
| `clipboard.rs` `opener.rs` `files.rs` | System integration |
| `navigation.rs` | Cross-links between the Outlook and Teams sides |

## Authentication

There is no first-party MSAL for Rust, so the device-code flow is implemented
directly: POST the scopes to `/devicecode`, show the user the code, then poll
`/token` until they finish in a browser. `offline_access` yields a refresh token,
and the result is cached as a `0600` JSON file in the user's config directory.
Tokens refresh silently 60 seconds before expiry.

**Scopes are deliberately stable.** Changing the requested scope set invalidates
an existing consent grant, and in tenants that disallow user consent that turns
every sign-in into an admin-approval request. New optional capabilities are
therefore opt-in (see `PRESENCE_WRITE_SCOPE`) rather than added to the defaults.

## The Graph client

`GraphClient` wraps `reqwest` and centralises the things every call needs:

- **Throttling** — honours `Retry-After` on 429, exponential backoff on 5xx.
- **Auth retry** — one forced token refresh on a 401.
- **Paging** — `get_page` (single page), `get_page_with_next` (page + a
  `@odata.nextLink` for incremental loading), `get_collection` (follows every
  link until exhausted).
- **Delta** — persists `@odata.deltaLink` for incremental sync.
- **Uploads** — `put_upload_chunk` for attachment upload sessions. Upload URLs
  are pre-authenticated, so the bearer token is deliberately *not* sent to them.
  Chunks are passed as `Bytes` — refcounted views into the single buffer holding
  the file — so the payload is never copied per chunk. `Content-Length` is left
  to reqwest, which derives it from the body; setting it explicitly would append
  a second, conflicting header.

The distinction between `get_page` and `get_collection` matters: an inbox view
uses `get_page`, because `get_collection` would walk the entire mailbox.

## The UI loop

The terminal thread never blocks on the network. Key handlers spawn tokio tasks;
each sends an `AppMessage` back over an mpsc channel, and the main loop applies
it to the state before the next redraw:

```
key/paste ──> App::on_key ──> tokio::spawn ──> Graph
                                  │
     redraw <── App::apply <── AppMessage (mpsc)
```

`ui.rs` is a pure function of `&App`, so rendering can never mutate state. The
one exception is a `Cell<usize>` width hint the renderer writes so the editor's
`Up`/`Down` move by the rows actually on screen.

### List update semantics

Incoming pages carry a `ListUpdate` that says how they combine with what is
already displayed:

| Mode | When | Effect |
|---|---|---|
| `Replace` | folder switch, conversation opened, search | swap the list |
| `Append` | scrolled to the end | add older items after the current ones |
| `Merge` | 20s poll, push notification, post-send refresh | fold the newest page in, dedupe by id, **keep** older pages already loaded |

`Merge` exists because a plain refresh would otherwise discard everything the
user had scrolled back through. Selection is restored by message id rather than
index, so arriving messages don't move the cursor.

Consecutive messages from one person share a single author header, the way chat
clients do. A run breaks on a different sender, a day boundary, a deleted
message, or a pause over 15 minutes — long enough that repeating the name is
useful context again.

Every message opens with `marker + HH:MM`, so a run under one name still shows
when each line was sent, and body lines are indented by exactly that gutter to
keep the text in a single column. The marker staying in the margin means every
message remains individually selectable for reactions, grouped or not.

Teams conversations add the usual chat-client behaviour on top: messages are
newest-first, so an arrival lands *above* the reader. If the selection is already
on the newest message the view follows it; otherwise the position is held and the
new messages are counted into `unseen`, surfaced in the pane title. Without this
a new message would silently appear off-screen above wherever the user was
reading.

## Rendering message bodies

Mail and Teams bodies arrive as HTML. `content.rs` parses it with `html5ever`
and walks the DOM straight into styled ratatui text.

An earlier version converted HTML → Markdown → text; that produced escaping
artifacts and pipe-tables, so the intermediate step was removed.

Links are **not** printed inline. A link renders as its anchor text plus a `[n]`
marker, with the targets collected into a list the user can open or copy. This
keeps Microsoft Defender "Safelinks" — which expand a short URL into ~800
characters of tracking wrapper — from swamping the message. Those are also
unwrapped back to their original target by decoding the `url` parameter.

Parsing is done once when a message is opened and the result cached; only the
cheap layout runs per frame.

## Real-time updates

Two independent mechanisms, so the app is never dependent on the tunnel:

1. **Polling** — a 20-second timer refreshes the current view. Always on.
2. **Push** — Graph change notifications, when a tunnel is configured.

### Why a tunnel is needed at all

Graph change notifications are a *webhook*: Microsoft's servers make an inbound
HTTPS request to a URL you register. A desktop client has no public address, sits
behind NAT, and has no certificate, so Graph cannot reach it. The tunnel supplies
a public HTTPS hostname and forwards to the local webhook without opening a port.

It buys latency only — roughly 1–2 seconds instead of up to 20. Every feature
works identically without it.

### Subscription resources

Chat subscriptions must name the user explicitly —
`users/{id}/chats/getAllMessages`. The `/me/` shorthand is rejected with a 403
("User may only create user-scoped chat message subscriptions for their own
messages"), because the subscription service resolves the resource later,
outside the caller's context. Mail (`me/mailFolders('inbox')/messages`) does
accept it.

### Reporting health

Because push failures are silent by nature — the app just keeps polling — the
subscription manager reports a `PushState` (`Off`/`Connecting`/`Live`/`Failed`)
to the UI, shown in the tab bar. A misconfigured hostname previously degraded to
polling with nothing but a line in the log to show for it.

The push path is **notify-then-delta**:

```
Graph --HTTPS--> cloudflared --> webhook --> Redis --> TUI --delta fetch--> Graph
  ^                                                     |
  └──────────── TUI creates/renews subscriptions ───────┘
```

The webhook retries its Redis connection on startup: `depends_on` only waits for
the container to start, not for the server to accept connections, and exiting on
the first failure left the service dead until someone looked.

The webhook holds no Graph token and never calls Graph. It only echoes the
validation token during subscription setup, verifies `clientState`, and
republishes a small "something changed" signal. The TUI — which does hold the
delegated token — reacts with a targeted delta fetch.

Subscriptions use `includeResourceData: false`, which means **no encryption
certificate is required**. Requesting resource data inside notifications would
oblige us to manage a certificate and decrypt payloads; signalling instead
avoids that entirely, and keeps the internet-facing service credential-free.

Teams chat subscriptions expire after about an hour, so the TUI renews every 45
minutes and supplies a `lifecycleNotificationUrl` for reauthorization events.

## Sending attachments

Graph's one-shot `sendMail`/`reply`/`replyAll`/`forward` actions cannot carry
files, so a message with attachments takes a different route:

1. Create a draft (`POST /me/messages`, or `createReply`/`createReplyAll`/
   `createForward` for the reply kinds).
2. For replies, the draft already holds the quoted original, so the user's text
   is prepended with a `PATCH` — the one-shot actions do this implicitly.
3. Attach each file: inline `contentBytes` under 3 MB, otherwise an upload
   session with chunks that are a multiple of 320 KiB.
4. `POST /me/messages/{id}/send`.

Messages without attachments still use the simpler one-shot path.

`send_message` takes its attachments **by value** so each file's buffer can be
moved into `Bytes` once and then sliced per chunk without copying.

The whole file is held in memory while sending. That is a deliberate trade for a
single code path — the small-attachment route has to buffer anyway to base64 it
— and is fine at ordinary attachment sizes. Streaming from disk would be the
change if very large files ever mattered; it would mean `m365-core` doing
filesystem I/O, which it otherwise avoids.

## Chrome layout

State and guidance are kept apart, each with one home, so neither has to compete
for space with the other:

| Position | Content | Lifetime |
|---|---|---|
| Top left | Which app is active | — |
| Top right | Presence, push health, memory, last sync | Persistent |
| Bottom left | Latest action or error | Cleared after ~10s |
| Bottom right | Keys valid for the current focus | Follows focus |

Panes carry no key hints of their own: anything the user can press appears
bottom-right, where it tracks focus rather than going stale.

Transient text expires on a 2-second local tick that also samples memory. The
tick counts how long the message has been unchanged rather than timestamping
each write, which avoids threading an expiry through the ~30 places that set it.

## Notifications

Only messages actually addressed to the user raise one: direct chats always,
group chats and channels only on an `@mention`. Anything looser would be
unusable in a busy tenant.

Detection prefers a message's `mentions` array, which is exact. Chat *previews*
— which is all the chat list carries for conversations that aren't open — omit
it, so there is a fallback that scans the rendered `<at>…</at>` spans for the
user's name. Prose that merely contains the name is not a mention.

Two guards stop it becoming noise: notified message ids are remembered so a
20-second poll can't repeat itself, and the first chat-list sync only records a
baseline — otherwise every conversation's history would announce itself at
startup.

## Handling untrusted input

Two places take data from anyone who can send you a message:

- **Attachment names** (`files.rs`) — reduced to a bare file name before
  writing, so `../../.ssh/authorized_keys` lands in the download directory as
  `authorized_keys`. Control characters, including terminal escapes, are
  stripped, and existing files are never overwritten.
- **Link targets** (`opener.rs`) — only `http`, `https` and `mailto` are handed
  to the system opener.

## Known constraints

- **No OSC 8 hyperlinks.** Ratatui diffs a grid of cells and writes each cell's
  content; an escape sequence inside a cell breaks width accounting and can be
  left unterminated on a partial redraw. Hence numbered links instead.
- **`Merge` can retain a deleted message** until the conversation is reopened,
  if it was deleted outside the refreshed window. The alternative — re-fetching
  every loaded page on each poll — costs far more.
- **Presence has two halves.** `setUserPreferredPresence` records a sticky
  preference, but what colleagues see comes from an active *presence session* —
  normally the Teams client. With no session the user reads as Offline whatever
  the preference says. The app therefore also calls `setPresence` with its own
  client ID as `sessionId`, which delegated `Presence.ReadWrite` permits, so a
  status set from the terminal is actually visible. Sessions expire (5 min–4 h),
  so the app leases an hour, renews every 30 minutes, and clears the session on
  exit rather than leaving a stale status behind. A running Teams client still
  outranks the app's session, and the session vocabulary is narrower than the
  preference one (`Busy` must be `InACall`, `DoNotDisturb` must be
  `Presenting`).
- **No call media.** Joining Teams audio/video is not a terminal capability.

## Releases

Pushing a `v*.*` tag runs `.github/workflows/release.yml`: it builds both
binaries for `x86_64-unknown-linux-musl` — statically linked, so the artifact
has no libc dependency and runs on any distribution — and publishes them with a
SHA-256 checksum.

The asset filename is deliberately **not** versioned
(`m365-tui-x86_64-linux-musl.tar.gz`), so that
`/releases/latest/download/<asset>` always resolves and the README can offer a
copy-paste install. The version lives in the directory inside the archive.
