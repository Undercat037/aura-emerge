BIN := target/release/aura-emerge
VERSION := $(shell grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')

.PHONY: build test install uninstall clean completions man git all deny deltainst

build:
	cargo build --release

test: deny
	cargo test --release

git:
	git add .
	git commit -m "v$(VERSION) update"

push: git
	git push --follow-tags

uninstall:
	sudo rm -f /usr/bin/emerge /usr/bin/portageq /usr/bin/eselect

clean:
	cargo clean

link:
	sudo ln -sfn /usr/bin/emerge /usr/bin/portageq
	sudo ln -sfn /usr/bin/emerge /usr/bin/eselect

completions:
	$(BIN) --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge > /dev/null
	$(BIN) --gen-completions zsh | sudo tee /usr/share/zsh/site-functions/_emerge > /dev/null
	$(BIN) --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish > /dev/null
	sudo install -m644 misc/eselect.bash-completion /usr/share/bash-completion/completions/eselect
	sudo install -m644 misc/_eselect /usr/share/zsh/site-functions/_eselect
	sudo install -m644 misc/eselect.fish /usr/share/fish/vendor_completions.d/eselect.fish

man:
	$(BIN) --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 > /dev/null

install: build completions man
	sudo install -m755 -o root -g root $(BIN) /usr/bin/emerge
	make link

deltainst:
	RUSTFLAGS="-C target-cpu=znver3 -C opt-level=3 -C codegen-units=1 -C target-feature=+avx2,+fma,+bmi1,+bmi2,+aes,+sha,+sse4.2,+popcnt,+vaes,+vpclmulqdq,+movbe,+f16c" cargo build --release
	$(BIN) --gen-completions bash | sudo tee /usr/share/bash-completion/completions/emerge > /dev/null
	$(BIN) --gen-completions fish | sudo tee /usr/share/fish/vendor_completions.d/emerge.fish > /dev/null
	$(BIN) --gen-manpage | sudo tee /usr/share/man/man1/emerge.1 > /dev/null
	sudo install -m755 -o root -g root $(BIN) /usr/bin/emerge

deny:
	cargo deny check -A parse-error

all: install
