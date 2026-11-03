# editor's dev environment. The toolchain (rust 1.99.1, the rust-toolchain.toml
# pin), mold, cargo-hakari, cargo-rail, prek and kache's wrapper come from
# devenv-fleet's `rust` module, and dx with the wasm C toolchain from its
# `dioxus` module (devenv.yaml). This file adds what the native and wasm
# builds link against and run, the Playwright browsers the tests/browser
# suite drives, and the tools the Justfile calls.
{ pkgs, lib, inputs, ... }:
let
  # Playwright's browsers come from the nixpkgs-dx pin: its playwright-driver
  # matches the @playwright/test pin in tests/browser. Without it Playwright
  # falls back to a downloaded chromium that cannot load its shared libs on
  # NixOS (libnspr4.so).
  pkgsDx = import inputs.nixpkgs-dx { inherit (pkgs.stdenv) system; };

  # The libraries the linker finds, also on the loader path so a desktop test
  # binary runs from this shell. The C++ runtime too: a dynamically-linked C++
  # dep needs libstdc++ at run time on NixOS.
  libPath = lib.makeLibraryPath ([ pkgs.stdenv.cc.cc.lib pkgs.fontconfig pkgs.freetype pkgs.openssl ]
    ++ lib.optionals pkgs.stdenv.isLinux (with pkgs; [
      libGL vulkan-loader gtk3 glib
      gdk-pixbuf pango cairo atk
      libx11 libxcb libxkbcommon wayland
      webkitgtk_4_1 libsoup_3 xdotool
    ]));
in
{
  packages = with pkgs; [
    # The Justfile's nextest run (`just rust-test`) and CI's.
    cargo-nextest
    cargo-watch
    bacon
    # Node + pnpm: the Playwright suite under tests/browser (`just test`).
    nodejs_22
    pnpm
    # git — cargo shells out to it for every git dep.
    git
    # Native headers and build tools for the cargo builds. cmake drives
    # aws-lc-sys' C build; python3 is stylo's build.rs on the Blitz path.
    openssl
    openssl.dev
    libiconv
    fontconfig
    freetype
    cmake
    python3
    # bindgen's clang paths (clang-sys: libsqlite3-sys, aws-lc-sys, webkit2gtk).
    rustPlatform.bindgenHook
  ]
  ++ lib.optionals pkgs.stdenv.isLinux (with pkgs; [
    # The wry/tao webview stack behind `dx serve --platform desktop` and the
    # playground's desktop build: each is a pkg-config lookup at build time.
    glib gtk3 gdk-pixbuf pango cairo atk harfbuzz
    libsoup_3 webkitgtk_4_1 xdotool
    # tao/winit windowing and wgpu/vello rendering.
    libx11 libxcursor libxrandr libxi libxcb
    libxkbcommon wayland libGL vulkan-loader
    # cargo-sweep — reclaims stale target/ artifacts; cargo never GCs.
    cargo-sweep
    # dx's AppImage bundler (`dx bundle --package-types appimage`) shells out
    # to it. From the nixpkgs-dx set: the main pin doesn't carry it.
    pkgsDx.linuxdeploy
  ]);

  env = {
    LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
    OPENSSL_DIR = "${pkgs.openssl.dev}";
    OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
    # Playwright must find the browsers the nixpkgs-dx pin ships, in lockstep
    # with tests/browser's @playwright/test version.
    PLAYWRIGHT_BROWSERS_PATH = "${pkgsDx.playwright-driver.browsers}";
    PLAYWRIGHT_SKIP_VALIDATE_HOST_REQUIREMENTS = "true";
  }
  // lib.optionalAttrs pkgs.stdenv.isLinux {
    LD_LIBRARY_PATH = libPath;
    XDG_DATA_DIRS = "${pkgs.gsettings-desktop-schemas}/share/gsettings-schemas/${pkgs.gsettings-desktop-schemas.name}:${pkgs.gtk3}/share/gsettings-schemas/${pkgs.gtk3.name}";
    # WebKitGTK's accelerated compositing fails on NixOS (GBM buffer error,
    # white window). https://github.com/NixOS/nixpkgs/issues/32580
    WEBKIT_DISABLE_COMPOSITING_MODE = "1";
  };

  enterShell = ''
    # Machine-local values (.env, not committed) go into the shell's environment.
    if [ -f "$DEVENV_ROOT/.env" ]; then set -a; . "$DEVENV_ROOT/.env"; set +a; fi

    # Appended, not prepended: store-provided tools must win over stale
    # cargo-installed copies; cargo bins only need to be reachable.
    export PATH="$PATH:$HOME/.cargo/bin"
  '';
}
