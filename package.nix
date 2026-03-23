{
  lib,
  stdenv,
  rustPlatform,
  pkg-config,
  glib,
  gtk3,
  libsoup_3,
  cargo-tauri,
  fetchNpmDeps,
  npmHooks,
  glib-networking,
  openssl,
  webkitgtk_4_1,
  nodejs,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "y-agent";
  version = "0.1.2";

  src = ./.;

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    # cargo-tauri.hook
    nodejs
    npmHooks.npmConfigHook
    pkg-config
  ];

  buildInputs = [
    glib
    glib-networking
    gtk3
    openssl
    webkitgtk_4_1
    libsoup_3
  ];

  preBuild = ''
    # Run vite build for the y-gui crate
    pushd "${finalAttrs.npmRoot}"
    npm run build
    popd
  '';

  # Skip flaky test that times out in nix sandbox
  checkFlags = [
    "--skip=hook_handler::tests::handler_tests::test_command_hook_timeout_killed"
  ];


  npmRoot = "./crates/y-gui";
  npmDeps = fetchNpmDeps {
    inherit (finalAttrs) pname version;
    src = ./crates/y-gui;
    hash = "sha256-SYsSQopzDyvbdv1d4rkn2WwV2SIAD8BPv9zN5wf0Wgo=";
  };

  npmFlags = [ "--legacy-peer-deps" ];
})
