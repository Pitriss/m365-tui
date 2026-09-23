# m365-tui

A terminal Microsoft 365 client for **Outlook, Microsoft Teams, and Calendar**.

Switch directly between Outlook, Teams, and Calendar with `F1`, `F2`, and `F3`.
m365-tui uses Microsoft Graph and combines mail, chat, presence, calendar,
notifications, persistent conversation caching, and terminal-native image
previews in a single TUI.

![Reading an HTML email in the terminal, then jumping from its sender straight into a Teams chat with them](demo.gif)

> **Note:** This is a very old preview of m365-tui and no longer reflects the
> current interface or feature set. It is kept only to give a general idea of
> how the application can look and behave in a terminal.

Built in Rust on top of the Microsoft Graph API. For implementation details and
design notes, see [ARCHITECTURE.md](ARCHITECTURE.md).

---

## Features

### Outlook

- Read plain-text and HTML mail directly in the terminal.
- Browse nested Outlook folders as a compact tree.
- Folder and application-level unread indicators.
- Compose, reply, reply-all, and forward.
- Send and download attachments.
- Large outgoing attachments are uploaded through Graph upload sessions.
- Search mail.
- Open links through the system browser or copy them to the clipboard.
- Toggle a selected message read/unread with `u`.
- Automatically mark an opened unread message as read after a configurable
  delay.
- Preview JPEG, PNG, and GIF image attachments in terminals supporting the
  Kitty graphics protocol.
- Desktop notifications for new inbox mail.
- Optional forwarding of notification events to an ntfy server.
- Quick access to Calendar with `g`.

### Teams

- One-to-one, group, and meeting chats.
- Optional Teams/channel browsing.
- Server-side read-state synchronization with other Teams clients.
- Per-chat unread counts.
- Replies and emoji reactions.
- Presence indicators for contacts in one-to-one chats.
- Optional control of your own Teams presence.
- Configurable automatic Available → Away behavior.
- Teams system events such as recording, transcript, membership, pin, and chat
  rename events.
- Configurable filtering of low-value system events.
- Inline hosted-image previews.
- Optional previews for regular OneDrive / SharePoint image attachments.
- Persistent conversation and image cache.
- Cache-only conversation previews while moving through the chat list.
- Cached images can also be displayed during those previews.
- Cached previous conversation can be restored without immediately marking it
  read.
- Selected presence markers retain their status colour and use a high-contrast
  dark badge on the highlighted row.
- Desktop notifications for direct messages and relevant mentions.
- Optional ntfy forwarding with a Teams-specific notification tag.

### Calendar

- Dedicated Calendar screen on `F3`.
- Agenda view with selectable ranges from 7 to 365 days.
- Graphical month view with one to four months depending on terminal width.
- Navigate calendar events chronologically.
- Accept, decline, or tentatively accept invitations.
- Open event details.
- Open online meeting links using the system handler or a configured
  application.
- Visual RSVP and online-meeting indicators.
- Multi-day events rendered across calendar days.
- 15- and 5-minute event reminders with configurable internal, desktop,
  and ntfy delivery.
- Persistent Calendar view selection when persistent UI state is enabled.

#### Teams/Skype resource-consent probe

If F7 reports `consent required (AADSTS65001)` for the Skype resource token,
run the explicit one-off probe:

```sh
m365 teams-consent-probe
```

This command is isolated from normal startup. It requests only
`https://api.spaces.skype.com/.default`, does **not** request `offline_access`,
does not alter `M365_SCOPES`, and never writes the primary Graph token cache.
The interactive access token is discarded. After consent, it only verifies
whether the already-cached Graph refresh token can silently obtain the
Teams/Skype resource token. Normal `m365`, `m365 login`, F6 and F7 behavior is
unchanged unless this command is explicitly invoked.

## Diagnostics

- Global `F6` read-only diagnostics overlay.
- Contextual `F7` diagnostics for the selected Teams one-to-one contact. It
  classifies Graph/Teams identity shapes, detects a safe Teams user-MRI candidate,
  probes batch/direct Graph presence, and traces the read-only Teams path
  (Skype-resource token -> authz/Skype token -> Middle Tier MRI lookup -> UPS)
  without exporting names, email addresses, raw IDs, tokens, MRIs, or tenant IDs.
- Shows account/token health, Microsoft 365 work-plan hours and time zones,
  actual Graph token scopes, optional feature state, presence, push/cache and
  terminal integration state.
- `r` refreshes live diagnostics.
- `c` exports the same plain-text snapshot to a native clipboard helper
  (`wl-copy`, `xclip`, `xsel`, or `pbcopy`). If none is usable, the snapshot is
  automatically written to a private `/tmp/m365-tui-diagnostics-*.log` file.
- `l` always writes the log file.
- Access tokens, refresh tokens, `M365_CLIENT_STATE`, ntfy bearer tokens and
  authorization headers are never included in the diagnostics snapshot.

The work-plan section reads the same Microsoft 365
`workHoursAndLocations` data used by work-day ntfy modes. The existing
`Calendars.ReadWrite` delegated permission is sufficient; diagnostics do not add
another Graph scope.

---

## Requirements

### Microsoft 365 account

A work or school Microsoft 365 account is required for the Teams messaging APIs.

### Entra application registration

m365-tui authenticates as a public client through Microsoft device-code login.
You need an Entra application registration and its Application (client) ID.

See [Microsoft Entra setup](#microsoft-entra-setup).

### Platform

Prebuilt releases are provided for:

- Linux x86_64
- Linux aarch64

The release binaries are statically linked against musl and do not require
system OpenSSL or other shared runtime libraries.

### Optional external commands

Some desktop integrations use external commands when available.

| Feature | Command | Typical package | Fallback |
|---|---|---|---|
| Open links | `xdg-open` | `xdg-utils` | URL can still be copied |
| Clipboard on Wayland | `wl-copy` | `wl-clipboard` | OSC 52 |
| Clipboard on X11 | `xclip` or `xsel` | `xclip`, `xsel` | OSC 52 |
| Desktop notifications | `notify-send` | `libnotify-bin` | Terminal bell |

Only one clipboard utility is needed.

To check which helpers are installed:

```sh
for c in xdg-open wl-copy xclip xsel notify-send; do
  command -v "$c" >/dev/null && echo "✓ $c" || echo "✗ $c"
done
```

### Kitty image previews

Image previews require a terminal capable of displaying the image protocol used
by m365-tui. Kitty is the primary supported terminal for this functionality.

Mail and Teams image previews support:

- JPEG
- PNG
- GIF

GIF previews are displayed as static images.

The rest of the application works normally without image support.

---

## Microsoft Entra setup

Open the Microsoft Entra admin center and create a new application registration.

Recommended registration settings:

1. Choose any application name, for example `m365-tui`.
2. Use accounts from your organizational directory.
3. Leave the redirect URI empty.
4. Copy the **Application (client) ID**.
5. Copy the **Directory (tenant) ID** if you want to restrict sign-in to that
   tenant.
6. Under **Authentication**, enable **Allow public client flows**.

### Default delegated permissions

The default configuration requests:

```text
openid
profile
offline_access
User.Read
People.Read
Mail.ReadWrite
Mail.Send
Calendars.ReadWrite
Chat.ReadWrite
ChannelMessage.Send
ChannelMessage.Read.All
Presence.Read.All
```

Depending on tenant policy, some permissions may require administrator consent.

### Optional delegated permissions

Additional capabilities are deliberately opt-in so enabling a new feature does
not unexpectedly change the consent request for every user.

| Permission | Feature | Configuration |
|---|---|---|
| `Team.ReadBasic.All` | Teams and channel enumeration | `M365_TEAMS_CHANNELS=1` |
| `Presence.ReadWrite` | Set or publish your own presence | `M365_PRESENCE_WRITE=1` or `M365_PRESENCE_PRIMARY=1` |
| `Files.Read.All` | Regular Teams image files from OneDrive / SharePoint | `M365_TEAMS_FILE_IMAGES=1` |

If `M365_SCOPES` is set explicitly, optional permissions are **not**
automatically added. Include every required scope yourself.

After changing the requested scopes, an existing cached token may no longer have
the required consent. Remove the token cache and sign in again when necessary.

---

## Installation

### Release binary

Download the latest release for the current architecture:

```sh
curl -fsSL -o m365-tui.tar.gz "https://github.com/Pitriss/m365-tui/releases/latest/download/m365-tui-$(uname -m)-linux-musl.tar.gz"
tar xzf m365-tui.tar.gz
sudo install m365-tui-*/m365 /usr/local/bin/
```

The release archive also contains:

```text
m365-webhook
realtime/
README.md
ARCHITECTURE.md
LICENSE
```

`m365-webhook` and `realtime/` are needed only for optional real-time push
notifications.

A SHA-256 checksum is published alongside each release archive.

### Nix

The repository contains a Nix flake and can also be built or run through Nix.

For example:

```sh
nix run github:Pitriss/m365-tui
```

or:

```sh
nix profile install github:Pitriss/m365-tui
```

### Build from source

A recent stable Rust toolchain is required.

```sh
cargo build --release --locked
```

The resulting binary is:

```text
target/release/m365
```

With the supplied Nix development environment:

```sh
nix develop -c cargo build --release --locked
```

---

## Configuration

Configuration can come from environment variables or a `.env` file.

The application uses `dotenvy`, so a `.env` file in the current directory or a
parent directory is loaded automatically.

Start with:

```sh
cp .env.example .env
```

At minimum configure:

```dotenv
M365_CLIENT_ID=00000000-0000-0000-0000-000000000000
```

### General

| Variable | Default | Description |
|---|---|---|
| `M365_CLIENT_ID` | required | Entra Application (client) ID |
| `M365_TENANT_ID` | `organizations` | Tenant GUID, `organizations`, or `common` |
| `M365_SCOPES` | built-in defaults | Space-separated delegated Graph scopes |
| `M365_TOKEN_CACHE` | OS configuration directory | Token cache location |
| `M365_GRAPH_BASE` | Microsoft Graph | Alternate Graph endpoint, mainly for testing |
| `M365_NOTIFY` | enabled | Set to `0`, `false`, `no`, or `off` to disable desktop notifications |
| `M365_NTFY` | `never` | ntfy forwarding mode: `never`, `always`, `away`, `alwayswd`, or `awaywd` |
| `M365_NTFY_SERVER` | unset | ntfy server root URL; required when ntfy forwarding is enabled |
| `M365_NTFY_TOPIC` | unset | ntfy topic; required when ntfy forwarding is enabled |
| `M365_NTFY_TOKEN` | unset | Optional Bearer token for a protected ntfy topic |

### ntfy forwarding

m365-tui can forward the same notification events it handles locally to an
[ntfy](https://ntfy.sh/) server. Desktop delivery and ntfy delivery are
independent. `M365_NOTIFY` controls desktop notifications only. Set
`M365_NTFY=never` to disable ntfy forwarding.

Example:

```dotenv
M365_NTFY=awaywd
M365_NTFY_SERVER=https://ntfy.example.com
M365_NTFY_TOPIC=m365
# M365_NTFY_TOKEN=optional-bearer-token
```

Available forwarding modes:

| Mode | Behaviour |
|---|---|
| `never` | Disable ntfy forwarding (default) |
| `always` | Forward every eligible notification event |
| `away` | Forward only while Microsoft Graph reports your availability exactly as `Away` |
| `alwayswd` | Forward only while the current instant is inside your Microsoft 365 Work Plan |
| `awaywd` | Require both `Away` presence and an active Work Plan occurrence |

The Work Plan modes use Microsoft Graph
`/me/settings/workHoursAndLocations/occurrencesView` for the current instant.
`office`, `remote`, and `unspecified` occurrences count as working; a `timeOff`
occurrence takes precedence and suppresses forwarding. This uses the existing
`Calendars.ReadWrite` delegated permission, so enabling ntfy does not add a new
Graph scope.

Published ntfy messages carry an application tag so clients can distinguish the
source at a glance:

| Source | ntfy tag |
|---|---|
| Outlook mail | `email` (📧) |
| Teams | `speech_balloon` (💬) |
| Calendar | `calendar` (📆) |

Press `F4` while ntfy is enabled to temporarily snooze forwarding for 1, 2, 4,
8, 12, or 24 hours. The menu also contains **Resume now**; `c` or `0` resumes
immediately and `Esc` closes the menu without changing the current snooze.

Snooze does not modify `M365_NTFY`. Its absolute expiry time is persisted beside
the configured token cache, so an application restart or unexpected exit does
not reset or extend the selected snooze interval.

### Outlook

#### Delayed read marking

`M365_READ_MSG_TIMEOUT` controls how long an unread message must remain opened
before m365-tui marks it as read.

```dotenv
M365_READ_MSG_TIMEOUT=0
```

The value is in seconds.

`0` means the message is marked read immediately after its body is displayed.

Regardless of the automatic timer, press `u` to toggle the selected/open message
between read and unread.

### Teams

#### Channels

Chats work without extra configuration.

To enable Teams/channel enumeration:

```dotenv
M365_TEAMS_CHANNELS=1
```

This requires:

```text
Team.ReadBasic.All
```

#### Regular Teams image files

Hosted images embedded directly in Teams messages do not require this option.

For regular image files stored in OneDrive or SharePoint:

```dotenv
M365_TEAMS_FILE_IMAGES=1
```

This requires:

```text
Files.Read.All
```

When enabled, m365-tui resolves supported JPEG, PNG, and GIF attachments and can
display them through the terminal image renderer.

#### Teams system events

Control which system events appear in conversations:

```dotenv
M365_TEAMS_SYSTEM_EVENTS=useful
```

Accepted values:

| Value | Behaviour |
|---|---|
| `useful` | Show useful and unknown events; hide known low-value noise |
| `all` | Show all known and unknown system events |
| `none` | Hide all system events |

Default:

```text
useful
```

Useful events include, among others:

- recording events
- transcript events
- chat rename
- members added or removed
- message pinned or unpinned

Known low-value events such as call start/end and some policy/application update
events are hidden in `useful` mode.

Unknown future event types remain visible in `useful` mode so new Microsoft
event types are not silently discarded.

System-event rows are selectable for reading but cannot be replied to or reacted
to.

### Presence

#### Show contact presence

Enable status indicators for contacts in one-to-one Teams chats:

```dotenv
M365_PRESENCE_READ=1
```

This uses the default:

```text
Presence.Read.All
```

Presence is deliberately omitted for group and meeting chats.

The indicators are:

| Marker | Meaning |
|---|---|
| `●` | Available |
| `●` | Busy / In a call / In a meeting |
| `◐` | Away / Be right back |
| `×` | Do not disturb / Presenting |
| `○` | Offline / unknown |

The marker on the selected chat remains colour-coded and is rendered on a small
dark badge so the status remains visible against the selected-row background.

#### Set your own presence

Enable the presence picker:

```dotenv
M365_PRESENCE_WRITE=1
```

This adds:

```text
Presence.ReadWrite
```

Press `p` in the application to select a status.

#### Primary application presence

m365-tui can also maintain its own active presence session:

```dotenv
M365_PRESENCE_PRIMARY=1
```

This also requires `Presence.ReadWrite`.

The application refreshes the session while running and clears it when exiting.

A running Microsoft Teams client can still override or outrank the status
published by m365-tui.

#### Automatic Away

When primary presence is enabled:

```dotenv
M365_PRESENCE_AVAILABLE_TIMEOUT_MIN=5
```

controls the number of minutes of local m365-tui inactivity before the
application changes its own session from Available to Away.

Any keypress or bracketed paste makes it Available again.

Set:

```dotenv
M365_PRESENCE_AVAILABLE_TIMEOUT_MIN=0
```

to disable the automatic Away transition.

### Calendar

#### Event reminders

m365-tui can remind you about timed Calendar events 15 and 5 minutes before
they start. Reminders work while Outlook, Teams, or Calendar is active; the
Calendar screen does not need to be open.

Configure reminder delivery with:

```dotenv
M365_CALENDAR_NOTIFY=all
```

Available modes:

| Value | Behaviour |
|---|---|
| `all` | Show a non-blocking reminder inside m365-tui and send a desktop notification |
| `internal` | Show only the non-blocking reminder inside m365-tui |
| `external` | Send only a desktop notification |
| `none` | Disable Calendar reminders completely |

The default is `all`.

Cancelled, all-day, and declined events do not generate reminders.

Desktop Calendar reminders also respect the global `M365_NOTIFY` setting.
For example:

```dotenv
M365_NOTIFY=0
M365_CALENDAR_NOTIFY=all
```

keeps the internal Calendar reminder but suppresses its desktop notification.

#### Meeting opener

Online meeting links normally use the operating-system URL handler.

To force a specific executable:

```dotenv
M365_MEETING_OPENER=/usr/bin/firefox
```

The URL is passed directly as one argument. No shell is involved.

If additional command-line arguments are required, use a wrapper script.

### Persistent Teams cache

Persistent Teams state is enabled by configuring:

```dotenv
M365_TEAMS_IMAGE_CACHE_DIR=/home/user/.cache/m365-tui/teams
```

The variable name is historical and is retained for backward compatibility.

The directory is now the common root for:

- Teams image cache
- cached chat conversations
- persistent UI state

Without this variable:

- Teams images are cached only in RAM,
- persistent chat conversation cache is disabled,
- persistent UI state is disabled.

On Unix, cache directories and files are created with restrictive permissions.

Cached conversation JSON is stored as plaintext and may contain message content,
participant information, links, reactions, attachments, and chat identifiers.
Protect the cache directory accordingly.

#### Cache size

```dotenv
M365_TEAMS_IMAGE_CACHE_MAX_MB=256
```

controls the persistent image-cache size.

The default is 256 MiB.

#### Conversation cache warm-up

When persistent caching is configured, m365-tui can prefill chat caches in the
background.

Enabled by default.

Disable it with:

```dotenv
M365_TEAMS_CACHE_WARMUP=0
```

Disabling warm-up does not disable caching itself. Chats are still cached when
opened, and existing cached conversations can still be previewed locally.

### Real-time push

Polling is always available and refreshes the application every 20 seconds.

Real-time push is optional.

The main variables are:

```dotenv
M365_TUNNEL_BASE_URL=
M365_CLIENT_STATE=
M365_REDIS_URL=redis://127.0.0.1:6379
CLOUDFLARE_TUNNEL_TOKEN=
```

For normal poll-only operation, leave `M365_TUNNEL_BASE_URL` empty.

See [Real-time updates](#real-time-updates) for the complete setup.

---

## Keyboard controls

Press `?` at any time outside the Teams composer to display the built-in help.

### Global

| Key | Action |
|---|---|
| `F1` | Outlook |
| `F2` | Teams |
| `F3` | Calendar |
| `F5` | Force an immediate poll |
| `F6` | Open generic diagnostics |
| `F7` | Diagnose the selected Teams 1:1 contact |
| `Ctrl+P` | Command palette |
| `p` | Presence picker |
| `?` | Open help |
| `y` | Copy focused message |
| `Y` | Copy complete current view |
| `z` | Copy mode |
| `q` | Quit |
| `Ctrl+C` | Quit |

The Help overlay is scrollable when its contents do not fit in the terminal.
Use `j`/`k` or `Up`/`Down` to scroll one row, `PageUp`/`PageDown` for larger
steps, `Home`/`End` to jump to the beginning or end, and `Esc` to close it.

The Diagnostics overlay uses the same scrolling keys. Inside it, `r` refreshes
the live checks, `c` copies/exports the snapshot, and `l` forces a log export.
`F7` uses the same controls for the selected Teams one-to-one contact and writes
fallback logs as `/tmp/m365-tui-contact-diagnostics-*.log`.

### Navigation

The UI follows a left-to-right pane model.

| Key | Action |
|---|---|
| `h` / `Left` / `Esc` | Move out to the pane on the left |
| `l` / `Right` | Enter/open the selected item to the right |
| `j` / `Down` | Move or scroll down |
| `k` / `Up` | Move or scroll up |
| `Tab` | Cycle focus where supported |
| `PageUp` / `PageDown` | Move by larger increments |
| `Home` / `End` | First/last or beginning/end depending on context |

### Outlook

| Key | Action |
|---|---|
| `Enter` / `l` | Open selected folder/message |
| `u` | Toggle selected/open mail read/unread |
| `c` | Compose |
| `r` | Reply |
| `a` | Reply all |
| `f` | Forward |
| `/` | Search |
| `g` | Quick Calendar view |
| `A` | Attachment list for opened mail |
| `o` | Link list for opened mail |

In the reading pane, `j`/`k` scroll rather than changing the selected message.

### Teams

#### Chat list

| Key | Action |
|---|---|
| `j` / `k` | Move through chats and show local cached preview |
| `Enter` / `l` | Open selected conversation |
| `t` | Toggle chats / Teams channels |
| `i` / `a` | Open selected chat and start writing |

Moving with `j`/`k` in the chat list does not open the conversation on the
server. When a persistent cache exists, m365-tui displays the cached conversation
and cached images locally.

Opening the conversation performs normal Graph refresh and read-state handling.

#### Conversation

| Key | Action |
|---|---|
| `j` / `k` | Select previous/next visible message |
| `Home` | Oldest loaded selectable message |
| `End` / `g` | Newest selectable message |
| `r` | Reply to selected message |
| `e` | React |
| `i` / `a` | Enter composer |
| `h` / `Esc` | Return to chat list |

Deleted and filtered system-event rows are skipped during message navigation.

#### Composer

| Key | Action |
|---|---|
| `Enter` | Send |
| `Shift+Enter` / `Alt+Enter` | Insert newline |
| `Esc` | Return to conversation |
| `Tab` | Return to chat list |
| `Ctrl+W` | Delete previous word |
| `Ctrl+U` | Delete to start of line |
| `Ctrl+K` | Delete to end of line |
| `Ctrl+Left` / `Ctrl+Right` | Move by word |
| `Home` / `End` | Start/end of line |
| `Ctrl+Home` / `Ctrl+End` | Start/end of text |

### Calendar agenda

| Key | Action |
|---|---|
| `j` / `k` | Select event |
| `PageUp` / `PageDown` | Move by ten events |
| `Home` / `End` | First / last event |
| `Enter` / `g` | Event details |
| `o` | Open online meeting |
| `n` | Jump to today |
| `a` | Accept |
| `d` | Decline |
| `t` | Tentative |
| `r` | Refresh |
| `w` | Cycle agenda range |
| `v` | Month view |

### Calendar month

| Key | Action |
|---|---|
| `j` / `k` | Previous / next event in active month |
| `PageUp` / `PageDown` | Move by five events |
| `Home` / `End` | First / last event in active month |
| `Left` / `Right` | Previous / next month |
| `Enter` / `g` | Event details |
| `o` | Open online meeting |
| `n` | Current month |
| `a` / `d` / `t` | RSVP |
| `v` | Agenda view |

### Copy mode

Press `z` to enter a borderless full-width representation intended for clean
terminal mouse selection.

| Key | Action |
|---|---|
| `j` / `k` | Scroll |
| `PageUp` / `PageDown` | Scroll by 20 rows |
| `g` | Top |
| `y` | Copy complete view |
| `z` / `Esc` / `q` | Leave copy mode |
| `Ctrl+C` | Quit application |

---

## Outlook behaviour

### Read state

Opening an unread message starts the read timer.

The timer applies only to the selected/displayed message and is cancelled when
the user leaves it before the configured timeout.

`M365_READ_MSG_TIMEOUT=0` marks the message read immediately after its content
has been displayed.

Press `u` to explicitly toggle read/unread state at any time.

### Nested folders and unread counts

Outlook folders are displayed as a compact tree using characters such as:

```text
├
└
│
```

Unread counts are aligned on the right.

The application tab also indicates when unread mail exists.

### Attachments

Incoming messages containing attachments show an attachment indicator.

Press `A` and select an item with `1`–`9` to save it to the Downloads
directory.

Existing files are not overwritten.

While composing mail, use the attachment field to stage files before sending.

Small files are sent directly through Graph. Larger files use a Graph upload
session.

### Links

Links are rendered as text with numbered references rather than long raw URLs.

Press `o` to display the link list and `1`–`9` to open a link.

Microsoft Safelinks wrappers are reduced back to their actual destination when
possible.

### Image previews

JPEG, PNG, and GIF mail attachments can be previewed directly in a compatible
terminal.

The normal attachment-saving workflow remains available regardless of image
support.

---

## Teams behaviour

### Chats, channels, and unread state

Chats are available using the default Graph scopes.

Teams/channel browsing is optional and requires:

```dotenv
M365_TEAMS_CHANNELS=1
```

plus `Team.ReadBasic.All`.

One-to-one chats display unread counts.

When a chat is actually opened, m365-tui marks it read on the Microsoft 365
server so read state is synchronized with other Teams clients.

Simply moving over a chat in the list and displaying a cached preview does
**not** mark it read.

### Cached conversation preview

With persistent Teams caching enabled, moving through chat rows with `j` and `k`
can immediately show a locally cached conversation without performing a Graph
request.

Preview lookup order for images is:

```text
RAM cache
    ↓
persistent disk cache
    ↓
stop
```

A cache-only preview never downloads a missing image from Graph.

When the user actually opens the chat, the normal path becomes:

```text
RAM cache
    ↓
persistent disk cache
    ↓
Microsoft Graph
```

This keeps list navigation fast and prevents simply browsing cached chats from
causing network activity or read-state changes.

### Conversation history

Chat conversations are stored chronologically, oldest at the top and newest at
the bottom.

Messages from the same sender are visually grouped where appropriate.

A pinned date row shows the day associated with the current scroll position.

If new messages arrive while the user has moved away from the newest message,
the application keeps the current reading position and displays a new-message
count.

Press `g` to jump back to the newest message.

### Replies and reactions

Select a message and press:

```text
r    reply
e    react
```

Chat replies preserve the Teams message-reference information where available.

System-event messages are intentionally not replyable or reactable.

### Presence

One-to-one chats can show contact presence when:

```dotenv
M365_PRESENCE_READ=1
```

The selected chat keeps the presence colour rather than allowing the generic
row-selection foreground to hide it.

A dark one-cell badge gives the marker additional contrast on highlighted rows.

### Experimental directory contact profiles

> **Untested:** this path is implemented but has not been validated against a
> tenant that grants the required permission.

By default, contact profile enrichment continues to use the existing
`People.Read` / `/me/people` path.

For internal one-to-one Teams contacts whose Entra user GUID is known, an
experimental direct directory lookup can be enabled with:

```dotenv
M365_DIRECTORY_PROFILE=1
```

When default scopes are used, this adds the delegated Microsoft Graph
`User.Read.All` permission. `User.Read.All` requires administrator consent.

The direct `/users/{GUID}` lookup can request richer directory fields including:

- business phone numbers
- mobile phone
- job title
- department
- office location
- company name

If `M365_SCOPES` is set explicitly, enabling `M365_DIRECTORY_PROFILE` does not
alter that custom list; include `User.Read.All` in `M365_SCOPES` yourself.

Leave this option disabled unless intentionally testing the directory-profile
path.

### System events

Teams uses special messages for events that are not ordinary user-authored
messages.

m365-tui recognizes useful events such as:

- recording availability
- transcript availability
- chat rename
- members added or removed
- messages pinned or unpinned

Low-value system noise can be filtered through
`M365_TEAMS_SYSTEM_EVENTS`.

Unknown event types are retained in the default `useful` mode so future Graph
event kinds remain visible instead of silently disappearing.

### Images

m365-tui supports two Teams image sources.

#### Hosted inline images

Images hosted directly as Teams message content are handled without
`Files.Read.All`.

#### Regular file attachments

Images stored as normal OneDrive / SharePoint files require:

```dotenv
M365_TEAMS_FILE_IMAGES=1
```

and:

```text
Files.Read.All
```

Supported preview formats are JPEG, PNG, and GIF.

Persistent image caching is available when `M365_TEAMS_IMAGE_CACHE_DIR` is
configured.

---

## Calendar behaviour

### Agenda view

Agenda is the default Calendar view.

Available ranges:

```text
7
14
30
60
90
180
365 days
```

The default range is 30 days.

Press `w` to cycle through ranges.

The list displays independent RSVP and online-meeting indicators.

Typical RSVP markers include:

```text
[A] accepted
[T] tentative
[D] declined
[?] awaiting response
[O] organizer
[!] cancelled
```

A separate meeting indicator shows when a usable online-meeting join URL is
available.

### Event details

Press `Enter` or `g` to display details for the selected event.

The detail includes information such as:

- subject
- start/end time
- response status
- organizer
- location
- online-meeting information
- description preview

### RSVP

From Agenda, Month, or the selected event workflow:

```text
a    accept
d    decline
t    tentative
```

### Online meetings

Press `o` to open the join URL of the selected event.

m365-tui delegates actual meeting participation to an external application. It
does not implement Teams audio or video itself.

### Month view

Press `v` to switch between Agenda and Month.

The month view:

- uses a Monday-to-Sunday grid,
- displays between one and four months depending on available width,
- treats the leftmost displayed month as the active month,
- renders multi-day events across date and week boundaries,
- preserves RSVP/status styling,
- highlights the selected event.

Minimum terminal size:

```text
68 x 23
```

If Month view cannot fit, m365-tui stays in or falls back to Agenda and displays
a short notice.

### Event reminders

Timed events are checked independently of the currently visible screen.

When enabled, each eligible event can generate two reminders:

```text
15 minutes before start
5 minutes before start
```

Each reminder is emitted only once for that event and threshold during the
running m365-tui process.

Internal reminders use the normal status area and are non-blocking, so they do
not open an overlay or interrupt keyboard control.

Cancelled, all-day, and declined events are skipped.

---

## Persistent data and cache

### Authentication token

The authentication token cache defaults to the operating system configuration
directory and can be overridden with:

```dotenv
M365_TOKEN_CACHE=/path/to/token-cache.json
```

Delete the token cache when a changed set of Graph permissions requires a new
consent flow.

### Teams persistent root

Persistent Teams data uses:

```dotenv
M365_TEAMS_IMAGE_CACHE_DIR=/path/to/cache
```

Despite the historical name, this is the common persistence root for several
Teams-related features.

It contains data such as:

```text
image cache
conversation cache
UI state
```

### Conversation cache

Conversation caching currently applies to Teams chats, not channel
conversations.

Cached data is based on the original Graph message objects so replies,
reactions, links, attachment metadata, and system-event information can be
restored.

Up to the newest 2000 loaded messages are retained per chat.

Invalid, corrupt, or oversized cache files are discarded safely and the
application falls back to Graph when the chat is opened.

### UI state

Persistent UI state remembers information such as the previous screen,
Calendar view, and last Teams conversation.

Restoring cached content does not by itself have to perform the same operations
as actively opening a conversation.

### Privacy

Teams conversation cache content is stored locally as plaintext JSON.

Anyone able to read that cache may be able to read cached message content and
metadata.

Use normal filesystem protections and do not place the cache in a publicly
accessible directory.

---

## Notifications

Desktop notifications are enabled by default.

Disable desktop notifications globally with:

```dotenv
M365_NOTIFY=0
```

m365-tui can send desktop notifications for:

- new unread inbox mail,
- direct Teams messages,
- group/meeting chat messages relevant to you, such as mentions,
- Calendar event reminders when `M365_CALENDAR_NOTIFY` includes external
  delivery.

Calendar reminders have their own delivery setting,
`M365_CALENDAR_NOTIFY`. `M365_NOTIFY=0` suppresses their desktop part but does
not disable an internal Calendar reminder.

Where supported, `notify-send` is used.

Without a desktop notification helper, m365-tui falls back to a terminal bell.

The initial mail and Teams synchronization establishes a baseline instead of
generating notifications for existing history.

---

## Real-time updates

m365-tui always supports polling.

The normal poll interval is:

```text
20 seconds
```

Press `F5` to request an immediate refresh.

### Optional push notifications

For lower latency, Microsoft Graph can send change notifications through a
public HTTPS webhook.

A desktop computer is normally behind NAT and has no directly reachable HTTPS
endpoint, so the release contains an optional real-time stack under:

```text
realtime/
```

It contains the webhook service, Redis integration, Cloudflare tunnel setup, and
helper scripts.

From an extracted release:

```sh
cd m365-tui-*/realtime
./up.sh
```

The helper can use either:

- a temporary `trycloudflare.com` tunnel, or
- a named Cloudflare tunnel.

It prints the values that need to be used by m365-tui.

Typical configuration:

```dotenv
M365_TUNNEL_BASE_URL=https://example
M365_CLIENT_STATE=shared-secret
M365_REDIS_URL=redis://127.0.0.1:6379
```

The webhook does not hold the user's Microsoft Graph access token.

It validates incoming notifications and publishes a small change signal through
Redis. The TUI then performs the actual Graph fetch using its own delegated
token.

If push setup fails, the application continues operating through normal polling.

### Push status

The top status area indicates states such as:

```text
push live
push …
push FAILED
push off
```

A failed push configuration does not disable ordinary 20-second polling.

---

## Troubleshooting

### Sign-in asks for administrator approval

The requested scopes may differ from those already consented.

If permissions were intentionally changed:

1. update the Entra application permissions,
2. grant the required consent,
3. delete the old token cache,
4. sign in again.

If `M365_SCOPES` is set manually, verify that it contains every permission
required by the enabled features.

### Teams channels are unavailable

Channels require:

```text
Team.ReadBasic.All
```

and:

```dotenv
M365_TEAMS_CHANNELS=1
```

Chats do not require this option.

### Teams regular image attachments are unavailable

Regular OneDrive / SharePoint image attachments require:

```text
Files.Read.All
```

and:

```dotenv
M365_TEAMS_FILE_IMAGES=1
```

Hosted inline Teams images are handled separately.

### Presence can be viewed but not changed

Reading presence and writing presence use different permissions.

Contact presence uses:

```text
Presence.Read.All
```

Changing your own presence requires:

```text
Presence.ReadWrite
```

plus either:

```dotenv
M365_PRESENCE_WRITE=1
```

or:

```dotenv
M365_PRESENCE_PRIMARY=1
```

### Push is not working

Check the push indicator in the application.

Also inspect:

```text
$TMPDIR/m365-tui.log
```

For detailed logs:

```sh
RUST_LOG=debug m365
```

Common causes include:

- tunnel URL does not match the actual public hostname,
- `M365_CLIENT_STATE` differs between components,
- webhook or Redis is unavailable,
- Graph rejected subscription creation.

The application continues polling even when push is unavailable.

### Month view does not open

Month view requires at least:

```text
68 x 23
```

terminal cells.

Use Agenda view in smaller terminal windows.

---

## Development

Build the complete workspace:

```sh
cargo build --workspace --locked
```

Run tests:

```sh
cargo test --workspace --locked
```

Run Clippy:

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

CI also validates the deployment shell scripts and Docker Compose configuration.

### Architecture

Implementation notes are maintained separately in:

[ARCHITECTURE.md](ARCHITECTURE.md)

The TUI keeps network work off the rendering thread. Graph operations run
asynchronously and return state updates to the application loop.

### Releases

The current CI workflow runs build, lint, tests, deployment-script checks, and
Compose validation for changes pushed to `main`.

After successful CI on `main`, the release workflow builds static musl binaries
for:

```text
x86_64
aarch64
```

and publishes architecture-specific archives plus SHA-256 checksums.

Release asset names are stable so the GitHub `releases/latest/download/...`
installation URL can be used without embedding a version number.

---

## Relationship to upstream

This repository is a downstream fork of:

[github.com/rootHytx/m365-tui](https://github.com/rootHytx/m365-tui)

It retains the original project's Microsoft Graph and terminal-client
foundations but now contains substantial additional Outlook, Teams, Calendar,
presence, caching, image, navigation, and workflow functionality.

The fork is maintained and released independently.

The historical development of those changes is available through Git history
and release history rather than being duplicated as a separate
"Local enhancements" section in this README.

---

## Known limitations

- Teams audio/video calls are not implemented inside the terminal.
- Opening an online Calendar meeting delegates to an external application.
- Persistent Teams conversation caching currently covers chats, not channels.
- GIF image previews are static.
- Month view requires a sufficiently large terminal.
- Calendar events close to local date boundaries or daylight-saving transitions
  may expose timezone edge cases; month-boundary handling should not be assumed
  to be DST-perfect.
- A running Microsoft Teams client can override presence published by m365-tui.
- Full bulk Teams chat export is not implemented.
- OSC 8 terminal hyperlinks are intentionally not used; numbered links avoid
  terminal rendering problems and keep long tracked URLs out of message text.

---

## AI-assisted development

Code changes in this fork are generated with ChatGPT under maintainer direction.
Changes are built, linted, tested, and runtime-tested before release, but
AI-generated code can still contain subtle defects or incorrect assumptions.

Independent human code review is therefore recommended, especially before using
the software in production, security-sensitive environments, or workflows that
depend on Microsoft 365 data integrity.

---

## License

MIT — see [LICENSE](LICENSE).

m365-tui is not affiliated with or endorsed by Microsoft.

"Microsoft 365", "Outlook", and "Teams" are trademarks of Microsoft
Corporation.


### Presence for Microsoft personal / non-Entra contacts

Microsoft Graph presence requires a usable Entra user identity. Some Teams
one-to-one contacts are returned as `microsoftAccountUserConversationMember`
without an Entra GUID or usable Teams MRI, so their presence cannot currently
be displayed through the normal Graph path.

An experimental read-only Teams/Skype presence diagnostic path is retained for
future use, but it requires Microsoft Teams Services to be configured for the
app registration. In the current tenant the explicit consent probe stops with
`AADSTS650057` because `https://api.spaces.skype.com` is not listed in the
app registration's requested API resources.

See [`docs/non-entra-presence.md`](docs/non-entra-presence.md) for the findings,
privacy constraints, and continuation procedure.
