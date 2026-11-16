{
  lib,
  rustPlatform,
  pkg-config,
  sqlite,
}:

let
  cargoToml = lib.importTOML ../Cargo.toml;
in
rustPlatform.buildRustPackage {
  pname = "lemonup";
  inherit (cargoToml.workspace.package) version;

  # Only the files the build reads, so docs changes don't trigger a rebuild.
  src = lib.fileset.toSource {
    root = ./..;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
    ];
  };

  cargoLock.lockFile = ../Cargo.lock;

  # The nixpkgs Rust hooks turn off the bundled SQLite of rusqlite and link
  # the system one through pkg-config.
  nativeBuildInputs = [ pkg-config ];
  buildInputs = [ sqlite ];

  cargoBuildFlags = [
    "-p"
    "lemonup-app"
    "--bin"
    "lemonup"
  ];

  # Run the tests of every crate in the workspace.
  doCheck = true;
  cargoTestFlags = [ "--workspace" ];

  # Some tests look up the home directory, and the build sandbox has none.
  preCheck = ''
    export HOME="$TMPDIR"
  '';

  # The HTTP client tests start a server on 127.0.0.1.
  __darwinAllowLocalNetworking = true;

  meta = {
    description = "Terminal addon manager for World of Warcraft (Retail and WoW Forever)";
    homepage = "https://lemonup.org";
    license = lib.licenses.mit;
    mainProgram = "lemonup";
    platforms = lib.platforms.linux ++ lib.platforms.darwin;
  };
}
