PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
SHAREDIR ?= $(PREFIX)/share/hydrogen
SYSTEMD_USER_DIR ?= $(HOME)/.config/systemd/user
SERVICE_NAME := quickshell-hydrogen.service

.PHONY: build test install ocr-deps service uninstall clean

build:
	cargo build --release

test:
	cargo test

install: build
	install -Dm755 target/release/hydrogen $(BINDIR)/hydrogen
	install -Dm755 scripts/ocr_worker.py $(SHAREDIR)/ocr_worker.py
	install -Dm755 scripts/install-ocr.sh $(SHAREDIR)/install-ocr.sh

ocr-deps: install
	PREFIX="$(PREFIX)" bash $(SHAREDIR)/install-ocr.sh

service: install
	mkdir -p $(SYSTEMD_USER_DIR)
	printf '%s\n' \
		'[Unit]' \
		'Description=Hydrogen Real-Time Indexer & Desktop Backend for Quickshell' \
		'After=graphical-session.target' \
		'' \
		'[Service]' \
		'Type=simple' \
		'ExecStart=$(BINDIR)/hydrogen daemon' \
		'Restart=on-failure' \
		'RestartSec=2' \
		'Nice=15' \
		'' \
		'[Install]' \
		'WantedBy=default.target' \
		> $(SYSTEMD_USER_DIR)/$(SERVICE_NAME)
	systemctl --user daemon-reload
	systemctl --user enable --now $(SERVICE_NAME)

uninstall:
	-systemctl --user disable --now $(SERVICE_NAME)
	rm -f $(SYSTEMD_USER_DIR)/$(SERVICE_NAME)
	rm -f $(BINDIR)/hydrogen
	rm -rf $(SHAREDIR)
	systemctl --user daemon-reload

clean:
	cargo clean
