# Build ReelDrive and assemble the portable app.
#
#   make build                       -> leaves it in src-tauri/target/release
#   make build DEST=/Volumes/STICK   -> and copies it onto the stick
#
# The build always finishes the app where it is made — binary plus the static
# ffmpeg and ffprobe it needs — so the thing in `target/` is a complete,
# runnable product and testing means opening that. A destination is for the
# stick: copy it there, add a `media/` folder beside it, and there is nothing
# to install.

DEST ?=

# Each platform produces a different thing: a bundle on macOS, a bare .exe on
# Windows, a single AppImage on Linux.
UNAME := $(shell uname -s)
ifeq ($(UNAME),Darwin)
  ifeq ($(shell uname -m),arm64)
    FFMPEG_TARGET := macos-arm64
  else
    FFMPEG_TARGET := macos-x86_64
  endif
else ifeq ($(UNAME),Linux)
  FFMPEG_TARGET := linux-x86_64
else
  FFMPEG_TARGET := windows-x86_64
endif

BUNDLE_MACOS := src-tauri/target/release/bundle/macos/ReelDrive.app
BUNDLE_LINUX := src-tauri/target/release/bundle/appimage
RELEASE_DIR := src-tauri/target/release

.DEFAULT_GOAL := build
.PHONY: build test test-rust test-frontend check clean help

help:
	@echo "make build [DEST=<path>]  build in place, and copy to DEST if given"
	@echo "make test                 Rust and frontend tests"
	@echo "make check                tests, clippy, formatting and the frontend build"
	@echo "make clean                remove what a build leaves in DEST"

build:
	npm install
	npm run build
ifeq ($(UNAME),Darwin)
	npm run tauri build -- --bundles app
	@# Into the bundle itself, so what sits in target/ is the whole product and
	@# opening it is a real test rather than an app that cannot probe anything.
	sh scripts/fetch-ffmpeg.sh $(FFMPEG_TARGET) $(BUNDLE_MACOS)/Contents/MacOS
	@# Adding files to a bundle breaks its seal; re-sign it ad-hoc, which is
	@# what the build produced in the first place.
	codesign --force --sign - $(BUNDLE_MACOS)
	@echo "built: $(BUNDLE_MACOS)"
ifneq ($(DEST),)
	rm -rf "$(DEST)/ReelDrive.app"
	mkdir -p "$(DEST)"
	cp -R $(BUNDLE_MACOS) "$(DEST)/ReelDrive.app"
	@echo "copied: $(DEST)/ReelDrive.app"
endif
else ifeq ($(UNAME),Linux)
	npm run tauri build -- --bundles appimage
	@# An AppImage runs from a read-only mount, so the tools cannot travel
	@# inside it: they sit next to it, where $$APPIMAGE points.
	sh scripts/fetch-ffmpeg.sh $(FFMPEG_TARGET) $(BUNDLE_LINUX)
	@echo "built: $(BUNDLE_LINUX)"
ifneq ($(DEST),)
	mkdir -p "$(DEST)"
	@# Newest, not the glob: a version bump without `cargo clean` leaves two
	@# AppImages there, and `cp` with two sources onto one file just fails.
	cp "$$(ls -t $(BUNDLE_LINUX)/*.AppImage | head -1)" "$(DEST)/ReelDrive.AppImage"
	chmod +x "$(DEST)/ReelDrive.AppImage"
	sh scripts/fetch-ffmpeg.sh $(FFMPEG_TARGET) "$(DEST)"
	@echo "copied: $(DEST)/ReelDrive.AppImage"
endif
else
	npm run tauri build -- --no-bundle
	sh scripts/fetch-ffmpeg.sh $(FFMPEG_TARGET) $(RELEASE_DIR)
	@echo "built: $(RELEASE_DIR)/reeldrive.exe"
ifneq ($(DEST),)
	mkdir -p "$(DEST)"
	cp $(RELEASE_DIR)/reeldrive.exe "$(DEST)/ReelDrive.exe"
	sh scripts/fetch-ffmpeg.sh $(FFMPEG_TARGET) "$(DEST)"
	@echo "copied: $(DEST)/ReelDrive.exe"
endif
endif

test: test-rust test-frontend

test-rust:
	cd src-tauri && cargo test

test-frontend:
	npm run test

check: test
	cd src-tauri && cargo clippy --all-targets -- -D warnings
	cd src-tauri && cargo fmt --check
	npm run build

# `build` treats an unset DEST as "do not copy anywhere"; here the same emptiness
# aimed every one of these at the root of the filesystem. The justfile has always
# defaulted this to the current directory, which is where a build without a
# destination leaves its copy.
CLEAN_DEST := $(or $(DEST),.)

clean:
	rm -rf "$(CLEAN_DEST)/ReelDrive.app" "$(CLEAN_DEST)/ReelDrive.AppImage" "$(CLEAN_DEST)/ReelDrive.exe"
	rm -f "$(CLEAN_DEST)/ffmpeg" "$(CLEAN_DEST)/ffprobe" "$(CLEAN_DEST)/ffmpeg.exe" "$(CLEAN_DEST)/ffprobe.exe"
