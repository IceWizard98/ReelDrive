# Build ReelDrive and assemble the portable app.
#
#   just build                    -> leaves it in src-tauri/target/release
#   just build /Volumes/STICK     -> and copies it onto the stick
#
# The build always finishes the app where it is made — binary plus the static
# ffmpeg and ffprobe it needs — so the thing in `target/` is a complete,
# runnable product and testing means opening that. A destination is for the
# stick: copy it there, add a `media/` folder beside it, and there is nothing
# to install.

default: build

# Each platform produces a different thing: a bundle on macOS, a bare .exe on
# Windows, a single AppImage on Linux.
ffmpeg_target := if os() == "macos" {
    if arch() == "aarch64" { "macos-arm64" } else { "macos-x86_64" }
} else if os() == "linux" {
    "linux-x86_64"
} else {
    "windows-x86_64"
}

bundle_macos := "src-tauri/target/release/bundle/macos/ReelDrive.app"
bundle_linux := "src-tauri/target/release/bundle/appimage"
release_dir := "src-tauri/target/release"

# Build, then copy to `dest` only if one was named.
build dest="":
    npm install
    npm run build
    just _assemble
    {{ if dest == "" { "true" } else { "just _install " + quote(dest) } }}

[macos]
_assemble:
    npm run tauri build -- --bundles app
    # Into the bundle itself, so what sits in target/ is the whole product and
    # opening it is a real test rather than an app that cannot probe anything.
    sh scripts/fetch-ffmpeg.sh {{ffmpeg_target}} {{bundle_macos}}/Contents/MacOS
    # Adding files to a bundle breaks its seal; re-sign it ad-hoc, which is what
    # the build produced in the first place.
    codesign --force --sign - {{bundle_macos}}
    @echo "built: {{bundle_macos}}"

[macos]
_install dest:
    rm -rf "{{dest}}/ReelDrive.app"
    mkdir -p "{{dest}}"
    cp -R {{bundle_macos}} "{{dest}}/ReelDrive.app"
    @echo "copied: {{dest}}/ReelDrive.app"

[linux]
_assemble:
    npm run tauri build -- --bundles appimage
    # An AppImage runs from a read-only mount, so the tools cannot travel inside
    # it: they sit next to it, where $APPIMAGE points.
    sh scripts/fetch-ffmpeg.sh {{ffmpeg_target}} {{bundle_linux}}
    @echo "built: {{bundle_linux}}"

[linux]
_install dest:
    mkdir -p "{{dest}}"
    # Newest, not the glob: a version bump without `cargo clean` leaves two
    # AppImages there, and `cp` with two sources onto one file just fails.
    cp "$(ls -t {{bundle_linux}}/*.AppImage | head -1)" "{{dest}}/ReelDrive.AppImage"
    chmod +x "{{dest}}/ReelDrive.AppImage"
    sh scripts/fetch-ffmpeg.sh {{ffmpeg_target}} "{{dest}}"
    @echo "copied: {{dest}}/ReelDrive.AppImage"

[windows]
_assemble:
    npm run tauri build -- --no-bundle
    sh scripts/fetch-ffmpeg.sh {{ffmpeg_target}} {{release_dir}}
    @echo "built: {{release_dir}}/reeldrive.exe"

[windows]
_install dest:
    mkdir -p "{{dest}}"
    cp {{release_dir}}/reeldrive.exe "{{dest}}/ReelDrive.exe"
    sh scripts/fetch-ffmpeg.sh {{ffmpeg_target}} "{{dest}}"
    @echo "copied: {{dest}}/ReelDrive.exe"

# Rust and frontend tests.
test: test-rust test-frontend

test-rust:
    cd src-tauri && cargo test

test-frontend:
    npm run test

# Everything the release workflow checks, before pushing anything.
check: test
    cd src-tauri && cargo clippy --all-targets -- -D warnings
    cd src-tauri && cargo fmt --check
    npm run build

# Remove what a build leaves in `dest`.
clean dest=".":
    rm -rf "{{dest}}/ReelDrive.app" "{{dest}}/ReelDrive.AppImage" "{{dest}}/ReelDrive.exe"
    rm -f "{{dest}}/ffmpeg" "{{dest}}/ffprobe" "{{dest}}/ffmpeg.exe" "{{dest}}/ffprobe.exe"
