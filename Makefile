PREFIX ?= $(HOME)/.local
BINDIR ?= $(PREFIX)/bin
SYSTEMD_USER_DIR ?= $(HOME)/.config/systemd/user
SERVICE_NAME := quickshell-hydrogen.service

.PHONY: all build release run test check fmt fmt-check lint install models extension-package service uninstall clean help

all: check test build

build:
	cargo build

release:
	cargo build --release

run:
	cargo run -- daemon

test:
	cargo test

check:
	cargo check

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

lint:
	cargo clippy --all-targets --all-features -- -D warnings

install: release
	install -Dm755 target/release/hydrogen $(BINDIR)/hydrogen

models:
	bash scripts/install-models.sh

# Build Firefox XPI and Chrome-compatible ZIP without modifying source manifests.
extension-package:
	bash scripts/package-browser-extension.sh

service: install
	mkdir -p $(SYSTEMD_USER_DIR)
	printf '%s\n' '[Unit]' 'Description=Hydrogen local knowledge service' 'After=graphical-session.target' '' '[Service]' 'Type=simple' 'ExecStart=$(BINDIR)/hydrogen daemon' 'Restart=on-failure' 'RestartSec=2' 'Nice=15' '' '[Install]' 'WantedBy=default.target' > $(SYSTEMD_USER_DIR)/$(SERVICE_NAME)
	systemctl --user daemon-reload
	systemctl --user enable --now $(SERVICE_NAME)

uninstall:
	-systemctl --user disable --now $(SERVICE_NAME)
	rm -f $(SYSTEMD_USER_DIR)/$(SERVICE_NAME) $(BINDIR)/hydrogen
	systemctl --user daemon-reload

clean:
	cargo clean

help:
	@echo "Hydrogen commands:"
	@echo "  make             Run checks, tests and development build"
	@echo "  make build       Development build"
	@echo "  make release     Optimised release build"
	@echo "  make run         Run Hydrogen daemon"
	@echo "  make test        Run tests"
	@echo "  make check       Check compilation"
	@echo "  make fmt         Format Rust code"
	@echo "  make fmt-check   Verify formatting"
	@echo "  make lint        Run Clippy"
	@echo "  make install     Install release binary"
	@echo "  make models      Install OCR models"
	@echo "  make extension-package  Build Firefox XPI and Chrome ZIP"
	@echo "  make service     Install and start user service"
	@echo "  make uninstall   Stop service and remove installed binary"
	@echo "  make clean       Clean build artefacts"
