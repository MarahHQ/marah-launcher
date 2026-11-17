# Everyday tasks for Marah Launcher. `make` alone lists them. The scripts in scripts/ do the work, so CI
# and this file run the same commands.

.DEFAULT_GOAL := help
.PHONY: help check fmt fmt-check lint test run run-windowed run-session run-session-windowed build package \
	linux-packages appimage test-rpm deny hooks-install hooks-run clean catalog catalog-refresh

VERSION := $(shell sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
# gamescope's options for run-session: the ones packaging/linux/marah-session uses, and -f, as
# here gamescope is a window in the desktop: full screen, not a small window.
GAMESCOPE_ARGS ?= -f -e --xwayland-count 1

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  %-22s %s\n", $$1, $$2}'

check: ## Format check, clippy (warnings are errors) and every test, as CI runs them
	scripts/check.sh

fmt: ## Format the code
	cargo fmt --all

fmt-check: ## Fail when the code isn't formatted
	scripts/check.sh fmt

lint: ## The format check, and clippy on every crate and test with warnings as errors
	scripts/check.sh lint

test: ## Every test
	scripts/check.sh test

run: ## Run the launcher
	cargo run -p launcher-app

run-windowed: ## Run the launcher in a normal window
	cargo run -p launcher-app -- --windowed

run-session: ## Run the launcher as the Marah Launcher session does: in gamescope, with MARAH_SESSION=1 (GAMESCOPE_ARGS)
	@command -v gamescope > /dev/null || { echo "run-session needs gamescope; run-session-windowed needs none" >&2; exit 1; }
	cargo build -p launcher-app
	@# The X11 settings are Marah Launcher's, not gamescope's: gamescope draws its own window with GTK on
	@# Wayland, and GDK_BACKEND=x11 would stop it ("Failed to initialize GTK").
	gamescope $(GAMESCOPE_ARGS) -- env MARAH_SESSION=1 SDL_VIDEO_DRIVER=x11 SDL_VIDEODRIVER=x11 \
		QT_QPA_PLATFORM=xcb GDK_BACKEND=x11 $${CARGO_TARGET_DIR:-target}/debug/marah-launcher

run-session-windowed: ## The session's screens (MARAH_SESSION=1) in a normal window, without gamescope
	MARAH_SESSION=1 cargo run -p launcher-app -- --windowed

build: ## Build the launcher
	cargo build -p launcher-app --release

package: ## Release build into dist/marah-launcher-<os>-<arch>/
	scripts/package.sh

linux-packages: package ## The deb, rpm and Arch packages into dist/packages/, checked (Linux)
	NFPM=$${NFPM:-scripts/nfpm.sh} packaging/linux/build-packages.sh dist/marah-launcher-linux-x86_64 $(VERSION) dist/packages
	packaging/linux/check-packages.sh dist/packages

appimage: package ## dist/Marah-Launcher-x86_64.AppImage (Linux; needs LINUXDEPLOY, the libmpv action's linuxdeploy)
	scripts/appimage.sh

test-rpm: ## A quick rpm of this checkout for this PC, into dist/test-rpm/ (the rest from the latest release)
	scripts/test-rpm.sh


deny: ## Advisories, licences and sources of every dependency (needs cargo-deny)
	cargo deny check

hooks-install: ## Install the git hooks from .pre-commit-config.yaml (needs pre-commit)
	pre-commit install --hook-type pre-commit --hook-type pre-push

hooks-run: ## Run the git hooks from .pre-commit-config.yaml (needs pre-commit)
	pre-commit run --all-files

catalog: ## Read the forums again (Chrome opens) into assets/catalog/; CONSOLE=pc for one catalog, STEAM=1 adds Steam data, CONCURRENCY=4 reads faster
	scripts/catalog/update-catalogs.sh $(if $(CONSOLE),--console $(CONSOLE))

catalog-refresh: ## Update assets/catalog/ from the forums' feed and API, without a browser; CONSOLE=pc for one catalog
	scripts/catalog/update-catalogs.sh --refresh $(if $(CONSOLE),--console $(CONSOLE))

clean: ## Remove the build output and dist/ (with the catalog backups)
	cargo clean
	rm -rf dist target
