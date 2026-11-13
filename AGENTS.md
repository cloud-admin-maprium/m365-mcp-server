# Working notes for agents

## Invariants — break these and the server stops working

- **stdout is the JSON-RPC channel.** Every log line goes through `tracing` to
  stderr (configured in `src/main.rs::init_logging`). A stray `println!` in the
  server path corrupts the protocol stream. Only `smoke` and `print-tools` may
  print to stdout, and they never serve. `login` is interactive but writes its
  prompts to stderr, so stdout stays clean there too.
- **Auth is delegated and single-user.** `GraphClient::access_token()` only
  refreshes a token written by `login` (device-code flow). Never reintroduce a
  `client_credentials` path: an app-only token can read every mailbox in the
  tenant. `graph_fetch` also rejects any `mailbox` argument that is not
  `user_email`. Tokens (in `src/token.rs`) are secrets like the client secret.
- **Tool names, descriptions and response shapes are a compatibility contract.**
  They match `@lnwebworks/outlook-mcp-server` 2.3.1 exactly, because client
  configs and the `daily-briefing` skill reference tool names and read specific
  JSON fields. Changing a name or a key is a breaking change.
- **`undefined`-omitted vs `null`-emitted.** JS drops keys whose value is
  `undefined` and keeps `null` when the source wrote `|| null`. Use exactly one
  of the `src/out.rs` helpers per field: `put_opt_str`/`put_opt_bool` (omitted)
  vs `put_null_str` (null) vs `put_or_empty` (`|| ''`). There are tests for this
  in `src/out.rs` and `src/graph/mail.rs`.
- **202/204 short-circuit before any body parse** (`src/graph/mod.rs`,
  `graph_fetch`). Send, move, cancel and delete all depend on it.
- **Query strings are built by hand**, never with `query_pairs`: Graph's OData
  syntax needs literal `$`/space/quote handling. Values go through `enc()`
  (`encodeURIComponent` semantics), OData literals through `odata_quote()`.
- **Secrets never reach a log line.** `Config::summary()` is the only sanctioned
  way to describe configuration.

## Before finishing a change

```bash
make check     # cargo fmt --check, clippy --all-targets -D warnings, cargo test
make tools     # tools/list via a real MCP client handshake — diff when changing schemas
```

No network or Azure credentials are needed for any of it.

`cargo test` alone can run against a stale `target/debug/m365-mcp-server`; the
end-to-end test detects that and skips with a message, so use `make test`
(builds first) when you touched code the server binary exercises.
