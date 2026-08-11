BIN := target/release/aura-emerge
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')

.PHONY: build test install uninstall clean completions man git all

build:
	cargo build --release

test:
	cargo test --release

git:
	git add .
	git commit -m "v$(VERSION) commit"

install: build test
	sudo cp -r $(BIN) /usr/bin/emerge

uninstall:
	sudo rm -f /usr/bin/emerge

clean:
	cargo clean

completions: build
	$(BIN) --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge > /dev/null
	$(BIN) --gen-completions zsh | sudo tee /usr/share/zsh/site-functions/_emerge > /dev/null
	$(BIN) --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish > /dev/null

man: build
	sudo install -Dm644 <($(BIN) --gen-manpage) /usr/share/man/man1/emerge.1

all: install
