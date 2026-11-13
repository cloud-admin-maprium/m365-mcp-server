BIN := m365-mcp-server
PREFIX ?= $(HOME)/.local/bin

.PHONY: all build release test lint fmt check install uninstall clean smoke tools

all: build

build:
	cargo build

release:
	cargo build --release

test: build
	cargo test

fmt:
	cargo fmt

lint:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

check: lint test

install: release
	@mkdir -p $(PREFIX)
	@# Atomic replace: writing over a running binary makes macOS SIGKILL it,
	@# and reusing the inode can leave a stale code-signature cache behind.
	cp target/release/$(BIN) $(PREFIX)/.$(BIN).new
	chmod 755 $(PREFIX)/.$(BIN).new
	mv -f $(PREFIX)/.$(BIN).new $(PREFIX)/$(BIN)
	@echo "installed $(PREFIX)/$(BIN)"

uninstall:
	rm -f $(PREFIX)/$(BIN)

# Print the tools/list payload the MCP client will see (real client, real handshake).
tools:
	./target/release/$(BIN) print-tools

# Live get_todays_briefing against the configured mailbox. Exit 1 on Graph errors.
smoke:
	./target/release/$(BIN) smoke

clean:
	cargo clean
