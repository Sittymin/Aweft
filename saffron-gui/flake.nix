{
  description = "Tauri development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      nixpkgs,
      rust-overlay,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs { inherit system overlays; };
      in
      with pkgs;
      {
        devShells.default = mkShell rec {
          nativeBuildInputs = [
            pkg-config
            gobject-introspection
            cargo
            cargo-tauri
            bun
          ];

          buildInputs = [
            # GTK and related libraries
            at-spi2-atk
            atkmm
            cairo
            gdk-pixbuf
            glib
            gtk3
            harfbuzz
            librsvg
            libsoup_3
            pango
            webkitgtk_4_1

            # SSL support
            openssl
          ];

          # Set up library paths
          LD_LIBRARY_PATH = "${lib.makeLibraryPath buildInputs}";

          shellHook = ''
            export XDG_DATA_DIRS="$GSETTINGS_SCHEMAS_PATH"
          '';
        };
      }
    );
}
