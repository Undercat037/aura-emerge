BIN := target/release/aura-emerge

.PHONY: build test install uninstall clean completions man all

build:
	cargo build --release

test:
	cargo test --release

install: build test
	sudo cp -r $(BIN) /usr/bin/emerge

uninstall:
	sudo rm -f /usr/bin/emerge

clean:
	cargo clean


completions: build
	sudo install -Dm644 <($(BIN) --gen-completions bash) /usr/share/bash-completion/completions/emerge
	sudo install -Dm644 <($(BIN) --gen-completions zsh)  /usr/share/zsh/site-functions/_emerge
	sudo install -Dm644 <($(BIN) --gen-completions fish) /usr/share/fish/vendor_completions.d/emerge.fish

man: build
	sudo install -Dm644 <($(BIN) --gen-manpage) /usr/share/man/man1/emerge.1

all: install