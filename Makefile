PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
SYSTEMD_USER_DIR ?= $(HOME)/.config/systemd/user
SERVICE_NAME := quickshell-hydrogen.service
.PHONY: build test install service uninstall clean
build:
	cargo build --release
test:
	cargo test
install: build
	install -Dm755 target/release/hydrogen $(BINDIR)/hydrogen
service: install
	mkdir -p $(SYSTEMD_USER_DIR)
	printf '%s\n' '[Unit]' 'Description=Hydrogen Rust Indexer and Local OCR Backend for Quickshell' 'After=graphical-session.target' '' '[Service]' 'Type=simple' 'ExecStart=$(BINDIR)/hydrogen daemon' 'Restart=on-failure' 'RestartSec=2' 'Nice=15' '' '[Install]' 'WantedBy=default.target' > $(SYSTEMD_USER_DIR)/$(SERVICE_NAME)
	systemctl --user daemon-reload
	systemctl --user enable --now $(SERVICE_NAME)
uninstall:
	-systemctl --user disable --now $(SERVICE_NAME)
	rm -f $(SYSTEMD_USER_DIR)/$(SERVICE_NAME) $(BINDIR)/hydrogen
	systemctl --user daemon-reload
clean:
	cargo clean
