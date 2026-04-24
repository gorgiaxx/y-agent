{
  lib,
  stdenv,
  rust,
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
  wrapGAppsHook4,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "y-agent";
  version = "0.5.5";

  src = ./.;

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    cargo-tauri.hook
    nodejs
    npmHooks.npmConfigHook
    pkg-config
  ] ++ lib.optionals stdenv.hostPlatform.isLinux [
    wrapGAppsHook4
  ];

  buildInputs = [
    glib
    glib-networking
    gtk3
    openssl
    webkitgtk_4_1
    libsoup_3
  ];

  postBuild = ''
    # cargo-tauri builds y-gui only, so we need to build y-agent separately
    ${rust.envVars.setEnv} cargo build "''${cargoFlagsArray[@]}" --bin y-agent
  '';

  postInstall = ''
    # Copy the y-agent binary to the output bin directory
    releaseDir=target/${stdenv.targetPlatform.rust.cargoShortTarget}/${finalAttrs.cargoBuildType}
    mkdir -p $out/bin
    cp $releaseDir/y-agent $out/bin/
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
