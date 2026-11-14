# m365-mcp-server

Outlook / Microsoft 365 MCP server over Microsoft Graph — a Rust port of
[`@lnwebworks/outlook-mcp-server`](https://github.com/access-ctrl/outlook-mcp-server)
(MIT), built as a single local binary.

Delegated, single-user authentication (device-code flow), stdio transport.
**Read-only by default**: 12 read tools, with the 12 write tools hidden from
`tools/list` and rejected by `tools/call`. The server is bound to one signed-in
account: it reaches that user's own mailbox plus any shared mailbox the user has
been delegated in Exchange and that is listed in `shared_mailboxes`. One binary,
one config file, one token file.

## Build and install

```bash
make release      # target/release/m365-mcp-server, ~5.5MB
make install      # copies to ~/.local/bin
make check        # fmt --check + clippy -D warnings + tests
```

Rust 1.88+ (rmcp's MSRV). Nothing else is needed: TLS comes from the system
framework on macOS, so there is no vendored OpenSSL or aws-lc build step.

## Configure

```bash
mkdir -p ~/.config/m365-mcp
cp config.example.toml ~/.config/m365-mcp/config.toml
chmod 600 ~/.config/m365-mcp/config.toml
$EDITOR ~/.config/m365-mcp/config.toml
```

`tenant_id` and `client_id` are required; `client_secret` is optional (the
device-code flow works as a public client). Everything else has a default. The
file must be readable only by you — the server warns at startup if it is not. A
config path can also be passed with `--config` or `M365_CONFIG`.

Then sign in once. This runs the device-code flow, prints a URL and code, and
writes delegated tokens to `~/.config/m365-mcp/tokens.json` (mode `0600`):

```bash
m365-mcp-server login
```

`user_email` must be the account you sign in as. Any `mailbox` argument must
match that address or an entry in `shared_mailboxes`; everything else is
rejected before a request is made.

`calendar_timezone` defaults to `GMT Standard Time` (Ireland/UK, which Graph
maps from `Europe/Dublin`); an explicit value in the config file or
`OUTLOOK_CALENDAR_TIMEZONE` keeps taking precedence, so existing setups are
unaffected.

Every value can be overridden by environment variables, which is handy for a
one-off run: `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`,
`OUTLOOK_USER_EMAIL`, `OUTLOOK_SHARED_MAILBOXES`, `M365_TOKEN_PATH`,
`OUTLOOK_ATTACHMENT_ROOT`, `OUTLOOK_CALENDAR_TIMEZONE`,
`OUTLOOK_EXCLUDED_FOLDER_IDS`, `OUTLOOK_SKIP_EXCLUDED_FOLDERS`,
`OUTLOOK_SIGNOFF_NAME`, `OUTLOOK_READ_ONLY`.

The Azure app registration needs the **delegated** Graph permissions `Mail.Read`
and `Calendars.Read` for read-only operation (add `Mail.ReadWrite`, `Mail.Send`
and `Calendars.ReadWrite` only if you turn writes back on), plus `offline_access`,
`openid` and `profile`. Enable **"Allow public client flows"** so the device-code
flow works, then `m365-mcp-server login` performs the one-time consent.

Because the token is delegated, the app can only reach mailboxes the signed-in
user already has Exchange access to — this is the boundary that an application
permission like `Mail.Read.All` does not have.

### Shared mailboxes

Delegated access covers shared mailboxes without a second sign-in: you keep
using `cloud.admin@maprium.com`, and pass `mailbox: "devops@maprium.com"` to any
tool. Three things have to line up.

1. **Exchange**: the signed-in user needs **Full Access** on the shared mailbox
   (Exchange admin center → Recipients → Mailboxes → Delegation). Sending *from*
   it additionally needs **Send As** or **Send on Behalf**. Permission changes
   can take up to an hour to replicate.
2. **Config**: name the mailbox explicitly — Graph has no endpoint that lists the
   mailboxes a delegated user can reach:

   ```toml
   user_email = "cloud.admin@maprium.com"
   shared_mailboxes = ["devops@maprium.com"]
   ```

   or `OUTLOOK_SHARED_MAILBOXES=devops@maprium.com,support@maprium.com`.
3. **App registration + consent**: add `Mail.Read.Shared` and
   `Calendars.Read.Shared` to the app's delegated API permissions (plus the
   `ReadWrite` / `Send` variants when `read_only = false`), then run
   `m365-mcp-server login` again — a token refresh does not widen an existing
   token. The server warns at startup when the saved token predates this.

A request for a mailbox outside the allowlist fails locally with a clear message;
an Exchange permission problem surfaces as Graph `403` (`ErrorAccessDenied`, or
`ErrorSendAsDenied` when sending). A `404` usually means the SMTP address did not
resolve — use the mailbox's user principal name or object id.

Known limitation: Graph rejects `$search` on shared and delegated mailboxes, so
`search_emails` returns an explicit error for them. Use `get_recent_emails`,
`get_unread_emails` or `get_email_folders` with the shared `mailbox` instead.
`excluded_folder_ids` is also per-mailbox — folder ids configured for your own
mailbox do not match the shared mailbox's folders.

## Read-only mode

Enabled by default. The gate is the tool router in `src/tools.rs`, which is the
only place enforcement lives:

```rust
fn build_router(read_only: bool) -> ToolRouter<McpService> {
    let mut router = McpService::tool_router();
    if read_only {
        for name in WRITE_TOOLS {
            router.disable_route(*name);
        }
    }
    router
}
```

A disabled route disappears from `tools/list` *and* fails `tools/call` with
`tool not found`, so a model that guesses a write tool name still cannot reach
it. Nothing is compiled out — flipping the flag back is all it takes.

The 12 write tools are `create_email_draft`, `create_reply_draft`,
`update_email_draft`, `send_draft`, `reply_to_email`, `send_email`, `move_email`,
`mark_email_read`, `create_calendar_event`, `update_calendar_event`,
`cancel_calendar_event`, `delete_calendar_event`.

To re-enable them for one run:

```bash
OUTLOOK_READ_ONLY=false m365-mcp-server print-tools   # 24 tools
```

or permanently, in `config.toml`:

```toml
read_only = false
```

Environment wins over the file, so a single run can override a read-only config.
The startup log line reports the effective mode (`read_only=true|false`), and
`print-tools` shows exactly what a client will see.

## Wire into OpenCode

`~/.config/opencode/opencode.json`:

```json
{
  "mcp": {
    "servers": {
      "m365": {
        "type": "local",
        "command": ["m365-mcp-server"],
        "timeout": 60000
      }
    }
  }
}
```

The `timeout` matters: OpenCode defaults MCP requests to 5000 ms, which a cold
token fetch plus two Graph round-trips can exceed.

## Verify without a client

```bash
m365-mcp-server login        # one-time device-code sign-in, writes tokens
m365-mcp-server print-tools  # tools/list JSON, resolved through a real MCP client
m365-mcp-server smoke        # live get_todays_briefing, exit 1 on Graph errors
```

`print-tools` spawns this binary as a child and performs the real MCP handshake,
so it exercises the transport as well as the tool schemas. Diff it against the
Node server once when changing tool descriptions.

## Tools

24 in total, named and described identically to upstream so existing skills keep
working. 12 are read tools and are always available; the other 12 mutate the
mailbox or calendar and are gated behind `read_only = false`.

| Area | Tools | Read-only |
|---|---|---|
| Mail read | `get_recent_emails`, `get_recent_emails_all_folders`, `get_unread_emails`, `get_conversation_thread`, `search_emails`, `get_sent_emails`, `get_email_folders`, `get_email_attachments` | yes |
| Mail write | `create_email_draft`, `create_reply_draft`, `update_email_draft`, `send_draft`, `reply_to_email`, `send_email`, `move_email`, `mark_email_read` | no |
| Calendar read | `get_upcoming_events`, `get_calendar_events` | yes |
| Calendar write | `create_calendar_event`, `update_calendar_event`, `cancel_calendar_event`, `delete_calendar_event` | no |
| Other | `triage_email`, `get_todays_briefing` | yes |

`create_calendar_event` / `update_calendar_event` refuse to double-book: they
return `conflict: true` plus the overlapping events unless `force: true` is
passed.

`triage_email` is keyword scoring (regexes), not a model call — same as upstream.

## Deliberate differences from upstream

Response shapes, key order, tool names and descriptions are otherwise
byte-identical.

1. **Failures are flagged.** Upstream returns `{"error": "..."}` with `isError`
   unset, so an expired token looks like a successful tool call. Here the
   payload is unchanged but the result is marked `isError`, and a `WARN` goes to
   stderr with the Graph status and message.
2. **Attachments are inline-only, up to 3MB combined.** The upstream
   `createUploadSession` chunked path (3–25MB) is not ported; over 3MB returns a
   clear error. The combined-total rule (not per-file) is preserved.
3. **`attachment_root` replaces the hardcoded `/opt/data`**, defaulting to
   `~/Downloads`. Paths are canonicalised, then checked component-wise against
   the root — stricter than upstream's string-prefix test.
4. **`calendar_timezone` replaces the hardcoded `W. Australia Standard Time`**
   and defaults to `GMT Standard Time` (Ireland/UK, `Europe/Dublin`). Graph wants
   Windows timezone names here, not IANA, so set e.g. `Pacific Standard Time`
   for a mailbox elsewhere.
5. **OData literals are escaped.** `conversationId` goes into a `$filter` with
   `'` doubled; upstream interpolates it raw.
6. **`bodyPreview` truncation counts characters**, not UTF-16 code units, so it
   never splits a surrogate pair.
7. **Config file instead of `.env`**, with env vars still overriding for parity.
8. **Read-only by default.** Upstream exposes all 24 tools unconditionally; here
   the 12 write tools are disabled at the router unless `read_only = false`. The
   names, schemas and responses of the remaining 12 are unchanged.
9. **Delegated, single-user auth instead of app-only.** Upstream uses client
   credentials (`Mail.Read.All` application permission), which can read every
   mailbox in the tenant. Here the server signs in as one user via the
   device-code flow: the `mailbox` argument is limited to that identity plus an
   explicit `shared_mailboxes` allowlist, and Graph still enforces the Exchange
   permissions behind it.

## Testing

```bash
make test         # builds the bin first, then 59 tests
```

- `src/http_tests.rs` runs the client against a mock Graph server and asserts the
  exact endpoint strings, query encoding (`$search="..."`, `%20` not `+`),
  `Content-Type`/`Authorization`/`ConsistencyLevel` headers, JSON request bodies,
  a single refresh-token exchange reused across calls, and 202/204 handling.
  Tokens are seeded in memory, so no Azure credentials are needed.
- Output-shape tests pin the `undefined`-omitted vs `null`-emitted distinction,
  which is the easiest thing to break when refactoring.
- Two end-to-end tests spawn the real binary and speak MCP over stdio: one
  asserts the default read-only surface (12 tools, a `send_email` call rejected
  with `tool not found`, a read call still dispatching and flagging its 403), the
  other sets `OUTLOOK_READ_ONLY=false` and gets all 24. They catch stdout
  pollution and dispatch regressions that request-level tests cannot see, and
  skip (loudly) if the bin is older than the test harness.
- Router tests pin `WRITE_TOOLS ∪ READ_TOOLS` against the real `tools/list` and
  check the two sets are disjoint, so adding a tool without classifying it fails
  the build.
- No network and no credentials required.

Extras: `M365_LOG=debug` sets the log filter, and `M365_GRAPH_BASE` /
`M365_LOGIN_BASE` redirect the client at another base URL (the tests use the
latter). All logs go to stderr; stdout is the JSON-RPC channel.

## Layout

```
src/main.rs            CLI: serve (default) | smoke | print-tools | login
src/config.rs          TOML + env override, permission check, path expansion
src/auth.rs            delegated device-code flow, scope set
src/token.rs           on-disk access/refresh token cache (0600)
src/graph/mod.rs       delegated token cache + refresh, graph_fetch, query encoding
src/graph/mail.rs      mail ops + response mappers
src/graph/calendar.rs  calendar ops + daily briefing
src/graph/attach.rs    attachment resolution, limits, inline base64
src/triage.rs          keyword triage
src/tools.rs           24 #[tool] definitions, WRITE_TOOLS + read-only router
src/out.rs             CallToolResult plumbing + JSON null/omitted helpers
src/http_tests.rs      mock-Graph wire tests
```

## Attribution

Port of `@lnwebworks/outlook-mcp-server` v2.3.1 by access-ctrl, MIT licensed.
Original tool descriptions and Graph endpoint strings are reproduced from that
project.
