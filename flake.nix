# Nix flake, Linux only (x86_64 and aarch64): `nix run` (the game; it brings no game data: the setup screen converts
# the player's own disc, or `--assets DIR` names converted data), `nix build` (result/bin/okbeast, with a desktop entry
# and icon), `nix develop` (the toolchain and libraries for the checks in CONTRIBUTING.md and the web build).
# macOS, Windows, the web and Android use the release packages.
{
  description = "OpenKumiteBeast, a 3D fighting game engine in Rust + wgpu that plays data converted from the player's own disc";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      lib = nixpkgs.lib;
      forAll = f: lib.genAttrs [ "x86_64-linux" "aarch64-linux" ] (system: f nixpkgs.legacyPackages.${system});
      # what winit and wgpu open at start (dlopen): the GPU's loaders, Wayland, X11
      runtimeLibs = pkgs: with pkgs; [ vulkan-loader libGL wayland libxkbcommon libX11 libXcursor libXi libXrandr libxcb ];
      # trunk runs the wasm-bindgen whose version is the one in Cargo.lock; the prebuilt one it would download does
      # not start on NixOS, so the shell carries nixpkgs' build of that version (a wasm-bindgen bump nixpkgs does not
      # carry yet needs `nix flake update`); without that version the shell still opens (the native checks need no
      # wasm-bindgen) with nixpkgs' default one and a warning, and only the web build may then fail
      wasmBindgenVersion = (lib.findFirst (p: p.name == "wasm-bindgen") { version = "none"; }
        (lib.importTOML ./Cargo.lock).package).version;
      wasmBindgenCli = pkgs:
        let attr = "wasm-bindgen-cli_${lib.replaceStrings [ "." ] [ "_" ] wasmBindgenVersion}";
        in pkgs.${attr} or (lib.warn "nixpkgs has no ${attr} (Cargo.lock's wasm-bindgen ${wasmBindgenVersion}); the shell carries wasm-bindgen-cli ${pkgs.wasm-bindgen-cli.version}, so trunk build may refuse it" pkgs.wasm-bindgen-cli);
      okbeast = pkgs: pkgs.rustPlatform.buildRustPackage {
        pname = "okbeast";
        version = (lib.importTOML ./Cargo.toml).package.version;
        # a `path:` flake or a non-git checkout would otherwise copy the build outputs (target/ etc.) into the store
        src = lib.cleanSourceWith {
          src = lib.cleanSource ./.;
          filter = path: type:
            !(builtins.elem (baseNameOf path) [ "target" "dist" "result" ] && dirOf path == toString ./.);
        };
        cargoLock = {
          lockFile = ./Cargo.lock;
          # the converters (ok_br2_converter, ok_br1_converter, ok_q3_converter) come from GitHub at the commits
          # Cargo.lock pins, so a converter bump needs no hash here
          allowBuiltinFetchGit = true;
        };
        cargoBuildFlags = [ "--bin" "okbeast" ];
        nativeBuildInputs = with pkgs; [ pkg-config makeWrapper copyDesktopItems ];
        # what the binary links: ALSA (sound), libudev (gamepads)
        buildInputs = with pkgs; [ alsa-lib udev ];
        # cargo-husky (a dev-dependency) would install git hooks; the tests' config folder is a scratch one
        CARGO_HUSKY_DONT_INSTALL_HOOKS = "1";
        preCheck = ''
          export HOME=$TMPDIR OKB_SAVE_DIR=$TMPDIR/save
        '';
        desktopItems = [
          (pkgs.makeDesktopItem {
            name = "okbeast";
            desktopName = "OpenKumiteBeast";
            comment = "3D fighting game engine; plays data converted from your own disc";
            exec = "okbeast";
            icon = "okbeast";
            categories = [ "Game" "ActionGame" ];
          })
        ];
        postInstall = ''
          install -Dm644 web/icons/icon.svg $out/share/icons/hicolor/scalable/apps/okbeast.svg
        '';
        postFixup = ''
          wrapProgram $out/bin/okbeast --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath (runtimeLibs pkgs)}
        '';
        meta = {
          description = "3D fighting game engine in Rust + wgpu; plays data converted from the player's own disc";
          homepage = "https://github.com/openkumite/open_kumite_beast";
          license = lib.licenses.mit;
          platforms = [ "x86_64-linux" "aarch64-linux" ];
          mainProgram = "okbeast";
        };
      };
    in {
      packages = forAll (pkgs: { default = okbeast pkgs; });

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ (okbeast pkgs) ];
          # the checks (fmt, clippy, cargo-deny, cargo-audit) and the web build (trunk, wasm-bindgen, binaryen's
          # wasm-opt; nixpkgs' rustc carries the wasm32 std and calls `lld` to link it)
          packages = with pkgs; [ clippy rustfmt cargo-deny cargo-audit trunk (wasmBindgenCli pkgs) binaryen lld ];
          LD_LIBRARY_PATH = lib.makeLibraryPath (runtimeLibs pkgs);
        };
      });
    };
}
