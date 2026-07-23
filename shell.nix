# Dev shell for the m365-tui workspace.
#
# The rustup toolchain on this machine is broken (its patched glibc interpreter
# was garbage-collected), so we use nixpkgs' rustc + cargo instead:
#
#   nix-shell            # drops you into a shell with a working toolchain
#   nix-shell --run 'cargo build'
#
{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  name = "m365-tui";

  packages = with pkgs; [
    rustc
    cargo
    rustfmt
    clippy
    pkg-config
  ];

  # reqwest is built with rustls (bundled webpki roots), so no system OpenSSL or
  # CA bundle is required. RUST_BACKTRACE helps while developing.
  RUST_BACKTRACE = "1";
}
